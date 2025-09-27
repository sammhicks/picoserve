use std::time::Duration;

use picoserve::{
    response::{Content, ContentUsingState},
    routing::get,
};

struct AppState {
    key: u8,
}

struct EncryptedContent {
    message: &'static str,
}

impl picoserve::response::ContentUsingState<AppState> for EncryptedContent {
    fn content_type(&self, _state: &AppState) -> &'static str {
        self.message.content_type()
    }

    fn content_length(&self, _state: &AppState) -> usize {
        self.message.content_length()
    }

    async fn write_content<W: picoserve::io::Write>(
        self,
        state: &AppState,
        mut writer: W,
    ) -> Result<(), W::Error> {
        let rotate = |d: u8| (d + (state.key % 26)) % 26;

        for decoded_byte in self.message.bytes() {
            let encoded_byte = if decoded_byte.is_ascii_lowercase() {
                rotate(decoded_byte - b'a') + b'a'
            } else if decoded_byte.is_ascii_uppercase() {
                rotate(decoded_byte - b'A') + b'A'
            } else {
                decoded_byte
            };

            writer.write_all(&[encoded_byte]).await?;
        }

        Ok(())
    }
}

struct EncryptedMessage {
    message: &'static str,
}

impl picoserve::response::IntoResponseWithState<AppState> for EncryptedMessage {
    async fn write_to_with_state<
        R: picoserve::io::Read,
        W: picoserve::response::ResponseWriter<Error = R::Error>,
    >(
        self,
        state: &AppState,
        connection: picoserve::response::Connection<'_, R>,
        response_writer: W,
    ) -> Result<picoserve::ResponseSent, W::Error> {
        use picoserve::response::IntoResponse;

        (
            ("X-Encrypted", "true"),
            EncryptedContent {
                message: self.message,
            }
            .using_state(state),
        )
            .write_to(connection, response_writer)
            .await
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let app = std::rc::Rc::new(
        picoserve::Router::new()
            .route(
                "/",
                get(|| async { (("X-Encrypted", "false"), "Hello World") }),
            )
            .route(
                "/encrypted",
                get(|| async {
                    EncryptedMessage {
                        message: "Hello World",
                    }
                }),
            )
            .with_state(AppState { key: 13 }),
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
