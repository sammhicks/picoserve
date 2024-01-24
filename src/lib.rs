#![cfg_attr(not(feature = "std"), no_std)]
#![allow(async_fn_in_trait)]

//! An async `no_std` HTTP server suitable for bare-metal environments, heavily inspired by [axum](https://github.com/tokio-rs/axum).
//!
//! It was designed with [embassy](https://embassy.dev/) on the Raspberry Pi Pico W in mind, but should work with other embedded runtimes and hardware.
//!
//! For examples on how to use picoserve, see the [examples](https://github.com/sammhicks/picoserve/tree/main/examples) directory

pub mod extract;
pub mod io;
pub mod request;
pub mod response;
pub mod routing;
pub mod time;
pub mod url_encoded;

// TODO - Replace with dependency when const_sha1 has published `no_std` support
mod const_sha1;

pub use routing::Router;
pub use time::Timer;

use time::TimerExt;

/// A Marker showing that the response has been sent.
pub struct ResponseSent(());

/// Errors arising while handling a request.
#[derive(Debug)]
pub enum Error<E: embedded_io_async::Error> {
    ReadTimeout,
    Read(request::ReadError<E>),
    Write(E),
    WriteTimeout,
}

impl<E: embedded_io_async::Error> embedded_io_async::Error for Error<E> {
    fn kind(&self) -> embedded_io_async::ErrorKind {
        match self {
            Error::ReadTimeout | Error::WriteTimeout => embedded_io_async::ErrorKind::TimedOut,
            Error::Read(request::ReadError::BadRequestLine)
            | Error::Read(request::ReadError::UnexpectedEof) => {
                embedded_io_async::ErrorKind::InvalidData
            }
            Error::Read(request::ReadError::Other(err)) | Error::Write(err) => err.kind(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Timeouts<D> {
    pub start_read_request: Option<D>,
    pub read_request: Option<D>,
    pub write: Option<D>,
}

#[derive(Debug, Clone, Copy)]
/// After the response has been sent, should the connection be kept open to allow the client to make further requests on the same TCP connection?
pub enum KeepAlive {
    /// Close the connection after the response has been sent, i.e. each TCP connection serves a single request.
    Close,
    /// Keep the connection alive after the response has been sent, allowing the client to make further requests on the same TCP connection.
    KeepAlive,
}

impl core::fmt::Display for KeepAlive {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            KeepAlive::Close => "close",
            KeepAlive::KeepAlive => "keep-alive",
        }
        .fmt(f)
    }
}

impl KeepAlive {
    fn default_for_http_version(http_version: &str) -> Self {
        if http_version == "HTTP/1.1" {
            Self::KeepAlive
        } else {
            Self::Close
        }
    }

    fn from_request(http_version: &str, headers: request::Headers) -> Self {
        match headers.get("connection") {
            None => Self::default_for_http_version(http_version),
            Some(close_header) if close_header.eq_ignore_ascii_case("close") => Self::Close,
            Some(connection_headers) => {
                if connection_headers
                    .split(',')
                    .map(str::trim)
                    .any(|connection_header| connection_header.eq_ignore_ascii_case("upgrade"))
                {
                    Self::Close
                } else {
                    Self::default_for_http_version(http_version)
                }
            }
        }
    }
}

/// Server Configuration.
#[derive(Debug, Clone)]
pub struct Config<D> {
    pub timeouts: Timeouts<D>,
    pub connection: KeepAlive,
}

impl<D> Config<D> {
    /// Create a new configuration, setting the timeouts.
    /// All other configuration is set to the defaults.
    pub const fn new(timeouts: Timeouts<D>) -> Self {
        Self {
            timeouts,
            connection: KeepAlive::Close,
        }
    }

    /// Keep the connection alive after the response has been sent, allowing the client to make further requests on the same TCP connection.
    /// This should only be called if multiple sockets are handling HTTP connections to avoid a single client hogging the connection
    /// and preventing other clients from making requests.
    pub fn keep_connection_alive(mut self) -> Self {
        self.connection = KeepAlive::KeepAlive;

        self
    }

    /// Close the connection after the response has been sent, i.e. each TCP connection serves a single request.
    /// This is the default, but allows the configuration to be more explicit.
    pub fn close_connection_after_response(mut self) -> Self {
        self.connection = KeepAlive::Close;

        self
    }
}

/// Maps Read errors to [Error]s
struct MapReadErrorReader<R: embedded_io_async::Read>(R);

impl<R: embedded_io_async::Read> embedded_io_async::ErrorType for MapReadErrorReader<R> {
    type Error = Error<R::Error>;
}

impl<R: embedded_io_async::Read> embedded_io_async::Read for MapReadErrorReader<R> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0
            .read(buf)
            .await
            .map_err(|err| Error::Read(request::ReadError::Other(err)))
    }

