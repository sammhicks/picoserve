#![feature(async_fn_in_trait)]

use std::time::Duration;

use picoserve::routing::get;

#[derive(serde::Deserialize)]
struct FormValue {
    a: i32,
    b: heapless::String<32>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let app = std::rc::Rc::new(picoserve::Router::new().route(
        "/",
        get(|| picoserve::response::File::html(include_str!("index.html"))).post(
            |picoserve::extract::Form(FormValue { a, b })| {
                picoserve::response::DebugValue((("a", a), ("b", b)))
            },
        ),
    ));

    let config = picoserve::Config {
        start_read_request_timeout: Some(Duration::from_secs(5)),
        read_request_timeout: Some(Duration::from_secs(1)),
    };

    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 8000)).await?;

    println!("http://localhost:{port}/");

    tokio::task::LocalSet::new()
        .run_until(async {
            loop {
                let (mut stream, remote_address) = socket.accept().await?;

                println!("Connection from {remote_address}");

                let app = app.clone();
                let config = config.clone();

                tokio::task::spawn_local(async move {
                    let (stream_rx, stream_tx) = stream.split();

                    match picoserve::serve(&app, &config, &mut [0; 2048], stream_rx, stream_tx)
                        .await
                    {
                        Ok(handled_requests_count) => {
                            println!(
                                "{handled_requests_count} requests handled from {remote_address}"
                            )
                        }
                        Err(err) => println!("{err:?}"),
                    }
                });
            }
        })
        .await
}
