use std::time::{Duration, Instant};

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

impl<'r, W: ResponseWriter> ResponseWriter for TimedResponseWriter<'r, W> {
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

        println!(
            "Path: {}; Status Code: {}; Response Time: {}ms",
            self.path,
            status_code,
            self.start_time.elapsed().as_secs_f32() * 1000.0
        );

        result
    }
}

struct TimeLayer;

impl<State, PathParameters> picoserve::routing::Layer<State, PathParameters> for TimeLayer {
    type NextState = State;
    type NextPathParameters = PathParameters;

    async fn call_layer<
        R: Read,
        NextLayer: picoserve::routing::Next<R, Self::NextState, Self::NextPathParameters>,
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

    let config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Some(Duration::from_secs(5)),
        read_request: Some(Duration::from_secs(1)),
        write: Some(Duration::from_secs(1)),
    })
    .keep_connection_alive();

    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 8000)).await?;

    println!("http://localhost:{port}/");

    tokio::task::LocalSet::new()
        .run_until(async {
            loop {
                let (stream, remote_address) = socket.accept().await?;

                println!("Connection from {remote_address}");

                let app = app.clone();
                let config = config.clone();

                tokio::task::spawn_local(async move {
                    match picoserve::serve(&app, &config, &mut [0; 2048], stream).await {
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