    async fn read_exact(
        &mut self,
        buf: &mut [u8],
    ) -> Result<(), embedded_io_async::ReadExactError<Self::Error>> {
        self.0.read_exact(buf).await.map_err(|err| match err {
            embedded_io_async::ReadExactError::UnexpectedEof => {
                embedded_io_async::ReadExactError::UnexpectedEof
            }
            embedded_io_async::ReadExactError::Other(err) => {
                embedded_io_async::ReadExactError::Other(Error::Read(request::ReadError::Other(
                    err,
                )))
            }
        })
    }
}

async fn do_serve<
    State,
    T: Timer,
    P: routing::PathRouter<State>,
    R: io::Read,
    W: io::Write<Error = R::Error>,
>(
    Router { router, .. }: &Router<P, State>,
    mut timer: T,
    config: &Config<T::Duration>,
    buffer: &mut [u8],
    reader: R,
    mut writer: W,
    state: &State,
) -> Result<u64, Error<W::Error>> {
    let mut reader = MapReadErrorReader(reader);

    for request_count in 0.. {
        let mut reader = match timer
            .run_with_maybe_timeout(
                config.timeouts.start_read_request.clone(),
                request::Reader::new(&mut reader, buffer),
            )
            .await
        {
            Ok(Ok(Some(reader))) => reader,
            Ok(Ok(None)) | Err(_) => return Ok(request_count),
            Ok(Err(err)) => return Err(err),
        };

        match timer
            .run_with_maybe_timeout(config.timeouts.read_request.clone(), reader.read())
            .await
        {
            Ok(Ok((request, connection))) => {
                let connection_header = match config.connection {
                    KeepAlive::Close => KeepAlive::Close,
                    KeepAlive::KeepAlive => {
                        KeepAlive::from_request(request.http_version(), request.headers())
                    }
                };

                let mut writer = time::WriteWithTimeout {
                    inner: &mut writer,
                    timer: &mut timer,
                    timeout_duration: config.timeouts.write.clone(),
                };

                router
                    .call_path_router(
                        state,
                        routing::NoPathParameters,
                        request.path(),
                        request,
                        response::ResponseStream::new(connection, &mut writer, connection_header),
                    )
                    .await?;

                if let KeepAlive::Close = connection_header {
                    return Ok(request_count + 1);
                }
            }
            Ok(Err(err)) => {
                return Err(match err {
                    request::ReadError::BadRequestLine => {
                        Error::Read(request::ReadError::BadRequestLine)
                    }
                    request::ReadError::UnexpectedEof => {
                        Error::Read(request::ReadError::UnexpectedEof)
                    }
                    request::ReadError::Other(err) => err,
                })
            }
            Err(..) => return Err(Error::ReadTimeout),
        }
    }

    Ok(0)
}

#[cfg(feature = "tokio")]
/// Serve incoming requests read from `reader`, route them to `app`, and write responses to `writer`. App has no state.
pub async fn serve<
    P: routing::PathRouter,
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
>(
    app: &Router<P>,
    config: &Config<std::time::Duration>,
    buffer: &mut [u8],
    reader: R,
    writer: W,
) -> Result<u64, Error<io::tokio_support::TokioIoError>> {
    do_serve(
        app,
        time::TokioTimer,
        config,
        buffer,
        io::tokio_support::TokioIo(reader),
        io::tokio_support::TokioIo(writer),
        &(),
    )
    .await
}

#[cfg(feature = "tokio")]
/// Serve incoming requests read from `reader`, route them to `app`, and write responses to `writer`. App has a state of `State`.
pub async fn serve_with_state<
    State,
    P: routing::PathRouter<State>,
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
>(
    app: &Router<P, State>,
    config: &Config<std::time::Duration>,
    buffer: &mut [u8],
    reader: R,
    writer: W,
    state: &State,
) -> Result<u64, Error<io::tokio_support::TokioIoError>> {
    do_serve(
        app,
        time::TokioTimer,
        config,
        buffer,
        io::tokio_support::TokioIo(reader),
        io::tokio_support::TokioIo(writer),
        state,
    )
    .await
}

#[cfg(not(feature = "tokio"))]
/// Serve incoming requests read from `reader`, route them to `app`, and write responses to `writer`. App has no state.
pub async fn serve<
    T: Timer,
    P: routing::PathRouter,
    R: io::Read,
    W: io::Write<Error = R::Error>,
>(
    app: &Router<P>,
    timer: T,
    config: &Config<T::Duration>,
    buffer: &mut [u8],
    reader: R,
    writer: W,
) -> Result<u64, Error<W::Error>> {
    do_serve(app, timer, config, buffer, reader, writer, &()).await
}

#[cfg(not(feature = "tokio"))]
/// Serve incoming requests read from `reader`, route them to `app`, and write responses to `writer`. App has a state of `State`.
pub async fn serve_with_state<
    State,
    T: Timer,
    P: routing::PathRouter<State>,
    R: io::Read,
    W: io::Write<Error = R::Error>,
>(
    app: &Router<P, State>,
    timer: T,
    config: &Config<T::Duration>,
    buffer: &mut [u8],
    reader: R,
    writer: W,
    state: &State,
) -> Result<u64, Error<W::Error>> {
    do_serve(app, timer, config, buffer, reader, writer, state).await
}
