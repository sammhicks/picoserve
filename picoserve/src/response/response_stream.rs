use crate::io::{Read, Write};

pub(crate) struct ResponseSentCore(());

/// A marker showing that the response has been sent.
pub struct ResponseSent(pub(crate) ResponseSentCore);

pub(crate) struct ResponseStream<W: Write> {
    writer: W,
    connection_header: super::KeepAlive,
}

impl<W: Write> ResponseStream<W> {
    pub(crate) fn new(writer: W, connection_header: super::KeepAlive) -> Self {
        Self {
            writer,
            connection_header,
        }
    }
}

impl<W: Write> super::ResponseWriter for ResponseStream<W> {
    type Error = W::Error;

    async fn write_response<R: Read<Error = Self::Error>, H: super::HeadersIter, B: super::Body>(
        mut self,
        connection: super::Connection<'_, R>,
        super::Response {
            status_code,
            headers,
            body,
        }: super::Response<H, B>,
    ) -> Result<ResponseSent, Self::Error> {
        struct HeadersWriter<WW: Write> {
            writer: WW,
            connection_header: Option<super::KeepAlive>,
        }

        impl<WW: Write> super::ForEachHeader for HeadersWriter<WW> {
            type Output = ();
            type Error = WW::Error;

            async fn call<Value: core::fmt::Display>(
                &mut self,
                name: &str,
                value: Value,
            ) -> Result<(), Self::Error> {
                if name.eq_ignore_ascii_case("connection") {
                    self.connection_header = None;
                }
                write!(self.writer, "{name}: {value}\r\n").await
            }

            async fn finalize(mut self) -> Result<(), Self::Error> {
                if let Some(connection_header) = self.connection_header {
                    self.call("Connection", connection_header).await?;
                }

                Ok(())
            }
        }

        use crate::io::WriteExt;
        write!(self.writer, "HTTP/1.1 {status_code} \r\n").await?;

        headers
            .for_each_header(HeadersWriter {
                writer: &mut self.writer,
                connection_header: Some(self.connection_header),
            })
            .await?;

        self.writer.write_all(b"\r\n").await?;
        self.writer.flush().await?;

        body.write_response_body(connection, &mut self.writer)
            .await
            .map(|()| super::ResponseSent(ResponseSentCore(())))
    }
}
