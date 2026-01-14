use picoserve::routing::get;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let app = picoserve::Router::new().route("/", get(|| async { "Hello World" }));

    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;

    println!("http://localhost:{port}/");

    loop {
        let (stream, remote_address) = socket.accept().await?;

        println!("Connection from {remote_address}");

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
                println!("{handled_requests_count} requests handled from {remote_address}")
            }
            Err(err) => println!("{err:?}"),
        }
    }
}
