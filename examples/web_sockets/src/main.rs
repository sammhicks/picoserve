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
            let message = match rx
                .next_message(&mut message_buffer, messages_rx.recv())
                .await?
            {
                picoserve::futures::Either::First(Ok(message)) => message,
                picoserve::futures::Either::First(Err(error)) => {
                    eprintln!("Websocket error: {error:?}");
                    break Some((error.code(), "Websocket Error"));
                }
                picoserve::futures::Either::Second(message_changed) => match message_changed {
                    Ok(message) => {
                        tx.send_display(format_args!("Message: {message}")).await?;
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tx.send_display(format_args!("Missed {n} messages")).await?;
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break Some((1011, "Server has an error"));
                    }
                },
            };

            println!("Message: {message:?}");
            match message {
                Message::Text(new_message) => {
                    let _ = messages_tx.send(new_message.into());
                }
                Message::Binary(message) => {
                    println!("Ignoring binary message: {message:?}")
                }
                ws::Message::Close(reason) => {
                    eprintln!("Websocket close reason: {reason:?}");
                    break None;
                }
                Message::Ping(ping) => tx.send_pong(ping).await?,
                Message::Pong(_) => (),
            };
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
                get(async move |upgrade: ws::WebSocketUpgrade| {
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
