use xitca_service::ServiceExt;
use xitca_unsafe_collection::futures::NowOrPanic;

use crate::{
    http::{Response, StatusCode},
    unspecified_socket_addr,
};

use super::*;

async fn handler(
    method: &Method,
    addr: &SocketAddr,
    uri: &Uri,
    headers: &HeaderMap,
    (_, ext): (&Request<RequestExt<()>>, &Extensions),
) -> StatusCode {
    assert_eq!(method, Method::GET);
    assert_eq!(*addr, unspecified_socket_addr());
    assert_eq!(uri.path(), "/");
    assert!(headers.is_empty());
    assert!(ext.is_empty());

    StatusCode::MULTI_STATUS
}

impl Responder<Request<RequestExt<()>>> for StatusCode {
    type Response = Response<()>;
    type Error = Infallible;

    async fn respond(self, _: Request<RequestExt<()>>) -> Result<Self::Response, Self::Error> {
        let mut res = Response::new(());
        *res.status_mut() = self;
        Ok(res)
    }
}

#[test]
fn concurrent_extract_with_enclosed_fn() {
    async fn enclosed<S, Req>(service: &S, req: Req) -> Result<S::Response, S::Error>
    where
        S: Service<Req>,
    {
        service.call(req).await
    }

    let res = handler_service(handler)
        .enclosed_fn(enclosed)
        .call(())
        .now_or_panic()
        .unwrap()
        .call(Request::default())
        .now_or_panic()
        .unwrap();

    assert_eq!(res.status(), StatusCode::MULTI_STATUS);
}

#[cfg(feature = "router")]
#[test]
fn handler_in_router() {
    use crate::util::service::{Router, route::get};

    let res = Router::new()
        .insert("/", get(handler_service(handler)))
        .call(())
        .now_or_panic()
        .unwrap()
        .call(Request::default())
        .now_or_panic()
        .unwrap();

    assert_eq!(res.status(), StatusCode::MULTI_STATUS);
}
