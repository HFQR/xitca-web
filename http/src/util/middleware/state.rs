use std::future::{ready, Future, Ready};

use xitca_service::{Service, ServiceFactory};

use crate::http::Request;

#[derive(Clone)]
pub struct State<F: Clone = ()> {
    factory: F,
}

impl State {
    pub fn new<S, Err>(state: S) -> State<impl Fn() -> Ready<Result<S, Err>> + Clone>
    where
        S: Send + Sync + Clone + 'static,
    {
        State {
            factory: move || ready(Ok(state.clone())),
        }
    }

    pub fn factory<F, Fut, Res, Err>(factory: F) -> State<F>
    where
        F: Fn() -> Fut + Clone,
        Fut: Future<Output = Result<Res, Err>>,
        Res: Send + Sync + Clone + 'static,
    {
        State { factory }
    }
}

impl<S, ReqB, F, Fut, Res, Err> ServiceFactory<Request<ReqB>, S> for State<F>
where
    S: Service<Request<ReqB>>,
    F: Fn() -> Fut + Clone,
    Fut: Future<Output = Result<Res, Err>>,
    Res: Send + Sync + Clone + 'static,
    S::Error: From<Err>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Service = StateService<S, Res>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn new_service(&self, service: S) -> Self::Future {
        let fut = (self.factory)();

        async move {
            let state = fut.await?;
            Ok(StateService { service, state })
        }
    }
}

pub struct StateService<S, St> {
    service: S,
    state: St,
}

impl<S, St> Clone for StateService<S, St>
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

impl<S, ReqB, St> Service<Request<ReqB>> for StateService<S, St>
where
    S: Service<Request<ReqB>>,
    St: Send + Sync + Clone + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Ready<'f>
    where
        S: 'f,
    = S::Ready<'f>;
    type Future<'f>
    where
        S: 'f,
    = S::Future<'f>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        self.service.ready()
    }

    #[inline]
    fn call(&self, mut req: Request<ReqB>) -> Self::Future<'_> {
        req.extensions_mut().insert(self.state.clone());
        self.service.call(req)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use xitca_service::{fn_service, ServiceFactory, ServiceFactoryExt};

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
}
