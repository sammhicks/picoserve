use core::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::TryFuture;
use pin_project::pin_project;

#[pin_project::pin_project(project = EitherProj)]
pub enum Either<A, B> {
    First(#[pin] A),
    Second(#[pin] B),
}

/// Polls whichever variant is present in Either instnace.
impl<A: Future, B: Future<Output = A::Output>> Future for Either<A, B> {
    type Output = A::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project() {
            EitherProj::First(f) => f.poll(cx),
            EitherProj::Second(f) => f.poll(cx),
        }
    }
}

impl<A> Either<A, core::convert::Infallible> {
    pub fn ignore_never_b(self) -> A {
        match self {
            Self::First(a) => a,
            Self::Second(b) => match b {},
        }
    }
}

impl<B> Either<core::convert::Infallible, B> {
    pub fn ignore_never_a(self) -> B {
        match self {
            Self::First(a) => match a {},
            Self::Second(b) => b,
        }
    }
}

impl<A, B> Either<A, B> {
    pub fn first_is_error(self) -> Result<B, A> {
        match self {
            Either::First(a) => Err(a),
            Either::Second(b) => Ok(b),
        }
    }
}

/// [`Future`] returned by [`select_either`], polling `a` before `b`.
///
/// Storing the futures in a struct keeps exactly one copy of each. An `async fn`
/// taking them by value and `pin!`-ing them internally would instead keep both the
/// argument slot and the post-move local live, doubling the size of every future
/// passed in (see the composition in [`Select`]).
#[pin_project]
pub(crate) struct SelectEither<A, B> {
    #[pin]
    a: A,
    #[pin]
    b: B,
}

impl<A: Future, B: Future> Future for SelectEither<A, B> {
    type Output = Either<A::Output, B::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.project();

        if let Poll::Ready(output) = this.a.poll(cx) {
            return Poll::Ready(Either::First(output));
        }

        if let Poll::Ready(output) = this.b.poll(cx) {
            return Poll::Ready(Either::Second(output));
        }

        Poll::Pending
    }
}

pub(crate) fn select_either<A: Future, B: Future>(a: A, b: B) -> SelectEither<A, B> {
    SelectEither { a, b }
}

/// [`Future`] returned by [`select`].
///
/// Wrapping [`SelectEither`] in a struct (rather than an `async fn` that awaits it)
/// avoids re-storing both futures: the nested `select` -> `select_either` then holds
/// each future once instead of three times.
#[pin_project]
pub(crate) struct Select<A, B> {
    #[pin]
    inner: SelectEither<A, B>,
}

impl<A: Future, B: Future<Output = A::Output>> Future for Select<A, B> {
    type Output = A::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        match self.project().inner.poll(cx) {
            Poll::Ready(Either::First(output) | Either::Second(output)) => Poll::Ready(output),
            Poll::Pending => Poll::Pending,
        }
    }
}

pub(crate) fn select<A: Future, B: Future<Output = A::Output>>(a: A, b: B) -> Select<A, B> {
    Select {
        inner: select_either(a, b),
    }
}

#[pin_project::pin_project]
pub(crate) struct ThenPendForeverFuture<F: Future, T> {
    #[pin]
    maybe_future: Option<F>,
    _output: PhantomData<fn() -> T>,
}

impl<F: Future, T> Future for ThenPendForeverFuture<F, T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut maybe_future = self.project().maybe_future;

        if let Some(mut future) = maybe_future.as_mut().as_pin_mut() {
            if future.as_mut().poll(cx).is_ready() {
                maybe_future.set(None);
            }
        }

        Poll::Pending
    }
}

#[pin_project::pin_project]
pub(crate) struct TryThenPendForeverFuture<F: TryFuture, T> {
    #[pin]
    maybe_future: Option<F>,
    _output: PhantomData<fn() -> T>,
}

impl<F: TryFuture, T> Future for TryThenPendForeverFuture<F, T> {
    type Output = Result<T, F::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut maybe_future = self.project().maybe_future;

        if let Some(mut future) = maybe_future.as_mut().as_pin_mut() {
            match future.as_mut().try_poll(cx) {
                Poll::Ready(Ok(_)) => {
                    maybe_future.set(None);
                    Poll::Pending
                }
                Poll::Ready(Err(error)) => {
                    maybe_future.set(None);
                    Poll::Ready(Err(error))
                }
                Poll::Pending => Poll::Pending,
            }
        } else {
            Poll::Pending
        }
    }
}

pub(crate) trait ThenPendForever: Future + Sized {
    fn then_pend_forever<T>(self) -> ThenPendForeverFuture<Self, T> {
        ThenPendForeverFuture {
            maybe_future: Some(self),
            _output: PhantomData,
        }
    }

    #[cfg(feature = "embassy")]
    fn try_then_pend_forever<T>(self) -> TryThenPendForeverFuture<Self, T>
    where
        Self: TryFuture,
    {
        TryThenPendForeverFuture {
            maybe_future: Some(self),
            _output: PhantomData,
        }
    }
}

impl<F: Future> ThenPendForever for F {}

#[cfg(test)]
mod tests {
    use core::future::pending;

    use futures_util::FutureExt;

    use super::select;

    struct Success;

    #[test]
    #[ntest::timeout(1000)]
    fn select_first() {
        let Success = select(async { Success }, pending())
            .now_or_never()
            .expect("Future must resolve");
    }

    #[test]
    #[ntest::timeout(1000)]
    fn select_second() {
        let Success = select(pending(), async { Success })
            .now_or_never()
            .expect("Future must resolve");
    }

    #[test]
    #[ntest::timeout(1000)]
    fn select_neither() {
        enum Never {}

        assert!(select(pending::<Never>(), pending::<Never>())
            .now_or_never()
            .is_none());
    }
}
