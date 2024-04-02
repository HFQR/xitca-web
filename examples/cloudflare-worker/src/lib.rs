//! a cloudflare worker example that do both rest api and static file serving in one place.
//!
//! *. static files are packed into compiled binary and served directly from worker memory.

#![feature(impl_trait_in_assoc_type)]

mod file;
mod utils;

use std::{cell::OnceCell, rc::Rc};

use http_file::{ServeDir, ServeError};
use worker::{Context, Env};
use xitca_web::{
    body::{BoxBody, RequestBody, ResponseBody},
    bytes::Bytes,
    error::{BodyError, Error},
    handler::redirect::Redirect,
    http::{self, RequestExt, StatusCode, WebRequest, WebResponse},
    route::Route,
    service::{fn_service, object, Service, ServiceExt},
    App, WebContext,
};

// type alias to reduce type complexity.
type Request = http::Request<worker::Body>;
type Response = http::Response<worker::Body>;
type AppService = Rc<dyn object::ServiceObject<Request, Response = Response, Error = worker::Error>>;

// thread local for storing router service.
thread_local! {
    static R: OnceCell<AppService> = OnceCell::new();
}

// construct router and store it in thread local.
#[worker::event(start)]
pub fn start() {
    utils::set_panic_hook();

    let app = App::new()
        .at("/", Redirect::see_other("/index.html"))
        .at(
            "/*path",
            Route::new([http::Method::GET, http::Method::HEAD]).route(fn_service(serve)),
        )
        .finish()
        .enclosed_fn(map_type);

    use xitca_unsafe_collection::futures::NowOrPanic;

    let service = app.call(()).now_or_panic().unwrap();

    let _ = R.with(|r| r.set(Rc::new(service)));
}

#[worker::event(fetch)]
pub async fn main(mut req: Request, env: Env, _: Context) -> Result<Response, worker::Error> {
    // insert env to request extension where it can extract later in version function.
    req.extensions_mut().insert(env);

    // clone router service to async context.
    let router = R.with(|r| r.get().cloned().unwrap());

    // call router service
    router.call(req).await
}

// static file handler
async fn serve(ctx: WebContext<'_>) -> Result<WebResponse, Error> {
    // get file and handle error.
    match ServeDir::with_fs("", file::Files).serve(ctx.req()).await {
        // box file body stream
        Ok(res) => Ok(res.map(ResponseBody::box_stream)),
        Err(e) => match e {
            ServeError::NotFound => Err(StatusCode::NOT_FOUND.into()),
            ServeError::MethodNotAllowed => unreachable!("method is taken cared by Route"),
            // note: worker-rs does not know how to handle 304 status code.
            ServeError::NotModified => Ok(WebResponse::builder()
                .status(StatusCode::NOT_MODIFIED)
                .body(ResponseBody::none())
                .unwrap()),
            ServeError::PreconditionFailed => Err(StatusCode::PRECONDITION_FAILED.into()),
            ServeError::InvalidPath => Err(StatusCode::BAD_REQUEST.into()),
            _ => Err(StatusCode::INTERNAL_SERVER_ERROR.into()),
        },
    }
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
