use crate::service::Service;

/// middleware for bridging `xitca-http` and `xitca-web` types. useful for utilizing xitca-web
/// when manually constructing http services.
///
/// # Examples
/// ```rust
/// use xitca_http::util::{
///     middleware::context::ContextBuilder,
///     service::{handler::handler_service, route::get, Router}
/// };
/// use xitca_service::{Service, ServiceExt};
/// use xitca_web::{error::Error, handler::state::StateRef};
///
/// // handler function depending on xitca-web types.
/// async fn handler(StateRef(_): StateRef<'_, ()>) -> Error {
///     xitca_web::error::ErrorStatus::bad_request().into()
/// }
///
/// # async fn test() {
/// // low level router from xitca-http
/// let router = Router::new()
///     // utilize handler function with xitca-web types
///     .insert("/", get(handler_service(handler)))
///     // utilize xitca-web's middleware to xitca-http's router
///     .enclosed(xitca_web::middleware::CatchUnwind)
///     // converting xitca-http types to xitca-web types with web context middleware.
///     .enclosed(xitca_web::middleware::WebContext)
///     // xitca-http's context builder middleware is also needed for helping type conversion.
///     // this middleware is tasked with constructing typed application state and pass down it
///     // to xitca_web's WebContext middleware. In case of absence of application state a tuple
///     // is constructed to act as the default type.
///     .enclosed(ContextBuilder::new(|| async { Ok::<_, std::convert::Infallible>(()) }));
///
/// // call the router service and observe the output. in real world the service caller should be a http
/// // library and server.
/// let res = router
///     .call(())
///     .await
///     .unwrap()
///     .call(Default::default())
///     .await
///     .unwrap();
///
/// assert_eq!(res.status(), xitca_http::http::StatusCode::BAD_REQUEST);
/// # }
/// ```
pub struct WebContext;

impl<S, E> Service<Result<S, E>> for WebContext {
    type Response = service::WebContextService<S>;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res.map(|service| service::WebContextService { service })
    }
}

mod service {
    use core::{cell::RefCell, convert::Infallible};

    use xitca_http::util::middleware::context::Context;

    use crate::{
        body::{Either, ResponseBody},
        context::WebContext,
        http::{WebRequest, WebResponse},
        service::{Service, ready::ReadyService},
    };

    pub struct WebContextService<S> {
        pub(super) service: S,
    }

    type EitherResBody<B> = Either<B, ResponseBody>;

    impl<'c, S, C, ResB, SE> Service<Context<'c, WebRequest, C>> for WebContextService<S>
    where
        S: for<'r> Service<WebContext<'r, C>, Response = WebResponse<ResB>, Error = SE>,
        SE: for<'r> Service<WebContext<'r, C>, Response = WebResponse, Error = Infallible>,
    {
        type Response = WebResponse<EitherResBody<ResB>>;
        type Error = Infallible;

        async fn call(&self, ctx: Context<'c, WebRequest, C>) -> Result<Self::Response, Self::Error> {
            let (req, state) = ctx.into_parts();
            let (parts, ext) = req.into_parts();
            let (ext, body) = ext.replace_body(());
            let mut req = WebRequest::from_parts(parts, ext);
            let mut body = RefCell::new(body);
            let mut ctx = WebContext::new(&mut req, &mut body, state);

            match self.service.call(ctx.reborrow()).await {
                Ok(res) => Ok(res.map(Either::left)),
                Err(e) => e.call(ctx).await.map(|res| res.map(Either::right)),
            }
        }
    }

    impl<S> ReadyService for WebContextService<S>
    where
        S: ReadyService,
    {
        type Ready = S::Ready;

        #[inline]
        async fn ready(&self) -> Self::Ready {
            self.service.ready().await
        }
    }
}
