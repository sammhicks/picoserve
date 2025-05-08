use std::time::Duration;

use picoserve::{io::Read, response::IntoResponse, routing::get_service};

struct MeasureBody;

impl picoserve::routing::RequestHandlerService<()> for MeasureBody {
    async fn call_request_handler_service<
        R: Read,
        W: picoserve::response::ResponseWriter<Error = R::Error>,
    >(
        &self,
        (): &(),
        (): (),
        mut request: picoserve::request::Request<'_, R>,
        response_writer: W,
    ) -> Result<picoserve::ResponseSent, W::Error> {
        let mut reader = request.body_connection.body().reader();

        let mut buffer = [0; 1024];

        let mut total_size = 0;

        loop {
            let read_size = reader.read(&mut buffer).await?;
            if read_size == 0 {
                break;
            }

            total_size += read_size;
        }

        format!("Total Size: {total_size}\r\n")
            .write_to(request.body_connection.finalize().await?, response_writer)
            .await
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let app = std::rc::Rc::new(
        picoserve::Router::new().route(
            "/",
            get_service(picoserve::response::File::html(include_str!("index.html")))
                .post_service(MeasureBody),
        ),
    );

    let config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Some(Duration::from_secs(5)),persistent_start_read_request: Some(Duration::from_secs(1)),
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
                    match picoserve::serve(&app, &config, &mut [0; 1024], stream).await {
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
