mod object;
mod router;

use core::{
    cell::RefCell,
    convert::Infallible,
    fmt,
    future::{ready, Future},
    pin::Pin,
};

use std::{borrow::Cow, error};

use futures_core::stream::Stream;
use xitca_http::util::{
    middleware::context::{Context, ContextBuilder},
    service::router::{IntoObject, RouterGen, TypedRoute},
};

use crate::{
    body::{Either, RequestBody, ResponseBody},
    bytes::Bytes,
    context::WebContext,
    error::{Error, RouterError},
    http::{WebRequest, WebResponse},
    middleware::eraser::TypeEraser,
    service::{
        object::BoxedSyncServiceObject, ready::ReadyService, AsyncClosure, EnclosedBuilder, EnclosedFnBuilder,
        MapBuilder, Service, ServiceExt,
    },
};

use self::{object::WebObject, router::AppRouter};

/// composed application type with router, stateful context and default middlewares.
pub struct App<R = (), CF = ()> {
    router: R,
    ctx_builder: CF,
}

type BoxFuture<C> = Pin<Box<dyn Future<Output = Result<C, Box<dyn fmt::Debug>>>>>;
type CtxBuilder<C> = Box<dyn Fn() -> BoxFuture<C> + Send + Sync>;
type DefaultWebObject<C> = WebObject<C, RequestBody, WebResponse, RouterError<Error<C>>>;
type DefaultAppRouter<C> = AppRouter<BoxedSyncServiceObject<(), DefaultWebObject<C>, Infallible>>;

// helper trait to poly between () and Box<dyn Fn()> as application state.
pub trait IntoCtx<C> {
    fn into_ctx(self) -> impl Fn() -> BoxFuture<C> + Send + Sync;
}

impl IntoCtx<()> for () {
    fn into_ctx(self) -> impl Fn() -> BoxFuture<()> + Send + Sync {
        || Box::pin(ready(Ok(())))
    }
}

impl<C> IntoCtx<C> for CtxBuilder<C> {
    fn into_ctx(self) -> impl Fn() -> BoxFuture<C> + Send + Sync {
        self
    }
}

/// type alias for concrete type of nested App.
///
/// # Example
/// ```rust
/// # use xitca_web::{handler::handler_service, App, NestApp, WebContext};
/// // a function return an App instance.
/// fn app() -> NestApp<usize> {
///     App::new().at("/index", handler_service(|_: &WebContext<'_, usize>| async { "" }))
/// }
///
/// // nest app would be registered with /v2 as prefix therefore "/v2/index" become accessible.
/// App::new().at("/v2", app()).with_state(996usize);
/// ```
pub type NestApp<C> = App<DefaultAppRouter<C>>;

impl App {
    /// Construct a new application instance.
    pub fn new<Obj>() -> App<AppRouter<Obj>> {
        App {
            router: AppRouter::new(),
            ctx_builder: (),
        }
    }
}

