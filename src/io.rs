use core::fmt;

pub use embedded_io_async::{self, Error, ErrorKind, ErrorType, Read, Write, WriteAllError};

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
    ) -> Result<&str, FormatBufferWriteError<&[u8]>> {
        use fmt::Write;
        write!(self, "{value}")
            .map(|()| {
                // Safety: We've just written UTF8 data
                unsafe { core::str::from_utf8_unchecked(self.data.as_slice()) }
            })
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
    async fn write_fmt(
        &mut self,
        args: fmt::Arguments<'_>,
    ) -> Result<(), WriteAllError<Self::Error>> {
        let mut ignore_count = 0;

        loop {
            match FormatBuffer::new(ignore_count).write(args) {
                Ok(data) => return self.write_all(data.as_bytes()).await,
                Err(FormatBufferWriteError::FormatError) => return Ok(()),
                Err(FormatBufferWriteError::OutOfSpace(data)) => {
                    self.write_all(data).await?;
                    ignore_count += data.len();
                }
            }
        }
    }
}

impl<W: Write> WriteExt for W {}

#[cfg(feature = "tokio")]
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
}
