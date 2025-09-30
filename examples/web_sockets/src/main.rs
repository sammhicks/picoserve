use std::time::Duration;

use picoserve::{
    response::ws,
    routing::{get, get_service},
};

#[derive(Clone)]
struct AppState {
    messages_tx: tokio::sync::broadcast::Sender<String>,
}

struct WebsocketHandler;

impl ws::WebSocketCallbackWithState<AppState> for WebsocketHandler {
    async fn run_with_state<R: picoserve::io::Read, W: picoserve::io::Write<Error = R::Error>>(
        self,
        state: &AppState,
        mut rx: ws::SocketRx<R>,
        mut tx: ws::SocketTx<W>,
    ) -> Result<(), W::Error> {
        use picoserve::response::ws::Message;

        let messages_tx = &state.messages_tx;
        let mut messages_rx = state.messages_tx.subscribe();

        let mut message_buffer = [0; 128];

        let close_reason = loop {
            tokio::select! {
                message_changed = messages_rx.recv() => match message_changed {
                    Ok(message) => tx.send_display(format_args!("Message: {message}")).await?,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => tx.send_display(format_args!("Missed {n} messages")).await?,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break None,
                },
                new_message = rx.next_message(&mut message_buffer) => match new_message {
                    Ok(Message::Text(new_message)) => { let _ = messages_tx.send(new_message.into()); },
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

    let state = AppState { messages_tx };

    let app = std::rc::Rc::new(
        picoserve::Router::new()
            .route(
                "/",
                get_service(picoserve::response::File::html(include_str!("index.html"))),
            )
            .nest_service(
                "/static",
                const {
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
                        ..picoserve::response::Directory::DEFAULT
                    }
                },
            )
            .route(
                "/index.css",
                get_service(picoserve::response::File::css(include_str!("index.css"))),
            )
            .route(
                "/index.js",
                get_service(picoserve::response::File::javascript(include_str!(
                    "index.js"
                ))),
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
                        .on_upgrade_using_state(WebsocketHandler)
                        .with_protocol("messages")
                }),
            )
            .with_state(state),
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
