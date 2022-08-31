use std::{
    cell::RefCell,
    convert::Infallible,
    future::Future,
    marker::PhantomData,
    rc::Rc,
    task::{Context, Poll},
};

use tower_layer::Layer;
use xitca_http::request::{RemoteAddr, Request};
use xitca_unsafe_collection::no_send_send::NoSendSend;

use crate::{
    dev::service::{BuildService, Service},
    http,
    request::WebRequest,
    response::WebResponse,
    service::tower_http_compat::{CompatBody, TowerCompatService},
};

/// A middleware type that bridge `xitca-service` and `tower-service`.
/// Any `tower-http` type that impl [Layer] trait can be passed to it and used as xitca-web's middleware.
pub struct TowerHttpCompat<L, C, ReqB, ResB, Err> {
    layer: L,
    _phantom: PhantomData<(C, ReqB, ResB, Err)>,
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
    /// tower::Service::poll_ready implementation is ingorend by TowerHttpCompat.
    /// if a tower Service is sensitive to poll_ready implemetnation it should not be used.
    ///
    /// # Example:
    /// ```rust
    /// # use std::convert::Infallible;
    /// # use xitca_web::{dev::service::fn_service, request::WebRequest, response::WebResponse, App};
    /// # use xitca_web::http::StatusCode;
    /// use xitca_web::middleware::tower_http_compat::TowerHttpCompat;
    /// use tower_http::set_status::SetStatusLayer;
    ///
    /// # fn doc_example() {
    /// App::new()
    ///     .at("/", fn_service(handler))
    ///     .enclosed(TowerHttpCompat::new(SetStatusLayer::new(StatusCode::NOT_FOUND)));
    /// # }
    ///
    /// # async fn handler(req: WebRequest<'_>) -> Result<WebResponse, Infallible> {
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

impl<L, S, C, ReqB, ResB, Err> BuildService<S> for TowerHttpCompat<L, C, ReqB, ResB, Err>
where
    L: Layer<CompatLayer<S, C, ReqB, ResB, Err>>,
    S: for<'r> Service<WebRequest<'r, C, ReqB>, Response = WebResponse<ResB>, Error = Err>,
{
    type Service = TowerCompatService<L::Service>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self::Service, Self::Error>>;

    fn build(&self, service: S) -> Self::Future {
        let service = self.layer.layer(CompatLayer {
            service: Rc::new(service),
            _phantom: PhantomData,
        });
        async { Ok(TowerCompatService::new(service)) }
    }
}

pub struct CompatLayer<S, C, ReqB, ResB, Err> {
    service: Rc<S>,
    _phantom: PhantomData<(C, ReqB, ResB, Err)>,
}

impl<S, C, ReqB, ResB, Err> tower_service::Service<http::Request<CompatBody<NoSendSend<ReqB>>>>
    for CompatLayer<S, C, ReqB, ResB, Err>
where
    S: for<'r> Service<WebRequest<'r, C, ReqB>, Response = WebResponse<ResB>, Error = Err>,
    C: Send + Sync + Clone + 'static,
{
    type Response = http::Response<CompatBody<ResB>>;
    type Error = Err;
    type Future = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut req: http::Request<CompatBody<NoSendSend<ReqB>>>) -> Self::Future {
        let service = self.service.clone();
        async move {
            let remote_addr = *req.extensions().get::<RemoteAddr>().unwrap();
            let ctx = req.extensions_mut().get_mut::<Option<C>>().unwrap().take().unwrap();
            let (parts, body) = req.into_parts();
            let req = http::Request::from_parts(parts, ());
            let mut req = Request::from_http(req, remote_addr);
            let mut body = RefCell::new(body.into_inner().into_inner());
            let req = WebRequest::new(&mut req, &mut body, &ctx);
            service.call(req).await.map(|res| res.map(CompatBody::new))
        }
    }
}

#[cfg(test)]
mod test {
    use tower_http::set_status::SetStatusLayer;
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        dev::{bytes::Bytes, service::fn_service},
        http::StatusCode,
        App,
    };

    use super::*;

    async fn handler(req: WebRequest<'_>) -> Result<WebResponse, Infallible> {
        Ok(req.into_response(Bytes::new()))
    }

    #[test]
    fn tower_set_status() {
        let res = App::new()
            .at("/", fn_service(handler))
            .enclosed(TowerHttpCompat::new(SetStatusLayer::new(StatusCode::NOT_FOUND)))
            .finish()
            .build(())
            .now_or_panic()
            .unwrap()
            .call(Request::default())
            .now_or_panic()
            .unwrap();

        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
