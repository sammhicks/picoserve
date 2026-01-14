use std::time::Instant;

use picoserve::{
    io::Read,
    request::Path,
    response::ResponseWriter,
    routing::{get, parse_path_segment},
};

struct TimedResponseWriter<'r, W> {
    path: Path<'r>,
    start_time: Instant,
    response_writer: W,
}

impl<W: ResponseWriter> ResponseWriter for TimedResponseWriter<'_, W> {
    type Error = W::Error;

    async fn write_response<
        R: Read<Error = Self::Error>,
        H: picoserve::response::HeadersIter,
        B: picoserve::response::Body,
    >(
        self,
        connection: picoserve::response::Connection<'_, R>,
        response: picoserve::response::Response<H, B>,
    ) -> Result<picoserve::ResponseSent, Self::Error> {
        let status_code = response.status_code();

        let result = self
            .response_writer
            .write_response(connection, response)
            .await;

        log::info!(
            "Path: {}; Status Code: {}; Response Time: {}ms",
            self.path,
            status_code,
            self.start_time.elapsed().as_secs_f32() * 1000.0,
        );

        result
    }
}

struct TimeLayer;

impl<State, PathParameters> picoserve::routing::Layer<State, PathParameters> for TimeLayer {
    type NextState = State;
    type NextPathParameters = PathParameters;

    async fn call_layer<
        'a,
        R: Read + 'a,
        NextLayer: picoserve::routing::Next<'a, R, Self::NextState, Self::NextPathParameters>,
        W: ResponseWriter<Error = R::Error>,
    >(
        &self,
        next: NextLayer,
        state: &State,
        path_parameters: PathParameters,
        request_parts: picoserve::request::RequestParts<'_>,
        response_writer: W,
    ) -> Result<picoserve::ResponseSent, W::Error> {
        let path = request_parts.path();

        next.run(
            state,
            path_parameters,
            TimedResponseWriter {
                path,
                start_time: Instant::now(),
                response_writer,
            },
        )
        .await
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let port = 8000;

    let app = std::rc::Rc::new(
        picoserve::Router::new()
            .route("/", get(|| async { "Hello World" }))
            .route(
                ("/delay", parse_path_segment()),
                get(|millis| async move {
                    tokio::time::sleep(std::time::Duration::from_millis(millis)).await;
                    format!("Waited {millis}ms")
                }),
            )
            .layer(TimeLayer),
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
