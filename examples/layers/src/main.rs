use std::time::{Duration, Instant};

use picoserve::{
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

    async fn write_response<H: picoserve::response::HeadersIter, B: picoserve::response::Body>(
        self,
        response: picoserve::response::Response<H, B>,
    ) -> Result<picoserve::ResponseSent, W::Error> {
        let status_code = response.status_code();

        let result = self.response_writer.write_response(response).await;

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
        NextLayer: picoserve::routing::Next<Self::NextState, Self::NextPathParameters>,
        W: ResponseWriter,
    >(
        &self,
        next: NextLayer,
        state: &State,
        path_parameters: PathParameters,
        request: picoserve::request::Request<'_>,
        response_writer: W,
    ) -> Result<picoserve::ResponseSent, W::Error> {
        let path = request.path();

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
