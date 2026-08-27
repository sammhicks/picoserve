#![no_std]
#![allow(
    async_fn_in_trait,
    reason = "`picoserve` is single-threaded, so it's OK that trait async functions return non-Send futures "
)]
#![deny(
    unsafe_code,
    clippy::allow_attributes_without_reason,
    clippy::let_underscore_must_use,
    clippy::let_underscore_untyped,
    clippy::missing_safety_doc,
    clippy::multiple_unsafe_ops_per_block,
    clippy::undocumented_unsafe_blocks
)]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! An async `no_std` HTTP server suitable for bare-metal environments, heavily inspired by [axum](https://github.com/tokio-rs/axum).
//!
//! It was designed with [embassy](https://embassy.dev/) on the Raspberry Pi Pico W in mind, but should work with other embedded runtimes and hardware.
//!
//! For examples on how to use picoserve, see the [examples](https://github.com/sammhicks/picoserve/tree/main/examples) directory.

#[cfg(any(feature = "alloc", test))]
extern crate alloc;

#[cfg(any(feature = "std", test))]
extern crate std;

use core::{future::Future, marker::PhantomData};

use futures_util::{FutureExt, TryFutureExt};

#[cfg(feature = "json")]
mod json;

#[macro_use]
mod logging;

pub mod extract;
pub mod futures;
pub mod io;
pub mod mem;
pub mod request;
pub mod response;
pub mod routing;
mod sync;
pub mod time;
pub mod url_encoded;

#[cfg(test)]
mod tests;

#[doc(hidden)]
pub mod doctests_utils;

pub use logging::LogDisplay;
pub use response::response_stream::ResponseSent;
pub use routing::Router;
pub use time::Timer;

use {sync::oneshot_broadcast, time::Duration};

/// Errors arising while handling a request.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error<E: io::Error> {
    /// Bad Request from the client
    #[error("Bad Request")]
    BadRequest,
    /// Error while reading from the socket.
    #[error("Read error: {0}")]
    Read(#[source] E),
    /// Timeout while reading from the socket.
    #[error("Read timeout")]
    ReadTimeout(time::TimeoutError),
    /// Error while writing to the socket.
    #[error("Write error: {0}")]
    Write(#[source] E),
    /// Timeout while writing to the socket.
    #[error("Write timeout")]
    WriteTimeout(time::TimeoutError),
}

impl<E: io::Error + 'static> io::Error for Error<E> {
    fn kind(&self) -> io::ErrorKind {
        match self {
            Self::BadRequest => io::ErrorKind::InvalidData,
            Self::ReadTimeout(error) | Self::WriteTimeout(error) => error.kind(),
            Self::Read(error) | Self::Write(error) => error.kind(),
        }
    }
}

trait SwapErrors {
    type Output;

    fn swap_errors(self) -> Self::Output;
}

impl<T, E0, E1> SwapErrors for Result<Result<T, E0>, E1> {
    type Output = Result<Result<T, E1>, E0>;

    fn swap_errors(self) -> Self::Output {
        match self {
            Ok(Ok(value)) => Ok(Ok(value)),
            Ok(Err(error)) => Err(error),
            Err(error) => Ok(Err(error)),
        }
    }
}

/// How long to wait before timing out for different operations.
/// If set to None, the operation never times out.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Timeouts {
    /// The duration of time to wait when starting to read the first request before the connection is closed due to inactivity.
    pub start_read_request: Duration,
    /// The duration of time to wait when starting to read persistent (i.e. not the first) requests before the connection is closed due to inactivity.
    pub persistent_start_read_request: Duration,
    /// The duration of time to wait when partway reading a request before the connection is aborted and closed.
    pub read_request: Duration,
    /// The duration of time to wait when writing the response before the connection is aborted and closed.
    pub write: Duration,
}

impl Timeouts {
    pub const fn const_default() -> Self {
        Self {
            start_read_request: Duration::from_secs(5),
            persistent_start_read_request: Duration::from_secs(1),
            read_request: Duration::from_secs(3),
            write: Duration::from_secs(1),
        }
    }
}

