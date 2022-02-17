use std::{
    borrow::BorrowMut,
    future::{ready, Future, Ready},
    marker::PhantomData,
};

use xitca_service::{ready::ReadyService, Service, ServiceFactory};

use crate::http;

#[derive(Clone)]
pub struct State<ReqB, F: Clone = ()> {
    factory: F,
    _req_body: PhantomData<ReqB>,
}

impl<ReqB> State<ReqB> {
    pub fn new<S, Err>(state: S) -> State<ReqB, impl Fn() -> Ready<Result<S, Err>> + Clone>
    where
        S: Send + Sync + Clone + 'static,
    {
        State {
            factory: move || ready(Ok(state.clone())),
            _req_body: PhantomData,
        }
    }

    pub fn factory<F, Fut, Res, Err>(factory: F) -> State<ReqB, F>
    where
        F: Fn() -> Fut + Clone,
        Fut: Future<Output = Result<Res, Err>>,
        Res: Send + Sync + Clone + 'static,
    {
        State {
            factory,
            _req_body: PhantomData,
        }
    }
}

impl<S, Req, ReqB, F, Fut, Res, Err> ServiceFactory<Req, S> for State<ReqB, F>
where
    S: Service<Req>,
    Req: BorrowMut<http::Request<ReqB>>,
    F: Fn() -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
    Res: Send + Sync + Clone + 'static,
    S::Error: From<Err>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Service = StateService<S, ReqB, Res>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, service: S) -> Self::Future {
        let fut = (self.factory)();

        async move {
            let state = fut.await?;
            Ok(StateService {
                service,
                state,
                _req_body: PhantomData,
            })
        }
    }
}

pub struct StateService<S, ReqB, St> {
    service: S,
    state: St,
    _req_body: PhantomData<ReqB>,
}

impl<S, ReqB, St> Clone for StateService<S, ReqB, St>
where
    S: Clone,
    St: Clone,
{
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
            state: self.state.clone(),
            _req_body: PhantomData,
        }
    }
}

impl<S, Req, ReqB, St> Service<Req> for StateService<S, ReqB, St>
where
    S: Service<Req>,
    Req: BorrowMut<http::Request<ReqB>>,
    St: Send + Sync + Clone + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future<'f>
    where
        S: 'f,
        ReqB: 'f,
    = S::Future<'f>;

    #[inline]
    fn call(&self, mut req: Req) -> Self::Future<'_> {
        req.borrow_mut().extensions_mut().insert(self.state.clone());
        self.service.call(req)
    }
}

impl<S, Req, ReqB, St> ReadyService<Req> for StateService<S, ReqB, St>
where
    S: ReadyService<Req>,
    Req: BorrowMut<http::Request<ReqB>>,
    St: Send + Sync + Clone + 'static,
{
    type Ready = S::Ready;

    type ReadyFuture<'f>
    where
        Self: 'f,
        ReqB: 'f,
    = S::ReadyFuture<'f>;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        self.service.ready()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use xitca_service::{fn_service, ServiceFactory, ServiceFactoryExt};

    use crate::request::Request;

    #[tokio::test]
    async fn state_middleware() {
        let service = fn_service(|req: Request<()>| async move {
            assert_eq!("state", req.extensions().get::<String>().unwrap());
            Ok::<_, ()>("996")
        })
        .transform(State::new(String::from("state")))
        .new_service(())
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
        .transform(State::factory(|| async move { Ok::<_, ()>(String::from("state")) }))
        .new_service(())
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
        .transform(State::new(String::from("state")))
        .new_service(())
        .await
        .unwrap();

        let res = service.call(http::Request::new(())).await.unwrap();

        assert_eq!("996", res);
    }
}
