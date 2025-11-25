use std::time::Duration;

use picoserve::response::IntoResponse;

struct ShowRequestInfo;

impl picoserve::routing::MethodHandlerService for ShowRequestInfo {
    async fn call_request_handler_service<
        R: picoserve::io::Read,
        W: picoserve::response::ResponseWriter<Error = R::Error>,
    >(
        &self,
        (): &(),
        (): (),
        method: &str,
        mut request: picoserve::request::Request<'_, R>,
        response_writer: W,
    ) -> Result<picoserve::ResponseSent, W::Error> {
        use picoserve::io::Read;

        let headers = request.parts.headers();

        let mut body = request.body_connection.body().reader();

        let mut body_byte_histogram = [0_u16; 256];

        loop {
            let mut buffer = [0; 32];

            let read_size = body.read(&mut buffer).await?;

            if read_size == 0 {
                break;
            }

            for &b in &buffer[..read_size] {
                body_byte_histogram[usize::from(b)] += 1;
            }
        }

        format_args!("Method: {method}\r\nHeaders: {headers:?}\r\nRequest Body Byte Histogram: {body_byte_histogram:?}\r\n")
            .write_to(request.body_connection.finalize().await?, response_writer)
            .await
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let app = std::rc::Rc::new(picoserve::Router::new().route_service("/", ShowRequestInfo));

    let config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Some(Duration::from_secs(5)),
        persistent_start_read_request: Some(Duration::from_secs(1)),
        read_request: Some(Duration::from_secs(1)),
        write: Some(Duration::from_secs(1)),
    })
    .keep_connection_alive();

    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;

    println!("http://localhost:{port}/");

    tokio::task::LocalSet::new()
        .run_until(async {
            loop {
                let (stream, remote_address) = socket.accept().await?;

                println!("Connection from {remote_address}");

                let app = app.clone();
                let config = config.clone();

                tokio::task::spawn_local(async move {
                    match picoserve::Server::new(&app, &config, &mut [0; 2048])
                        .serve(stream)
                        .await
                    {
                        Ok(picoserve::DisconnectionInfo {
                            handled_requests_count,
                            ..
                        }) => {
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
