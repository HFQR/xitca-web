//! compatibility between tower-http layer and xitca-web middleware.

use tower_layer::Layer;

use crate::service::{Service, tower_http_compat::TowerCompatService};

/// A middleware type that bridge `xitca-service` and `tower-service`.
/// Any `tower-http` type that impl [Layer] trait can be passed to it and used as xitca-web's middleware.
///
/// # Type mutation
/// `TowerHttpCompat` would mutate response body type from `B` to `CompatBody<ResB>`. Service enclosed
/// by it must be able to handle it's mutation or utilize [TypeEraser] to erase the mutation.
///
/// [TypeEraser]: crate::middleware::eraser::TypeEraser
#[derive(Clone)]
pub struct TowerHttpCompat<L>(L);

impl<L> TowerHttpCompat<L> {
    /// Construct a new xitca-web middleware from tower-http layer type.
    ///
    /// # Limitation:
    /// tower::Service::poll_ready implementation is ignored by TowerHttpCompat.
    /// if a tower Service is sensitive to poll_ready implementation it should not be used.
    ///
    /// # Example:
    /// ```rust
    /// # use std::convert::Infallible;
    /// # use xitca_web::{http::{StatusCode, WebResponse}, service::fn_service, App, WebContext};
    /// use xitca_web::middleware::tower_http_compat::TowerHttpCompat;
    /// use tower_http::set_status::SetStatusLayer;
    ///
    /// # fn doc_example() {
    /// App::new()
    ///     .at("/", fn_service(handler))
    ///     .enclosed(TowerHttpCompat::new(SetStatusLayer::new(StatusCode::NOT_FOUND)));
    /// # }
    ///
    /// # async fn handler(ctx: WebContext<'_>) -> Result<WebResponse, Infallible> {
    /// #   todo!()
    /// # }
    /// ```
    pub const fn new(layer: L) -> Self {
        Self(layer)
    }
}

impl<L, S, E> Service<Result<S, E>> for TowerHttpCompat<L>
where
    L: Layer<compat_layer::CompatLayer<S>>,
{
    type Response = TowerCompatService<L::Service>;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res.map(|service| {
            let service = self.0.layer(compat_layer::CompatLayer::new(service));
            TowerCompatService::new(service)
        })
    }
}

mod compat_layer {
    use core::{
        cell::RefCell,
        future::Future,
        pin::Pin,
        task::{Context, Poll},
    };

    use std::rc::Rc;

    use crate::{
        WebContext,
        http::{Request, RequestExt, Response, WebResponse},
        service::tower_http_compat::{CompatReqBody, CompatResBody},
    };

    use super::*;

    pub struct CompatLayer<S>(Rc<S>);

    impl<S> CompatLayer<S> {
        pub(super) fn new(service: S) -> Self {
            Self(Rc::new(service))
        }
    }

    impl<S, C, ReqB, ResB, Err> tower_service::Service<Request<CompatReqBody<RequestExt<ReqB>, C>>> for CompatLayer<S>
    where
        S: for<'r> Service<WebContext<'r, C, ReqB>, Response = WebResponse<ResB>, Error = Err> + 'static,
        C: 'static,
        ReqB: 'static,
    {
        type Response = Response<CompatResBody<ResB>>;
        type Error = Err;
        type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

        #[inline]
        fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: Request<CompatReqBody<RequestExt<ReqB>, C>>) -> Self::Future {
            let service = self.0.clone();
            Box::pin(async move {
                let (parts, body) = req.into_parts();
                let (body, ctx) = body.into_parts();
                let (ext, body) = body.replace_body(());

                let mut req = Request::from_parts(parts, ext);
                let mut body = RefCell::new(body);
                let req = WebContext::new(&mut req, &mut body, &ctx);

                service.call(req).await.map(|res| res.map(CompatResBody::new))
            })
        }
    }
}

#[cfg(test)]
mod test {
    use core::convert::Infallible;

    use tower_http::set_status::SetStatusLayer;
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        App, WebContext,
        body::ResponseBody,
        http::WebRequest,
        http::{StatusCode, WebResponse},
        service::fn_service,
    };

    use super::*;

    async fn handler(ctx: WebContext<'_, &'static str>) -> Result<WebResponse, Infallible> {
        assert_eq!(*ctx.state(), "996");
        Ok(ctx.into_response(ResponseBody::empty()))
    }

    #[test]
    fn tower_set_status() {
        let res = App::new()
            .with_state("996")
            .at("/", fn_service(handler))
            .enclosed(TowerHttpCompat::new(SetStatusLayer::new(StatusCode::OK)))
            .enclosed(TowerHttpCompat::new(SetStatusLayer::new(StatusCode::NOT_FOUND)))
            .finish()
            .call(())
            .now_or_panic()
            .unwrap()
            .call(WebRequest::default())
            .now_or_panic()
            .unwrap();

        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