impl Default for Timeouts {
    fn default() -> Self {
        Self::const_default()
    }
}

/// After the response has been sent, should the connection be kept open to allow the client to make further requests on the same TCP connection?
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum KeepAlive {
    /// Close the connection after the response has been sent, i.e. each TCP connection serves a single request.
    Close,
    /// Keep the connection alive after the response has been sent, allowing the client to make further requests on the same TCP connection.
    KeepAlive,
}

impl KeepAlive {
    pub const fn const_default() -> Self {
        Self::Close
    }
}

impl Default for KeepAlive {
    fn default() -> Self {
        Self::const_default()
    }
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
pub struct Config {
    /// The timeout information
    pub timeouts: Timeouts,
    /// Whether to close the connection after handling a request or keeping it open to allow further requests on the same connection.
    pub connection: KeepAlive,
}

impl Config {
    /// Create a new configuration, setting the timeouts.
    /// All other configuration is set to the defaults.
    pub const fn new(timeouts: Timeouts) -> Self {
        Self {
            timeouts,
            connection: KeepAlive::Close,
        }
    }

    pub const fn const_default() -> Self {
        Self {
            timeouts: Timeouts::const_default(),
            connection: KeepAlive::const_default(),
        }
    }

    /// Keep the connection alive after the response has been sent, allowing the client to make further requests on the same TCP connection.
    /// This should only be called if multiple sockets are handling HTTP connections to avoid a single client hogging the connection
    /// and preventing other clients from making requests.
    ///
    /// If the request handler doesn't read the entire request body or upgrade the connection, the connection with be closed.
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

impl Default for Config {
    fn default() -> Self {
        Self::const_default()
    }
}

/// Maps Read errors to [`Error`]s
struct MapReadErrorReader<R: io::Read>(R);

impl<R: io::Read> io::ErrorType for MapReadErrorReader<R>
where
    R::Error: 'static,
{
    type Error = Error<R::Error>;
}

impl<R: io::Read> io::Read for MapReadErrorReader<R>
where
    R::Error: 'static,
{
    fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = Result<usize, Self::Error>> {
        self.0.read(buf).map_err(Error::Read)
    }

    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), io::ReadExactError<Self::Error>> {
        self.0.read_exact(buf).await.map_err(|err| match err {
            io::ReadExactError::UnexpectedEof => io::ReadExactError::UnexpectedEof,
            io::ReadExactError::Other(err) => io::ReadExactError::Other(Error::Read(err)),
        })
    }
}

/// Information gathered once a [`Server`] has disconnection,
/// such as how many requests were handled and the shutdown reason if the server has graceful shutdown enabled.
pub struct DisconnectionInfo<S> {
    pub handled_requests_count: u64,
    pub shutdown_reason: Option<S>,
}

impl<S> DisconnectionInfo<S> {
    fn no_shutdown_reason(handled_requests_count: u64) -> Self {
        Self {
            handled_requests_count,
            shutdown_reason: None,
        }
    }

