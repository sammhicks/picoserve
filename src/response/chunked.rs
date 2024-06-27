//! A Response broken up into chunks, allowing for a response of a size not known ahead of time.

/// A marker showing that all of the chunks have been written.
pub struct ChunksWritten(());

/// Writing chunks to a [ChunkWriter] will send them to the client and flush the stream
pub struct ChunkWriter<W: crate::io::Write> {
    writer: W,
}

impl<W: crate::io::Write> ChunkWriter<W> {
    /// Write a chunk to the client
    pub async fn write_chunk(&mut self, chunk: &[u8]) -> Result<(), W::Error> {
        use crate::io::WriteExt;

        if chunk.is_empty() {
            return Ok(());
        }

        write!(&mut self.writer, "{:x}\r\n", chunk.len()).await?;

        self.writer.write_all(chunk).await?;
        self.writer.write_all(b"\r\n").await?;

        self.writer.flush().await
    }

    /// Finish writing chunks
    pub async fn finalize(mut self) -> Result<ChunksWritten, W::Error> {
        self.writer.write_all(b"0\r\n\r\n").await?;

        Ok(ChunksWritten(()))
    }
}

/// A series of chunks forming the response body
pub trait Chunks {
    /// The Content Type of the response.
    fn content_type(&self) -> &'static str;

    /// Write the chunks to the [ChunkWriter] then finalize it.
    async fn write_chunks<W: crate::io::Write>(
        self,
        chunk_writer: ChunkWriter<W>,
    ) -> Result<ChunksWritten, W::Error>;
}

/// A response with a Chunked body. Implements [super::IntoResponse], so can be returned by handlers.
/// By default, it sends a status code of 200 (OK), to customise the response, call [into_response](Self::into_response),
/// which converts it into [response](super::Response) which can have the status code changed or headers added.
pub struct ChunkedResponse<C: Chunks> {
    chunks: C,
}

impl<C: Chunks> ChunkedResponse<C> {
    /// Create a response from [Chunks].
    pub fn new(chunks: C) -> Self {
        Self { chunks }
    }

    /// Convert the response into a [Response](super::Response), which can then have its status code changed or headers added.
    pub fn into_response(self) -> super::Response<impl super::HeadersIter, impl super::Body> {
        struct Body<C: Chunks>(C);

        impl<C: Chunks> super::Body for Body<C> {
            async fn write_response_body<
                R: embedded_io_async::Read,
                W: embedded_io_async::Write<Error = R::Error>,
            >(
                self,
                _connection: super::Connection<'_, R>,
                writer: W,
            ) -> Result<(), W::Error> {
                self.0
                    .write_chunks(ChunkWriter { writer })
                    .await
                    .map(|ChunksWritten(())| ())
            }
        }

        let content_type = self.chunks.content_type();

        super::Response {
            status_code: super::StatusCode::OK,
            headers: [
                ("Content-Type", content_type),
                ("Transfer-Encoding", "chunked"),
            ],
            body: Body(self.chunks),
        }
    }
}

impl<C: Chunks> super::IntoResponse for ChunkedResponse<C> {
    async fn write_to<R: embedded_io_async::Read, W: super::ResponseWriter<Error = R::Error>>(
        self,
        connection: super::Connection<'_, R>,
        response_writer: W,
    ) -> Result<crate::ResponseSent, W::Error> {
        response_writer
            .write_response(connection, self.into_response())
            .await
    }
}
