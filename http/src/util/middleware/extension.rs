use xitca_service::{Service, ready::ReadyService};

use crate::http::{BorrowReqMut, Extensions};

/// builder for middleware attaching typed data to [`Request`]'s [`Extensions`]
///
/// [`Request`]: crate::http::Request
#[derive(Clone)]
pub struct Extension<S> {
    state: S,
}

impl<S> Extension<S> {
    pub fn new(state: S) -> Self
    where
        S: Send + Sync + Clone + 'static,
    {
        Extension { state }
    }
}

impl<T, S, E> Service<Result<S, E>> for Extension<T>
where
    T: Clone,
{
    type Response = ExtensionService<S, T>;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res.map(|service| ExtensionService {
            service,
            state: self.state.clone(),
        })
    }
}

pub struct ExtensionService<S, St> {
    service: S,
    state: St,
}

impl<S, St> Clone for ExtensionService<S, St>
where
    S: Clone,
    St: Clone,
{
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            state: self.state.clone(),
        }
    }
}

impl<S, St, Req> Service<Req> for ExtensionService<S, St>
where
    S: Service<Req>,
    St: Send + Sync + Clone + 'static,
    Req: BorrowReqMut<Extensions>,
{
    type Response = S::Response;
    type Error = S::Error;

    #[inline]
    async fn call(&self, mut req: Req) -> Result<Self::Response, Self::Error> {
        req.borrow_mut().insert(self.state.clone());
        self.service.call(req).await
    }
}

impl<S, St> ReadyService for ExtensionService<S, St>
where
    S: ReadyService,
    St: Send + Sync + Clone + 'static,
{
    type Ready = S::Ready;

    #[inline]
    async fn ready(&self) -> Self::Ready {
        self.service.ready().await
    }
}

#[cfg(test)]
mod tests;
