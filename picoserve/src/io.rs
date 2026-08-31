//! IO Utility

use core::fmt;

pub use embedded_io_async::{
    self, Error, ErrorKind, ErrorType, Read, ReadExactError, Write as BaseWrite,
};

use crate::{
    mem::{BorrowedBuffer, BorrowedCursor},
    time::Timer,
};

/// An extension trait for [`Read`] which allows discarding of all incoming data until the client closes the connection.
pub trait ReadExt: Read {
    async fn discard_all_data(&mut self) -> Result<(), Self::Error> {
        let mut buffer = [0; 128];

        while self.read(&mut buffer).await? > 0 {}

        Ok(())
    }
}

impl<R: Read> ReadExt for R {}

pub(crate) enum FormatBufferWriteError {
    FormatError,
    OutOfSpace,
}

struct FormatBuffer<'data> {
    // The underlying buffer to write to.
    cursor: BorrowedCursor<'data>,
    // How many bytes to skip before starting to write into the buffer.
    skip_count: usize,
    /// The type of error that has occured if the formatter returns [`Err(core::fmt::Error`)](core::fmt::Error).
    error_state: FormatBufferWriteError,
}

impl fmt::Write for FormatBuffer<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if let Some(data_to_write) = s.as_bytes().get(self.skip_count..) {
            self.skip_count = 0;

            self.cursor.try_append(data_to_write).map_err(|_| {
                self.error_state = FormatBufferWriteError::OutOfSpace;
                fmt::Error
            })
        } else {
            self.skip_count -= s.len();

            Ok(())
        }
    }
}

impl<'data> FormatBuffer<'data> {
    fn new(cursor: BorrowedCursor<'data>, skip_count: usize) -> Self {
        Self {
            cursor,
            skip_count,
            error_state: FormatBufferWriteError::FormatError,
        }
    }

    fn write_fmt(mut self, args: core::fmt::Arguments<'_>) -> Result<(), FormatBufferWriteError> {
        core::fmt::write(&mut self, args).map_err(|fmt::Error| self.error_state)
    }
}

/// Async writer which can lend its write buffer.
pub trait Write: BaseWrite {
    /// Call f with the largest contiguous slice of octets in the transmit buffer, and enqueue the filled bytes.
    ///
    /// If the writer is not ready to accept data, it waits until it is.
    async fn write_with<F: FnOnce(BorrowedCursor<'_>) -> R, R>(
        &mut self,
        f: F,
    ) -> Result<R, Self::Error>;

    /// Write a formatted string into the writer. If the string cannot be written in one go, the string will be formatted multiple times.
    /// It's crucial that the same output is produced each time the string is formatted.
    async fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> Result<(), Self::Error> {
        let mut skip_count = 0;

        loop {
            return match self
                .write_with(|mut cursor| {
                    let remaining_capacity = cursor.remaining_capacity();

                    FormatBuffer::new(cursor.reborrow(), skip_count)
                        .write_fmt(args)
                        .inspect_err(|_| skip_count += remaining_capacity)
                })
                .await?
            {
                Ok(()) => Ok(()),
                Err(FormatBufferWriteError::FormatError) => {
                    log_warn!("Skipping writing due to Format Error");
                    Ok(())
                }
                Err(FormatBufferWriteError::OutOfSpace) => {
                    self.flush().await?;

                    continue;
                }
            };
        }
    }
}

impl<W: Write> Write for &mut W {
    fn write_with<F: FnOnce(BorrowedCursor<'_>) -> R, R>(
        &mut self,
        f: F,
    ) -> impl Future<Output = Result<R, Self::Error>> {
        W::write_with(self, f)
    }

    fn write_fmt(
        &mut self,
        args: fmt::Arguments<'_>,
    ) -> impl Future<Output = Result<(), Self::Error>> {
        W::write_fmt(self, args)
    }
}

