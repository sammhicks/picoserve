use std::time::Duration;

use picoserve::{
    response::chunked::{ChunkWriter, ChunkedResponse, Chunks, ChunksWritten},
    routing::get,
};

struct TextChunks {
    text: &'static str,
}

impl Chunks for TextChunks {
    fn content_type(&self) -> &'static str {
        "text/plain"
    }

    async fn write_chunks<W: picoserve::io::Write>(
        self,
        mut chunk_writer: ChunkWriter<W>,
    ) -> Result<ChunksWritten, W::Error> {
        for word in self.text.split_inclusive(char::is_whitespace) {
            chunk_writer.write_chunk(word.as_bytes()).await?;

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        chunk_writer.finalize().await
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let port = 8000;

    let app = std::rc::Rc::new(picoserve::Router::new().route(
        "/",
        get(|| async move {
            ChunkedResponse::new(TextChunks {
                text: "This is a chunked response\r\n",
            })
            .into_response()
            .with_header("x-header", "x-value")
        }),
    ));

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
