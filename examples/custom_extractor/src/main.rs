//! Test with `curl -d 42 http://localhost:8000/number`

use std::time::Duration;

use picoserve::{
    extract::FromRequest,
    response::{ErrorWithStatusCode, IntoResponse},
    routing::{get_service, post},
};

struct Number {
    value: f32,
}

#[derive(Debug, thiserror::Error, ErrorWithStatusCode)]
#[status_code(BAD_REQUEST)]
enum BadRequest {
    #[error("Read Error")]
    #[status_code(INTERNAL_SERVER_ERROR)]
    ReadError,
    #[error("Request Body is not UTF-8: {0}")]
    NotUtf8(core::str::Utf8Error),
    #[error("Request Body is not a valid integer: {0}")]
    BadNumber(core::num::ParseFloatError),
}

impl<'r, State> FromRequest<'r, State> for Number {
    type Rejection = BadRequest;

    async fn from_request<R: picoserve::io::Read>(
        _state: &'r State,
        _request_parts: picoserve::request::RequestParts<'r>,
        request_body: picoserve::request::RequestBody<'r, R>,
    ) -> Result<Self, Self::Rejection> {
        Ok(Number {
            value: core::str::from_utf8(
                request_body
                    .read_all()
                    .await
                    .map_err(|_err| BadRequest::ReadError)?,
            )
            .map_err(BadRequest::NotUtf8)?
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

    let config = picoserve::Config::new(picoserve::Timeouts {
        start_read_request: Some(Duration::from_secs(5)),
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