#[cfg(test)]
impl Write for alloc::vec::Vec<u8> {
    async fn write_with<F: FnOnce(BorrowedCursor<'_>) -> R, R>(
        &mut self,
        f: F,
    ) -> Result<R, Self::Error> {
        let mut buffer = [0; 1024];
        let mut buffer = BorrowedBuffer::new(&mut buffer);

        let output = f(buffer.unfilled());

        self.extend_from_slice(buffer.filled());

        Ok(output)
    }
}

/// A connection socket, which can be split into its read and write half, and shut down when finished.
pub trait Socket<Runtime>: Sized {
    /// Error type of all the IO operations on this type.
    type Error: Error + 'static;

    /// The "read" half of the socket
    type ReadHalf<'a>: Read<Error = Self::Error>
    where
        Self: 'a;

    /// The "write" half of the socket
    type WriteHalf<'a>: Write<Error = Self::Error>
    where
        Self: 'a;

    /// Split the socket into its "read" and "write" half
    fn split(&mut self) -> (Self::ReadHalf<'_>, Self::WriteHalf<'_>);

    /// Abort the connection
    async fn abort<T: Timer<Runtime>>(
        self,
        timeouts: &crate::Timeouts,
        timer: &T,
    ) -> Result<(), super::Error<Self::Error>>;

    /// Perform a graceful shutdown
    async fn shutdown<T: Timer<Runtime>>(
        self,
        timeouts: &crate::Timeouts,
        timer: &T,
    ) -> Result<(), super::Error<Self::Error>>;
}

#[cfg(any(feature = "tokio", test))]
pub(crate) mod tokio_support {
    use super::{BaseWrite, Error, ErrorKind, ErrorType, Read, Write};

    #[derive(Debug, thiserror::Error)]
    #[error(transparent)]
    pub struct TokioIoError(pub std::io::Error);

    impl Error for TokioIoError {
        fn kind(&self) -> super::ErrorKind {
            ErrorKind::Other
        }
    }

    pub struct TokioIo<T>(pub T);

    impl<T> ErrorType for TokioIo<T> {
        type Error = TokioIoError;
    }

    impl<T: tokio::io::AsyncRead + Unpin> Read for TokioIo<T> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            use tokio::io::AsyncReadExt;
            self.0.read(buf).await.map_err(TokioIoError)
        }
    }

    impl BaseWrite for TokioIo<tokio::net::tcp::WriteHalf<'_>> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            use tokio::io::AsyncWriteExt;
            self.0.write(buf).await.map_err(TokioIoError)
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            use tokio::io::AsyncWriteExt;
            self.0.flush().await.map_err(TokioIoError)
        }
    }

    impl Write for TokioIo<tokio::net::tcp::WriteHalf<'_>> {
        async fn write_with<F: FnOnce(super::BorrowedCursor<'_>) -> R, R>(
            &mut self,
            f: F,
        ) -> Result<R, Self::Error> {
            use tokio::io::AsyncWriteExt;

            let mut buffer = [0; 1024];
            let mut buffer = super::BorrowedBuffer::new(&mut buffer);

            let output = f(buffer.unfilled());

            self.0
                .write_all(buffer.filled())
                .await
                .map(|()| output)
                .map_err(TokioIoError)
        }
    }

    impl super::Socket<crate::TokioRuntime> for tokio::net::TcpStream {
        type Error = TokioIoError;
        type ReadHalf<'a> = TokioIo<tokio::net::tcp::ReadHalf<'a>>;
        type WriteHalf<'a> = TokioIo<tokio::net::tcp::WriteHalf<'a>>;

        fn split(&mut self) -> (Self::ReadHalf<'_>, Self::WriteHalf<'_>) {
            let (read_half, write_half) = tokio::net::TcpStream::split(self);

            (TokioIo(read_half), TokioIo(write_half))
        }

        async fn abort<T: crate::Timer<crate::TokioRuntime>>(
            self,
            _timeouts: &crate::Timeouts,
            _timer: &T,
        ) -> Result<(), crate::Error<Self::Error>> {
            // Dropping a TcpStream closes it.

            Ok(())
        }

        async fn shutdown<T: crate::Timer<crate::TokioRuntime>>(
            mut self,
            timeouts: &crate::Timeouts,
            timer: &T,
        ) -> Result<(), crate::Error<Self::Error>> {
            timer
                .run_with_timeout(
                    timeouts.write,
                    tokio::io::AsyncWriteExt::shutdown(&mut self),
                )
                .await
                .map_err(crate::Error::WriteTimeout)?
                .map_err(|error| crate::Error::Write(TokioIoError(error)))?;

            let mut buffer = [0; 128];

            while timer
                .run_with_timeout(
                    timeouts.read_request,
                    tokio::io::AsyncReadExt::read(&mut self, &mut buffer),
                )
                .await
                .map_err(crate::Error::ReadTimeout)?
                .map_err(|error| crate::Error::Read(TokioIoError(error)))?
                > 0
            {}

            Ok(())
        }
    }
}

