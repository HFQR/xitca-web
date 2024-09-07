//! a cloudflare worker example that do both rest api and static file serving in one place.
//!
//! *. static files are packed into compiled binary and served directly from worker memory.

#![feature(impl_trait_in_assoc_type)]

mod file;

use std::{cell::OnceCell, rc::Rc};

use worker::{Context, Env};
use xitca_unsafe_collection::futures::NowOrPanic;
use xitca_web::{
    body::{BoxBody, RequestBody},
    bytes::Bytes,
    error::BodyError,
    handler::redirect::Redirect,
    http::{self, RequestExt, WebRequest, WebResponse},
    service::file::ServeDir,
    service::{object, Service, ServiceExt},
    App,
};

// type alias to reduce type complexity.
type Request = http::Request<worker::Body>;
type Response = http::Response<worker::Body>;
type AppService = Rc<dyn object::ServiceObject<Request, Response = Response, Error = worker::Error>>;

// thread local for storing router service.
thread_local! {
    static R: OnceCell<AppService> = OnceCell::new();
}

// store router in thread local
fn get_router() -> AppService {
    let router_builder = || {
        let service = App::new()
            .at("/", Redirect::see_other("/index.html"))
            .at("/", ServeDir::with_fs("", file::Files))
            .finish()
            .enclosed_fn(map_type)
            .call(())
            .now_or_panic()
            .unwrap();
        Rc::new(service) as _
    };

    R.with(|r| r.get_or_init(router_builder).clone())
}

#[worker::event(fetch)]
pub async fn fetch(req: Request, _: Env, _: Context) -> Result<Response, worker::Error> {
    console_error_panic_hook::set_once();
    // get router from thread local and call router service
    get_router().call(req).await
}

// middleware for map types and bridge xitca-web and worker-rs
async fn map_type<S, B>(service: &S, req: Request) -> Result<Response, worker::Error>
where
    S: Service<WebRequest, Response = WebResponse<B>, Error = std::convert::Infallible>,
    B: futures_core::Stream<Item = Result<Bytes, BodyError>> + 'static,
{
    // convert worker request type to xitca-web types.
    let (head, body) = req.into_parts();
    // extended request type carries extra typed info of a request.
    let body = RequestExt::<()>::default().map_body(|_| RequestBody::Unknown(BoxBody::new(body)));
    let req = WebRequest::from_parts(head, body);

    // execute application service
    let res = service.call(req).await.unwrap();

    // convert xitca-web response types to worker types.
    let (head, body) = res.into_parts();
    let body = worker::Body::from_stream(body)?;
    Ok(Response::from_parts(head, body))
}
