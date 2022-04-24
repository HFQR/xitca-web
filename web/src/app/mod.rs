mod object;

use std::{
    convert::Infallible,
    future::{ready, Future, Ready},
};

use xitca_http::{
    util::service::{
        context::{Context, ContextBuilder},
        GenericRouter,
    },
    Request, RequestBody,
};
use xitca_service::ready::ReadyService;
use xitca_service::{
    object::ObjectConstructor, AsyncClosure, EnclosedFactory, EnclosedFnFactory, Service, ServiceFactory,
    ServiceFactoryExt,
};

use crate::request::WebRequest;

use self::object::WebObjectConstructor;

pub struct App<CF = (), R = ()> {
    ctx_factory: CF,
    router: R,
}

type Router<C, SF> = GenericRouter<WebObjectConstructor<C>, SF>;

impl App {
    pub fn new<SF>() -> App<impl Fn() -> Ready<Result<(), Infallible>>, Router<(), SF>> {
        Self::with_async_state(|| ready(Ok(())))
    }

    /// Construct App with a thread local state.
    ///
    /// State would still be shared among tasks on the same thread.
    pub fn with_current_thread_state<C, SF>(state: C) -> App<impl Fn() -> Ready<Result<C, Infallible>>, Router<C, SF>>
    where
        C: Clone + 'static,
    {
        Self::with_async_state(move || ready(Ok(state.clone())))
    }

    /// Construct App with a thread safe state.
    ///
    /// State would be shared among all tasks and worker threads.
    pub fn with_multi_thread_state<C, SF>(state: C) -> App<impl Fn() -> Ready<Result<C, Infallible>>, Router<C, SF>>
    where
        C: Send + Sync + Clone + 'static,
    {
        Self::with_async_state(move || ready(Ok(state.clone())))
    }

    #[doc(hidden)]
    /// Construct App with async closure which it's output would be used as state.
    pub fn with_async_state<CF, Fut, E, C, SF>(ctx_factory: CF) -> App<CF, Router<C, SF>>
    where
        CF: Fn() -> Fut,
        Fut: Future<Output = Result<C, E>>,
    {
        App {
            ctx_factory,
            router: GenericRouter::with_custom_object(),
        }
    }
}

impl<CF, C, SF> App<CF, Router<C, SF>> {
    pub fn at<F>(mut self, path: &'static str, factory: F) -> Self
    where
        WebObjectConstructor<C>: ObjectConstructor<F, Object = SF>,
    {
        self.router = self.router.insert(path, factory);
        self
    }
}

impl<CF, R> App<CF, R> {
    pub fn enclosed<Req, T>(self, transform: T) -> App<CF, EnclosedFactory<R, T>>
    where
        R: ServiceFactory<Req>,
        T: ServiceFactory<Req, R::Service> + Clone,
    {
        App {
            ctx_factory: self.ctx_factory,
            router: self.router.enclosed(transform),
        }
    }

    pub fn enclosed_fn<Req, T>(self, transform: T) -> App<CF, EnclosedFnFactory<R, T>>
    where
        R: ServiceFactory<Req>,
        T: for<'s> AsyncClosure<(&'s R::Service, Req)> + Clone,
    {
        App {
            ctx_factory: self.ctx_factory,
            router: self.router.enclosed_fn(transform),
        }
    }

    pub fn finish<Req, Fut, C, CErr>(self) -> ContextBuilder<CF, C, MapRequest<R>>
    where
        CF: Fn() -> Fut,
        Fut: Future<Output = Result<C, CErr>>,
        ContextBuilder<CF, C, MapRequest<R>>: ServiceFactory<Req>,
    {
        let App { ctx_factory, router } = self;

        ContextBuilder::new(ctx_factory).service(MapRequest { factory: router })
    }
}

pub struct MapRequest<F> {
    factory: F,
}

