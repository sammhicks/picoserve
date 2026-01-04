#![no_std]
#![allow(async_fn_in_trait)]
#![deny(
    unsafe_code,
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

#[cfg(feature = "json")]
mod json;

#[macro_use]
mod logging;

pub mod extract;
pub mod futures;
pub mod io;
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

use core::marker::PhantomData;

pub use logging::LogDisplay;
pub use routing::Router;
pub use time::Timer;

use time::TimerExt;

use crate::sync::oneshot_broadcast;

pub use response::response_stream::ResponseSent;

/// Errors arising while handling a request.
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error<E: io::Error> {
    /// Bad Request from the client
    BadRequest,
    /// Error while reading from the socket.
    Read(E),
    /// Timeout while reading from the socket.
    ReadTimeout(crate::time::TimeoutError),
    /// Error while writing to the socket.
    Write(E),
    /// Timeout while writing to the socket.
    WriteTimeout(crate::time::TimeoutError),
}

impl<E: io::Error> io::Error for Error<E> {
    fn kind(&self) -> io::ErrorKind {
        match self {
            Self::BadRequest => io::ErrorKind::InvalidData,
            Self::ReadTimeout(error) | Self::WriteTimeout(error) => error.kind(),
            Self::Read(error) | Self::Write(error) => error.kind(),
        }
    }
}

/// How long to wait before timing out for different operations.
/// If set to None, the operation never times out.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Timeouts<D> {
    /// The duration of time to wait when starting to read the first request before the connection is closed due to inactivity.
    pub start_read_request: Option<D>,
    /// The duration of time to wait when starting to read persistent (i.e. not the first) requests before the connection is closed due to inactivity.
    pub persistent_start_read_request: Option<D>,
    /// The duration of time to wait when partway reading a request before the connection is aborted and closed.
    pub read_request: Option<D>,
    /// The duration of time to wait when writing the response before the connection is aborted and closed.
    pub write: Option<D>,
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

/// Maps Read errors to [Error]s
struct MapReadErrorReader<R: io::Read>(R);

impl<R: io::Read> io::ErrorType for MapReadErrorReader<R> {
    type Error = Error<R::Error>;
}

impl<R: io::Read> io::Read for MapReadErrorReader<R> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf).await.map_err(Error::Read)
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
    ShutdownSignal: core::future::Future<Output = (ShutdownReason, Option<T::Duration>)>,