    fn with_shutdown_reason(handled_requests_count: u64, shutdown_reason: S) -> Self {
        Self {
            handled_requests_count,
            shutdown_reason: Some(shutdown_reason),
        }
    }
}

async fn serve_and_shutdown<
    Runtime,
    T: Timer<Runtime>,
    P: routing::PathRouter,
    S: io::Socket<Runtime>,
    ShutdownReason,
    ShutdownSignal: Future<Output = (ShutdownReason, Duration)>,
>(
    app: &Router<P>,
    timer: &T,
    config: &Config,
    http_buffer: &mut [u8],
    mut socket: S,
    shutdown_signal: ShutdownSignal,
) -> Result<DisconnectionInfo<ShutdownReason>, Error<S::Error>> {
    let mut connection_flags = request::ConnectionFlags::new();

    let result: Result<DisconnectionInfo<ShutdownReason>, Error<S::Error>> = async {
        let (reader, writer) = socket.split();

        let reader = MapReadErrorReader(reader);

        let mut writer = time::WriteWithTimeout {
            inner: writer,
            timer,
            timeout_duration: config.timeouts.write,
            _runtime: PhantomData,
        };

        let mut request_reader = request::Reader::new(reader, http_buffer, &mut connection_flags);

        // If `shutdown_signal` triggers, notify components which want to gracefully shutdown.
        let mut shutdown_broadcast = oneshot_broadcast::Signal::core();
        let (shutdown_broadcast_sender, shutdown_broadcast_signal) =
            shutdown_broadcast.make_signal();

        // Broadcast the shutdown signal when the given signal resolves.
        let mut shutdown_signal =
            core::pin::pin!(shutdown_signal.inspect(|_| shutdown_broadcast_sender.send(())));

        let mut request_count_iter = {
            let mut n = 0_u64;
            move || {
                let request_count = n;
                n = n.saturating_add(1);
                request_count
            }
        };

        // What to do after handling one request: keep looping, or stop serving and return.
        enum LoopResult<ShutdownReason, E: io::Error> {
            Continue,
            Stop(Result<DisconnectionInfo<ShutdownReason>, Error<E>>),
        }

        loop {
            let request_count = request_count_iter();

            let request_is_pending = match timer
                .run_with_timeout(
                    if request_count == 0 {
                        config.timeouts.start_read_request
                    } else {
                        config.timeouts.persistent_start_read_request
                    },
                    futures::select_either(
                        shutdown_signal.as_mut(),
                        request_reader.request_is_pending(),
                    ),
                )
                .await
            {
                Ok(futures::Either::First((shutdown_reason, _))) => {
                    return Ok(DisconnectionInfo::with_shutdown_reason(
                        request_count,
                        shutdown_reason,
                    ));
                }
                Ok(futures::Either::Second(Ok(Some(request_is_pending)))) => request_is_pending,
                Ok(futures::Either::Second(Ok(None))) | Err(time::TimeoutError) => {
                    return Ok(DisconnectionInfo::no_shutdown_reason(request_count))
                }
                Ok(futures::Either::Second(Err(err))) => return Err(err),
            };

            let mut read_request_timeout_signal = oneshot_broadcast::Signal::core();
            let (read_request_timeout_signal, read_request_timeout_listener) =
                read_request_timeout_signal.make_signal();

            let request_signals = request::RequestSignals {
                shutdown_signal: shutdown_broadcast_signal.clone(),
                read_request_timeout_signal: read_request_timeout_listener.clone(),
                make_read_timeout_error: || Error::ReadTimeout(time::TimeoutError),
            };

            let mut read_request_timeout =
                core::pin::pin!(timer.timeout(config.timeouts.read_request).map(|timeout| {
                    read_request_timeout_signal.send(());

                    Error::ReadTimeout(timeout)
                }));

            let request = futures::select_either(
                read_request_timeout.as_mut(),
                request_reader.read(request_is_pending, request_signals),
            )
            .await
            .first_is_error()?;

            // Both async match arms on `request` contain large variables or futures,
            // and the compiler does not recognize these are mutually exclusive blocks
            // of code, so memory is allocated in the `serve and shutdown` future for
            // both arms of the match.
            // Force the compiler to alias this memory by encapsulating each arm in a
            // async future held in a `futures::either`. Then we await the generic Either
            // future. Because the two futures are forced into one enum, the compiler
            // will always alias the memory.
            let result_future = match request {
                Ok(request) => futures::Either::First(async {
                    let connection_header = match config.connection {
                        KeepAlive::Close => KeepAlive::Close,
                        KeepAlive::KeepAlive => KeepAlive::from_request(
                            request.parts.http_version(),
                            request.parts.headers(),
                        ),
                    };

                    let mut handle_request = core::pin::pin!(futures::select_either(
                        // The timeout is handled by the socket returning an error when reads are attempted after the
                        read_request_timeout
                            .map(futures::ignore_output)
                            .then(futures::pend_forever),
                        app.handle_request(
                            request,
                            response::ResponseStream::new(&mut writer, connection_header),
                        ),
                    )
                    .map(futures::Either::ignore_never_a));

                    match futures::select_either(shutdown_signal.as_mut(), handle_request.as_mut())
                        .await
                    {
                        futures::Either::First((shutdown_reason, shutdown_timeout)) => {
                            LoopResult::Stop(Ok(DisconnectionInfo::with_shutdown_reason(
                                match timer
                                    .run_with_timeout(shutdown_timeout, handle_request)
                                    .await
                                    .swap_errors()
                                {
                                    Ok(Ok(ResponseSent(_))) => request_count + 1,
                                    Ok(Err(time::TimeoutError)) => request_count,
                                    Err(err) => return LoopResult::Stop(Err(err)),
                                },
                                shutdown_reason,
                            )))
                        }
                        futures::Either::Second(response_sent) => {
                            let ResponseSent(_) = match response_sent {
                                Ok(sent) => sent,
                                Err(err) => return LoopResult::Stop(Err(err)),
                            };

                            if let KeepAlive::KeepAlive = connection_header {
                                LoopResult::Continue
                            } else {
                                LoopResult::Stop(Ok(DisconnectionInfo::no_shutdown_reason(
                                    request_count + 1,
                                )))
                            }
                        }
                    }
                }),
                Err(err) => futures::Either::Second(async {
                    use response::IntoResponse;

                    let message = match err {
                        request::ReadError::BadRequestLine => "Bad Request Line",
                        request::ReadError::HeaderDoesNotContainColon => {
                            "Invalid Header line: No ':' character"
                        }
                        request::ReadError::UnexpectedEof => "Unexpected EOF while reading request",
                        request::ReadError::IO(err) => return LoopResult::Stop(Err(err)),
                    };

                    LoopResult::Stop(
                        match timer
                            .run_with_timeout(
                                config.timeouts.write,
                                (response::StatusCode::BAD_REQUEST, message).write_to(
                                    response::Connection::empty(&mut Default::default()),
                                    response::ResponseStream::new(&mut writer, KeepAlive::Close),
                                ),
                            )
                            .await
                        {
                            Ok(Ok(ResponseSent { .. })) => Err(Error::BadRequest),
                            Ok(Err(err)) => Err(err),
                            Err(err) => Err(Error::WriteTimeout(err)),
                        },
                    )
                }),
            };

            match result_future.await {
                LoopResult::Continue => continue,
                LoopResult::Stop(result) => return result,
            }
        }
    }
    .await;

    match result {
        Ok(disconnection_info) => {
            if connection_flags.connection_must_be_aborted() {
                futures::Either::First(socket.abort(&config.timeouts, timer))
            } else {
                futures::Either::Second(socket.shutdown(&config.timeouts, timer))
            }
            .await?;

            Ok(disconnection_info)
        }
        Err(error) => {
            // Ignore any subsequent errors
            _ = socket.abort(&config.timeouts, timer).await;

            Err(error)
        }
    }
}

/// Indicates that graceful shutdown is not enabled, so the [`Server`] cannot report a graceful shutdown reason.
pub enum NoGracefulShutdown {}

impl NoGracefulShutdown {
    /// Covert [`NoGracefulShutdown`] into another "never" type.
    pub fn into_never<T>(self) -> T {
        match self {}
    }
}

/// A HTTP Server.
pub struct Server<'a, Runtime, T: Timer<Runtime>, P: routing::PathRouter, ShutdownSignal: Future> {
    app: &'a Router<P>,
    timer: T,
    config: &'a Config,
    http_buffer: &'a mut [u8],
    shutdown_signal: ShutdownSignal,
    _runtime: PhantomData<fn(&Runtime)>,
}

