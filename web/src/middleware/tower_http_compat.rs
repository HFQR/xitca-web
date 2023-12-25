use core::{
    cell::RefCell,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use std::rc::Rc;

use tower_layer::Layer;
use xitca_unsafe_collection::fake::{FakeClone, FakeSend, FakeSync};

use crate::{
    context::WebContext,
    http::{Request, RequestExt, Response, WebResponse},
    service::{
        tower_http_compat::{CompatReqBody, CompatResBody, TowerCompatService},
        Service,
    },
};

/// A middleware type that bridge `xitca-service` and `tower-service`.
/// Any `tower-http` type that impl [Layer] trait can be passed to it and used as xitca-web's middleware.
///
/// # Type mutation
/// `TowerHttpCompat` would mutate response body type from `B` to `CompatBody<ResB>`. Service enclosed
/// by it must be able to handle it's mutation or utilize [TypeEraser] to erase the mutation.
///
/// [TypeEraser]: crate::middleware::eraser::TypeEraser
pub struct TowerHttpCompat<L, C, ReqB, ResB, Err> {
    layer: L,
    _phantom: PhantomData<fn(C, ReqB, ResB, Err)>,
}

impl<L, C, ReqB, ResB, Err> Clone for TowerHttpCompat<L, C, ReqB, ResB, Err>
where
    L: Clone,
{
    fn clone(&self) -> Self {
        Self {
            layer: self.layer.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<L, C, ReqB, ResB, Err> TowerHttpCompat<L, C, ReqB, ResB, Err> {
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
    pub fn new(layer: L) -> Self {
        Self {
            layer,
            _phantom: PhantomData,
        }
    }
}

impl<L, S, E, C, ReqB, ResB, Err> Service<Result<S, E>> for TowerHttpCompat<L, C, ReqB, ResB, Err>
where
    L: Layer<CompatLayer<S, C, ResB, Err>>,
    S: for<'r> Service<WebContext<'r, C, ReqB>, Response = WebResponse<ResB>, Error = Err>,
    ReqB: 'static,
{
    type Response = TowerCompatService<L::Service>;
    type Error = E;

    async fn call(&self, res: Result<S, E>) -> Result<Self::Response, Self::Error> {
        res.map(|service| {
            let service = self.layer.layer(CompatLayer {
                service: Rc::new(service),
                _phantom: PhantomData,
            });
            TowerCompatService::new(service)
        })
    }
}

pub struct CompatLayer<S, C, ResB, Err> {
    service: Rc<S>,
    _phantom: PhantomData<fn(C, ResB, Err)>,
}

impl<S, C, ReqB, ResB, Err> tower_service::Service<Request<CompatReqBody<RequestExt<ReqB>>>>
    for CompatLayer<S, C, ResB, Err>
where
    S: for<'r> Service<WebContext<'r, C, ReqB>, Response = WebResponse<ResB>, Error = Err> + 'static,
    C: Clone + 'static,
    ReqB: 'static,
{
    type Response = Response<CompatResBody<ResB>>;
    type Error = Err;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    #[inline]
    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<CompatReqBody<RequestExt<ReqB>>>) -> Self::Future {
        let service = self.service.clone();
        Box::pin(async move {
            let (mut parts, body) = req.into_parts();

            let ctx = parts
                .extensions
                .remove::<FakeClone<FakeSync<FakeSend<C>>>>()
                .unwrap()
                .into_inner()
                .into_inner()
                .into_inner();

            let (ext, body) = body.into_inner().replace_body(());

            let mut req = Request::from_parts(parts, ext);
            let mut body = RefCell::new(body);
            let req = WebContext::new(&mut req, &mut body, &ctx);

            service.call(req).await.map(|res| res.map(CompatResBody::new))
        })
    }
}

#[cfg(test)]
mod test {
    use core::convert::Infallible;

    use tower_http::set_status::SetStatusLayer;
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        body::ResponseBody, http::StatusCode, http::WebRequest, middleware::eraser::TypeEraser, service::fn_service,
        App,
    };

    use super::*;

    async fn handler(ctx: WebContext<'_, &'static str>) -> Result<WebResponse, Infallible> {
        assert_eq!(*ctx.state(), "996");
        Ok(ctx.into_response(ResponseBody::empty()))
    }

    #[test]
    fn tower_set_status() {
        let res = App::with_state("996")
            .at("/", fn_service(handler))
            .enclosed(TowerHttpCompat::new(SetStatusLayer::new(StatusCode::NOT_FOUND)))
            .enclosed(TypeEraser::response_body())
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
