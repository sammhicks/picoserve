//! Server-Sent Events. See [server_sent_events](https://github.com/sammhicks/picoserve/blob/main/examples/server_sent_events/src/main.rs) for usage example.

use core::future::Future;

use crate::io::{Read, Write};

use super::status;

/// Types which can be used as the data of an event.
pub trait EventData {
    async fn write_to<W: Write>(self, writer: &mut W) -> Result<(), W::Error>;
}

impl<'a> EventData for &'a str {
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
        self.writer.write_all(b"event:").await?;
        self.writer.write_all(event.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;

        self.writer.write_all(b"data:").await?;
        data.write_to(&mut self.writer).await?;
        self.writer.write_all(b"\n").await?;

        self.writer.write_all(b"\n").await?;

        self.writer.flush().await
    }
}

/// Implement this trait to generate events to send to the client.
pub trait EventSource {
    async fn write_events<W: Write>(self, writer: EventWriter<W>) -> Result<(), W::Error>;
}

/// A stream of Events sent by the server. Return an instance of this from the handler function.
pub struct EventStream<S: EventSource>(pub S);

impl<S: EventSource> EventStream<S> {
    /// Convert SSE stream into a [super::Response] with a status code of "OK"
    pub fn into_response(self) -> super::Response<impl super::HeadersIter, impl super::Body> {
        super::Response {
            status_code: status::OK,
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
        connection: super::Connection<R>,
        mut writer: W,
    ) -> Result<(), W::Error> {
        writer.flush().await?;

        let mut disconnection = core::pin::pin!(connection.wait_for_disconnection());
        let mut write_events = core::pin::pin!(self.0.write_events(EventWriter { writer }));

        core::future::poll_fn(|cx| match disconnection.as_mut().poll(cx) {
            core::task::Poll::Ready(result) => core::task::Poll::Ready(result),
            core::task::Poll::Pending => write_events.as_mut().poll(cx),
        })
        .await
    }
}

impl<S: EventSource> super::IntoResponse for EventStream<S> {
    async fn write_to<R: Read, W: super::ResponseWriter, WW: Write<Error = R::Error>>(
        self,
        writer: WW,
        connection: super::Connection<R>,
        response_writer: W,
    ) -> Result<crate::ResponseSent, R::Error> {
        response_writer.write_response(writer, connection, self.into_response()).await
    }
}

impl<S: EventSource> core::future::IntoFuture for EventStream<S> {
    type Output = Self;
    type IntoFuture = core::future::Ready<Self>;

    fn into_future(self) -> Self::IntoFuture {
        core::future::ready(self)
    }
}