impl<'a, Runtime, T: Timer<Runtime>, P: routing::PathRouter>
    Server<'a, Runtime, T, P, core::future::Pending<(NoGracefulShutdown, Duration)>>
{
    /// Create a new [`Router`] with a custom timer.
    ///
    /// Normally the functions behind the `embassy` feature will be used.
    pub fn custom(
        app: &'a Router<P>,
        timer: T,
        config: &'a Config,
        http_buffer: &'a mut [u8],
    ) -> Self {
        Self {
            app,
            timer,
            config,
            http_buffer,
            shutdown_signal: core::future::pending(),
            _runtime: PhantomData,
        }
    }

    /// Prepares a server to handle graceful shutdown when the provided future completes.
    ///
    /// If `shutdown_timeout` is not None and the request handler does not complete within that time, it is killed abruptly.
    pub fn with_graceful_shutdown<ShutdownSignal: Future>(
        self,
        shutdown_signal: ShutdownSignal,
        shutdown_timeout: impl Into<Duration>,
    ) -> Server<'a, Runtime, T, P, impl Future<Output = (ShutdownSignal::Output, Duration)>> {
        let Self {
            app,
            timer,
            config,
            http_buffer,
            shutdown_signal: _,
            _runtime,
        } = self;

        let shutdown_timeout = shutdown_timeout.into();

        Server {
            app,
            timer,
            config,
            http_buffer,
            shutdown_signal: async move {
                let shutdown_reason = shutdown_signal.await;

                (shutdown_reason, shutdown_timeout)
            },
            _runtime: PhantomData,
        }
    }
}

