//! [Timer] for creating timeouts during request parsing and request handling.

pub trait Timer {
    type Duration: Clone;
    type TimeoutError;

    async fn run_with_timeout<F: core::future::Future>(
        &mut self,
        duration: Self::Duration,
        future: F,
    ) -> Result<F::Output, Self::TimeoutError>;
}

pub(crate) trait TimerExt: Timer {
    async fn run_with_maybe_timeout<F: core::future::Future>(
        &mut self,
        duration: Option<Self::Duration>,
        future: F,
    ) -> Result<F::Output, Self::TimeoutError> {
        if let Some(duration) = duration {
            self.run_with_timeout(duration, future).await
        } else {
            Ok(future.await)
        }
    }
}

impl<T: Timer> TimerExt for T {}

#[cfg(any(feature = "tokio", test))]
pub struct TokioTimer;

#[cfg(any(feature = "tokio", test))]
impl Timer for TokioTimer {
    type Duration = std::time::Duration;
    type TimeoutError = tokio::time::error::Elapsed;

    async fn run_with_timeout<F: core::future::Future>(
        &mut self,
        duration: Self::Duration,
        future: F,
    ) -> Result<F::Output, Self::TimeoutError> {
        tokio::time::timeout(duration, future).await
    }
}

#[cfg(feature = "embassy")]
pub struct EmbassyTimer;

#[cfg(feature = "embassy")]
impl Timer for EmbassyTimer {
    type Duration = embassy_time::Duration;
    type TimeoutError = embassy_time::TimeoutError;

    async fn run_with_timeout<F: core::future::Future>(
        &mut self,
        duration: Self::Duration,
        future: F,
    ) -> Result<F::Output, Self::TimeoutError> {
        embassy_time::with_timeout(duration, future).await
    }
}

pub(crate) struct WriteWithTimeout<'t, W: embedded_io_async::Write, T: Timer> {
    pub inner: W,
    pub timer: &'t mut T,
    pub timeout_duration: Option<T::Duration>,
}

impl<'t, W: embedded_io_async::Write, T: Timer> embedded_io_async::ErrorType
    for WriteWithTimeout<'t, W, T>
{
    type Error = super::Error<W::Error>;
}

impl<'t, W: embedded_io_async::Write, T: Timer> embedded_io_async::Write
    for WriteWithTimeout<'t, W, T>
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.timer
            .run_with_maybe_timeout(self.timeout_duration.clone(), self.inner.write(buf))
            .await
            .map_err(|_| super::Error::WriteTimeout)?
            .map_err(super::Error::Write)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.timer
            .run_with_maybe_timeout(self.timeout_duration.clone(), self.inner.flush())
            .await
            .map_err(|_| super::Error::WriteTimeout)?
            .map_err(super::Error::Write)
    }
}
