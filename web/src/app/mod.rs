mod object;

use core::{
    cell::RefCell,
    convert::Infallible,
    fmt,
    future::{ready, Future, Ready},
};

use std::{borrow::Cow, error};

use futures_core::stream::Stream;
use xitca_http::util::{
    middleware::context::{Context, ContextBuilder},
    service::router::{IntoObject, Router, RouterGen, RouterMapErr},
};

use crate::{
    body::{RequestBody, ResponseBody},
    bytes::Bytes,
    context::WebContext,
    dev::service::{ready::ReadyService, AsyncClosure, EnclosedFactory, EnclosedFnFactory, Service, ServiceExt},
    error::{Error, RouterError},
    http::{Request, RequestExt, WebResponse},
};

/// composed application type with router, stateful context and default middlewares.
pub struct App<CF = (), R = ()> {
    ctx_factory: CF,
    router: R,
}

impl App {
    /// Construct a new application instance.
    pub fn new<Obj>() -> App<impl Fn() -> Ready<Result<(), Infallible>>, AppRouter<Obj>> {
        Self::with_state(())
    }

    /// Construct App with a thread safe state.
    ///
    /// State would be shared among all tasks and worker threads.
    pub fn with_state<C, Obj>(state: C) -> App<impl Fn() -> Ready<Result<C, Infallible>>, AppRouter<Obj>>
    where
        C: Send + Sync + Clone + 'static,
    {
        Self::with_async_state(move || ready(Ok(state.clone())))
    }

    /// Construct App with async closure which it's output would be used as state.
    /// async state is used to produce thread per core and/or non thread safe state copies.
    /// The output state is not bound to `Send` and `Sync` auto traits.
    pub fn with_async_state<CF, Obj>(ctx_factory: CF) -> App<CF, AppRouter<Obj>> {
        App {
            ctx_factory,
            router: AppRouter(Router::new()),
        }
    }
}

impl<CF, Obj> App<CF, AppRouter<Obj>> {
    /// insert routed service with given path to application.
    pub fn at<C, F, B>(mut self, path: &'static str, factory: F) -> App<CF, AppRouter<Obj>>
    where
        F: RouterGen + Service + Send + Sync,
        F::Response: for<'r> Service<WebContext<'r, C, B>>,
        for<'r> WebContext<'r, C, B>: IntoObject<F::Route<F>, (), Object = Obj>,
    {
        self.router = AppRouter(self.router.0.insert(path, factory));
        self
    }
}

impl<CF, Fut, C, CErr, R> App<CF, R>
where
    CF: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = Result<C, CErr>>,
    CErr: fmt::Debug + 'static,
    R: Service + Send + Sync,
    R::Error: fmt::Debug + 'static,
{
    /// Enclose App with middleware type.
    /// Middleware must impl [Service] trait.
    pub fn enclosed<T>(self, transform: T) -> App<CF, EnclosedFactory<R, T>>
    where
        T: Service<Result<R::Response, R::Error>>,
    {
        App {
            ctx_factory: self.ctx_factory,
            router: self.router.enclosed(transform),
        }
    }

    /// Enclose App with function as middleware type.
    pub fn enclosed_fn<Req, T>(self, transform: T) -> App<CF, EnclosedFnFactory<R, T>>
    where
        T: for<'s> AsyncClosure<(&'s R::Response, Req)> + Clone,
    {
        App {
            ctx_factory: self.ctx_factory,
            router: self.router.enclosed_fn(transform),
        }
    }

