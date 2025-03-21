use core::fmt;

use xitca_service::{ready::ReadyService, Service, ServiceExt};
use xitca_web::{
    http::{StatusCode, WebResponse},
    WebContext,
};

struct Test;

#[derive(Clone)]
struct TestFactory;

#[xitca_codegen::service_impl]
impl Test {
    async fn new_service(_: &TestFactory, mut cfg123: String) -> Result<Self, Box<dyn std::error::Error>> {
        cfg123.push_str("+da_gong_ren");
        assert_eq!(cfg123.as_str(), "996+da_gong_ren");
        Ok(Test)
    }

    async fn ready(&self) -> Result<(), Box<dyn std::error::Error>> {
        Err("251".into())
    }

    async fn call(&self, req: String) -> Result<usize, Box<dyn std::error::Error>> {
        assert_eq!(req.as_str(), "007");
        Ok(233)
    }
}

struct TestMiddleware;

struct TestMiddlewareService<S>(S);

#[xitca_codegen::middleware_impl]
impl<S> TestMiddlewareService<S>
where
    S: ReadyService + Service<String, Error = Box<dyn std::error::Error>, Response = usize>,
{
    async fn new_service<E>(_m: &TestMiddleware, res: Result<S, E>) -> Result<Self, E> {
        res.map(TestMiddlewareService)
    }

    async fn ready(&self) -> S::Ready {
        self.0.ready().await
    }

    async fn call(&self, req: String) -> Result<usize, Box<dyn std::error::Error>> {
        assert_eq!(req.as_str(), "007");
        self.0.call(req).await
    }
}

struct TestWithoutReady;

#[derive(Clone)]
struct TestWithoutReadyFactory;

#[xitca_codegen::service_impl]
impl TestWithoutReady {
    async fn new_service(_: &TestWithoutReadyFactory, mut cfg123: String) -> Result<Self, Box<dyn std::error::Error>> {
        cfg123.push_str("+da_gong_ren");
        assert_eq!(cfg123.as_str(), "996+da_gong_ren");
        Ok(TestWithoutReady)
    }

    async fn call(&self, req: String) -> Result<usize, Box<dyn std::error::Error>> {
        assert_eq!(req.as_str(), "007");
        Ok(7)
    }
}

#[tokio::test]
async fn http_codegen() {
    let factory = TestFactory;
    let cfg = String::from("996");
    let service = Service::call(&factory, cfg.clone()).await.unwrap();

    ReadyService::ready(&service).await.err().unwrap();
    let res = Service::call(&service, String::from("007")).await.unwrap();
    assert_eq!(res, 233);

    let transform = factory.enclosed(TestMiddleware);
    let middlware = Service::call(&transform, cfg.clone()).await.unwrap();

    let res = Service::call(&middlware, String::from("007")).await.unwrap();
    assert_eq!(res, 233);

    let factory = TestWithoutReadyFactory;
    let service = Service::call(&factory, cfg).await.unwrap();
    let res = Service::call(&service, String::from("007")).await.unwrap();
    assert_eq!(res, 7);
}

#[derive(xitca_codegen::State)]
struct MyState {
    #[borrow]
    field1: String,
    #[borrow]
    field2: u32,
}

#[test]
fn state_borrow() {
    use xitca_web::handler::state::BorrowState;

    let state = MyState {
        field1: String::from("996"),
        field2: 251,
    };

    let string: &String = state.borrow();
    let num: &u32 = state.borrow();

    assert_eq!(string.as_str(), "996");
    assert_eq!(num, &251);
}

#[derive(Debug)]
struct MyError;

impl fmt::Display for MyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("huh")
    }
}

impl std::error::Error for MyError {}

#[xitca_codegen::error_impl]
impl MyError {
    async fn call(&self, ctx: WebContext<'_, Request<'_>>) -> WebResponse {
        let status = ctx.state().request_ref::<StatusCode>().unwrap();
        WebResponse::builder().status(*status).body(Default::default()).unwrap()
    }
}

#[tokio::test]
async fn web_handler() {
    #[xitca_web::codegen::route("/", method = get)]
    async fn test(_: &WebContext<'_>) -> &'static str {
        ""
    }

    #[xitca_web::codegen::route("/2", method = get)]
    async fn test2(_: &WebContext<'_, ()>) -> &'static str {
        ""
    }

    #[xitca_web::codegen::route("/3", method = get)]
    async fn test3(_: &WebContext<'_, (), xitca_web::body::RequestBody>) -> &'static str {
        ""
    }

    let _ = xitca_web::App::new()
        .at_typed(test)
        .at_typed(test2)
        .at_typed(test3)
        .finish();
}
