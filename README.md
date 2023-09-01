# picoserve

An async `no_std` HTTP server suitable for bare-metal environments, heavily inspired by [axum](https://github.com/tokio-rs/axum).
It was designed with [embassy](https://embassy.dev/) on the Raspberry Pi Pico W in mind, but should work with other embedded runtimes and hardware.

Features:
+ No heap usage
+ Handler functions are just async functions that accept zero or more extractors as arguments and returns something that implements IntoResponse
+ Query and Form parsing using serde
+ JSON responses
+ Server-Sent Events
+ Web Sockets
+ HEAD method is automatically handled

Shortcomings:
+ While in version `0.*.*`, there may be breaking API changes
+ Requires the nightly toolchain, which is currently broken, thus is compiled against `nightly-2023-06-01`
+ Only the GET, POST, and HEAD methods are supported.
+ URL-Encoded strings, for example in Query and Form parsing, have a maximum length of 1024.
+ This has relatively little stress-testing so I advise not to expose it directly to the internet, but place it behind a proxy such as nginx, which will act as a security layer.
+ Certain serialization methods, such as the DebugValue response and JSON serialisation might be called several times if the response payload is large. The caller MUST ensure that the output of serialisation is the same during repeated calls with the same value.
+ The framework does not verify that the specified length of a reponse body, i.e. the value stored in the "Content-Length" header is actually the length of the body.

## Usage examples

### tokio (for testing purposes)

    #![feature(async_fn_in_trait)]

    use std::time::Duration;

    use picoserve::routing::get;

    #[tokio::main(flavor = "current_thread")]
    async fn main() -> anyhow::Result<()> {
        let port = 8000;

        let app =
            std::rc::Rc::new(picoserve::Router::new().route("/", get(|| async { "Hello World" })));

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

                        match picoserve::Server::new(&app, &config)
                            .serve(&mut [0; 2048], stream_rx, stream_tx)
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

