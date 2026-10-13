use xitca_service::{ServiceExt, fn_service};
use xitca_unsafe_collection::futures::NowOrPanic;

use crate::http::Request;

use super::*;

#[test]
fn state_middleware() {
    let service = fn_service(|req: Request<()>| async move {
        assert_eq!("state", req.extensions().get::<String>().unwrap());
        Ok::<_, ()>("996")
    })
    .enclosed(Extension::new(String::from("state")))
    .call(())
    .now_or_panic()
    .unwrap();

    let res = service.call(Request::new(())).now_or_panic().unwrap();

    assert_eq!("996", res);
}

#[test]
fn state_middleware_http_request() {
    let service = fn_service(|req: http::Request<()>| async move {
        assert_eq!("state", req.extensions().get::<String>().unwrap());
        Ok::<_, ()>("996")
    })
    .enclosed(Extension::new(String::from("state")))
    .call(())
    .now_or_panic()
    .unwrap();

    let res = service.call(http::Request::new(())).now_or_panic().unwrap();

    assert_eq!("996", res);
}
