use picoserve::routing::get;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let port = 8000;

    let app = picoserve::Router::new().route("/", get(|| async { "Hello World" }));

    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;

    log::info!("http://localhost:{port}/");

    loop {
        let (stream, remote_address) = socket.accept().await?;

        log::info!("Connection from {remote_address}");

        static CONFIG: picoserve::Config =
            picoserve::Config::const_default().keep_connection_alive();

        match picoserve::Server::new_tokio(&app, &CONFIG, &mut [0; 2048])
            .serve(stream)
            .await
        {
            Ok(picoserve::DisconnectionInfo {
                handled_requests_count,
                ..
            }) => log::info!("{handled_requests_count} requests handled from {remote_address}"),
            Err(error) => {
                log::error!("Error handling requests from {remote_address}: {error}")
            }
        }
    }
}
