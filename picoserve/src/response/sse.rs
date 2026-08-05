//! Server-Sent Events. See [server_sent_events](https://github.com/sammhicks/picoserve/blob/main/examples/server_sent_events/src/main.rs) for usage example.

use futures_util::FutureExt;

use core::future::Future;

use crate::{
    futures::ThenPendForever,
    io::{BaseWrite, Read, Write},
};

use super::StatusCode;

struct EventDataWriterCore<W: Write> {
    is_at_start_of_line: bool,
    writer: W,
}

impl<W: Write> EventDataWriterCore<W> {
    async fn write_start_of_line_if_needed(&mut self) -> Result<(), W::Error> {
        if self.is_at_start_of_line {
            self.writer.write_all(b"data:").await?;

            self.is_at_start_of_line = false;
        }

        Ok(())
    }

    async fn finalize(mut self) -> Result<(), W::Error> {
        self.writer
            .write_all(if self.is_at_start_of_line {
                b"\n"
            } else {
                b"\n\n"
            })
            .await
    }
}

impl<W: Write> crate::io::ErrorType for EventDataWriterCore<W> {
    type Error = W::Error;
}

impl<W: Write> BaseWrite for EventDataWriterCore<W> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }

        for line in buf.split_inclusive(|&b| b == b'\n') {
            self.write_start_of_line_if_needed().await?;

            self.writer.write_all(line).await?;

            self.is_at_start_of_line = line.ends_with(b"\n");
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> impl Future<Output = Result<(), Self::Error>> {
        self.writer.flush()
    }
}

impl<W: Write> Write for EventDataWriterCore<W> {
    async fn write_with<F: FnOnce(&mut [u8]) -> (usize, R), R>(
        &mut self,
        f: F,
    ) -> Result<R, Self::Error> {
        let mut buffer = [0; 128];

        let (write_size, output) = f(&mut buffer);

        self.write_all(&buffer[..(write_size.min(buffer.len()))])
            .await?;

        Ok(output)
    }
}

/// A [`Write`](BaseWrite)r which can also write [`Arguments`](core::fmt::Arguments) and thus be the writer in [`write!`](core::fmt::write).
pub struct EventDataWriter<W: Write> {
    core: EventDataWriterCore<W>,
}

impl<W: Write> EventDataWriter<W> {
    fn finalize(self) -> impl Future<Output = Result<(), W::Error>> {
        self.core.finalize()
    }

    /// Write `s` as event data.
    pub fn write<'a>(&'a mut self, s: &'a str) -> impl Future<Output = Result<(), W::Error>> + 'a {
        self.core.write_all(s.as_bytes())
    }

    /// Write `args` as event data, allowing `EventDataWriter` to be used in [`write!`](core::fmt::write).
    pub fn write_fmt<'a>(
        &'a mut self,
        args: core::fmt::Arguments<'a>,
    ) -> impl Future<Output = Result<(), W::Error>> + 'a {
        self.core.write_fmt(args)
    }

    /// Format `value` as event data.
    pub async fn write_display(&mut self, value: impl core::fmt::Display) -> Result<(), W::Error> {
        write!(self, "{value}").await
    }

    #[cfg(feature = "json")]
    /// Encode `value` as JSON and write it as event data.
    pub async fn write_json<T: serde::Serialize>(&mut self, value: T) -> Result<(), W::Error> {
        self.core.write_start_of_line_if_needed().await?;
        write!(self.core.writer, "{}", crate::json::Json(value).display()).await
    }
}

/// Types which can be used as the data of an event.
pub trait EventData {
    /// Write event data to the socket.
    async fn write_to<W: Write>(self, writer: &mut EventDataWriter<W>) -> Result<(), W::Error>;
}

impl<'a> EventData for core::fmt::Arguments<'a> {
    async fn write_to<W: Write>(self, writer: &mut EventDataWriter<W>) -> Result<(), W::Error> {
        writer.write_fmt(self).await
    }
}

impl EventData for &str {
    async fn write_to<W: Write>(self, writer: &mut EventDataWriter<W>) -> Result<(), W::Error> {
        writer.write(self).await
    }
}

#[cfg(feature = "json")]
impl<T: serde::Serialize> EventData for super::json::Json<T> {
    fn write_to<W: Write>(
        self,
        writer: &mut EventDataWriter<W>,
    ) -> impl Future<Output = Result<(), W::Error>> {
        writer.write_json(self.0)
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
    async fn do_write<F: Future>(
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
        Self::do_write(self.event_writer_state, async {
            self.writer.write_all(b"event:").await?;
            self.writer.write_all(event.as_bytes()).await?;
            self.writer.write_all(b"\ndata:").await?;

            let mut event_data_writer = EventDataWriter {
                core: EventDataWriterCore {
                    is_at_start_of_line: false,
                    writer: &mut self.writer,
                },
            };

            data.write_to(&mut event_data_writer).await?;

            event_data_writer.finalize().await
        })
        .await
    }

    /// Flush buffered written events.
    pub fn flush(&mut self) -> impl Future<Output = Result<(), W::Error>> + '_ {
        self.writer.flush()
    }
}

