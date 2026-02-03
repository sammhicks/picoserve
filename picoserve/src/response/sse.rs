//! Server-Sent Events. See [server_sent_events](https://github.com/sammhicks/picoserve/blob/main/examples/server_sent_events/src/main.rs) for usage example.

use crate::io::{Read, Write, WriteExt};

use super::StatusCode;

/// Types which can be used as the data of an event.
pub trait EventData {
    /// Write event data to the socket.
    async fn write_to<W: Write>(self, writer: &mut W) -> Result<(), W::Error>;
}

impl EventData for core::fmt::Arguments<'_> {
    async fn write_to<W: Write>(self, writer: &mut W) -> Result<(), W::Error> {
        writer.write_fmt(self).await
    }
}

impl EventData for &str {
    async fn write_to<W: Write>(self, writer: &mut W) -> Result<(), W::Error> {
        writer.write_all(self.as_bytes()).await
    }
}

#[cfg(feature = "json")]
impl<T: serde::Serialize> EventData for super::json::Json<T> {
    async fn write_to<W: Write>(self, writer: &mut W) -> Result<(), W::Error> {
        self.do_write_to(writer).await
    }
}

struct EventWriterState {
    is_currently_writing_event: core::cell::Cell<bool>,
    is_running: core::cell::Cell<bool>,
}

impl EventWriterState {
    fn new() -> Self {
        Self {
            is_currently_writing_event: false.into(),
            is_running: true.into(),
        }
    }
}

/// Writing events to an [`EventWriter`] will send the events to the client.
pub struct EventWriter<'a, W: Write> {
    writer: W,
    event_writer_state: &'a EventWriterState,
}

impl<W: Write> EventWriter<'_, W> {
    async fn do_write<F: core::future::Future>(
        event_writer_state: &EventWriterState,
        write_task: F,
    ) -> F::Output {
        event_writer_state.is_currently_writing_event.set(true);

        let result = write_task.await;

        event_writer_state.is_currently_writing_event.set(false);

        // If the connection was shutting down, block writing suspend the task to allow `write_events_until_shutdown` to terminate.
        if !event_writer_state.is_running.get() {
            return core::future::pending().await;
        };

        result
    }

    /// Send an event with an empty name, keeping the connection alive.
    pub async fn write_keepalive(&mut self) -> Result<(), W::Error> {
        Self::do_write(self.event_writer_state, async {
            self.writer.write_all(b":\n\n").await?;

            self.writer.flush().await
        })
        .await
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

        impl<W: Write> crate::io::ErrorType for DataWriter<W> {
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

        Self::do_write(self.event_writer_state, async {
            self.writer.write_all(b"event:").await?;
            self.writer.write_all(event.as_bytes()).await?;
            self.writer.write_all(b"\n").await?;

            data.write_to(&mut DataWriter {
                writer: &mut self.writer,
            })
            .await?;

            self.writer.write_all(b"\n").await?;

            self.writer.flush().await
        })
        .await
    }
}

async fn write_events_until_shutdown<E, F: core::future::Future<Output = Result<(), E>>>(
    event_writer_state: &EventWriterState,
    shutdown_signal: impl core::future::Future<Output = ()> + Unpin,
    mut write_events: core::pin::Pin<&mut F>,
) -> Result<(), E> {
    let shutdown_task = async {
        shutdown_signal.await;
        event_writer_state.is_running.set(false);

        core::future::pending().await
    };

    let write_events_task = core::future::poll_fn(|cx| {
        use core::task::Poll;

        if event_writer_state.is_running.get() {
            return write_events.as_mut().poll(cx);
        }

        if !event_writer_state.is_currently_writing_event.get() {
            return Poll::Ready(Ok(()));
        }

        if let Poll::Ready(result) = write_events.as_mut().poll(cx) {
            return Poll::Ready(result);
        }

        if !event_writer_state.is_currently_writing_event.get() {
            return Poll::Ready(Ok(()));
        }

        Poll::Pending
    });

    crate::futures::select(shutdown_task, write_events_task).await
}

/// Implement this trait to generate events to send to the client.
pub trait EventSource {
    /// Produce a stream of events and write them to `writer`
    async fn write_events<W: Write>(self, writer: EventWriter<W>) -> Result<(), W::Error>;
}

/// A stream of Events sent by the server. Return an instance of this from the handler function.
pub struct EventStream<S: EventSource>(pub S);

impl<S: EventSource> EventStream<S> {
    /// Convert SSE stream into a [`Response`](super::Response) with a status code of "OK"
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

        let shutdown_signal = connection.shutdown_signal.clone();

        let event_writer_state = &EventWriterState::new();

        let write_events = core::pin::pin!(connection.run_until_disconnection(
            (),
            self.0.write_events(EventWriter {
                writer,
                event_writer_state
            })
        ));

        write_events_until_shutdown(event_writer_state, shutdown_signal, write_events).await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct TestEventSource {
        event: &'static str,
        data: &'static str,
        write_count: usize,
    }

    impl TestEventSource {
        fn with_write_count(mut self, write_count: usize) -> Self {
            self.write_count = write_count;
            self
        }
    }

    impl EventSource for TestEventSource {
        async fn write_events<W: Write>(
            self,
            mut writer: EventWriter<'_, W>,
        ) -> Result<(), W::Error> {
            for _ in 0..self.write_count {
                writer.write_event(self.event, self.data).await?;
            }

            Ok(())
        }
    }

    struct CountWriteSize(usize);

    impl crate::io::ErrorType for CountWriteSize {
        type Error = core::convert::Infallible;
    }

    impl Write for CountWriteSize {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            let write_size = buf.len();

            self.0 += write_size;

            Ok(write_size)
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    struct ThrottledWriter {
        write_size: usize,
    }

    impl crate::io::ErrorType for ThrottledWriter {
        type Error = core::convert::Infallible;
    }

    impl Write for ThrottledWriter {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            if buf.is_empty() {
                Ok(0)
            } else {
                self.write_size += 1;

                tokio::task::yield_now().await;

                Ok(1)
            }
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[tokio::test]
    #[ntest::timeout(1000)]
    async fn wait_event_to_finish_writing() {
        use futures_util::FutureExt;

        let (shutdown_signal_tx, shutdown_signal_rx) = tokio::sync::oneshot::channel::<()>();

        let event_writer_state = &EventWriterState::new();

        let source = TestEventSource {
            event: "test",
            data: "test",
            write_count: 1,
        };

        let write_size = {
            let mut count_write_size = CountWriteSize(0);

            let _ = source
                .clone()
                .write_events(EventWriter {
                    writer: &mut count_write_size,
                    event_writer_state,
                })
                .await;

            count_write_size.0
        };

        assert!(!event_writer_state.is_currently_writing_event.get());
        assert!(event_writer_state.is_running.get());

        let mut throttle_writer = ThrottledWriter { write_size: 0 };

        let write_events = async {
            source
                .with_write_count(3)
                .write_events(EventWriter {
                    writer: &mut throttle_writer,
                    event_writer_state,
                })
                .await
        };

        {
            let task_shutdown_signal = core::pin::pin!(async {
                let _ = shutdown_signal_rx.await;
            });

            let task_write_events = core::pin::pin!(write_events);

            let mut task = core::pin::pin!(write_events_until_shutdown(
                event_writer_state,
                task_shutdown_signal,
                task_write_events,
            ));

            for _ in 0..3 {
                assert_eq!(task.as_mut().now_or_never(), None);
            }

            let _ = shutdown_signal_tx.send(());

            let _ = task.await;
        }

        assert_eq!(throttle_writer.write_size, write_size);
    }
}
