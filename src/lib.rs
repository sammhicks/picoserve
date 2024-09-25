#![cfg_attr(not(any(feature = "std", test)), no_std)]
#![allow(async_fn_in_trait)]

//! An async `no_std` HTTP server suitable for bare-metal environments, heavily inspired by [axum](https://github.com/tokio-rs/axum).
//!
//! It was designed with [embassy](https://embassy.dev/) on the Raspberry Pi Pico W in mind, but should work with other embedded runtimes and hardware.
//!
//! For examples on how to use picoserve, see the [examples](https://github.com/sammhicks/picoserve/tree/main/examples) directory

#[cfg(all(feature = "tokio", feature = "embassy"))]
compile_error!("You cannot enable both tokio and embassy support");

#[cfg(feature = "alloc")]
extern crate alloc;

mod json;

#[macro_use]
mod logging;

pub mod extract;
pub mod io;
pub mod request;
pub mod response;
pub mod routing;
pub mod time;
pub mod url_encoded;

#[cfg(test)]
mod tests;

pub use logging::LogDisplay;
pub use routing::Router;
pub use time::Timer;

use time::TimerExt;

/// A Marker showing that the response has been sent.
pub struct ResponseSent(());

/// Errors arising while handling a request.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error<E: embedded_io_async::Error> {
    /// Error while reading from the socket.
    Read(E),
    /// Timeout while reading from the socket.
    ReadTimeout,
    /// Error while writing to the socket.
    Write(E),
    /// Timeout while writing to the socket.
    WriteTimeout,
}

impl<E: embedded_io_async::Error> embedded_io_async::Error for Error<E> {
    fn kind(&self) -> embedded_io_async::ErrorKind {
        match self {
            Error::ReadTimeout | Error::WriteTimeout => embedded_io_async::ErrorKind::TimedOut,
            Error::Read(err) | Error::Write(err) => err.kind(),
        }
    }
}

/// How long to wait before timing out for different operations.
/// If set to None, the operation never times out.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Timeouts<D> {
    /// The duration of time to wait when starting to read a new request before the connection is closed due to inactivity.
    pub start_read_request: Option<D>,
    /// The duration of time to wait when partway reading a request before the connection is aborted and closed.
    pub read_request: Option<D>,
    /// The duration of time to wait when writing the response before the connection is aborted and closed.
    pub write: Option<D>,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
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
            Some(close_header) if close_header == "close" => Self::Close,
            Some(connection_headers) => {
                if connection_headers
                    .split(b',')
                    .any(|connection_header| connection_header == "upgrade")
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
    /// The timeout information
    pub timeouts: Timeouts<D>,
    /// Whether to close the connection after handling a request or keeping it open to allow further requests on the same connection.
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
    pub const fn keep_connection_alive(mut self) -> Self {
        self.connection = KeepAlive::KeepAlive;

        self
    }

    /// Close the connection after the response has been sent, i.e. each TCP connection serves a single request.
    /// This is the default, but allows the configuration to be more explicit.
    pub const fn close_connection_after_response(mut self) -> Self {
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
        self.0.read(buf).await.map_err(Error::Read)
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
                embedded_io_async::ReadExactError::Other(Error::Read(err))
            }
        })
    }
}

async fn serve_and_shutdown<State, T: Timer, P: routing::PathRouter<State>, S: io::Socket>(
    Router { router, .. }: &Router<P, State>,
    mut timer: T,
    config: &Config<T::Duration>,
    buffer: &mut [u8],
    mut socket: S,
    state: &State,
) -> Result<u64, Error<S::Error>> {
    let result = async {
        let (reader, mut writer) = socket.split();

        let mut reader = request::Reader::new(MapReadErrorReader(reader), buffer);

        for request_count in 0.. {
            match timer
                .run_with_maybe_timeout(
                    config.timeouts.start_read_request.clone(),
                    reader.request_is_pending(),
                )
                .await
            {
                Ok(Ok(true)) => (),
                Ok(Ok(false)) | Err(_) => return Ok(request_count),
                Ok(Err(err)) => return Err(err),
            };

            match timer
                .run_with_maybe_timeout(config.timeouts.read_request.clone(), reader.read())
                .await
            {
                Ok(Ok(request)) => {
                    let connection_header = match config.connection {
                        KeepAlive::Close => KeepAlive::Close,
                        KeepAlive::KeepAlive => KeepAlive::from_request(
                            request.parts.http_version(),
                            request.parts.headers(),
                        ),
                    };

                    let mut writer = time::WriteWithTimeout {
                        inner: &mut writer,
                        timer: &mut timer,
                        timeout_duration: config.timeouts.write.clone(),
                    };

                    let ResponseSent(()) = router
                        .call_path_router(
                            state,
                            routing::NoPathParameters,
                            request.parts.path(),
                            request,
                            response::ResponseStream::new(&mut writer, connection_header),
                        )
                        .await?;

                    if let KeepAlive::Close = connection_header {
                        return Ok(request_count + 1);
                    }
                }
                Ok(Err(err)) => {
                    use response::IntoResponse;

                    let message = match err {
                        request::ReadError::BadRequestLine => "Bad Request Line",
                        request::ReadError::HeaderDoesNotContainColon => {
                            "Invalid Header line: No ':' character"
                        }
                        request::ReadError::UnexpectedEof => "Unexpected EOF while reading request",
                        request::ReadError::IO(err) => return Err(err),
                    };

                    let ResponseSent(()) = timer
                        .run_with_maybe_timeout(
                            config.timeouts.write.clone(),
                            (response::StatusCode::BAD_REQUEST, message).write_to(
                                response::Connection::empty(&mut false),
                                response::ResponseStream::new(writer, KeepAlive::Close),
                            ),
                        )
                        .await
                        .map_err(|_| Error::WriteTimeout)?
                        .map_err(Error::Write)?;

                    return Ok(request_count + 1);
                }
                Err(..) => return Err(Error::ReadTimeout),
            }
        }

        Ok(0)
    }
    .await;

    let shutdown_result = socket.shutdown(&config.timeouts, &mut timer).await;

    let request_count = result?;

    shutdown_result?;

    Ok(request_count)
}