>(
    app: &Router<P>,
    timer: &mut T,
    config: &Config<T::Duration>,
    http_buffer: &mut [u8],
    mut socket: S,
    shutdown_signal: ShutdownSignal,
) -> Result<DisconnectionInfo<ShutdownReason>, Error<S::Error>> {
    let result: Result<DisconnectionInfo<ShutdownReason>, Error<S::Error>> = async {
        let (reader, writer) = socket.split();

        let reader = MapReadErrorReader(reader);

        let mut writer = time::WriteWithTimeout {
            inner: writer,
            timer,
            timeout_duration: config.timeouts.write.clone(),
            _runtime: PhantomData,
        };

        let mut must_close_connection_notification =
            request::MustCloseConnectionNotification::new();

        let mut request_reader =
            request::Reader::new(reader, http_buffer, &mut must_close_connection_notification);

        let mut shutdown_signal = core::pin::pin!(shutdown_signal);

        // If `shutdown_signal` triggers, notify components which want to gracefully shutdown.
        let mut shutdown_broadcast = oneshot_broadcast::Signal::core();
        let shutdown_broadcast = shutdown_broadcast.make_signal();

        let mut request_count_iter = {
            let mut n = 0_u64;
            move || {
                let request_count = n;
                n = n.saturating_add(1);
                request_count
            }
        };

        loop {
            let request_count = request_count_iter();

            let request_is_pending = match timer
                .run_with_maybe_timeout(
                    if request_count == 0 {
                        config.timeouts.start_read_request.clone()
                    } else {
                        config.timeouts.persistent_start_read_request.clone()
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
            let read_request_timeout_signal = read_request_timeout_signal.make_signal();

            let request_signals = request::RequestSignals {
                shutdown_signal: shutdown_broadcast.listen(),
                read_request_timeout_signal: read_request_timeout_signal.listen(),
                make_read_timeout_error: || Error::ReadTimeout(crate::time::TimeoutError),
            };

            let mut read_request_timeout = core::pin::pin!(async {
                let timeout = timer
                    .maybe_timeout(config.timeouts.read_request.clone())
                    .await;

                read_request_timeout_signal.notify(());

                Error::ReadTimeout(timeout)
            });

            let request = futures::select_either(
                read_request_timeout.as_mut(),
                request_reader.read(request_is_pending, request_signals),
            )
            .await
            .first_is_error()?;

            match request {
                Ok(request) => {
                    let connection_header = match config.connection {
                        KeepAlive::Close => KeepAlive::Close,
                        KeepAlive::KeepAlive => KeepAlive::from_request(
                            request.parts.http_version(),
                            request.parts.headers(),
                        ),
                    };

                    let mut handle_request = core::pin::pin!(crate::futures::select(
                        async {
                            read_request_timeout.await;

                            core::future::pending().await
                        },
                        app.handle_request(
                            request,
                            response::ResponseStream::new(&mut writer, connection_header),
                        )
                    ));

                    return Ok(
                        match crate::futures::select_either(
                            shutdown_signal.as_mut(),
                            handle_request.as_mut(),
                        )
                        .await
                        {
                            futures::Either::First((shutdown_reason, shutdown_timeout)) => {
                                shutdown_broadcast.notify(());

                                DisconnectionInfo::with_shutdown_reason(
                                    if let Ok(handle_request_response) = timer
                                        .run_with_maybe_timeout(shutdown_timeout, handle_request)
                                        .await
                                    {
                                        let ResponseSent(_) = handle_request_response?;

                                        request_count + 1
                                    } else {
                                        request_count
                                    },
                                    shutdown_reason,
                                )
                            }
                            futures::Either::Second(response_sent) => {
                                let ResponseSent(_) = response_sent?;

                                if let KeepAlive::KeepAlive = connection_header {
                                    continue;
                                }

                                DisconnectionInfo::no_shutdown_reason(request_count + 1)
                            }
                        },
                    );
                }
                Err(err) => {
                    use response::IntoResponse;

                    let message = match err {
                        request::ReadError::BadRequestLine => "Bad Request Line",
                        request::ReadError::HeaderDoesNotContainColon => {
                            "Invalid Header line: No ':' character"
                        }
                        request::ReadError::UnexpectedEof => "Unexpected EOF while reading request",
                        request::ReadError::IO(err) => return Err(err),
                    };

                    let ResponseSent(_) = timer
                        .run_with_maybe_timeout(
                            config.timeouts.write.clone(),
                            (response::StatusCode::BAD_REQUEST, message).write_to(
                                response::Connection::empty(&mut Default::default()),
                                response::ResponseStream::new(writer, KeepAlive::Close),
                            ),
                        )
                        .await
                        .map_err(Error::WriteTimeout)??;

                    return Err(Error::BadRequest);
                }
            }
        }
    }
    .await;

    let shutdown_result = socket.shutdown(&config.timeouts, timer).await;

    let request_count = result?;

    shutdown_result?;

    Ok(request_count)
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
pub struct Server<
    'a,
    Runtime,
    T: Timer<Runtime>,
    P: routing::PathRouter,
    ShutdownSignal: core::future::Future,
> {
    app: &'a Router<P>,
    timer: T,
    config: &'a Config<T::Duration>,
    http_buffer: &'a mut [u8],
    shutdown_signal: ShutdownSignal,
    _runtime: PhantomData<fn(&Runtime)>,
}

impl<'a, Runtime, T: Timer<Runtime>, P: routing::PathRouter>
    Server<'a, Runtime, T, P, core::future::Pending<(NoGracefulShutdown, Option<T::Duration>)>>
{
    /// Create a new [`Router`] with a custom timer.
    ///
    /// Normally the functions behind the `embassy` feature will be used.
    pub fn custom(
        app: &'a Router<P>,
        timer: T,
        config: &'a Config<T::Duration>,
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
    #[allow(clippy::type_complexity)]
    pub fn with_graceful_shutdown<ShutdownSignal: core::future::Future>(
        self,
        shutdown_signal: ShutdownSignal,
        shutdown_timeout: impl Into<Option<T::Duration>>,
    ) -> Server<
        'a,
        Runtime,
        T,
        P,
        impl core::future::Future<Output = (ShutdownSignal::Output, Option<T::Duration>)>,
    > {
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
        ShutdownSignal: core::future::Future<Output = (ShutdownReason, Option<T::Duration>)>,
    > Server<'_, Runtime, T, P, ShutdownSignal>
{
    /// Serve requests read from the connected socket.
    pub async fn serve<S: io::Socket<Runtime>>(
        self,
        socket: S,
    ) -> Result<DisconnectionInfo<ShutdownReason>, Error<S::Error>> {
        let Self {
            app,
            mut timer,
            config,
            http_buffer,
            shutdown_signal,
            _runtime,
        } = self;

        serve_and_shutdown(
            app,
            &mut timer,
            config,
            http_buffer,
            socket,
            shutdown_signal,
        )
        .await
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
        core::future::Pending<(NoGracefulShutdown, Option<std::time::Duration>)>,
    >
{
    /// Create a new server using the `tokio` runtime, and typically with a `tokio::net::TcpSocket` as the socket.
    pub fn new(
        app: &'a Router<P>,
        config: &'a Config<std::time::Duration>,
        http_buffer: &'a mut [u8],
    ) -> Self {
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
        core::future::Pending<(NoGracefulShutdown, Option<embassy_time::Duration>)>,
    >
{
    /// Create a new server using the `embassy` runtime.
    pub fn new(
        app: &'a Router<P>,
        config: &'a Config<embassy_time::Duration>,
        http_buffer: &'a mut [u8],
    ) -> Self {
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
        ShutdownSignal: core::future::Future<Output = (ShutdownReason, Option<embassy_time::Duration>)>,
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
            mut timer,
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
                &mut timer,
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
                    log_error!("{:?}", crate::logging::Debug2Format(&err));
                    continue;
                }
            };
        }
    }
}

/// A helper trait which simplifies creating a static [Router] with no state.
///
/// In practice usage requires the nightly Rust toolchain.
pub trait AppBuilder {
    type PathRouter: routing::PathRouter;

    fn build_app(self) -> Router<Self::PathRouter>;
}

/// A helper trait which simplifies creating a static [Router] with a declared state.
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

/// The [Router] for the app constructed from the Props (which implement [AppBuilder]).
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
