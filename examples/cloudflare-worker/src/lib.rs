mod utils;

use std::{cell::RefCell, rc::Rc};

use serde_json::json;
use worker::*;
use xitca_http::{
    http,
    util::service::{
        route::{get, post, RouteError},
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
        .insert("/", get(fn_service(|_| async { Response::ok("Hello from Workers!") })))
        .insert("/form/:field", post(fn_service(form)))
        .insert("/worker-version", get(fn_service(version)))
        .enclosed_fn(error_handler);

    let service = router.call(()).now_or_panic().unwrap();

    R.with(|r| *r.borrow_mut() = Some(Rc::new(service)));
}

#[event(fetch)]
pub async fn main(req: Request, env: Env, _: Context) -> Result<Response> {
    log_request(&req);

    // clone router service to async context.
    let router = R.with(|r| r.borrow().as_ref().cloned().unwrap());

    // convert worker request to http request.
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
        _ => http::Method::DELETE, // not interested methods.
    };

    // potential body conversion if include middleware wants body type.

    // store Env and Request in type map to use later.
    http_req.extensions_mut().insert(FakeSync::new(FakeSend::new(env)));
    http_req.extensions_mut().insert(FakeSync::new(FakeSend::new(req)));

    // call router service
    router.call(http_req).await
}

async fn form(mut http: HttpRequest) -> Result<Response> {
    // extract worker::Request from http::Request.
    let mut req = http
        .extensions_mut()
        .remove::<FakeSync<FakeSend<Request>>>()
        .unwrap()
        .into_inner()
        .into_inner();

    fn bad_req() -> Result<Response> {
        Response::error("BadRequest", 400)
    }

    let Some(name) = http.body().params().get("field") else { return bad_req() };
    let Some(entry) = req.form_data().await.ok().and_then(|form| form.get(name)) else { return bad_req() };
    match entry {
        FormEntry::Field(value) => Response::from_json(&json!({ name: value })),
        FormEntry::File(_) => Response::error("`field` param in form shouldn't be a File", 422),
    }
}

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

// error handler middleware
async fn error_handler<S>(service: &S, req: HttpRequest) -> Result<Response>
where
    S: Service<HttpRequest, Response = Response, Error = RouterError<RouteError<Error>>>,
{
    match service.call(req).await {
        Ok(res) => Ok(res),
        Err(RouterError::First(_)) => Response::error("NotFound", 404),
        Err(RouterError::Second(RouteError::First(_))) => Response::error("MethodNotAllowed", 405),
        Err(RouterError::Second(RouteError::Second(e))) => {
            console_log!("unhandled error: {e}");
            Response::error("InternalServerError", 500)
        }
    }
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
