use picoserve::{
    response,
    routing::{get, get_service, post},
    time::Duration,
};

#[derive(Clone)]
enum ServerState {
    Running,
    Shutdown,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

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
                "/counter",
                get(async || {
                    struct Counter;

                    impl picoserve::response::sse::EventSource for Counter {
                        async fn write_events<W: picoserve::io::Write>(
                            self,
                            mut writer: picoserve::response::sse::EventWriter<'_, W>,
                        ) -> Result<(), W::Error> {
                            let mut ticker =
                                tokio::time::interval(std::time::Duration::from_millis(300));

                            for tick in 0_u32.. {
                                ticker.tick().await;

                                writer
                                    .write_event("tick", format_args!("Count: {tick}"))
                                    .await?;
                            }

                            Ok(())
                        }
                    }

                    picoserve::response::sse::EventStream(Counter)
                }),
            )
            .route(
                "/shutdown",
                post(move || {
                    let _ = update_server_state.send(ServerState::Shutdown);
                    async { "Shutting Down\n" }
                }),
            ),
    );

    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;

    log::info!("http://localhost:{port}/");

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

                log::info!("Connection from {remote_address}");

                let app = app.clone();
                let mut server_state = server_state.clone();
                let wait_handle = wait_handle.clone();

                tokio::task::spawn_local(async move {
                    // Larger timeouts to demonstrate rapid graceful shutdown
                    static CONFIG: picoserve::Config =
                        picoserve::Config::new(picoserve::Timeouts {
                            start_read_request: Some(Duration::from_secs(10)),
                            persistent_start_read_request: Some(Duration::from_secs(10)),
                            read_request: Some(Duration::from_secs(1)),
                            write: Some(Duration::from_secs(1)),
                        })
                        .keep_connection_alive();

                    match picoserve::Server::new_tokio(&app, &CONFIG, &mut [0; 2048])
                        .with_graceful_shutdown(
                            server_state.wait_for(|state| matches!(state, ServerState::Shutdown)),
                            Duration::from_secs(1),
                        )
                        .serve(stream)
                        .await
                    {
                        Ok(picoserve::DisconnectionInfo {
                            handled_requests_count,
                            shutdown_reason,
                        }) => {
                            log::info!(
                                "{handled_requests_count} requests handled from {remote_address}",
                            );

                            if shutdown_reason.is_some() {
                                log::info!("Shutdown signal received");
                            }
                        }
                        Err(error) => {
                            log::error!("Error handling requests from {remote_address}: {error}")
                        }
                    }

                    drop(wait_handle);
                });
            }

            log::info!("Waiting for connections to close...");
            drop(wait_handle);

            #[allow(clippy::single_match)]
            match waiter.await {
                Ok(never) => match never {},
                Err(_) => (),
            }

            log::info!("All connections are closed");

            Ok(())
        })
        .await
}
