use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::FutureExt;
use pin_project::pin_project;

#[pin_project(project = EitherProj)]
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

pub(crate) fn select_either<A: Future, B: Future>(
    a: A,
    b: B,
) -> impl Future<Output = Either<A::Output, B::Output>> {
    /// Storing the futures in a struct keeps exactly one copy of each. An `async fn`
    /// taking them by value and `pin!`-ing them internally would instead keep both the
    /// argument slot and the post-move local live, doubling the size of every future
    /// passed in (see the composition in [`Select`]).
    #[pin_project]
    struct SelectEither<A, B> {
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

    SelectEither { a, b }
}

pub(crate) fn select<A: Future, B: Future<Output = A::Output>>(
    a: A,
    b: B,
) -> impl Future<Output = A::Output> {
    select_either(a, b).map(|output| match output {
        Either::First(output) | Either::Second(output) => output,
    })
}

pub(crate) struct IgnoredOutput;

pub(crate) fn ignore_output<T>(_: T) -> IgnoredOutput {
    IgnoredOutput
}

pub(crate) fn pend_forever<T>(_: IgnoredOutput) -> impl Future<Output = T> {
    core::future::pending()
}

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
