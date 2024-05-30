use std::time::Duration;

use picoserve::{response::ws, routing::get};

struct WebsocketHandler {
    tx: std::rc::Rc<tokio::sync::broadcast::Sender<String>>,
    rx: tokio::sync::broadcast::Receiver<String>,
}

impl ws::WebSocketCallback for WebsocketHandler {
    async fn run<R: picoserve::io::Read, W: picoserve::io::Write<Error = R::Error>>(
        mut self,
        mut rx: ws::SocketRx<R>,
        mut tx: ws::SocketTx<W>,
    ) -> Result<(), W::Error> {
        use picoserve::response::ws::Message;

        let mut message_buffer = [0; 128];

        let close_reason = loop {
            tokio::select! {
                message_changed = self.rx.recv() => match message_changed {
                    Ok(message) => tx.send_display(format_args!("Message: {message}")).await?,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => tx.send_display(format_args!("Missed {n} messages")).await?,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break None,
                },
                new_message = rx.next_message(&mut message_buffer) => match new_message {
                    Ok(Message::Text(new_message)) => { let _ = self.tx.send(new_message.into()); },
                    Ok(Message::Binary(message)) => println!("Ignoring binary message: {message:?}"),
                    Ok(ws::Message::Close(reason)) => {
                        eprintln!("Websocket close reason: {reason:?}");
                        break None;
                    }
                    Ok(Message::Ping(ping)) => tx.send_pong(ping).await?,
                    Ok(Message::Pong(_)) => (),
                    Err(err) => {
                        eprintln!("Websocket Error: {err:?}");

                        let code = match err {
                            ws::ReadMessageError::Io(err) => return Err(err),
                            ws::ReadMessageError::ReadFrameError(_)
                            | ws::ReadMessageError::MessageStartsWithContinuation
                            | ws::ReadMessageError::UnexpectedMessageStart => 1002,
                            ws::ReadMessageError::ReservedOpcode(_) => 1003,
                            ws::ReadMessageError::TextIsNotUtf8 => 1007,
                        };

                        break Some((code, "Websocket Error"));
                    }
                }
            }
        };

        tx.close(close_reason).await
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let (messages_tx, mut messages_rx) = tokio::sync::broadcast::channel(16);

    tokio::spawn(async move {
        loop {
            match messages_rx.recv().await {
                Ok(message) => println!("message: {message:?}"),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    println!("Lost {n} messages")
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let messages_tx = std::rc::Rc::new(messages_tx);

    let app = std::rc::Rc::new(
        picoserve::Router::new()
            .route(
                "/",
                get(|| picoserve::response::File::html(include_str!("index.html"))),
            )
            .nest_service("/static", {
                const STATIC_FILES: picoserve::response::Directory =
                    picoserve::response::Directory {
                        files: &[
                            (
                                "index.css",
                                picoserve::response::File::css(include_str!("index.css")),
                            ),
                            (
                                "index.js",
                                picoserve::response::File::css(include_str!("index.js")),
                            ),
                        ],
                        sub_directories: &[],
                    };

                STATIC_FILES
            })
            .route(
                "/index.css",
                get(|| picoserve::response::File::css(include_str!("index.css"))),
            )
            .route(
                "/index.js",
                get(|| picoserve::response::File::javascript(include_str!("index.js"))),
            )
            .route(
                "/ws",
                get(move |upgrade: ws::WebSocketUpgrade| {
                    if let Some(protocols) = upgrade.protocols() {
                        println!("Protocols:");
                        for protocol in protocols {
                            println!("\t{protocol}");
                        }
                    }

                    upgrade
                        .on_upgrade(WebsocketHandler {
                            tx: messages_tx.clone(),
                            rx: messages_tx.subscribe(),
                        })
                        .with_protocol("messages")
                }),
            ),
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
