//! [Timer] for creating timeouts during request parsing and request handling.

/// A timer which can be used to abort futures if they take to long to resolve.
pub trait Timer<Runtime> {
    /// The measure of time duration for this timer.
    type Duration: Clone;
    /// The error returned if a future fails to resolve in time.
    type TimeoutError;

    /// Drive the future, failing if it takes to long to resolve.
    async fn run_with_timeout<F: core::future::Future>(
        &mut self,
        duration: Self::Duration,
        future: F,
    ) -> Result<F::Output, Self::TimeoutError>;
}

pub(crate) trait TimerExt<Runtime>: Timer<Runtime> {
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

impl<Runtime, T: Timer<Runtime>> TimerExt<Runtime> for T {}

#[cfg(any(feature = "tokio", test))]
#[doc(hidden)]
pub struct TokioTimer;

#[cfg(any(feature = "tokio", test))]
impl Timer<super::TokioRuntime> for TokioTimer {
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
#[doc(hidden)]
pub struct EmbassyTimer;

#[cfg(feature = "embassy")]
impl Timer<super::EmbassyRuntime> for EmbassyTimer {
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

pub(crate) struct WriteWithTimeout<'t, Runtime, W: embedded_io_async::Write, T: Timer<Runtime>> {
    pub inner: W,
    pub timer: &'t mut T,
    pub timeout_duration: Option<T::Duration>,
    pub _runtime: core::marker::PhantomData<fn(&Runtime)>,
}

impl<'t, Runtime, W: embedded_io_async::Write, T: Timer<Runtime>> embedded_io_async::ErrorType
    for WriteWithTimeout<'t, Runtime, W, T>
{
    type Error = super::Error<W::Error>;
}

impl<'t, Runtime, W: embedded_io_async::Write, T: Timer<Runtime>> embedded_io_async::Write
    for WriteWithTimeout<'t, Runtime, W, T>
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
