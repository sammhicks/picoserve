#![cfg_attr(not(feature = "std"), no_std)]
#![feature(async_fn_in_trait, type_alias_impl_trait)]

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

/// Server Configuration.
#[derive(Debug, Clone)]
pub struct Config<D> {
    pub start_read_request_timeout: Option<D>,
    pub read_request_timeout: Option<D>,
    pub write_timeout: Option<D>,
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
                config.start_read_request_timeout.clone(),
                request::Reader::new(&mut reader, buffer),
            )
            .await
        {
            Ok(Ok(Some(reader))) => reader,
            Ok(Ok(None)) | Err(_) => return Ok(request_count),
            Ok(Err(err)) => return Err(err),
        };

        match timer
            .run_with_maybe_timeout(config.read_request_timeout.clone(), reader.read())
            .await
        {
            Ok(Ok((request, connection))) => {
                let mut writer = time::WriteWithTimeout {
                    inner: &mut writer,
                    timer: &mut timer,
                    timeout_duration: config.write_timeout.clone(),
                };

                router
                    .call_path_router(
                        state,
                        routing::NoPathParameters,
                        request.path(),
                        request,
                        response::ResponseStream::new(connection, &mut writer),
                    )
                    .await?;
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
