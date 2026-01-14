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
    pretty_env_logger::init();

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

    let socket = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;

    log::info!("http://localhost:{port}/");

    tokio::task::LocalSet::new()
        .run_until(async {
            loop {
                let (stream, remote_address) = socket.accept().await?;

                log::info!("Connection from {remote_address}");

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
                            log::info!(
                                "{handled_requests_count} requests handled from {remote_address}"
                            )
                        }
                        Err(error) => {
                            log::error!("Error handling requests from {remote_address}: {error}")
                        }
                    }
                });
            }
        })
        .await
}