impl<
        Runtime,
        T: Timer<Runtime>,
        P: routing::PathRouter,
        ShutdownReason,
        ShutdownSignal: Future<Output = (ShutdownReason, Duration)>,
    > Server<'_, Runtime, T, P, ShutdownSignal>
{
    /// Serve requests read from the connected socket.
    pub async fn serve<S: io::Socket<Runtime>>(
        self,
        socket: S,
    ) -> Result<DisconnectionInfo<ShutdownReason>, Error<S::Error>> {
        let Self {
            app,
            timer,
            config,
            http_buffer,
            shutdown_signal,
            _runtime,
        } = self;

        serve_and_shutdown(app, &timer, config, http_buffer, socket, shutdown_signal).await
    }
}

#[cfg(any(feature = "tokio", test))]
#[doc(hidden)]
pub struct TokioRuntime;

#[cfg(any(feature = "tokio", test))]
impl<'a, P: routing::PathRouter>
    Server<
        'a,
        TokioRuntime,
        time::TokioTimer,
        P,
        core::future::Pending<(NoGracefulShutdown, time::Duration)>,
    >
{
    /// Create a new server using the `tokio` runtime, and typically with a `tokio::net::TcpSocket` as the socket.
    pub fn new_tokio(app: &'a Router<P>, config: &'a Config, http_buffer: &'a mut [u8]) -> Self {
        Self {
            app,
            timer: time::TokioTimer,
            config,
            http_buffer,
            shutdown_signal: core::future::pending(),
            _runtime: PhantomData,
        }
    }
}

#[cfg(feature = "embassy")]
#[doc(hidden)]
pub struct EmbassyRuntime;

#[cfg(feature = "embassy")]
impl<'a, P: routing::PathRouter>
    Server<
        'a,
        EmbassyRuntime,
        time::EmbassyTimer,
        P,
        core::future::Pending<(NoGracefulShutdown, Duration)>,
    >
{
    /// Create a new server using the `embassy` runtime.
    pub fn new(app: &'a Router<P>, config: &'a Config, http_buffer: &'a mut [u8]) -> Self {
        Self {
            app,
            timer: time::EmbassyTimer,
            config,
            http_buffer,
            shutdown_signal: core::future::pending(),
            _runtime: PhantomData,
        }
    }
}