impl<Obj, CF> App<AppRouter<Obj>, CF> {
    /// insert routed service with given path to application.
    pub fn at<F, C, B>(mut self, path: &'static str, builder: F) -> Self
    where
        F: RouterGen + Service + Send + Sync,
        F::Response: for<'r> Service<WebContext<'r, C, B>>,
        for<'r> WebContext<'r, C, B>: IntoObject<F::Route<F>, (), Object = Obj>,
    {
        self.router = self.router.insert(path, builder);
        self
    }

    /// insert typed route service with given path to application.
    pub fn at_typed<T, C>(mut self, typed: T) -> Self
    where
        T: TypedRoute<C, Route = Obj>,
    {
        self.router = self.router.insert_typed(typed);
        self
    }
}

impl<R, CF> App<R, CF> {
    /// Construct App with a thread safe state that will be shared among all tasks and worker threads.
    ///
    /// State accessing is based on generic type approach where the State type and it's typed fields are generally
    /// opaque to middleware and routing services of the application. In order to cast concrete type from generic
    /// state type [std::borrow::Borrow] trait is utilized. See example below for explanation.
    ///
    /// # Example
    /// ```rust
    /// # use std::borrow::Borrow;
    /// # use xitca_web::{
    /// #   error::Error,
    /// #   handler::{handler_service, state::StateRef, FromRequest},
    /// #   service::Service,
    /// #   App, WebContext
    /// # };
    /// // our typed state.
    /// #[derive(Clone, Default)]
    /// struct State {
    ///     string: String,
    ///     usize: usize
    /// }
    ///
    /// // implement Borrow trait to enable borrowing &String type from State.
    /// impl Borrow<String> for State {
    ///     fn borrow(&self) -> &String {
    ///         &self.string
    ///     }
    /// }
    ///
    /// App::new()
    ///     .with_state(State::default())// construct app with state type.
    ///     .at("/", handler_service(index)) // a function service that have access to state.
    ///     # .at("/nah", handler_service(|_: &WebContext<'_, State>| async { "used for infer type" }))
    ///     .enclosed_fn(middleware_fn); // a function middleware that have access to state
    ///
    /// // the function service don't know what the real type of application state is.
    /// // it only needs to know &String can be borrowed from it.
    /// async fn index(_: StateRef<'_, String>) -> &'static str {
    ///     ""
    /// }
    ///
    /// // similar to function service. the middleware does not need to know the real type of C.
    /// // it only needs to know it implement according trait.
    /// async fn middleware_fn<S, C, Res>(service: &S, ctx: WebContext<'_, C>) -> Result<Res, Error<C>>
    /// where
    ///     S: for<'r> Service<WebContext<'r, C>, Response = Res, Error = Error<C>>,
    ///     C: Borrow<String> // annotate we want to borrow &String from generic C state type.
    /// {
    ///     // WebContext::state would return &C then we can call Borrow::borrow on it to get &String
    ///     let _string = ctx.state().borrow();
    ///     // or use extractor manually like in function service.
    ///     let _string = StateRef::<'_, String>::from_request(&ctx).await?;
    ///     service.call(ctx).await
    /// }
    /// ```
    pub fn with_state<C>(self, state: C) -> App<R, CtxBuilder<C>>
    where
        C: Send + Sync + Clone + 'static,
    {
        self.with_async_state(move || ready(Ok::<_, Infallible>(state.clone())))
    }

    /// Construct App with async closure which it's output would be used as state.
    /// async state is used to produce thread per core and/or non thread safe state copies.
    /// The output state is not bound to `Send` and `Sync` auto traits.
    pub fn with_async_state<CF1, Fut, C, E>(self, builder: CF1) -> App<R, CtxBuilder<C>>
    where
        CF1: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<C, E>> + 'static,
        E: fmt::Debug + 'static,
    {
        let ctx_builder = Box::new(move || {
            let fut = builder();
            Box::pin(async { fut.await.map_err(|e| Box::new(e) as Box<dyn fmt::Debug>) }) as _
        });

        App {
            router: self.router,
            ctx_builder,
        }
    }
}

impl<R, CF> App<R, CF>
where
    R: Service + Send + Sync,
    R::Error: fmt::Debug + 'static,
{
    /// Enclose App with middleware type.
    /// Middleware must impl [Service] trait.
    pub fn enclosed<T>(self, transform: T) -> App<EnclosedBuilder<R, T>, CF>
    where
        T: Service<Result<R::Response, R::Error>>,
    {
        App {
            router: self.router.enclosed(transform),
            ctx_builder: self.ctx_builder,
        }
    }

    /// Enclose App with function as middleware type.
    pub fn enclosed_fn<Req, T>(self, transform: T) -> App<EnclosedFnBuilder<R, T>, CF>
    where
        T: for<'s> AsyncClosure<(&'s R::Response, Req)> + Clone,
    {
        App {
            router: self.router.enclosed_fn(transform),
            ctx_builder: self.ctx_builder,
        }
    }

