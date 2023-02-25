use core::{
    convert::Infallible,
    future::{ready, Future, Ready},
};

use xitca_service::{ready::ReadyService, Service};

use crate::http::{BorrowReqMut, Extensions};

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

impl<S, St, Req> Service<Req> for ExtensionService<S, St>
where
    S: Service<Req>,
    St: Send + Sync + Clone + 'static,
    Req: BorrowReqMut<Extensions>,
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

    type Future<'f> = S::Future<'f> where S: 'f;

    #[inline]
    fn ready(&self) -> Self::Future<'_> {
        self.service.ready()
    }
}

#[cfg(test)]
mod test {
    use xitca_service::{fn_service, ServiceExt};
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::http::Request;

    use super::*;

    #[test]
    fn state_middleware() {
        let service = fn_service(|req: Request<()>| async move {
            assert_eq!("state", req.extensions().get::<String>().unwrap());
            Ok::<_, ()>("996")
        })
        .enclosed(Extension::new(String::from("state")))
        .call(())
        .now_or_panic()
        .unwrap();

        let res = service.call(Request::new(())).now_or_panic().unwrap();

        assert_eq!("996", res);
    }

    #[test]
    fn state_factory_middleware() {
        let service = fn_service(|req: Request<()>| async move {
            assert_eq!("state", req.extensions().get::<String>().unwrap());
            Ok::<_, ()>("996")
        })
        .enclosed(Extension::factory(|| async move {
            Ok::<_, Infallible>(String::from("state"))
        }))
        .call(())
        .now_or_panic()
        .unwrap();

        let res = service.call(Request::new(())).now_or_panic().unwrap();

        assert_eq!("996", res);
    }

    #[test]
    fn state_middleware_http_request() {
        let service = fn_service(|req: http::Request<()>| async move {
            assert_eq!("state", req.extensions().get::<String>().unwrap());
            Ok::<_, ()>("996")
        })
        .enclosed(Extension::new(String::from("state")))
        .call(())
        .now_or_panic()
        .unwrap();

        let res = service.call(http::Request::new(())).now_or_panic().unwrap();

        assert_eq!("996", res);
    }
}
