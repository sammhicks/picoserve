use core::future::Future;

pub enum Either<A, B> {
    First(A),
    Second(B),
}

pub async fn select_either<A: Future, B: Future>(a: A, b: B) -> Either<A::Output, B::Output> {
    let mut a = core::pin::pin!(a);
    let mut b = core::pin::pin!(b);

    core::future::poll_fn(|cx| {
        use core::task::Poll;

        match a.as_mut().poll(cx) {
            Poll::Ready(output) => return Poll::Ready(Either::First(output)),
            Poll::Pending => (),
        }

        match b.as_mut().poll(cx) {
            Poll::Ready(output) => return Poll::Ready(Either::Second(output)),
            Poll::Pending => (),
        }

        Poll::Pending
    })
    .await
}

pub async fn select<A: Future, B: Future<Output = A::Output>>(a: A, b: B) -> A::Output {
    match select_either(a, b).await {
        Either::First(output) | Either::Second(output) => output,
    }
}

#[cfg(test)]
mod tests {
    use core::future::pending;

    use futures_util::FutureExt;

    use super::select;

    struct Success;

    #[test]
    #[ntest::timeout(1000)]
    fn first() {
        let Success = select(async { Success }, pending())
            .now_or_never()
            .expect("Future must resolve");
    }

    #[test]
    #[ntest::timeout(1000)]
    fn second() {
        let Success = select(pending(), async { Success })
            .now_or_never()
            .expect("Future must resolve");
    }

    #[test]
    #[ntest::timeout(1000)]
    fn neither() {
        enum Never {}

        assert!(select(pending::<Never>(), pending::<Never>())
            .now_or_never()
            .is_none());
    }
}