    /// Mutate `<<Self::Response as Service<Req>>::Future as Future>::Output` type with given
    /// closure.
    pub fn map<T, Res, ResMap>(self, mapper: T) -> App<MapBuilder<R, T>, CF>
    where
        T: Fn(Res) -> ResMap + Clone,
        Self: Sized,
    {
        App {
            router: self.router.map(mapper),
            ctx_builder: self.ctx_builder,
        }
    }
}

impl<R, CF> App<R, CF>
where
    R: Service + Send + Sync,
    R::Error: fmt::Debug + 'static,
{
    /// Finish App build. No other App method can be called afterwards.
    pub fn finish<C, ResB, SE>(
        self,
    ) -> impl Service<
        Response = impl ReadyService + Service<WebRequest, Response = WebResponse<EitherResBody<ResB>>, Error = Infallible>,
        Error = impl fmt::Debug,
    >
    where
        R::Response: ReadyService + for<'r> Service<WebContext<'r, C>, Response = WebResponse<ResB>, Error = SE>,
        SE: for<'r> Service<WebContext<'r, C>, Response = WebResponse, Error = Infallible>,
        CF: IntoCtx<C>,
        C: 'static,
    {
        let App { ctx_builder, router } = self;
        router
            .enclosed_fn(map_req_res)
            .enclosed(ContextBuilder::new(ctx_builder.into_ctx()))
    }

    /// Finish App build. No other App method can be called afterwards.
    pub fn finish_boxed<C, ResB, SE, BE>(
        self,
    ) -> AppObject<impl ReadyService + Service<WebRequest, Response = WebResponse, Error = Infallible>>
    where
        R: 'static,
        R::Response:
            ReadyService + for<'r> Service<WebContext<'r, C>, Response = WebResponse<ResB>, Error = SE> + 'static,
        SE: for<'r> Service<WebContext<'r, C>, Response = WebResponse, Error = Infallible> + 'static,
        ResB: Stream<Item = Result<Bytes, BE>> + 'static,
        BE: error::Error + Send + Sync + 'static,
        CF: IntoCtx<C> + 'static,
        C: 'static,
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

        Box::new(BoxApp(self.finish().enclosed(TypeEraser::response_body())))
    }

    #[cfg(feature = "__server")]
    /// Finish App build and serve is with [HttpServer]. No other App method can be called afterwards.
    ///
    /// [HttpServer]: crate::server::HttpServer
    pub fn serve<C, ResB, SE>(
        self,
    ) -> crate::server::HttpServer<
        impl Service<
            Response = impl ReadyService
                           + Service<WebRequest, Response = WebResponse<EitherResBody<ResB>>, Error = Infallible>,
            Error = impl fmt::Debug,
        >,
    >
    where
        R: 'static,
        R::Response: ReadyService + for<'r> Service<WebContext<'r, C>, Response = WebResponse<ResB>, Error = SE>,
        SE: for<'r> Service<WebContext<'r, C>, Response = WebResponse, Error = Infallible> + 'static,
        ResB: 'static,
        CF: IntoCtx<C> + 'static,
        C: 'static,
    {
        crate::server::HttpServer::serve(self.finish())
    }
}

impl<R, F> RouterGen for App<R, F>
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

impl<R, Arg, F> Service<Arg> for App<R, F>
where
    R: Service<Arg>,
{
    type Response = R::Response;
    type Error = R::Error;

    async fn call(&self, req: Arg) -> Result<Self::Response, Self::Error> {
        self.router.call(req).await
    }
}

/// object safe [App] instance. used for case where naming [App]'s type is needed.
pub type AppObject<S> =
    Box<dyn xitca_service::object::ServiceObject<(), Response = S, Error = Box<dyn fmt::Debug>> + Send + Sync>;

type EitherResBody<B> = Either<B, ResponseBody>;