#[cfg(feature = "embassy")]
impl<'a, 'd> Write for embassy_net::tcp::TcpWriter<'a, 'd> {
    fn write_with<F: FnOnce(BorrowedCursor<'_>) -> R, R>(
        &mut self,
        f: F,
    ) -> impl Future<Output = Result<R, Self::Error>> {
        embassy_net::tcp::TcpWriter::write_with(self, |buffer| {
            let mut buffer = BorrowedBuffer::new(buffer);

            let output = f(buffer.unfilled());

            (buffer.len(), output)
        })
    }
}

#[cfg(feature = "embassy")]
impl<'a, 'd> Socket<super::EmbassyRuntime> for embassy_net::tcp::TcpSocket<'a, 'd> {
    type Error = embassy_net::tcp::Error;
    type ReadHalf<'h>
        = embassy_net::tcp::TcpReader<'h, 'd>
    where
        Self: 'h;
    type WriteHalf<'h>
        = embassy_net::tcp::TcpWriter<'h, 'd>
    where
        Self: 'h;

    fn split(&mut self) -> (Self::ReadHalf<'_>, Self::WriteHalf<'_>) {
        embassy_net::tcp::TcpSocket::split(self)
    }

    async fn abort<Timer: crate::Timer<super::EmbassyRuntime>>(
        mut self,
        timeouts: &crate::Timeouts,
        timer: &Timer,
    ) -> Result<(), crate::Error<Self::Error>> {
        log_info!("Abort");

        Self::abort(&mut self);

        // Send the abort
        timer
            .run_with_timeout(timeouts.write, self.flush())
            .await
            .map_err(crate::Error::WriteTimeout)?
            .map_err(crate::Error::Write)
    }

    async fn shutdown<Timer: crate::Timer<super::EmbassyRuntime>>(
        mut self,
        timeouts: &crate::Timeouts,
        timer: &Timer,
    ) -> Result<(), crate::Error<Self::Error>> {
        use futures_util::{FutureExt, TryFutureExt};

        self.close();

        let (mut rx, mut tx) = self.split();

        // Flush the write half until the read half has been closed by the client
        crate::futures::select(
            timer
                .run_with_timeout(timeouts.read_request, rx.discard_all_data())
                .map(|result| {
                    result
                        .map_err(crate::Error::ReadTimeout)?
                        .map_err(crate::Error::Read)
                }),
            tx.flush()
                .map_ok(crate::futures::ignore_output)
                .map_err(crate::Error::Write)
                .and_then(crate::futures::pend_forever),
        )
        .await?;

        // Flush the write half until the socket is closed.
        timer
            .run_with_timeout(timeouts.write, self.flush())
            .await
            .map_err(crate::Error::WriteTimeout)?
            .map_err(crate::Error::Write)
    }
}
