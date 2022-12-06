//! An incomplete example of using xitca-http with cloudflare worker.

use std::{cell::RefCell, rc::Rc};

use worker::{self, event, Env, Error, Request, Response};
use xitca_http::{http, util::service::Router};
use xitca_service::{fn_service, object, Service, ServiceExt};

type RouterService = Rc<dyn object::ServiceObject<http::Request<()>, Response = Response, Error = Error>>;

// thread local for storing router service.
thread_local! {
    static R: RefCell<Option<RouterService>> = RefCell::new(None);
}

#[event(fetch)]
pub async fn main(_: Request, _: Env, _ctx: worker::Context) -> Result<Response, Error> {
    // initialize router once.
    if R.with(|r| r.borrow().is_none()) {
        let service = Router::new()
            .insert("/", fn_service(index))
            .enclosed_fn(error_handler)
            .call(())
            .await
            .unwrap();

        R.with(|r| *r.borrow_mut() = Some(Rc::new(service)));
    }

    // clone router service to async context.
    let router = R.with(|r| r.borrow().as_ref().cloned().unwrap());

    // convert worker request to http request. (ignored for now)
    let http_req = http::Request::new(());

    // call router service
    router.call(http_req).await
}

async fn error_handler<S>(service: &S, req: http::Request<()>) -> Result<Response, Error>
where
    S: Service<http::Request<()>, Response = Response>,
    S::Error: core::fmt::Debug,
{
    let res = service.call(req).await.unwrap();
    Ok(res)
}

async fn index(_: http::Request<()>) -> Result<Response, Error> {
    Response::from_bytes(b"Hello,Worker!".to_vec())
}

// remove main function when building worker with wrangler. this is here to satisfy build with cargo.
fn main() {}