// middleware for converting xitca_http types to xitca_web types.
// this is for enabling side effect see [WebContext::reborrow] for detail.
async fn map_req_res<C, S, SE, ResB>(
    service: &S,
    ctx: Context<'_, WebRequest, C>,
) -> Result<WebResponse<EitherResBody<ResB>>, Infallible>
where
    C: 'static,
    S: for<'r> Service<WebContext<'r, C>, Response = WebResponse<ResB>, Error = SE>,
    SE: for<'r> Service<WebContext<'r, C>, Response = WebResponse, Error = Infallible>,
{
    let (req, state) = ctx.into_parts();
    let (parts, ext) = req.into_parts();
    let (ext, body) = ext.replace_body(());
    let mut req = WebRequest::from_parts(parts, ext);
    let mut body = RefCell::new(body);
    let mut ctx = WebContext::new(&mut req, &mut body, state);

    match service.call(ctx.reborrow()).await {
        Ok(res) => Ok(res.map(Either::left)),
        Err(e) => e.call(ctx).await.map(|res| res.map(Either::right)),
    }
}

#[cfg(test)]
mod test {
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        handler::{
            extension::ExtensionRef, extension::ExtensionsRef, handler_service, path::PathRef, state::StateRef,
            uri::UriRef,
        },
        http::{const_header_value::TEXT_UTF8, header::CONTENT_TYPE, request, Method},
        middleware::UncheckedReady,
        route::get,
        service::Service,
    };

    use super::*;

    async fn middleware<S, C, B, Res, Err>(s: &S, req: WebContext<'_, C, B>) -> Result<Res, Err>
    where
        S: for<'r> Service<WebContext<'r, C, B>, Response = Res, Error = Err>,
    {
        s.call(req).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn handler(
        _res: Result<UriRef<'_>, Error<String>>,
        _opt: Option<UriRef<'_>>,
        _req: &WebRequest<()>,
        StateRef(state): StateRef<'_, String>,
        PathRef(path): PathRef<'_>,
        UriRef(_): UriRef<'_>,
        ExtensionRef(_): ExtensionRef<'_, Foo>,
        ExtensionsRef(_): ExtensionsRef<'_>,
        req: &WebContext<'_, String>,
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

    #[allow(clippy::borrow_interior_mutable_const)]
    #[test]
    fn test_app() {
        let state = String::from("state");

        let service = App::new()
            .at("/", get(handler_service(handler)))
            .with_state(state)
            .at(
                "/stateless",
                get(handler_service(stateless_handler)).head(handler_service(stateless_handler)),
            )
            .enclosed_fn(middleware)
            .enclosed(Middleware)
            .enclosed(UncheckedReady)
            .finish()
            .call(())
            .now_or_panic()
            .ok()
            .unwrap();

        let mut req = WebRequest::default();
        req.extensions_mut().insert(Foo);

        let res = service.call(req).now_or_panic().unwrap();

        assert_eq!(res.status().as_u16(), 200);

        assert_eq!(res.headers().get(CONTENT_TYPE).unwrap(), TEXT_UTF8);

        let req = request::Builder::default()
            .uri("/abc")
            .body(Default::default())
            .unwrap();

        let res = service.call(req).now_or_panic().unwrap();

        assert_eq!(res.status().as_u16(), 404);

        let req = request::Builder::default()
            .method(Method::POST)
            .body(Default::default())
            .unwrap();

        let res = service.call(req).now_or_panic().unwrap();

        assert_eq!(res.status().as_u16(), 405);
    }

    #[derive(Clone)]
    struct Foo;

    #[test]
    fn app_nest_router() {
        async fn handler(StateRef(state): StateRef<'_, String>, PathRef(path): PathRef<'_>) -> String {
            assert_eq!("state", state);
            assert_eq!("/scope/nest", path);
            state.to_string()
        }

        fn app() -> NestApp<String> {
            App::new().at("/nest", get(handler_service(handler)))
        }

        let state = String::from("state");
        let service = App::new()
            .with_state(state)
            .at("/root", get(handler_service(handler)))
            .at("/scope", app())
            .finish()
            .call(())
            .now_or_panic()
            .ok()
            .unwrap();

        let req = request::Builder::default()
            .uri("/scope/nest")
            .body(Default::default())
            .unwrap();

        let res = service.call(req).now_or_panic().unwrap();

        assert_eq!(res.status().as_u16(), 200);
    }
}
