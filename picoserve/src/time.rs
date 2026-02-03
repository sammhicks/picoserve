//! [`Timer`] for creating timeouts during request parsing and request handling.

/// This becomes an alias of `embassy_time::Duration` if the `embassy` features is enabled.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg(not(feature = "embassy"))]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Duration {
    milliseconds: u64,
}

#[cfg(not(feature = "embassy"))]
impl Duration {
    /// Convert the `Duration` to seconds, rounding down.
    pub const fn as_secs(&self) -> u64 {
        self.milliseconds / 1000
    }

    /// Convert the `Duration` to milliseconds, rounding down.
    pub const fn as_millis(&self) -> u64 {
        self.milliseconds
    }

    /// Creates a duration from the specified number of seconds.
    pub const fn from_secs(seconds: u64) -> Self {
        Self::from_millis(1000 * seconds)
    }

    /// Creates a duration from the specified number of milliseconds.
    pub const fn from_millis(milliseconds: u64) -> Self {
        Self { milliseconds }
    }
}

#[cfg(not(feature = "embassy"))]
impl core::fmt::Display for Duration {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:.3}s", self.milliseconds as f32 / 1000.0)
    }
}

#[cfg(feature = "embassy")]
pub use embassy_time::Duration;

#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[error("Timeout")]
pub struct TimeoutError;

impl crate::io::Error for TimeoutError {
    fn kind(&self) -> crate::io::ErrorKind {
        crate::io::ErrorKind::TimedOut
    }
}

/// A timer which can be used to abort futures if they take to long to resolve.
pub trait Timer<Runtime> {
    /// Create a future which resolves after `duration` has passed.
    async fn delay(&self, duration: Duration);

    /// Drive the future, failing if it takes too long to resolve.
    async fn run_with_timeout<F: core::future::Future>(
        &self,
        duration: Duration,
        future: F,
    ) -> Result<F::Output, TimeoutError>;
}

pub(crate) trait TimerExt<Runtime>: Timer<Runtime> {
    async fn timeout(&self, duration: Duration) -> TimeoutError {
        self.delay(duration).await;

        TimeoutError
    }
}

impl<Runtime, T: Timer<Runtime>> TimerExt<Runtime> for T {}

#[derive(Default)]
#[cfg(any(feature = "tokio", test))]
#[doc(hidden)]
pub struct TokioTimer;

#[cfg(any(feature = "tokio", test))]
impl Timer<super::TokioRuntime> for TokioTimer {
    async fn delay(&self, duration: Duration) {
        tokio::time::sleep(std::time::Duration::from_millis(duration.as_millis())).await
    }

    async fn run_with_timeout<F: core::future::Future>(
        &self,
        duration: Duration,
        future: F,
    ) -> Result<F::Output, TimeoutError> {
        tokio::time::timeout(
            std::time::Duration::from_millis(duration.as_millis()),
            future,
        )
        .await
        .map_err(|_: tokio::time::error::Elapsed| TimeoutError)
    }
}

#[derive(Default)]
#[cfg(feature = "embassy")]
#[doc(hidden)]
pub struct EmbassyTimer;

#[cfg(feature = "embassy")]
impl Timer<super::EmbassyRuntime> for EmbassyTimer {
    async fn delay(&self, duration: Duration) {
        embassy_time::Timer::after(duration).await
    }

    async fn run_with_timeout<F: core::future::Future>(
        &self,
        duration: Duration,
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
    pub timeout_duration: Duration,
    pub _runtime: core::marker::PhantomData<fn(&Runtime)>,
}

impl<Runtime, W: crate::io::Write, T: Timer<Runtime>> crate::io::ErrorType
    for WriteWithTimeout<'_, Runtime, W, T>
where
    W::Error: 'static,
{
    type Error = super::Error<W::Error>;
}

impl<Runtime, W: crate::io::Write, T: Timer<Runtime>> crate::io::Write
    for WriteWithTimeout<'_, Runtime, W, T>
where
    W::Error: 'static,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.timer
            .run_with_timeout(self.timeout_duration, self.inner.write(buf))
            .await
            .map_err(super::Error::WriteTimeout)?
            .map_err(super::Error::Write)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.timer
            .run_with_timeout(self.timeout_duration, self.inner.flush())
            .await
            .map_err(super::Error::WriteTimeout)?
            .map_err(super::Error::Write)
    }
}
