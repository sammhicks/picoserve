use core::{cell::Cell, task::Waker};

pub struct SignalCore<T: Copy> {
    value: Cell<Option<T>>,
    waker: Cell<Option<Waker>>,
}

impl<T: Copy> SignalCore<T> {
    // Take &mut self to avoid multiple calls to make_signal for a SignalCore.
    pub fn make_signal(&mut self) -> Signal<'_, T> {
        Signal { channel: self }
    }
}

pub struct Signal<'a, T: Copy> {
    channel: &'a SignalCore<T>,
}

impl<'a, T: Copy> Signal<'a, T> {
    pub fn core() -> SignalCore<T> {
        SignalCore {
            value: None.into(),
            waker: None.into(),
        }
    }

    pub fn notify(self, value: T) {
        self.channel.value.set(Some(value));

        if let Some(waker) = self.channel.waker.take() {
            waker.wake();
        }
    }

    pub fn listen(&self) -> Listener<'a, T> {
        Listener {
            channel: Some(self.channel),
        }
    }
}

#[derive(Clone)]
pub struct Listener<'a, T: Copy> {
    channel: Option<&'a SignalCore<T>>,
}

impl<T: Copy> Listener<'_, T> {
    pub fn never() -> Self {
        Self { channel: None }
    }
}

impl<T: Copy> core::future::Future for Listener<'_, T> {
    type Output = T;

    fn poll(
        mut self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        let Some(mux) = self.channel else {
            return core::task::Poll::Pending;
        };

        if let Some(value) = mux.value.get() {
            self.channel = None;

            return core::task::Poll::Ready(value);
        }

        let new_waker = if let Some(current_waker) = mux.waker.take() {
            if current_waker.will_wake(cx.waker()) {
                current_waker
            } else {
                current_waker.wake();

                cx.waker().clone()
            }
        } else {
            cx.waker().clone()
        };

        mux.waker.set(Some(new_waker));

        core::task::Poll::Pending
    }
}
