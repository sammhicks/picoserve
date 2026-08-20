//! [`Timer`] for creating timeouts during request parsing and request handling.

use core::future::Future;

use futures_util::FutureExt;

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

/// A wrapper future for `F` which returns a Result of either `Ok(F::Output)`
/// if the future `F` resolves within the timeout, otherwise it returns
/// `Err(TimeoutError)`.
/// This pattern of wrapping the given future in a custom generic polling
/// implementation was chosen instead of a simple `async fn` which wraps the
/// runtime timeout method, because the `future` argument would end up being
/// captured twice, both as the argument to the `async fn` and in the future
/// returned by the runtime.
#[pin_project::pin_project]
struct TimeoutFuture<F, TF> {
    #[pin]
    future: F,

    #[pin]
    timeout_future: TF,
}

impl<F: Future, TF: Future<Output = TimeoutError>> Future for TimeoutFuture<F, TF> {
    type Output = Result<F::Output, TimeoutError>;

    fn poll(
        self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        let this = self.project();

        if let core::task::Poll::Ready(output) = this.future.poll(cx) {
            return core::task::Poll::Ready(Ok(output));
        }

        if let core::task::Poll::Ready(timeout_error) = this.timeout_future.poll(cx) {
            return core::task::Poll::Ready(Err(timeout_error));
        }

        core::task::Poll::Pending
    }
}

/// A timer which can be used to abort futures if they take to long to resolve.
pub trait Timer<Runtime> {
    /// Create a future which resolves after `duration` has passed.
    fn delay(&self, duration: Duration) -> impl Future<Output = ()>;

    /// Create a future which returns a [`TimeoutError`] after `duration` has passed.
    fn timeout(&self, duration: Duration) -> impl Future<Output = TimeoutError> {
        self.delay(duration).map(|()| TimeoutError)
    }

    /// Return a future which will run the given future, failing with a
    /// `TimeoutError` if it takes too long to resolve.
    fn run_with_timeout<F: Future>(
        &self,
        duration: Duration,
        future: F,
    ) -> impl Future<Output = Result<F::Output, TimeoutError>> {
        TimeoutFuture {
            future,
            timeout_future: self.timeout(duration),
        }
    }
}

#[derive(Default)]
#[cfg(any(feature = "tokio", test))]
#[doc(hidden)]
pub struct TokioTimer;

#[cfg(any(feature = "tokio", test))]
impl Timer<super::TokioRuntime> for TokioTimer {
    fn delay(&self, duration: Duration) -> impl Future<Output = ()> {
        tokio::time::sleep(std::time::Duration::from_millis(duration.as_millis()))
    }
}

#[derive(Default)]
#[cfg(feature = "embassy")]
#[doc(hidden)]
pub struct EmbassyTimer;

#[cfg(feature = "embassy")]
impl Timer<super::EmbassyRuntime> for EmbassyTimer {
    fn delay(&self, duration: Duration) -> impl Future<Output = ()> {
        embassy_time::Timer::after(duration)
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

impl<Runtime, W: crate::io::Write, T: Timer<Runtime>> crate::io::BaseWrite
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

impl<Runtime, W: crate::io::Write, T: Timer<Runtime>> crate::io::Write
    for WriteWithTimeout<'_, Runtime, W, T>
where
    W::Error: 'static,
{
    async fn write_with<F: FnOnce(crate::mem::BorrowedCursor<'_>) -> R, R>(
        &mut self,
        f: F,
    ) -> Result<R, Self::Error> {
        self.timer
            .run_with_timeout(self.timeout_duration, self.inner.write_with(f))
            .await
            .map_err(super::Error::WriteTimeout)?
            .map_err(super::Error::Write)
    }

    async fn write_fmt(&mut self, args: core::fmt::Arguments<'_>) -> Result<(), Self::Error> {
        self.timer
            .run_with_timeout(self.timeout_duration, self.inner.write_fmt(args))
            .await
            .map_err(super::Error::WriteTimeout)?
            .map_err(super::Error::Write)
    }
}
