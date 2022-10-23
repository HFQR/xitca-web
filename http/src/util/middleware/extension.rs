use std::{
    convert::Infallible,
    future::{ready, Future, Ready},
};

use xitca_service::{ready::ReadyService, Service};

use crate::{http, request::BorrowReqMut};

#[derive(Clone)]
pub struct Extension<F: Clone = ()> {
    factory: F,
}

impl Extension {
    pub fn new<S>(state: S) -> Extension<impl Fn() -> Ready<Result<S, Infallible>> + Clone>
    where
        S: Send + Sync + Clone + 'static,
    {
        Extension {
            factory: move || ready(Ok(state.clone())),
        }
    }

    pub fn factory<F, Fut, Res, Err>(factory: F) -> Extension<F>
    where
        F: Fn() -> Fut + Clone,
        Fut: Future<Output = Result<Res, Err>>,
        Res: Send + Sync + Clone + 'static,
    {
        Extension { factory }
    }
}

impl<S, F, Fut, Res, Err> Service<S> for Extension<F>
where
    F: Fn() -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
    Res: Send + Sync + Clone + 'static,
{
    type Response = ExtensionService<S, Res>;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f, S: 'f;

    fn call<'s>(&'s self, service: S) -> Self::Future<'s>
    where
        S: 's,
    {
        async {
            let state = (self.factory)().await?;
            Ok(ExtensionService { service, state })
        }
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

impl<S, Req, St> Service<Req> for ExtensionService<S, St>
where
    S: Service<Req>,
    Req: BorrowReqMut<http::Extensions>,
    St: Send + Sync + Clone + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future<'f> = S::Future<'f> where S: 'f, Req: 'f;

    #[inline]
    fn call<'s>(&'s self, mut req: Req) -> Self::Future<'s>
    where
        Req: 's,
    {
        req.borrow_mut().insert(self.state.clone());
        self.service.call(req)
    }
}

impl<S, St> ReadyService for ExtensionService<S, St>
where
    S: ReadyService,
    St: Send + Sync + Clone + 'static,
{
    type Ready = S::Ready;

    type ReadyFuture<'f> = S::ReadyFuture<'f> where S: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.service.ready()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use xitca_service::{fn_service, ServiceExt};

    use crate::request::Request;

    #[tokio::test]
    async fn state_middleware() {
        let service = fn_service(|req: Request<()>| async move {
            assert_eq!("state", req.extensions().get::<String>().unwrap());
            Ok::<_, ()>("996")
        })
        .enclosed(Extension::new(String::from("state")))
        .call(())
        .await
        .unwrap();

        let res = service.call(Request::new(())).await.unwrap();

        assert_eq!("996", res);
    }

    #[tokio::test]
    async fn state_factory_middleware() {
        let service = fn_service(|req: Request<()>| async move {
            assert_eq!("state", req.extensions().get::<String>().unwrap());
            Ok::<_, ()>("996")
        })
        .enclosed(Extension::factory(|| async move {
            Ok::<_, Infallible>(String::from("state"))
        }))
        .call(())
        .await
        .unwrap();

        let res = service.call(Request::new(())).await.unwrap();

        assert_eq!("996", res);
    }

    #[tokio::test]
    async fn state_middleware_http_request() {
        let service = fn_service(|req: http::Request<()>| async move {
            assert_eq!("state", req.extensions().get::<String>().unwrap());
            Ok::<_, ()>("996")
        })
        .enclosed(Extension::new(String::from("state")))
        .call(())
        .await
        .unwrap();

        let res = service.call(http::Request::new(())).await.unwrap();

        assert_eq!("996", res);
    }
}
