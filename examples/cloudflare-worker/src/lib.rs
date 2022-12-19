//! a cloudflare worker example that do both rest api and static file serving in one place.
//!
//! *. static files are packed into compiled binary and served directly from worker memory.

#![feature(type_alias_impl_trait)]

mod file;
mod utils;

use std::{cell::RefCell, rc::Rc};

use futures_util::{pin_mut, StreamExt};
use http_file::{ServeDir, ServeError};
use serde_json::json;
use worker::*;
use xitca_http::{
    http,
    util::service::{
        route::{get, post, Route, RouteError},
        router::{Router, RouterError},
    },
};
use xitca_service::{fn_service, object, Service, ServiceExt};
use xitca_unsafe_collection::{
    fake_send_sync::{FakeSend, FakeSync},
    futures::NowOrPanic,
};

// type alias for http request type
type HttpRequest = http::Request<http::RequestExt<()>>;

// type alias to reduce type complexity.
type RouterService = Rc<dyn object::ServiceObject<HttpRequest, Response = Response, Error = Error>>;

// thread local for storing router service.
thread_local! {
    static R: RefCell<Option<RouterService>> = RefCell::new(None);
}

#[event(start)]
pub fn start() {
    utils::set_panic_hook();

    // initialize router service.
    let router = Router::new()
        .insert("/form/:field", post(fn_service(form)))
        .insert("/worker-version", get(fn_service(version)))
        // catch all other path and try to serve static file.
        .insert(
            "/*path",
            Route::new([http::Method::GET, http::Method::HEAD]).route(fn_service(serve)),
        )
        // middleware functions
        .enclosed_fn(error_handler)
        .enclosed_fn(path_handler);

    let service = router.call(()).now_or_panic().unwrap();

    R.with(|r| *r.borrow_mut() = Some(Rc::new(service)));
}

#[event(fetch)]
pub async fn main(req: Request, env: Env, _: Context) -> Result<Response> {
    log_request(&req);

    // clone router service to async context.
    let router = R.with(|r| r.borrow().as_ref().cloned().unwrap());

    // convert worker request to http request.
    let http_req = worker_to_http(req, env);

    // call router service
    router.call(http_req).await
}

fn worker_to_http(req: Request, env: Env) -> HttpRequest {
    let mut http_req = http::Request::default();

    // naive url to uri conversion. only request path is covered.
    *http_req.uri_mut() = req
        .url()
        .ok()
        .and_then(|url| std::str::FromStr::from_str(url.path()).ok())
        .unwrap_or_else(|| http::Uri::from_static("/not_found"));

    *http_req.method_mut() = match req.method() {
        Method::Get => http::Method::GET,
        Method::Post => http::Method::POST,
        Method::Head => http::Method::HEAD,
        _ => http::Method::DELETE, // not interested methods.
    };

    // potential body conversion if include middleware wants body type.

    // store Env and Request in type map to use later.
    http_req.extensions_mut().insert(FakeSync::new(FakeSend::new(env)));
    http_req.extensions_mut().insert(FakeSync::new(FakeSend::new(req)));

    http_req
}

// form data handler.
async fn form(mut http: HttpRequest) -> Result<Response> {
    // extract worker::Request from http::Request.
    let mut req = http
        .extensions_mut()
        .remove::<FakeSync<FakeSend<Request>>>()
        .unwrap()
        .into_inner()
        .into_inner();

    let Some(name) = http.body().params().get("field") else { return bad_req() };
    let Some(entry) = req.form_data().await.ok().and_then(|form| form.get(name)) else { return bad_req() };
    match entry {
        FormEntry::Field(value) => Response::from_json(&json!({ name: value })),
        FormEntry::File(_) => Response::error("`field` param in form shouldn't be a File", 422),
    }
}

// env version handler.
async fn version(mut req: HttpRequest) -> Result<Response> {
    // extract worker::Env from http::Request.
    let version = req
        .extensions_mut()
        .remove::<FakeSync<FakeSend<Env>>>()
        .unwrap()
        .into_inner()
        .into_inner()
        .var("WORKERS_RS_VERSION")?
        .to_string();

    Response::ok(version)
}

// static file handler
async fn serve(mut req: HttpRequest) -> Result<Response> {
    // extract worker::Request from http::Request.
    let mut worker_req = req
        .extensions_mut()
        .remove::<FakeSync<FakeSend<Request>>>()
        .unwrap()
        .into_inner()
        .into_inner();

    // convert headers as file serving have interest in them.
    if let Ok(headers) = worker_req.headers_mut() {
        for (k, v) in headers.into_iter() {
            if let (Ok(key), Ok(val)) = (
                http::header::HeaderName::from_bytes(k.as_bytes()),
                http::header::HeaderValue::from_bytes(v.as_bytes()),
            ) {
                req.headers_mut().append(key, val);
            }
        }
    }

    // get file and handle error.
    let dir = ServeDir::with_fs("", file::Files);
    let res = match dir.serve(&req).await {
        Ok(file) => file,
        Err(ServeError::NotFound) => return not_found(),
        Err(ServeError::MethodNotAllowed) => unreachable!("method is taken cared by Route"),
        Err(ServeError::NotModified) => return Response::error("NotModified", 304),
        Err(ServeError::PreconditionFailed) => return Response::error("NotModified", 412),
        Err(ServeError::InvalidPath) => return bad_req(),
        _ => return internal(),
    };

    // destruct http response and construct worker response
    let (parts, body) = res.into_parts();

    // collect static file streaming body to Vec<u8>.
    // this is a workaround because for some reason streaming directly does not work.

    pin_mut!(body);

    let mut v = Vec::new();

    while let Some(chunk) = body.next().await {
        let Ok(chunk) = chunk else { return internal() };
        v.extend_from_slice(chunk.as_ref());
    }

    let mut res = Response::from_bytes(v)?;

    // remove default content-type header because it's wrong.
    let _ = res.headers_mut().delete("content-type");

    // copy headers from http response to worker response.
    for (k, v) in parts.headers.iter() {
        if let Ok(v) = v.to_str() {
            let _ = res.headers_mut().append(k.as_str(), v);
        }
    }

    Ok(res)
}

// error handler middleware
async fn error_handler<S>(service: &S, req: HttpRequest) -> Result<Response>
where
    S: Service<HttpRequest, Response = Response, Error = RouterError<RouteError<Error>>>,
{
    match service.call(req).await {
        Ok(res) => Ok(res),
        Err(RouterError::First(_)) => not_found(),
        Err(RouterError::Second(RouteError::First(_))) => Response::error("MethodNotAllowed", 405),
        Err(RouterError::Second(RouteError::Second(e))) => {
            console_log!("unhandled error: {e}");
            internal()
        }
    }
}

// path handler middleware
async fn path_handler<S>(service: &S, mut req: HttpRequest) -> std::result::Result<S::Response, S::Error>
where
    S: Service<HttpRequest>,
{
    // just append index.html when request is targeting "/"
    match req.uri().path() {
        "/" | "" => *req.uri_mut() = http::Uri::from_static("/index.html"),
        _ => {}
    }
    service.call(req).await
}

fn bad_req() -> Result<Response> {
    Response::error("BadRequest", 400)
}

fn not_found() -> Result<Response> {
    Response::error("NotFound", 404)
}

fn internal() -> Result<Response> {
    Response::error("InternalServerError", 500)
}

fn log_request(req: &Request) {
    console_log!(
        "{} - [{}], located at: {:?}, within: {}",
        Date::now().to_string(),
        req.path(),
        req.cf().coordinates().unwrap_or_default(),
        req.cf().region().unwrap_or("unknown region".into())
    );
}
