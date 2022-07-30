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

use crate::{
    dev::service::{BuildService, Service},
    http,
    request::WebRequest,
    response::WebResponse,
};

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
        async {
            Ok(TowerCompatService {
                service: RefCell::new(service),
            })
        }
    }
}

pub struct TowerCompatService<TS> {
    service: RefCell<TS>,
}

impl<'r, C, ReqB, TS, ResB> Service<WebRequest<'r, C, ReqB>> for TowerCompatService<TS>
where
    TS: tower_service::Service<http::Request<ReqB>, Response = WebResponse<ResB>>,
    C: Send + Sync + Clone + 'static,
    ReqB: Default + 'r,
{
    type Response = WebResponse<ResB>;
    type Error = TS::Error;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>
    where
        Self: 'f;

    fn call(&self, mut req: WebRequest<'r, C, ReqB>) -> Self::Future<'_> {
        async move {
            let ctx = req.state().clone();
            let addr = *req.req().remote_addr();
            req.req_mut().extensions_mut().insert(Some(ctx));
            req.req_mut().extensions_mut().insert(addr);
            let (parts, body) = req.take_request().into_parts();
            let req = http::Request::from_parts(parts, body);
            let fut = tower_service::Service::call(&mut *self.service.borrow_mut(), req);
            fut.await
        }
    }
}

pub struct CompatLayer<S, C, ReqB, ResB, Err> {
    service: Rc<S>,
    _phantom: PhantomData<(C, ReqB, ResB, Err)>,
}

impl<S, C, ReqB, ResB, Err> tower_service::Service<http::Request<ReqB>> for CompatLayer<S, C, ReqB, ResB, Err>
where
    S: for<'r> Service<WebRequest<'r, C, ReqB>, Response = WebResponse<ResB>, Error = Err>,
    C: Send + Sync + Clone + 'static,
{
    type Response = WebResponse<ResB>;
    type Error = Err;
    type Future = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut req: http::Request<ReqB>) -> Self::Future {
        let service = self.service.clone();
        async move {
            let remote_addr = *req.extensions().get::<RemoteAddr>().unwrap();
            let ctx = req.extensions_mut().get_mut::<Option<C>>().unwrap().take().unwrap();
            let (parts, body) = req.into_parts();
            let req = http::Request::from_parts(parts, ());
            let mut req = Request::from_http(req, remote_addr);
            let mut body = RefCell::new(body);
            let req = WebRequest::new(&mut req, &mut body, &ctx);
            service.call(req).await
        }
    }
}

#[cfg(test)]
mod test {
    use tower_http::set_status::SetStatusLayer;
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        dev::{
            bytes::Bytes,
            service::{fn_service, middleware::UncheckedReady},
        },
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
            .enclosed(UncheckedReady)
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