#[cfg(any(feature = "tokio", test))]
/// Serve `app` with incoming requests. App has a no state.
pub async fn serve<P: routing::PathRouter>(
    app: &Router<P>,
    config: &Config<std::time::Duration>,
    buffer: &mut [u8],
    stream: tokio::net::TcpStream,
) -> Result<u64, Error<io::tokio_support::TokioIoError>> {
    serve_and_shutdown(app, time::TokioTimer, config, buffer, stream, &()).await
}

#[cfg(any(feature = "tokio", test))]
/// Serve incoming requests read from `reader`, route them to `app`, and write responses to `writer`. App has a state of `State`.
pub async fn serve_with_state<State, P: routing::PathRouter<State>>(
    app: &Router<P, State>,
    config: &Config<std::time::Duration>,
    buffer: &mut [u8],
    stream: tokio::net::TcpStream,
    state: &State,
) -> Result<u64, Error<io::tokio_support::TokioIoError>> {
    serve_and_shutdown(app, time::TokioTimer, config, buffer, stream, state).await
}

#[cfg(feature = "embassy")]
/// Serve `app` with incoming requests. App has a no state.
pub async fn serve<P: routing::PathRouter>(
    app: &Router<P>,
    config: &Config<embassy_time::Duration>,
    buffer: &mut [u8],
    socket: embassy_net::tcp::TcpSocket<'_>,
) -> Result<u64, Error<embassy_net::tcp::Error>> {
    serve_and_shutdown(app, time::EmbassyTimer, config, buffer, socket, &()).await
}

#[cfg(feature = "embassy")]
/// Serve `app` with incoming requests. App has a state of `State`.
pub async fn serve_with_state<State, P: routing::PathRouter<State>>(
    app: &Router<P, State>,
    config: &Config<embassy_time::Duration>,
    buffer: &mut [u8],
    socket: embassy_net::tcp::TcpSocket<'_>,
    state: &State,
) -> Result<u64, Error<embassy_net::tcp::Error>> {
    serve_and_shutdown(app, time::EmbassyTimer, config, buffer, socket, state).await
}

#[cfg(feature = "embassy")]
/// Serve `app` with incoming requests. App has a no state.
/// `task_id` is printed in log messages.
pub async fn listen_and_serve<P: routing::PathRouter<()>>(
    task_id: impl LogDisplay,
    app: &Router<P, ()>,
    config: &Config<embassy_time::Duration>,
    stack: embassy_net::Stack<'_>,
    port: u16,
    tcp_rx_buffer: &mut [u8],
    tcp_tx_buffer: &mut [u8],
    http_buffer: &mut [u8],
) -> ! {
    listen_and_serve_with_state(
        task_id,
        app,
        config,
        stack,
        port,
        tcp_rx_buffer,
        tcp_tx_buffer,
        http_buffer,
        &(),
    )
    .await
}

#[cfg(feature = "embassy")]
/// Serve `app` with incoming requests. App has a state of `State`.
/// `task_id` is printed in log messages.
pub async fn listen_and_serve_with_state<State, P: routing::PathRouter<State>>(
    task_id: impl LogDisplay,
    app: &Router<P, State>,
    config: &Config<embassy_time::Duration>,
    stack: embassy_net::Stack<'_>,
    port: u16,
    tcp_rx_buffer: &mut [u8],
    tcp_tx_buffer: &mut [u8],
    http_buffer: &mut [u8],
    state: &State,
) -> ! {
    loop {
        let mut socket = embassy_net::tcp::TcpSocket::new(stack, tcp_rx_buffer, tcp_tx_buffer);

        log_info!("{}: Listening on TCP:{}...", task_id, port);

        if let Err(err) = socket.accept(port).await {
            log_warn!("{}: accept error: {:?}", task_id, err);
            continue;
        }

        let remote_endpoint = socket.remote_endpoint();

        log_info!(
            "{}: Received connection from {:?}",
            task_id,
            remote_endpoint
        );

        match serve_with_state(app, config, http_buffer, socket, state).await {
            Ok(handled_requests_count) => {
                log_info!(
                    "{} requests handled from {:?}",
                    handled_requests_count,
                    remote_endpoint
                );
            }
            Err(err) => log_error!("{}", crate::logging::Debug2Format(&err)),
        }
    }
}

#[cfg(not(any(feature = "tokio", feature = "embassy", test)))]
/// Serve `app` with incoming requests. App has no state.
pub async fn serve<T: Timer, P: routing::PathRouter, S: io::Socket>(
    app: &Router<P>,
    timer: T,
    config: &Config<T::Duration>,
    buffer: &mut [u8],
    socket: S,
) -> Result<u64, Error<S::Error>> {
    serve_and_shutdown(app, timer, config, buffer, socket, &()).await
}

#[cfg(not(any(feature = "tokio", feature = "embassy", test)))]
/// Serve `app` with incoming requests. App has a state of `State`.
pub async fn serve_with_state<'r, State, T: Timer, P: routing::PathRouter<State>, S: io::Socket>(
    app: &Router<P, State>,
    timer: T,
    config: &Config<T::Duration>,
    buffer: &'r mut [u8],
    socket: S,
    state: &State,
) -> Result<u64, Error<S::Error>> {
    serve_and_shutdown(app, timer, config, buffer, socket, state).await
}