#[cfg(feature = "embassy")]
impl<
        'a,
        P: routing::PathRouter,
        ShutdownReason,
        ShutdownSignal: Future<Output = (ShutdownReason, embassy_time::Duration)>,
    > Server<'a, EmbassyRuntime, time::EmbassyTimer, P, ShutdownSignal>
{
    /// Listen for incoming connections, and serve requests read from the connection.
    ///
    /// This will serve at most 1 connection at a time, so you will typically have multiple tasks running `listen_and_serve`.
    pub async fn listen_and_serve(
        self,
        task_id: impl LogDisplay,
        stack: embassy_net::Stack<'_>,
        port: u16,
        tcp_rx_buffer: &mut [u8],
        tcp_tx_buffer: &mut [u8],
    ) -> ShutdownReason {
        let Self {
            app,
            timer,
            config,
            http_buffer,
            shutdown_signal,
            _runtime,
        } = self;

        let mut shutdown_signal = core::pin::pin!(shutdown_signal);

        loop {
            let mut socket = match futures::select_either(shutdown_signal.as_mut(), async {
                let mut socket =
                    embassy_net::tcp::TcpSocket::new(stack, tcp_rx_buffer, tcp_tx_buffer);

                log_info!("{}: Listening on TCP:{}...", task_id, port);

                socket.accept(port).await.map(|()| socket)
            })
            .await
            {
                futures::Either::First((shutdown_reason, _)) => return shutdown_reason,
                futures::Either::Second(Err(error)) => {
                    log_warn!("{}: accept error: {:?}", task_id, error);
                    continue;
                }
                futures::Either::Second(Ok(socket)) => socket,
            };

            let remote_endpoint = socket.remote_endpoint();

            log_info!(
                "{}: Received connection from {:?}",
                task_id,
                remote_endpoint
            );

            socket.set_keep_alive(Some(embassy_time::Duration::from_secs(30)));
            socket.set_timeout(Some(embassy_time::Duration::from_secs(45)));

            return match serve_and_shutdown(
                app,
                &timer,
                config,
                http_buffer,
                socket,
                shutdown_signal.as_mut(),
            )
            .await
            {
                Ok(DisconnectionInfo {
                    handled_requests_count,
                    shutdown_reason,
                }) => {
                    log_info!(
                        "{} requests handled from {:?}",
                        handled_requests_count,
                        remote_endpoint
                    );

                    match shutdown_reason {
                        Some(shutdown_reason) => shutdown_reason,
                        None => continue,
                    }
                }
                Err(err) => {
                    log_error!("{:?}", logging::Debug2Format(&err));
                    continue;
                }
            };
        }
    }
}

/// A helper trait which simplifies creating a static [`Router`] with no state.
///
/// In practice usage requires the nightly Rust toolchain.
pub trait AppBuilder {
    type PathRouter: routing::PathRouter;

    fn build_app(self) -> Router<Self::PathRouter>;
}

/// A helper trait which simplifies creating a static [`Router`] with a declared state.
///
/// In practice usage requires the nightly Rust toolchain.
pub trait AppWithStateBuilder {
    type State;
    type PathRouter: routing::PathRouter<Self::State>;

    fn build_app(self) -> Router<Self::PathRouter, Self::State>;
}

impl<T: AppBuilder> AppWithStateBuilder for T {
    type State = ();
    type PathRouter = <Self as AppBuilder>::PathRouter;

    fn build_app(self) -> Router<Self::PathRouter, Self::State> {
        <Self as AppBuilder>::build_app(self)
    }
}

/// The [`Router`] for the app constructed from the Props (which implement [`AppBuilder`]).
pub type AppRouter<Props> =
    Router<<Props as AppWithStateBuilder>::PathRouter, <Props as AppWithStateBuilder>::State>;

/// Replacement for [`static_cell::make_static`](https://docs.rs/static_cell/latest/static_cell/macro.make_static.html) for use cases when the type is known.
#[macro_export]
macro_rules! make_static {
    ($t:ty, $val:expr) => ($crate::make_static!($t, $val,));
    ($t:ty, $val:expr, $(#[$m:meta])*) => {{
        $(#[$m])*
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        STATIC_CELL.init($val)
    }};
}
