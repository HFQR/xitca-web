use std::future::{ready, Future, Ready};

use xitca_http::{
    http::{Response, StatusCode},
    ResponseBody,
};
use xitca_service::{Service, ServiceFactory};

use crate::response::WebResponse;

pub struct NotFoundService;

impl<Req> ServiceFactory<Req> for NotFoundService {
    type Response = WebResponse;
    type Error = ();
    type Config = ();
    type Service = NotFoundService;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        async { Ok(Self) }
    }
}

impl<Req> Service<Req> for NotFoundService {
    type Response = WebResponse;
    type Error = ();
    type Ready<'f> = Ready<Result<(), Self::Error>>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn ready(&self) -> Self::Ready<'_> {
        ready(Ok(()))
    }

    fn call(&self, _: Req) -> Self::Future<'_> {
        async {
            let res = Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(ResponseBody::None)
                .unwrap();

            Ok(res)
        }
    }
}