    /// Finish App build. No other App method can be called afterwards.
    pub fn finish<ReqB, ResB, SE, B, BE>(
        self,
    ) -> impl Service<
        Response = impl ReadyService
                       + Service<
            Request<RequestExt<ReqB>>,
            Response = WebResponse<ResponseBody<ResB>>,
            Error = Infallible,
        >,
        Error = impl fmt::Debug,
    >
    where
        C: 'static,
        R::Response: ReadyService + for<'r> Service<WebContext<'r, C, ReqB>, Response = WebResponse<ResB>, Error = SE>,
        SE: for<'r> Service<WebContext<'r, C>, Response = WebResponse, Error = Infallible>,
        ReqB: 'static,
        ResB: Stream<Item = Result<B, BE>>,
    {
        let App { ctx_factory, router } = self;
        router
            .enclosed_fn(map_req_res)
            .enclosed(ContextBuilder::new(ctx_factory))
    }

    /// Finish App build. No other App method can be called afterwards.
    pub fn finish_boxed<ReqB, ResB, SE, BE>(
        self,
    ) -> AppObject<impl ReadyService + Service<Request<RequestExt<ReqB>>, Response = WebResponse, Error = Infallible>>
    where
        CF: 'static,
        Fut: 'static,
        C: 'static,
        R: 'static,
        R::Response:
            ReadyService + for<'r> Service<WebContext<'r, C, ReqB>, Response = WebResponse<ResB>, Error = SE> + 'static,
        SE: for<'r> Service<WebContext<'r, C>, Response = WebResponse, Error = Infallible> + 'static,
        ReqB: 'static,
        ResB: Stream<Item = Result<Bytes, BE>> + 'static,
        BE: error::Error + Send + Sync + 'static,
    {
        struct BoxApp<S>(S);

        impl<S, Arg> Service<Arg> for BoxApp<S>
        where
            S: Service<Arg>,
            S::Error: fmt::Debug + 'static,
        {
            type Response = S::Response;
            type Error = Box<dyn fmt::Debug>;

            async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
                self.0.call(arg).await.map_err(|e| Box::new(e) as _)
            }
        }

        async fn box_res<S, Req, ResB, E>(service: &S, req: Req) -> Result<WebResponse, S::Error>
        where
            S: Service<Req, Response = WebResponse<ResponseBody<ResB>>>,
            ResB: Stream<Item = Result<Bytes, E>> + 'static,
            E: error::Error + Send + Sync + 'static,
        {
            service.call(req).await.map(|res| res.map(|body| body.into_boxed()))
        }

        Box::new(BoxApp(self.finish().enclosed_fn(box_res)))
    }

    #[cfg(feature = "__server")]
    /// Finish App build and serve is with [HttpServer]. No other App method can be called afterwards.
    ///
    /// [HttpServer]: crate::server::HttpServer
    pub fn serve<ReqB, ResB, SE, B, BE>(
        self,
    ) -> crate::server::HttpServer<
        impl Service<
            Response = impl ReadyService
                           + Service<
                Request<RequestExt<ReqB>>,
                Response = WebResponse<ResponseBody<ResB>>,
                Error = Infallible,
            >,
            Error = impl fmt::Debug,
        >,
    >
    where
        CF: 'static,
        Fut: 'static,
        C: 'static,
        R: 'static,
        R::Response: ReadyService + for<'r> Service<WebContext<'r, C, ReqB>, Response = WebResponse<ResB>, Error = SE>,
        SE: for<'r> Service<WebContext<'r, C>, Response = WebResponse, Error = Infallible> + 'static,
        ReqB: 'static,
        ResB: Stream<Item = Result<B, BE>> + 'static,
        B: 'static,
        BE: 'static,
    {
        crate::server::HttpServer::serve(self.finish())
    }
}

