use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

#[derive(Clone)]
pub(super) struct Limit {
    limit: usize,
    inner: Rc<LimitInner>,
}

struct LimitInner {
    current: Cell<usize>,
    waker: Cell<Option<Waker>>,
}

pub(crate) struct LimitGuard(Limit);

impl Drop for LimitGuard {
    fn drop(&mut self) {
        let current = self.0.inner.current.get();
        if current == self.0.limit {
            if let Some(waker) = self.0.inner.waker.take() {
                waker.wake();
            }
        }
        self.0.inner.current.set(current - 1);
    }
}

impl Limit {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            limit,
            inner: Rc::new(LimitInner {
                current: Cell::new(0),
                waker: Cell::new(None),
            }),
        }
    }

    pub(super) fn ready(&self) -> LimitReady<'_> {
        LimitReady(self)
    }
}

pub(super) struct LimitReady<'a>(&'a Limit);

impl Future for LimitReady<'_> {
    type Output = LimitGuard;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.0;
        let current = this.inner.current.get();
        if current == this.limit {
            this.inner.waker.set(Some(cx.waker().clone()));
            Poll::Pending
        } else {
            this.inner.current.set(current + 1);

            Poll::Ready(LimitGuard(this.clone()))
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn counter() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let limit = Limit::new(2);

                let mut guards = Vec::new();
                let guard = limit.ready().await;
                guards.push(guard);

                let guard = limit.ready().await;
                guards.push(guard);

                tokio::task::spawn_local(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    drop(guards);
                });

                let now = std::time::Instant::now();
                tokio::task::yield_now().await;

                let _guard = limit.ready().await;
                let _guard = limit.ready().await;

                assert!(now.elapsed() > std::time::Duration::from_secs(2));
            })
            .await
    }
}
