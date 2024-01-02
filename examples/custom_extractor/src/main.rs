//! Test with `curl -d 42 http://localhost:8000/number`

use std::time::Duration;

use picoserve::{extract::FromRequest, response::IntoResponse, routing::post};

struct Number {
    value: i32,
}

enum BadRequest {
    NotUtf8(core::str::Utf8Error),
    BadNumber(core::num::ParseIntError),
}

impl IntoResponse for BadRequest {
    async fn write_to<W: picoserve::response::ResponseWriter>(
        self,
        response_writer: W,
    ) -> Result<picoserve::ResponseSent, W::Error> {
        match self {
            BadRequest::NotUtf8(err) => {
                (
                    picoserve::response::status::BAD_REQUEST,
                    format_args!("Request Body is not UTF-8: {err}"),
                )
                    .write_to(response_writer)
                    .await
            }
            BadRequest::BadNumber(err) => {
                (
                    picoserve::response::status::BAD_REQUEST,
                    format_args!("Request Body is not a valid integer: {err}"),
                )
                    .write_to(response_writer)
                    .await
            }
        }
    }
}

impl<State> FromRequest<State> for Number {
    type Rejection = BadRequest;

    async fn from_request(
        _state: &State,
        request: &picoserve::request::Request<'_>,
    ) -> Result<Self, Self::Rejection> {
        Ok(Number {
            value: core::str::from_utf8(request.body())
                .map_err(BadRequest::NotUtf8)?
                .parse()
                .map_err(BadRequest::BadNumber)?,
        })
    }
}

async fn handler_with_extractor(Number { value }: Number) -> impl IntoResponse {
    picoserve::response::DebugValue(("number", value))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let app =
        std::rc::Rc::new(picoserve::Router::new().route("/number", post(handler_with_extractor)));

    let config = picoserve::Config {
        start_read_request_timeout: Some(Duration::from_secs(5)),
        read_request_timeout: Some(Duration::from_secs(1)),
        write_timeout: Some(Duration::from_secs(1)),
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
