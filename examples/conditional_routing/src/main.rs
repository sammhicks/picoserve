use picoserve::routing::get;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let port = 8000;

    let common_routes = picoserve::Router::new().route("/", get(|| async { "Hello World" }));

    // If you change this to false, `http://localhost:8000/other` will return a 404 instead of "Other Route!".
    let include_other_route = true;

    let app = std::rc::Rc::new(if include_other_route {
        common_routes
            .route("/other", get(|| async { "Other Route!" }))
            .either_left_route()
    } else {
        common_routes.either_right_route()
    });

    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;

    log::info!("http://localhost:{port}/");

    tokio::task::LocalSet::new()
        .run_until(async {
            loop {
                let (stream, remote_address) = socket.accept().await?;

                log::info!("Connection from {remote_address}");

                let app = app.clone();

                tokio::task::spawn_local(async move {
                    static CONFIG: picoserve::Config =
                        picoserve::Config::const_default().keep_connection_alive();

                    match picoserve::Server::new_tokio(&app, &CONFIG, &mut [0; 2048])
                        .serve(stream)
                        .await
                    {
                        Ok(picoserve::DisconnectionInfo {
                            handled_requests_count,
                            ..
                        }) => {
                            log::info!(
                                "{handled_requests_count} requests handled from {remote_address}"
                            )
                        }
                        Err(error) => {
                            log::error!("Error handling requests from {remote_address}: {error}")
                        }
                    }
                });
            }
        })
        .await
}
