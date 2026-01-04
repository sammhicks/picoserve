//! [Timer] for creating timeouts during request parsing and request handling.

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TimeoutError;

impl crate::io::Error for TimeoutError {
    fn kind(&self) -> crate::io::ErrorKind {
        crate::io::ErrorKind::TimedOut
    }
}

/// A timer which can be used to abort futures if they take to long to resolve.
pub trait Timer<Runtime> {
    /// The measure of time duration for this timer.
    type Duration: Clone;

    async fn delay(&self, duration: Self::Duration);

    /// Drive the future, failing if it takes to long to resolve.
    async fn run_with_timeout<F: core::future::Future>(
        &self,
        duration: Self::Duration,
        future: F,
    ) -> Result<F::Output, TimeoutError>;
}

pub(crate) trait TimerExt<Runtime>: Timer<Runtime> {
    async fn timeout(&self, duration: Self::Duration) -> TimeoutError {
        self.delay(duration).await;

        TimeoutError
    }

    async fn maybe_timeout(&self, duration: Option<Self::Duration>) -> TimeoutError {
        if let Some(duration) = duration {
            self.timeout(duration).await
        } else {
            core::future::pending().await
        }
    }

    async fn run_with_maybe_timeout<F: core::future::Future>(
        &self,
        duration: Option<Self::Duration>,
        future: F,
    ) -> Result<F::Output, TimeoutError> {
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

    async fn delay(&self, duration: Self::Duration) {
        tokio::time::sleep(duration).await
    }

    async fn run_with_timeout<F: core::future::Future>(
        &self,
        duration: Self::Duration,
        future: F,
    ) -> Result<F::Output, TimeoutError> {
        tokio::time::timeout(duration, future)
            .await
            .map_err(|_: tokio::time::error::Elapsed| TimeoutError)
    }
}

#[cfg(feature = "embassy")]
#[doc(hidden)]
pub struct EmbassyTimer;

#[cfg(feature = "embassy")]
impl Timer<super::EmbassyRuntime> for EmbassyTimer {
    type Duration = embassy_time::Duration;

    async fn delay(&self, duration: Self::Duration) {
        embassy_time::Timer::after(duration).await
    }

    async fn run_with_timeout<F: core::future::Future>(
        &self,
        duration: Self::Duration,
        future: F,
    ) -> Result<F::Output, TimeoutError> {
        embassy_time::with_timeout(duration, future)
            .await
            .map_err(|_: embassy_time::TimeoutError| TimeoutError)
    }
}

pub(crate) struct WriteWithTimeout<'t, Runtime, W: crate::io::Write, T: Timer<Runtime>> {
    pub inner: W,
    pub timer: &'t T,
    pub timeout_duration: Option<T::Duration>,
    pub _runtime: core::marker::PhantomData<fn(&Runtime)>,
}

impl<Runtime, W: crate::io::Write, T: Timer<Runtime>> crate::io::ErrorType
    for WriteWithTimeout<'_, Runtime, W, T>
{
    type Error = super::Error<W::Error>;
}

impl<Runtime, W: crate::io::Write, T: Timer<Runtime>> crate::io::Write
    for WriteWithTimeout<'_, Runtime, W, T>
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.timer
            .run_with_maybe_timeout(self.timeout_duration.clone(), self.inner.write(buf))
            .await
            .map_err(super::Error::WriteTimeout)?
            .map_err(super::Error::Write)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.timer
            .run_with_maybe_timeout(self.timeout_duration.clone(), self.inner.flush())
            .await
            .map_err(super::Error::WriteTimeout)?
            .map_err(super::Error::Write)
    }
}
