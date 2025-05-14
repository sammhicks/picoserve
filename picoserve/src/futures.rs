use core::future::Future;

pub async fn select<A: Future, B: Future<Output = A::Output>>(a: A, b: B) -> A::Output {
    let mut a = core::pin::pin!(a);
    let mut b = core::pin::pin!(b);

    core::future::poll_fn(|cx| {
        use core::task::Poll;

        match a.as_mut().poll(cx) {
            Poll::Ready(output) => return Poll::Ready(output),
            Poll::Pending => (),
        }

        match b.as_mut().poll(cx) {
            Poll::Ready(output) => return Poll::Ready(output),
            Poll::Pending => (),
        }

        Poll::Pending
    })
    .await
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
