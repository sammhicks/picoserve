use picoserve::routing::{get, get_service};

#[derive(serde::Deserialize)]
struct QueryParams {
    a: i32,
    b: heapless::String<32>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let port = 8000;

    let app = std::rc::Rc::new(
        picoserve::Router::new()
            .route(
                "/",
                get_service(picoserve::response::File::html(include_str!("index.html"))),
            )
            .route(
                "/get-thing",
                get(async |picoserve::extract::Query(QueryParams { a, b })| {
                    picoserve::response::DebugValue((("a", a), ("b", b)))
                }),
            ),
    );

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
                        }) => log::info!(
                            "{handled_requests_count} requests handled from {remote_address}",
                        ),
                        Err(error) => {
                            log::error!("Error handling requests from {remote_address}: {error}")
                        }
                    }
                });
            }
        })
        .await
}
