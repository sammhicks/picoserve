//! Test with `curl -d 42 http://localhost:8000/number`

use picoserve::{
    extract::FromRequest,
    response::{ErrorWithStatusCode, IntoResponse},
    routing::{get_service, post},
};

struct Number {
    value: f32,
}

#[derive(Debug, thiserror::Error, ErrorWithStatusCode)]

enum BadRequest {
    #[error("Read Error")]
    #[status_code(transparent)]
    FailedToExtractEntireBodyAsStringError(
        picoserve::extract::FailedToExtractEntireBodyAsStringError,
    ),
    #[error("Request Body is not a valid integer: {0}")]
    #[status_code(BAD_REQUEST)]
    BadNumber(core::num::ParseFloatError),
}

impl<'r, State> FromRequest<'r, State> for Number {
    type Rejection = BadRequest;

    async fn from_request<R: picoserve::io::Read>(
        state: &'r State,
        request_parts: picoserve::request::RequestParts<'r>,
        request_body: picoserve::request::RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        Ok(Number {
            value: <&str>::from_request(state, request_parts, request_body)
                .await
                .map_err(BadRequest::FailedToExtractEntireBodyAsStringError)?
                .parse()
                .map_err(BadRequest::BadNumber)?,
        })
    }
}

async fn handler_with_extractor(Number { value }: Number) -> impl IntoResponse {
    picoserve::response::DebugValue(value)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let app = std::rc::Rc::new(
        picoserve::Router::new()
            .route(
                "/",
                get_service(picoserve::response::File::html(include_str!("index.html"))),
            )
            .route(
                "/index.js",
                get_service(picoserve::response::File::html(include_str!("index.js"))),
            )
            .route("/number", post(handler_with_extractor)),
    );

    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;

    println!("http://localhost:{port}/");

    tokio::task::LocalSet::new()
        .run_until(async {
            loop {
                let (stream, remote_address) = socket.accept().await?;

                println!("Connection from {remote_address}");

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