impl<CF, R> RouterGen for App<CF, R>
where
    R: RouterGen,
{
    type Route<R1> = R::Route<R1>;

    fn path_gen(&mut self, prefix: &'static str) -> Cow<'static, str> {
        self.router.path_gen(prefix)
    }

    fn route_gen<R1>(route: R1) -> Self::Route<R1> {
        R::route_gen(route)
    }
}

impl<CF, R, Arg> Service<Arg> for App<CF, R>
where
    R: Service<Arg>,
{
    type Response = R::Response;
    type Error = R::Error;

    async fn call(&self, req: Arg) -> Result<Self::Response, Self::Error> {
        self.router.call(req).await
    }
}

/// application wrap around [Router] and transform it's error type into [Error]
pub struct AppRouter<Obj>(Router<Obj>);

impl<Obj> RouterGen for AppRouter<Obj> {
    type Route<R1> = RouterMapErr<<Router<Obj> as RouterGen>::Route<R1>>;

    fn path_gen(&mut self, prefix: &'static str) -> Cow<'static, str> {
        self.0.path_gen(prefix)
    }

    fn route_gen<R1>(route: R1) -> Self::Route<R1> {
        RouterMapErr(<Router<Obj> as RouterGen>::route_gen(route))
    }
}

impl<Arg, Obj> Service<Arg> for AppRouter<Obj>
where
    Router<Obj>: Service<Arg>,
{
    type Response = RouterService<<Router<Obj> as Service<Arg>>::Response>;
    type Error = <Router<Obj> as Service<Arg>>::Error;

    async fn call(&self, arg: Arg) -> Result<Self::Response, Self::Error> {
        self.0.call(arg).await.map(RouterService)
    }
}

pub struct RouterService<S>(S);

impl<'r, S, C, B, Res, E> Service<WebContext<'r, C, B>> for RouterService<S>
where
    S: for<'r2> Service<WebContext<'r2, C, B>, Response = Res, Error = RouterError<E>>,
    Error<C>: From<E>,
{
    type Response = Res;
    type Error = Error<C>;

    #[inline]
    async fn call(&self, req: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
        self.0.call(req).await.map_err(|e| match e {
            RouterError::Match(e) => Error::from_service(e),
            RouterError::NotAllowed(e) => Error::from_service(e),
            RouterError::Service(e) => Error::from(e),
        })
    }
}

impl<S> ReadyService for RouterService<S>
where
    S: ReadyService,
{
    type Ready = S::Ready;

    #[inline]
    async fn ready(&self) -> Self::Ready {
        self.0.ready().await
    }
}

/// object safe [App] instance. used for case where naming [App]'s type is needed.
pub type AppObject<S> =
    Box<dyn xitca_service::object::ServiceObject<(), Response = S, Error = Box<dyn fmt::Debug>> + Send + Sync>;

// middleware for converting xitca_http types to xitca_web types.
// this is for enabling side effect see [WebContext::reborrow] for detail.
async fn map_req_res<C, S, SE, ReqB, ResB, B, BE>(
    service: &S,
    ctx: Context<'_, Request<RequestExt<ReqB>>, C>,
) -> Result<WebResponse<ResponseBody<ResB>>, Infallible>
where
    C: 'static,
    S: for<'r> Service<WebContext<'r, C, ReqB>, Response = WebResponse<ResB>, Error = SE>,
    SE: for<'r> Service<WebContext<'r, C>, Response = WebResponse, Error = Infallible>,
    ReqB: 'static,
    ResB: Stream<Item = Result<B, BE>>,
{
    let (req, state) = ctx.into_parts();
    let (parts, ext) = req.into_parts();
    let (ext, body) = ext.replace_body(());
    let mut req = Request::from_parts(parts, ext);
    let mut body = RefCell::new(body);

    match service.call(WebContext::new(&mut req, &mut body, state)).await {
        Ok(res) => Ok(res.map(|body| ResponseBody::stream(body))),
        // TODO: mutate response header according to outcome of drop_stream_cast?
        Err(e) => {
            let mut body = RefCell::new(RequestBody::None);
            let ctx = WebContext::new(&mut req, &mut body, state);
            let res = e.call(ctx).await?;
            Ok(res.map(|body| body.drop_stream_cast()))
        }
    }
}

#[cfg(test)]
mod test {
    use std::{
        pin::Pin,
        task::{self, Poll},
    };

    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        body::RequestBody,
        dev::service::Service,
        handler::{
            extension::ExtensionRef, extension::ExtensionsRef, handler_service, path::PathRef, state::StateRef,
            uri::UriRef,
        },
        http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE, Method, Uri},
        middleware::UncheckedReady,
        route::get,
    };

    use super::*;

    async fn middleware<S, C, B, Res, Err>(s: &S, req: WebContext<'_, C, B>) -> Result<Res, Err>
    where
        S: for<'r> Service<WebContext<'r, C, B>, Response = Res, Error = Err>,
    {
        s.call(req).await
    }

    async fn handler(
        StateRef(state): StateRef<'_, String>,
        PathRef(path): PathRef<'_>,
        UriRef(_): UriRef<'_>,
        ExtensionRef(_): ExtensionRef<'_, Foo>,
        ExtensionsRef(_): ExtensionsRef<'_>,
        req: &WebContext<'_, String, NewBody<RequestBody>>,
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

    impl<S, E> Service<Result<S, E>> for Middleware {
        type Response = MiddlewareService<S>;
        type Error = E;

        async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
            res.map(MiddlewareService)
        }
    }

    struct MiddlewareService<S>(S);

    impl<'r, S, C, B, Res, Err> Service<WebContext<'r, C, B>> for MiddlewareService<S>
    where
        S: for<'r2> Service<WebContext<'r2, C, B>, Response = Res, Error = Err>,
        C: 'r,
        B: 'r,
    {
        type Response = Res;
        type Error = Err;

        async fn call(&self, mut req: WebContext<'r, C, B>) -> Result<Self::Response, Self::Error> {
            self.0.call(req.reborrow()).await
        }
    }

    // arbitrary body type mutation
    struct NewBody<B>(B);

    impl<B> Stream for NewBody<B>
    where
        B: Stream + Unpin,
    {
        type Item = B::Item;

        fn poll_next(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
            Pin::new(&mut self.get_mut().0).poll_next(cx)
        }
    }

    #[allow(clippy::borrow_interior_mutable_const)]
    #[test]
    fn test_app() {
        async fn middleware_fn<S, C, B, Res, Err>(service: &S, mut req: WebContext<'_, C, B>) -> Result<Res, Infallible>
        where
            S: for<'r> Service<WebContext<'r, C, NewBody<B>>, Response = Res, Error = Err>,
            Err: for<'r> Service<WebContext<'r, C>, Response = Res, Error = Infallible>,
            B: Default,
        {
            let mut body = RefCell::new(NewBody(req.take_body_mut()));
            match service.call(WebContext::new(req.req, &mut body, req.ctx)).await {
                Ok(res) => Ok(res),
                Err(e) => {
                    let mut body = RefCell::new(RequestBody::None);
                    e.call(WebContext::new(req.req, &mut body, req.ctx)).await
                }
            }
        }

        let state = String::from("state");

        let service = App::with_state(state)
            .at("/", get(handler_service(handler)))
            .at(
                "/stateless",
                get(handler_service(stateless_handler)).head(handler_service(stateless_handler)),
            )
            .enclosed_fn(middleware_fn)
            .enclosed(Middleware)
            .enclosed(UncheckedReady)
            .finish()
            .call(())
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

    #[test]
    fn app_nest_router() {
        async fn handler(StateRef(state): StateRef<'_, String>, PathRef(path): PathRef<'_>) -> String {
            assert_eq!("state", state);
            assert_eq!("/scope/nest", path);
            state.to_string()
        }

        let state = String::from("state");
        let service = App::with_state(state)
            .at("/root", get(handler_service(handler)))
            .at(
                "/scope",
                App::new()
                    .at("/nest", get(handler_service(handler)))
                    .enclosed_fn(middleware),
            )
            .finish()
            .call(())
            .now_or_panic()
            .ok()
            .unwrap();

        let req = Request::builder()
            .uri("/scope/nest")
            .body(RequestExt::<RequestBody>::default())
            .unwrap();

        let res = service.call(req).now_or_panic().unwrap();

        assert_eq!(res.status().as_u16(), 200);
    }
}
