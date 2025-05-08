use std::time::Duration;

use picoserve::routing::get;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let app = picoserve::Router::new().route("/", get(|| async { "Hello World" }));

    let config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Some(Duration::from_secs(5)),
        persistent_start_read_request: Some(Duration::from_secs(1)),
        read_request: Some(Duration::from_secs(1)),
        write: Some(Duration::from_secs(1)),
    });

    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;

    println!("http://localhost:{port}/");

    loop {
        let (stream, remote_address) = socket.accept().await?;

        println!("Connection from {remote_address}");

        match picoserve::serve(&app, &config, &mut [0; 2048], stream).await {
            Ok(handled_requests_count) => {
                println!("{handled_requests_count} requests handled from {remote_address}")
            }
            Err(err) => println!("{err:?}"),
        }
    }
}
