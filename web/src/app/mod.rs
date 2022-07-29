mod object;

use std::{
    cell::RefCell,
    convert::Infallible,
    future::{ready, Future, Ready},
};

use xitca_http::{
    util::service::{
        context::{Context, ContextBuilder},
        router::GenericRouter,
    },
    Request,
};
use xitca_service::{
    object::ObjectConstructor, ready::ReadyService, AsyncClosure, BuildService, BuildServiceExt, EnclosedFactory,
    EnclosedFnFactory, Service,
};

use crate::request::WebRequest;

use self::object::WebObjectConstructor;

pub struct App<CF = (), R = ()> {
    ctx_factory: CF,
    router: R,
}

type Router<C, B, SF> = GenericRouter<WebObjectConstructor<C, B>, SF>;

impl App {
    pub fn new<B, SF>() -> App<impl Fn() -> Ready<Result<(), Infallible>>, Router<(), B, SF>> {
        Self::with_async_state(|| ready(Ok(())))
    }

    /// Construct App with a thread local state.
    ///
    /// State would still be shared among tasks on the same thread.
    pub fn with_current_thread_state<C, B, SF>(
        state: C,
    ) -> App<impl Fn() -> Ready<Result<C, Infallible>>, Router<C, B, SF>>
    where
        C: Clone + 'static,
    {
        Self::with_async_state(move || ready(Ok(state.clone())))
    }

    /// Construct App with a thread safe state.
    ///
    /// State would be shared among all tasks and worker threads.
    pub fn with_multi_thread_state<C, B, SF>(
        state: C,
    ) -> App<impl Fn() -> Ready<Result<C, Infallible>>, Router<C, B, SF>>
    where
        C: Send + Sync + Clone + 'static,
    {
        Self::with_async_state(move || ready(Ok(state.clone())))
    }

    #[doc(hidden)]
    /// Construct App with async closure which it's output would be used as state.
    pub fn with_async_state<CF, Fut, E, C, B, SF>(ctx_factory: CF) -> App<CF, Router<C, B, SF>>
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

impl<CF, C, B, SF> App<CF, Router<C, B, SF>> {
    pub fn at<F>(mut self, path: &'static str, factory: F) -> App<CF, Router<C, B, SF>>
    where
        WebObjectConstructor<C, B>: ObjectConstructor<F, Object = SF>,
    {
        self.router = self.router.insert(path, factory);
        self
    }
}

impl<CF, R> App<CF, R>
where
    R: BuildService,
{
    /// Enclose App with middleware type.
    /// Middleware must impl [BuildService] trait.
    pub fn enclosed<T>(self, transform: T) -> App<CF, EnclosedFactory<R, T>>
    where
        T: BuildService<R::Service> + Clone,
    {
        App {
            ctx_factory: self.ctx_factory,
            router: self.router.enclosed(transform),
        }
    }

    /// Enclose App with function as middleware type.
    pub fn enclosed_fn<Req, Req2, T>(self, transform: T) -> App<CF, EnclosedFnFactory<R, T, Req2>>
    where
        T: for<'s> AsyncClosure<(&'s R::Service, Req)> + Clone,
    {
        App {
            ctx_factory: self.ctx_factory,
            router: self.router.enclosed_fn(transform),
        }
    }

    /// Finish App build. No other App method can be called afterwards.
    pub fn finish<Fut, C, CErr>(self) -> ContextBuilder<CF, C, MapRequest<R>>
    where
        CF: Fn() -> Fut,
        Fut: Future<Output = Result<C, CErr>>,
        C: 'static,
        R::Future: 'static,
        R::Error: From<CErr>,
    {
        let App { ctx_factory, router } = self;

        ContextBuilder::new(ctx_factory).service(MapRequest { factory: router })
    }
}

pub struct MapRequest<F> {
    factory: F,
}

impl<F, Arg> BuildService<Arg> for MapRequest<F>
where
    F: BuildService<Arg>,
    F::Future: 'static,
{
    type Service = MapRequestService<F::Service>;
    type Error = F::Error;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>> + 'static;

    fn build(&self, arg: Arg) -> Self::Future {
        let fut = self.factory.build(arg);
        async {
            let service = fut.await?;
            Ok(MapRequestService { service })
        }
    }
}

pub struct MapRequestService<S> {
    service: S,
}

impl<'c, C, B, S, Res, Err> Service<Context<'c, Request<B>, C>> for MapRequestService<S>
where
    B: 'static,
    C: 'static,
    S: for<'r> Service<WebRequest<'r, C, B>, Response = Res, Error = Err>,
{
    type Response = Res;
    type Error = Err;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where S: 'f;

    fn call(&self, req: Context<'c, Request<B>, C>) -> Self::Future<'_> {
        async move {
            let (req, state) = req.into_parts();
            let (mut req, body) = req.replace_body(());
            let mut body = RefCell::new(body);
            let req = WebRequest::new(&mut req, &mut body, state);
            self.service.call(req).await
        }
    }
}

impl<'c, C, B, S, R, Res, Err> ReadyService<Context<'c, Request<B>, C>> for MapRequestService<S>
where
    B: 'static,
    C: 'static,
    S: for<'r> ReadyService<WebRequest<'r, C, B>, Response = Res, Error = Err, Ready = R>,
{
    type Ready = R;
    type ReadyFuture<'f> = impl Future<Output = Self::Ready> where S: 'f;

    #[inline]
    fn ready(&self) -> Self::ReadyFuture<'_> {
        async { self.service.ready().await }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        dev::service::Service,
        handler::{
            extension::ExtensionRef, extension::ExtensionsRef, handler_service, path::PathRef, state::StateRef,
            uri::UriRef, Responder,
        },
        http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE, Method, Uri},
        request::RequestBody,
        route::get,
    };

