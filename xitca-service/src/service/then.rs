use core::{
    future::Future,
    task::{Context, Poll},
};

use super::Service;

pub struct ThenService<S1, S2> {
    s1: S1,
    s2: S2,
}

impl<S1, S2> ThenService<S1, S2> {
    pub fn new(s1: S1, s2: S2) -> Self {
        Self { s1, s2 }
    }
}

impl<S1, Req, S2> Service<Req> for ThenService<S1, S2>
where
    S1: Service<Req>,
    S2: Service<S1::Response>,

    S1::Error: From<S2::Error>,
{
    type Response = S2::Response;
    type Error = S1::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self.s1.poll_ready(cx) {
            Poll::Ready(res) => res?,
            Poll::Pending => return Poll::Pending,
        }

        self.s2.poll_ready(cx).map_err(<S1::Error as From<S2::Error>>::from)
    }

    fn call(&self, req: Req) -> Self::Future<'_> {
        async move {
            let res = self.s1.call(req).await?;
            self.s2.call(res).await.map_err(<S1::Error as From<S2::Error>>::from)
        }
    }
}
