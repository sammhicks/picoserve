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

#[cfg(feature = "tokio")]
pub struct TokioTimer;

#[cfg(feature = "tokio")]
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