    async fn handler(
        StateRef(state): StateRef<'_, String>,
        PathRef(path): PathRef<'_>,
        UriRef(_): UriRef<'_>,
        ExtensionRef(_): ExtensionRef<'_, Foo>,
        ExtensionsRef(_): ExtensionsRef<'_>,
        req: &WebRequest<'_, String, NewBody<RequestBody>>,
    ) -> String {
        assert_eq!("state", state);
        assert_eq!(state, req.state());
        assert_eq!("/", path);
        assert_eq!(path, req.req().uri().path());
        state.to_string()
    }

    // Handler with no state extractor
    async fn stateless_handler(_: PathRef<'_>) -> String {
        String::from("debug")
    }

    #[derive(Clone)]
    struct Middleware;

    impl<S> BuildService<S> for Middleware {
        type Service = MiddlewareService<S>;
        type Error = Infallible;
        type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

        fn build(&self, service: S) -> Self::Future {
            async { Ok(MiddlewareService(service)) }
        }
    }

    struct MiddlewareService<S>(S);

    impl<'r, S, C, B, Res, Err> Service<WebRequest<'r, C, B>> for MiddlewareService<S>
    where
        S: for<'r2> Service<WebRequest<'r2, C, B>, Response = Res, Error = Err>,
        C: 'r,
        B: 'r,
    {
        type Response = Res;
        type Error = Err;
        type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> where Self: 'f;

        fn call(&self, mut req: WebRequest<'r, C, B>) -> Self::Future<'_> {
            async move { self.0.call(req.reborrow()).await }
        }
    }

    // arbitrary body type mutation
    struct NewBody<B>(B);

    #[test]
    fn test_app() {
        async fn middleware_fn<S, C, B, Res, Err>(service: &S, mut req: WebRequest<'_, C, B>) -> Result<Res, Infallible>
        where
            S: for<'r> Service<WebRequest<'r, C, NewBody<B>>, Response = Res, Error = Err>,
            B: Default,
            Err: for<'r> Responder<WebRequest<'r, C, B>, Output = Res>,
        {
            let body = &mut RefCell::new(NewBody(req.take_body_mut()));
            let req2 = WebRequest {
                req: req.req,
                body,
                ctx: req.ctx,
            };
            match service.call(req2).await {
                Ok(res) => Ok(res),
                Err(e) => Ok(e.respond_to(req).await),
            }
        }

        let state = String::from("state");

        let service = App::with_current_thread_state(state)
            .at("/", get(handler_service(handler)))
            .at(
                "/stateless",
                get(handler_service(stateless_handler)).head(handler_service(stateless_handler)),
            )
            .enclosed_fn(middleware_fn)
            .enclosed(Middleware)
            .finish()
            .build(())
            .now_or_panic()
            .ok()
            .unwrap();

        let mut req = Request::default();
        req.extensions_mut().insert(Foo);

        let res = service.call(req).now_or_panic().unwrap();

        assert_eq!(res.status().as_u16(), 200);
        assert_eq!(res.headers().get(CONTENT_TYPE).unwrap(), TEXT_UTF8);

        let mut req = Request::default();
        *req.uri_mut() = Uri::from_static("/abc");

        let res = service.call(req).now_or_panic().unwrap();

        assert_eq!(res.status().as_u16(), 404);

        let mut req = Request::default();
        *req.method_mut() = Method::POST;

        let res = service.call(req).now_or_panic().unwrap();

        assert_eq!(res.status().as_u16(), 405);
    }

    struct Foo;
}