impl<'c, 's, C, F, Arg, S, Res, Err> ServiceFactory<&'c mut Context<'s, Request<RequestBody>, C>, Arg> for MapRequest<F>
where
    Arg: 'static,
    C: 'static,
    F: for<'c1, 's1> ServiceFactory<&'c1 mut WebRequest<'s1, C>, Arg, Service = S, Response = Res, Error = Err>
        + 'static,
    S: for<'c1, 's1> Service<&'c1 mut WebRequest<'s1, C>, Response = Res, Error = Err>,
{
    type Response = Res;
    type Error = Err;
    type Service = MapRequestService<S>;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>> + 'static;

    fn new_service(&self, arg: Arg) -> Self::Future {
        let fut = self.factory.new_service(arg);
        async move {
            let service = fut.await?;
            Ok(MapRequestService { service })
        }
    }
}

pub struct MapRequestService<S> {
    service: S,
}

impl<'c, 's, C, S, Res, Err> Service<&'c mut Context<'s, Request<RequestBody>, C>> for MapRequestService<S>
where
    S: for<'c1, 's1> Service<&'c1 mut WebRequest<'s1, C>, Response = Res, Error = Err>,
{
    type Response = Res;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where S: 'f;

    fn call(&self, req: &'c mut Context<'s, Request<RequestBody>, C>) -> Self::Future<'_> {
        let (req, state) = req.borrow_parts_mut();
        let mut req = WebRequest::new(req, state);
        async move { self.service.call(&mut req).await }
    }
}

impl<'c, 's, C, S, R, Res, Err> ReadyService<&'c mut Context<'s, Request<RequestBody>, C>> for MapRequestService<S>
where
    S: for<'c1, 's1> ReadyService<&'c1 mut WebRequest<'s1, C>, Response = Res, Error = Err, Ready = R>,
{
    type Ready = R;
    type ReadyFuture<'f> = impl Future<Output = Result<Self::Ready, Self::Error>> where S: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        async move { self.service.ready().await }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::{
        dev::Service,
        extract::{PathRef, StateRef},
        http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE},
        service::handler_service,
    };

    async fn handler(
        StateRef(state): StateRef<'_, String>,
        PathRef(path): PathRef<'_>,
        req: &WebRequest<'_, String>,
    ) -> String {
        assert_eq!("state", state);
        assert_eq!(state, req.state());
        assert_eq!("/", path);
        assert_eq!(path, req.req().uri().path());
        state.to_string()
    }

    #[derive(Clone)]
    struct Middleware;

    impl<S, State, Res, Err> ServiceFactory<&mut WebRequest<'_, State>, S> for Middleware
    where
        S: for<'r, 's> Service<&'r mut WebRequest<'s, State>, Response = Res, Error = Err>,
    {
        type Response = Res;
        type Error = Err;
        type Service = MiddlewareService<S>;
        type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

        fn new_service(&self, service: S) -> Self::Future {
            async { Ok(MiddlewareService(service)) }
        }
    }

    struct MiddlewareService<S>(S);

    impl<'r, 's, S, State, Res, Err> Service<&'r mut WebRequest<'s, State>> for MiddlewareService<S>
    where
        S: for<'r1, 's1> Service<&'r1 mut WebRequest<'s1, State>, Response = Res, Error = Err>,
    {
        type Response = Res;
        type Error = Err;
        type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

        fn call(&self, req: &'r mut WebRequest<'s, State>) -> Self::Future<'_> {
            self.0.call(req)
        }
    }

    #[tokio::test]
    async fn test_app() {
        async fn middleware_fn<S, State, Res, Err>(service: &S, req: &mut WebRequest<'_, State>) -> Result<Res, Err>
        where
            S: for<'r, 's> Service<&'r mut WebRequest<'s, State>, Response = Res, Error = Err>,
        {
            service.call(req).await
        }

        let state = String::from("state");

        let service = App::with_current_thread_state(state)
            .at("/", handler_service(handler))
            .enclosed_fn(middleware_fn)
            .enclosed(Middleware)
            .finish()
            .new_service(())
            .await
            .ok()
            .unwrap();

        let req = Request::default();

        let res = service.call(req).await.unwrap();

        assert_eq!(res.status().as_u16(), 200);
        assert_eq!(res.headers().get(CONTENT_TYPE).unwrap(), TEXT_UTF8);
    }
}
