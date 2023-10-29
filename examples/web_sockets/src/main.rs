#![feature(async_fn_in_trait)]

use std::time::Duration;

use picoserve::{response::status, routing::get, ResponseSent};

struct NewMessageRejection(std::str::Utf8Error);

impl picoserve::response::IntoResponse for NewMessageRejection {
    async fn write_to<W: picoserve::response::ResponseWriter>(
        self,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (
            status::BAD_REQUEST,
            format_args!("Body is not UTF-8: {}\n", self.0),
        )
            .write_to(response_writer)
            .await
    }
}

struct NewMessage(String);

impl<State> picoserve::extract::FromRequest<State> for NewMessage {
    type Rejection = NewMessageRejection;

    async fn from_request(
        _state: &State,
        request: &picoserve::request::Request<'_>,
    ) -> Result<Self, Self::Rejection> {
        core::str::from_utf8(request.body())
            .map(|message| NewMessage(message.into()))
            .map_err(NewMessageRejection)
    }
}

struct WebsocketHandler {
    tx: std::rc::Rc<tokio::sync::watch::Sender<String>>,
    rx: tokio::sync::watch::Receiver<String>,
}

impl picoserve::response::ws::WebSocketCallback for WebsocketHandler {
    async fn run<R: picoserve::io::Read, W: picoserve::io::Write<Error = R::Error>>(
        mut self,
        mut rx: picoserve::response::ws::SocketRx<R>,
        mut tx: picoserve::response::ws::SocketTx<W>,
    ) -> Result<(), W::Error> {
        use picoserve::response::ws::Message;

        let mut message_buffer = [0; 128];

        loop {
            tokio::select! {
                message_changed = self.rx.changed() => match message_changed {
                    Ok(()) => tx.send_text(self.rx.borrow_and_update().as_str()).await?,
                    Err(_) => break,
                },
                new_message = rx.next_message(&mut message_buffer) => match new_message {
                    Ok(Message::Text(new_message)) => { let _ = self.tx.send(new_message.into()); },
                    Ok(Message::Binary(message)) => println!("Ignoring binary message: {message:?}"),
                    Ok(Message::Ping(ping)) => tx.send_pong(ping).await?,
                    Ok(Message::Pong(_)) => (),
                    Err(picoserve::response::ws::ReadMessageError::Io(err)) => return Err(err),
                    Ok(Message::Close(_)) | Err(_) => break,
                }
            }
        }

        tx.close(None).await
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let (messages_tx, messages_rx) = tokio::sync::watch::channel(String::new());

    let messages_tx = std::rc::Rc::new(messages_tx);

    let app = std::rc::Rc::new(
        picoserve::Router::new()
            .route(
                "/",
                get(|| picoserve::response::File::html(include_str!("index.html"))),
            )
            .route(
                "/ws",
                get(move |upgrade: picoserve::response::ws::WebSocketUpgrade| {
                    if let Some(protocols) = upgrade.protocols() {
                        println!("Protocols:");
                        for protocol in protocols {
                            println!("\t{protocol}");
                        }
                    }

                    upgrade
                        .on_upgrade(WebsocketHandler {
                            tx: messages_tx.clone(),
                            rx: messages_rx.clone(),
                        })
                        .with_protocol("messages")
                }),
            ),
    );

    let config = picoserve::Config {
        start_read_request_timeout: Some(Duration::from_secs(5)),
        read_request_timeout: Some(Duration::from_secs(1)),
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
