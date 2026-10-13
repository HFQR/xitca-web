use std::convert::Infallible;

use xitca_service::{ServiceExt, fn_service};
use xitca_unsafe_collection::futures::NowOrPanic;

use crate::{
    body::{RequestBody, ResponseBody},
    http::{Request, Response},
};

use super::*;

async fn index(_: Request<RequestBody>) -> Result<Response<ResponseBody>, Infallible> {
    Ok(Response::new(ResponseBody::empty()))
}

#[test]
fn route_enclosed_fn2() {
    async fn enclosed<S, Req>(service: &S, req: Req) -> Result<S::Response, S::Error>
    where
        S: Service<Req>,
    {
        service.call(req).await
    }

    let route = get(fn_service(index))
        .post(fn_service(index))
        .trace(fn_service(index))
        .enclosed_fn(enclosed);

    let service = route.call(()).now_or_panic().ok().unwrap();
    let req = Request::new(RequestBody::None);
    let res = service.call(req).now_or_panic().ok().unwrap();
    assert_eq!(res.status().as_u16(), 200);

    let mut req = Request::new(RequestBody::None);
    *req.method_mut() = Method::POST;
    let res = service.call(req).now_or_panic().ok().unwrap();
    assert_eq!(res.status().as_u16(), 200);

    let mut req = Request::new(RequestBody::None);
    *req.method_mut() = Method::PUT;
    let err = service.call(req).now_or_panic().err().unwrap();
    assert!(matches!(err, RouterError::NotAllowed(_)));
}

#[test]
fn route_mixed() {
    let route = get(fn_service(index)).next(Route::new([Method::POST, Method::PUT]).route(fn_service(index)));

    let service = route.call(()).now_or_panic().ok().unwrap();
    let req = Request::new(RequestBody::None);
    let res = service.call(req).now_or_panic().ok().unwrap();
    assert_eq!(res.status().as_u16(), 200);

    let mut req = Request::new(RequestBody::None);
    *req.method_mut() = Method::POST;
    let res = service.call(req).now_or_panic().ok().unwrap();
    assert_eq!(res.status().as_u16(), 200);

    let mut req = Request::new(RequestBody::None);
    *req.method_mut() = Method::DELETE;
    let err = service.call(req).now_or_panic().err().unwrap();
    assert!(matches!(err, RouterError::NotAllowed(_)));

    let mut req = Request::new(RequestBody::None);
    *req.method_mut() = Method::PUT;
    let res = service.call(req).now_or_panic().ok().unwrap();
    assert_eq!(res.status().as_u16(), 200);
}

#[test]
#[should_panic]
fn nest_route_panic() {
    let _ = Route::new([Method::POST, Method::GET])
        .route(fn_service(index))
        .next(get(fn_service(index)));
}

#[test]
#[should_panic]
fn empty_route_panic() {
    let _ = Route::new([]).route(fn_service(index));
}

#[test]
fn route_struct() {
    let route = Route::new([Method::POST, Method::PUT])
        .route(fn_service(index))
        .next(Route::new([Method::GET]).route(fn_service(index)))
        .next(Route::new([Method::OPTIONS]).route(fn_service(index)))
        .next(trace(fn_service(index)));

    let service = route.call(()).now_or_panic().ok().unwrap();
    let req = Request::new(RequestBody::None);
    let res = service.call(req).now_or_panic().ok().unwrap();
    assert_eq!(res.status().as_u16(), 200);

    let mut req = Request::new(RequestBody::None);
    *req.method_mut() = Method::POST;
    let res = service.call(req).now_or_panic().ok().unwrap();
    assert_eq!(res.status().as_u16(), 200);

    let mut req = Request::new(RequestBody::None);
    *req.method_mut() = Method::DELETE;

    let RouterError::NotAllowed(e) = service.call(req).now_or_panic().err().unwrap() else {
        panic!("route does not return error on unallowed method request");
    };

    let allowed = e.allowed_methods();

    assert_eq!(allowed.len(), 5);
    // strict allowed method order does not matter.
    // as long as the test can produce deterministic prediction it's fine.
    assert_eq!(allowed[0], Method::GET);
    assert_eq!(allowed[1], Method::OPTIONS);
    assert_eq!(allowed[2], Method::TRACE);
    assert_eq!(allowed[3], Method::POST);
    assert_eq!(allowed[4], Method::PUT);

    let mut req = Request::new(RequestBody::None);
    *req.method_mut() = Method::PUT;
    let res = service.call(req).now_or_panic().ok().unwrap();
    assert_eq!(res.status().as_u16(), 200);

    let mut req = Request::new(RequestBody::None);
    *req.method_mut() = Method::TRACE;
    let res = service.call(req).now_or_panic().ok().unwrap();
    assert_eq!(res.status().as_u16(), 200);
}

#[test]
fn route_accept_crate_request() {
    get(fn_service(|_: Request<()>| async {
        Ok::<_, Infallible>(Response::new(()))
    }))
    .call(())
    .now_or_panic()
    .unwrap()
    .call(Request::default())
    .now_or_panic()
    .unwrap();
}
