use std::time::{Duration, Instant};

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
        use sha2::Digest;

        if request.body_connection.content_length() > 2_000_000 {
            let response = (
                picoserve::response::StatusCode::PAYLOAD_TOO_LARGE,
                "The file must be smaller than 2MB",
            )
                .write_to(request.body_connection.finalize().await?, response_writer)
                .await;

            log::info!("Too large");

            return response;
        }

        let timeout = Duration::from_nanos(request.body_connection.content_length() as u64 * 100);

        log::info!("Allowed time: {:.2}s", timeout.as_millis() as f32 / 1000.0);
        let start_time = Instant::now();

        let mut reader = request
            .body_connection
            .body()
            .reader()
            // If you use the embassy feature, using `with_different_timeout` is preferable.
            .with_different_timeout_signal(Box::pin(tokio::time::sleep(timeout)));

        let mut buffer = [0; 1024];

        let mut hasher = sha2::Sha256::new();
        let mut upload_byte_count = 0_usize;

        let mut last_log_time = Instant::now();

        let hash = loop {
            let read_size = reader.read(&mut buffer).await?;
            if read_size == 0 {
                break hasher.finalize();
            }

            hasher.update(&buffer[..read_size]);
            upload_byte_count += read_size;

            if last_log_time.elapsed() > Duration::from_secs(1) {
                last_log_time = Instant::now();

                log::info!(
                    "Upload progress: {:.2}%",
                    100.0 * (upload_byte_count as f32) / (reader.content_length() as f32),
                )
            }
        };

        log::info!(
            "Done in {:.2}s",
            start_time.elapsed().as_millis() as f32 / 1000.0,
        );

        format_args!("SHA2 hash: {hash:x}\r\n")
            .write_to(request.body_connection.finalize().await?, response_writer)
            .await
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let port = 8000;

    let app = std::rc::Rc::new(
        picoserve::Router::new().route(
            "/",
            get_service(picoserve::response::File::html(include_str!("index.html")))
                .post_service(MeasureBody),
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
