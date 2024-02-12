use std::time::Duration;

use picoserve::{
    response::{self, status},
    routing::{get, post},
    ResponseSent,
};

enum NewMessageRejection {
    ReadError,
    NotUtf8(std::str::Utf8Error),
}

impl response::IntoResponse for NewMessageRejection {
    async fn write_to<R: picoserve::io::Read, W: response::ResponseWriter<Error = R::Error>>(
        self,
        connection: response::Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        match self {
            NewMessageRejection::ReadError => {
                (status::BAD_REQUEST, "Read Error")
                    .write_to(connection, response_writer)
                    .await
            }
            NewMessageRejection::NotUtf8(err) => {
                (
                    status::BAD_REQUEST,
                    format_args!("Body is not UTF-8: {err}\n"),
                )
                    .write_to(connection, response_writer)
                    .await
            }
        }
    }
}

struct NewMessage(String);

impl<State> picoserve::extract::FromRequest<State> for NewMessage {
    type Rejection = NewMessageRejection;

    async fn from_request<R: picoserve::io::Read>(
        _state: &State,
        _request_parts: picoserve::request::RequestParts<'_>,
        request_body: picoserve::request::RequestBody<'_, R>,
    ) -> Result<Self, Self::Rejection> {
        core::str::from_utf8(
            request_body
                .read_all()
                .await
                .map_err(|_err| NewMessageRejection::ReadError)?,
        )
        .map(|message| NewMessage(message.into()))
        .map_err(NewMessageRejection::NotUtf8)
    }
}

struct Events(tokio::sync::watch::Receiver<String>);

impl response::sse::EventSource for Events {
    async fn write_events<W: picoserve::io::Write>(
        mut self,
        mut writer: response::sse::EventWriter<W>,
    ) -> Result<(), W::Error> {
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(15), self.0.changed()).await {
                Ok(Ok(())) => {
                    writer
                        .write_event("message_changed", self.0.borrow_and_update().as_str())
                        .await?
                }
                Ok(Err(_)) => return Ok(()),
                Err(_) => writer.write_keepalive().await?,
            }
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let (messages_tx, messages_rx) = tokio::sync::watch::channel(String::new());

    let app = std::rc::Rc::new(
        picoserve::Router::new()
            .route(
                "/",
                get(|| response::File::html(include_str!("index.html"))),
            )
            .route(
                "/set_message",
                post(move |NewMessage(message)| {
                    std::future::ready(
                        messages_tx
                            .send(message)
                            .map_err(|_| (status::INTERNAL_SERVER_ERROR, "Failed to send message")),
                    )
                }),
            )
            .route(
                "/events",
                get(move || response::EventStream(Events(messages_rx.clone()))),
            )
            .nest("/static", {
                const STATIC_FILES: response::Directory = response::Directory {
                    files: &[],
                    sub_directories: &[
                        (
                            "styles",
                            response::Directory {
                                files: &[(
                                    "index.css",
                                    response::File::css(include_str!("index.css")),
                                )],
                                sub_directories: &[],
                            },
                        ),
                        (
                            "scripts",
                            response::Directory {
                                files: &[(
                                    "index.js",
                                    response::File::css(include_str!("index.js")),
                                )],
                                sub_directories: &[],
                            },
                        ),
                    ],
                };

                STATIC_FILES
            }),
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
