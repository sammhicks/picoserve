use std::time::Duration;

use futures_util::FutureExt;
use picoserve::{
    response,
    routing::{get, get_service, post},
};

#[derive(Clone)]
enum ServerState {
    Running,
    Shutdown,
}

struct WebSocketCallback;

impl response::ws::WebSocketCallbackWithShutdownSignal for WebSocketCallback {
    async fn run_with_shutdown_signal<
        R: picoserve::io::Read,
        W: picoserve::io::Write<Error = R::Error>,
        S: core::future::Future<Output = ()> + Clone + Unpin,
    >(
        self,
        mut rx: response::ws::SocketRx<R>,
        mut tx: response::ws::SocketTx<W>,
        shutdown_signal: S,
    ) -> Result<(), W::Error> {
        use picoserve::response::ws::Message;

        #[derive(serde::Serialize)]
        #[serde(tag = "type")]
        enum ClientEvent<'a> {
            Echo { payload: &'a str },
            Count { value: u64 },
        }

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        let shared_tx = tokio::sync::Mutex::new(&mut tx);

        let echo_task = async {
            let mut shutdown_signal = shutdown_signal.clone();

            let mut buffer = [0; 128];

            loop {
                let message = match rx.next_message(&mut buffer, &mut shutdown_signal).await? {
                    picoserve::futures::Either::First(Ok(message)) => message,
                    picoserve::futures::Either::First(Err(error)) => {
                        eprintln!("Websocket error: {error:?}");
                        break Ok(Some((error.code(), "Websocket Error")));
                    }
                    picoserve::futures::Either::Second(()) => {
                        let _ = shutdown_tx.send(());
                        break Ok(Some((1001, "Server is shutting down")));
                    }
                };

                println!("Message: {message:?}");

                match message {
                    Message::Text(payload) => {
                        shared_tx
                            .lock()
                            .await
                            .send_json(ClientEvent::Echo { payload })
                            .await?;
                    }
                    Message::Binary(payload) => {
                        println!("Ignoring Binary message: {payload:?}")
                    }
                    Message::Close(_) => break Ok(None),
                    Message::Ping(data) => shared_tx.lock().await.send_pong(data).await?,
                    Message::Pong(_) => (),
                }
            }
        };

        let counter_task = async {
            let mut ticker = tokio::time::interval(Duration::from_secs(1));

            let mut shutdown_signal =
                futures_util::future::select(shutdown_signal.clone(), shutdown_rx.map(|_| ()))
                    .map(|_| ());

            for value in 0.. {
                match futures_util::future::select(
                    &mut shutdown_signal,
                    std::pin::pin!(ticker.tick()),
                )
                .await
                {
                    futures_util::future::Either::Left(((), _)) => break,
                    futures_util::future::Either::Right((_, _)) => {
                        shared_tx
                            .lock()
                            .await
                            .send_json(ClientEvent::Count { value })
                            .await?
                    }
                }
            }

            Ok(())
        };

        let (close_reason, ()) = futures_util::future::try_join(echo_task, counter_task).await?;

        tx.close(close_reason).await
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let (update_server_state, mut server_state) = tokio::sync::watch::channel(ServerState::Running);

    let app = std::rc::Rc::new(
        picoserve::Router::new()
            .route(
                "/",
                get_service(response::File::html(include_str!("index.html"))),
            )
            .route(
                "/index.css",
                get_service(response::File::css(include_str!("index.css"))),
            )
            .route(
                "/index.js",
                get_service(response::File::javascript(include_str!("index.js"))),
            )
            .route(
                "/shutdown",
                post(move || {
                    let _ = update_server_state.send(ServerState::Shutdown);
                    async { "Shutting Down\n" }
                }),
            )
            .route(
                "/ws",
                get(|upgrade: response::WebSocketUpgrade| async move {
                    upgrade.on_upgrade(WebSocketCallback)
                }),
            ),
    );

    // Larger timeouts to demonstrate rapid graceful shutdown
    let config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Some(Duration::from_secs(10)),
        persistent_start_read_request: Some(Duration::from_secs(10)),
        read_request: Some(Duration::from_secs(1)),
        write: Some(Duration::from_secs(1)),
    })
    .keep_connection_alive();

    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;

    println!("http://localhost:{port}/");

    let (wait_handle, waiter) = tokio::sync::oneshot::channel::<futures_util::never::Never>();
    let wait_handle = std::sync::Arc::new(wait_handle);

    tokio::task::LocalSet::new()
        .run_until(async {
            loop {
                let (stream, remote_address) = match futures_util::future::select(
                    std::pin::pin!(
                        server_state.wait_for(|state| matches!(state, ServerState::Shutdown))
                    ),
                    std::pin::pin!(socket.accept()),
                )
                .await
                {
                    futures_util::future::Either::Left((_, _)) => break,
                    futures_util::future::Either::Right((connection, _)) => connection?,
                };

                println!("Connection from {remote_address}");

                let app = app.clone();
                let config = config.clone();
                let mut server_state = server_state.clone();
                let wait_handle = wait_handle.clone();

                tokio::task::spawn_local(async move {
                    match picoserve::Server::new(&app, &config, &mut [0; 2048])
                        .with_graceful_shutdown(
                            server_state.wait_for(|state| matches!(state, ServerState::Shutdown)),
                            Duration::from_secs(5),
                        )
                        .serve(stream)
                        .await
                    {
                        Ok(picoserve::DisconnectionInfo {
                            handled_requests_count,
                            shutdown_reason,
                        }) => {
                            println!(
                                "{handled_requests_count} requests handled from {remote_address}"
                            );

                            if shutdown_reason.is_some() {
                                println!("Shutdown signal received");
                            }
                        }
                        Err(err) => println!("{err:?}"),
                    }

                    drop(wait_handle);
                });
            }

            println!("Waiting for connections to close...");
            drop(wait_handle);

            #[allow(clippy::single_match)]
            match waiter.await {
                Ok(never) => match never {},
                Err(_) => (),
            }

            println!("All connections are closed");

            Ok(())
        })
        .await
}
