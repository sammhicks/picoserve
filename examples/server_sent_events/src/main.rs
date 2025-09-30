use std::time::Duration;

use picoserve::{
    response::{self, ErrorWithStatusCode, StatusCode},
    routing::{get, get_service, post},
};

#[derive(Debug, thiserror::Error, ErrorWithStatusCode)]
#[status_code(BAD_REQUEST)]
enum NewMessageRejection {
    #[error("Read Error")]
    ReadError,
    #[error("Body is not UTF-8: {0}")]
    NotUtf8(std::str::Utf8Error),
}

struct NewMessage(String);

impl<'r, State> picoserve::extract::FromRequest<'r, State> for NewMessage {
    type Rejection = NewMessageRejection;

    async fn from_request<R: picoserve::io::Read>(
        _state: &'r State,
        _request_parts: picoserve::request::RequestParts<'r>,
        request_body: picoserve::request::RequestBody<'r, R>,
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

    let app =
        std::rc::Rc::new(
            picoserve::Router::new()
                .route(
                    "/",
                    get_service(response::File::html(include_str!("index.html"))),
                )
                .route(
                    "/set_message",
                    post(move |NewMessage(message)| {
                        std::future::ready(messages_tx.send(message).map_err(|_| {
                            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to send message")
                        }))
                    }),
                )
                .route(
                    "/events",
                    get(move || response::EventStream(Events(messages_rx.clone()))),
                )
                .nest_service(
                    "/static",
                    const {
                        response::Directory {
                            files: &[],
                            sub_directories: &[
                                (
                                    "styles",
                                    response::Directory {
                                        files: &[(
                                            "index.css",
                                            response::File::css(include_str!("index.css")),
                                        )],
                                        ..response::Directory::DEFAULT
                                    },
                                ),
                                (
                                    "scripts",
                                    response::Directory {
                                        files: &[(
                                            "index.js",
                                            response::File::css(include_str!("index.js")),
                                        )],
                                        ..response::Directory::DEFAULT
                                    },
                                ),
                            ],
                        }
                    },
                ),
        );

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
