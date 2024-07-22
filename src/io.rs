//! IO Utility

use core::fmt;

pub use embedded_io_async::{self, Error, ErrorKind, ErrorType, Read, Write};

pub trait ReadExt: Read {
    async fn discard_all_data(&mut self) -> Result<(), Self::Error> {
        let mut buffer = [0; 128];

        while self.read(&mut buffer).await? > 0 {}

        Ok(())
    }
}

impl<R: Read> ReadExt for R {}

pub(crate) enum FormatBufferWriteError<T> {
    FormatError,
    OutOfSpace(T),
}

pub(crate) struct FormatBuffer {
    pub data: heapless::Vec<u8, 128>,
    pub ignore_count: usize,
    pub error_state: FormatBufferWriteError<()>,
}

impl fmt::Write for FormatBuffer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &b in s.as_bytes() {
            match self.ignore_count.checked_sub(1) {
                Some(ignore_count) => self.ignore_count = ignore_count,
                None => {
                    if self.data.push(b).is_err() {
                        self.error_state = FormatBufferWriteError::OutOfSpace(());
                        return Err(fmt::Error);
                    }
                }
            }
        }

        Ok(())
    }
}

impl FormatBuffer {
    pub fn new(ignore_count: usize) -> Self {
        Self {
            data: heapless::Vec::new(),
            ignore_count,
            error_state: FormatBufferWriteError::FormatError,
        }
    }

    pub fn write(
        &mut self,
        value: impl fmt::Display,
    ) -> Result<&[u8], FormatBufferWriteError<&[u8]>> {
        use fmt::Write;
        write!(self, "{value}")
            .map(|()| self.data.as_slice())
            .map_err(|fmt::Error| match self.error_state {
                FormatBufferWriteError::FormatError => FormatBufferWriteError::FormatError,
                FormatBufferWriteError::OutOfSpace(()) => {
                    FormatBufferWriteError::OutOfSpace(self.data.as_slice())
                }
            })
    }
}

/// An extension trait for [Write] which allows writing of [core::fmt::Arguments].
pub trait WriteExt: Write {
    /// Write a formatted string into the writer. If the string cannot be written in one go, the string might be formatted multiple times.
    /// It's crucial that the same output is produced each time the string is formatted.
    async fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> Result<(), Self::Error> {
        let mut ignore_count = 0;

        loop {
            match FormatBuffer::new(ignore_count).write(args) {
                Ok(data) => return self.write_all(data).await,
                Err(FormatBufferWriteError::FormatError) => {
                    log_warn!("Skipping writing due to Format Error");
                    return Ok(());
                }
                Err(FormatBufferWriteError::OutOfSpace(data)) => {
                    self.write_all(data).await?;
                    ignore_count += data.len();
                }
            }
        }
    }
}

impl<W: Write> WriteExt for W {}

/// A connection socket, which can be split into its read and write half, and shut down when finished.
pub trait Socket: Sized {
    /// Error type of all the IO operations on this type.
    type Error: embedded_io_async::Error;

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

    /// Perform a graceful shutdown
    async fn shutdown<Timer: crate::Timer>(
        self,
        timeouts: &crate::Timeouts<Timer::Duration>,
        timer: &mut Timer,
    ) -> Result<(), super::Error<Self::Error>>;
}

#[cfg(any(feature = "tokio", test))]
pub(crate) mod tokio_support {
    use embedded_io_async::{Error, ErrorKind, ErrorType, Read, Write};

    #[derive(Debug)]
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

    impl<T: tokio::io::AsyncWrite + Unpin> Write for TokioIo<T> {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            use tokio::io::AsyncWriteExt;
            self.0.write(buf).await.map_err(TokioIoError)
        }
    }

    impl super::Socket for tokio::net::TcpStream {
        type Error = TokioIoError;
        type ReadHalf<'a> = TokioIo<tokio::net::tcp::ReadHalf<'a>>;
        type WriteHalf<'a> = TokioIo<tokio::net::tcp::WriteHalf<'a>>;

        fn split(&mut self) -> (Self::ReadHalf<'_>, Self::WriteHalf<'_>) {
            let (read_half, write_half) = tokio::net::TcpStream::split(self);

            (TokioIo(read_half), TokioIo(write_half))
        }

        async fn shutdown<Timer: crate::Timer>(
            mut self,
            timeouts: &crate::Timeouts<Timer::Duration>,
            timer: &mut Timer,
        ) -> Result<(), crate::Error<Self::Error>> {
            use crate::time::TimerExt;

            timer
                .run_with_maybe_timeout(
                    timeouts.write.clone(),
                    tokio::io::AsyncWriteExt::shutdown(&mut self),
                )
                .await
                .map_err(|_err| crate::Error::WriteTimeout)?
                .map_err(|err| crate::Error::Write(TokioIoError(err)))?;

            let mut buffer = [0; 128];

            while timer
                .run_with_maybe_timeout(
                    timeouts.read_request.clone(),
                    tokio::io::AsyncReadExt::read(&mut self, &mut buffer),
                )
                .await
                .map_err(|_err| crate::Error::ReadTimeout)?
                .map_err(|err| crate::Error::Read(TokioIoError(err)))?
                > 0
            {}

            Ok(())
        }
    }
}

#[cfg(feature = "embassy")]
impl<'s> Socket for embassy_net::tcp::TcpSocket<'s> {
    type Error = embassy_net::tcp::Error;
    type ReadHalf<'a> = embassy_net::tcp::TcpReader<'a> where 's: 'a;
    type WriteHalf<'a> = embassy_net::tcp::TcpWriter<'a> where 's: 'a;

    fn split(&mut self) -> (Self::ReadHalf<'_>, Self::WriteHalf<'_>) {
        embassy_net::tcp::TcpSocket::split(self)
    }

    async fn shutdown<Timer: crate::Timer>(
        mut self,
        timeouts: &crate::Timeouts<Timer::Duration>,
        timer: &mut Timer,
    ) -> Result<(), crate::Error<Self::Error>> {
        use crate::time::TimerExt;

        self.close();

        let (mut rx, mut tx) = self.split();

        // Flush the write half until the read half has been closed by the client
        futures_util::future::select(
            core::pin::pin!(async {
                timer
                    .run_with_maybe_timeout(timeouts.read_request.clone(), rx.discard_all_data())
                    .await
                    .map_err(|_err| crate::Error::ReadTimeout)?
                    .map_err(crate::Error::Read)
            }),
            core::pin::pin!(async {
                tx.flush().await.map_err(crate::Error::Write)?;
                core::future::pending().await
            }),
        )
        .await
        .factor_first()
        .0?;

        // Flush the write half until the socket is closed.
        // `embassy_net::tcp::TcpSocket` (possibly erroniously) keeps trying to flush until the tx buffer is empty,
        // but we don't care at this point if data is lost
        timer
            .run_with_maybe_timeout(
                timeouts.write.clone(),
                core::future::poll_fn(|cx| {
                    use core::future::Future;

                    if self.state() == embassy_net::tcp::State::Closed {
                        core::task::Poll::Ready(Ok(()))
                    } else {
                        core::pin::pin!(self.flush()).poll(cx)
                    }
                }),
            )
            .await
            .map_err(|_err| crate::Error::WriteTimeout)?
            .map_err(crate::Error::Write)
    }
}