async fn write_events_until_shutdown<E, F: Future<Output = Result<(), E>>>(
    event_writer_state: &EventWriterState,
    shutdown_signal: impl Future<Output = ()> + Unpin,
    mut write_events: core::pin::Pin<&mut F>,
) -> Result<(), E> {
    let shutdown_task = shutdown_signal
        .map(|()| event_writer_state.is_running.set(false))
        .map(crate::futures::IgnoredOutput::new)
        .then_pend_forever();

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
    use crate::io;

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

    impl io::ErrorType for CountWriteSize {
        type Error = core::convert::Infallible;
    }

    impl io::BaseWrite for CountWriteSize {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            let write_size = buf.len();

            self.0 += write_size;

            Ok(write_size)
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    impl Write for CountWriteSize {
        async fn write_with<F: FnOnce(&mut [u8]) -> (usize, R), R>(
            &mut self,
            f: F,
        ) -> Result<R, Self::Error> {
            let (write_size, output) = f(&mut [0; 1024]);

            self.0 += write_size;

            Ok(output)
        }
    }

    struct ThrottledWriter {
        write_size: usize,
    }

    impl io::ErrorType for ThrottledWriter {
        type Error = core::convert::Infallible;
    }

    impl io::BaseWrite for ThrottledWriter {
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

    impl io::Write for ThrottledWriter {
        async fn write_with<F: FnOnce(&mut [u8]) -> (usize, R), R>(
            &mut self,
            f: F,
        ) -> Result<R, Self::Error> {
            let (write_size, output) = f(&mut [0]);

            self.write_size += write_size;

            Ok(output)
        }
    }

    fn verify_sse_output(mut expected: &str, actual_with_headers: &[u8]) {
        let (&eol, actual_with_headers) = actual_with_headers.split_last().unwrap();

        assert_eq!(eol, b'\n');

        if let Some(&last_eol) = actual_with_headers.last() {
            assert_eq!(last_eol, b'\n');
        }

        let actual_with_headers =
            core::str::from_utf8(actual_with_headers).expect("SSE output must be UTF-8");

        assert!(!actual_with_headers.is_empty());

        for line in actual_with_headers.split_inclusive('\n').map(|line| {
            line.strip_prefix("data:").unwrap_or_else(|| {
                panic!(
                    r#"SSE line does not start with "data:": {:?} - {:?}"#,
                    line, actual_with_headers
                )
            })
        }) {
            expected = expected.strip_prefix(line).unwrap_or_else(|| {
                panic!(
                    "SSE Test does not match; Line: {:?}; Remaining Expected: {:?}",
                    line, expected
                )
            });
        }
    }

    #[tokio::test]
    async fn sse_correctly_handle_newlines() {
        crate::tests::fuzz::run_async("correctly_handle_newlines", async |test_data| {
            let mut buffer = std::vec::Vec::from(b"data:");

            let mut writer = super::EventDataWriter {
                core: EventDataWriterCore {
                    is_at_start_of_line: false,
                    writer: &mut buffer,
                },
            };

            let mut entire_generated_string = std::string::String::new();

            for _ in 0..test_data.generate_value_with_parameter::<usize, _>(1..=30) {
                #[derive(strum::VariantArray)]
                enum WriteType {
                    Write,
                    WriteDisplay,
                }

                match test_data.choose_value(strum::VariantArray::VARIANTS) {
                    WriteType::Write => {
                        let mut new_string = test_data.generate_string(10..100);

                        if test_data.generate_value() {
                            new_string.push('\n');
                        }

                        entire_generated_string.push_str(&new_string);

                        Ok(()) = writer.write(&new_string).await;
                    }
                    WriteType::WriteDisplay => {
                        let replayable_test_data = test_data.generate_replayable();

                        let item = core::fmt::from_fn(|f| {
                            let mut test_data = replayable_test_data.start();

                            for _ in 0..test_data.generate_value_with_parameter::<usize, _>(0..10) {
                                f.write_str(&test_data.generate_string(1..10))?;
                            }

                            if test_data.generate_value() {
                                f.write_str("\n")?;
                            }

                            Ok(())
                        });

                        entire_generated_string.push_str(&std::string::ToString::to_string(&item));

                        Ok(()) = writer.write_display(item).await;
                    }
                }
            }

            if !entire_generated_string.ends_with('\n') {
                entire_generated_string.push('\n');
            }

            Ok(()) = writer.finalize().await;

            verify_sse_output(&entire_generated_string, &buffer);
        })
        .await
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

            source
                .clone()
                .write_events(EventWriter {
                    writer: &mut count_write_size,
                    event_writer_state,
                })
                .await
                .unwrap();

            count_write_size.0
        };

        assert!(!event_writer_state.is_currently_writing_event.get());
        assert!(event_writer_state.is_running.get());

        let mut throttle_writer = ThrottledWriter { write_size: 0 };

        let write_events = source.with_write_count(3).write_events(EventWriter {
            writer: &mut throttle_writer,
            event_writer_state,
        });

        {
            let task_shutdown_signal = core::pin::pin!(async {
                // Ignore if the channel is closed
                _ = shutdown_signal_rx.await;
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

            // Ignore if the channel is closed
            _ = shutdown_signal_tx.send(());

            Ok(()) = task.await;
        }

        assert_eq!(throttle_writer.write_size, write_size);
    }
}
