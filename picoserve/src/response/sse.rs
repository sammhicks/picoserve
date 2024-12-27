//! Server-Sent Events. See [server_sent_events](https://github.com/sammhicks/picoserve/blob/main/examples/server_sent_events/src/main.rs) for usage example.

use crate::io::{Read, Write, WriteExt};

use super::StatusCode;

/// Types which can be used as the data of an event.
pub trait EventData {
    /// Write event data to the socket.
    async fn write_to<W: Write>(self, writer: &mut W) -> Result<(), W::Error>;
}

impl<'a> EventData for core::fmt::Arguments<'a> {
    async fn write_to<W: Write>(self, writer: &mut W) -> Result<(), W::Error> {
        writer.write_fmt(self).await
    }
}

impl EventData for &str {
    async fn write_to<W: Write>(self, writer: &mut W) -> Result<(), W::Error> {
        writer.write_all(self.as_bytes()).await
    }
}

impl<T: serde::Serialize> EventData for super::json::Json<T> {
    async fn write_to<W: Write>(self, writer: &mut W) -> Result<(), W::Error> {
        self.do_write_to(writer).await
    }
}

/// Writing events to an [EventWriter] will send the events to the client.
pub struct EventWriter<W: Write> {
    writer: W,
}

impl<W: Write> EventWriter<W> {
    /// Send an event with an empty name, keeping the connection alive.
    pub async fn write_keepalive(&mut self) -> Result<(), W::Error> {
        self.writer.write_all(b":\n\n").await?;

        self.writer.flush().await
    }

    /// Send an event with a given name and data.
    pub async fn write_event<T: EventData>(
        &mut self,
        event: &str,
        data: T,
    ) -> Result<(), W::Error> {
        pub struct DataWriter<W: Write> {
            writer: W,
        }

        impl<W: Write> embedded_io_async::ErrorType for DataWriter<W> {
            type Error = W::Error;
        }

        impl<W: Write> Write for DataWriter<W> {
            async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
                for line in buf.split_inclusive(|&b| b == b'\n') {
                    self.writer.write_all(b"data:").await?;
                    self.writer.write_all(line).await?;
                }

                self.writer.write_all(b"\n").await?;

                Ok(buf.len())
            }

            async fn flush(&mut self) -> Result<(), Self::Error> {
                self.writer.flush().await
            }
        }

        self.writer.write_all(b"event:").await?;
        self.writer.write_all(event.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;

        data.write_to(&mut DataWriter {
            writer: &mut self.writer,
        })
        .await?;

        self.writer.write_all(b"\n").await?;

        self.writer.flush().await
    }
}

/// Implement this trait to generate events to send to the client.
pub trait EventSource {
    /// Produce a stream of events and write them to `writer`
    async fn write_events<W: Write>(self, writer: EventWriter<W>) -> Result<(), W::Error>;
}

/// A stream of Events sent by the server. Return an instance of this from the handler function.
pub struct EventStream<S: EventSource>(pub S);

impl<S: EventSource> EventStream<S> {
    /// Convert SSE stream into a [super::Response] with a status code of "OK"
    pub fn into_response(self) -> super::Response<impl super::HeadersIter, impl super::Body> {
        super::Response {
            status_code: StatusCode::OK,
            headers: [
                ("Cache-Control", "no-cache"),
                ("Content-Type", "text/event-stream"),
            ],
            body: self,
        }
    }
}

impl<S: EventSource> super::Body for EventStream<S> {
    async fn write_response_body<R: Read, W: Write<Error = R::Error>>(
        self,
        connection: super::Connection<'_, R>,
        mut writer: W,
    ) -> Result<(), W::Error> {
        writer.flush().await?;

        connection
            .run_until_disconnection((), self.0.write_events(EventWriter { writer }))
            .await
    }
}

impl<S: EventSource> super::IntoResponse for EventStream<S> {
    async fn write_to<R: Read, W: super::ResponseWriter<Error = R::Error>>(
        self,
        connection: super::Connection<'_, R>,
        response_writer: W,
    ) -> Result<crate::ResponseSent, W::Error> {
        response_writer
            .write_response(connection, self.into_response())
            .await
    }
}

impl<S: EventSource> core::future::IntoFuture for EventStream<S> {
    type Output = Self;
    type IntoFuture = core::future::Ready<Self>;

    fn into_future(self) -> Self::IntoFuture {
        core::future::ready(self)
    }
}
