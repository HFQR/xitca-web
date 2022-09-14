#![feature(type_alias_impl_trait)]

use xitca_service::{ready::ReadyService, BuildService, BuildServiceExt, Service};

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

#[derive(Clone)]
struct TestMiddleware;

struct TestMiddlewareService<S>(S);

#[xitca_codegen::middleware_impl]
impl<S> TestMiddlewareService<S>
where
    S: ReadyService + Service<String, Error = Box<dyn std::error::Error>, Response = usize>,
{
    async fn new_service(_m: &TestMiddleware, service: S) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(TestMiddlewareService(service))
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
    let service = BuildService::build(&factory, cfg.clone()).await.unwrap();

    ReadyService::ready(&service).await.err().unwrap();
    let res = Service::call(&service, String::from("007")).await.unwrap();
    assert_eq!(res, 233);

    let transform = factory.enclosed(TestMiddleware);
    let middlware = BuildService::build(&transform, cfg.clone()).await.unwrap();

    let res = Service::call(&middlware, String::from("007")).await.unwrap();
    assert_eq!(res, 233);

    let factory = TestWithoutReadyFactory;
    let service = BuildService::build(&factory, cfg).await.unwrap();
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
    use core::borrow::Borrow;

    let state = MyState {
        field1: String::from("996"),
        field2: 251,
    };

    let string: &String = state.borrow();
    let num: &u32 = state.borrow();

    assert_eq!(string.as_str(), "996");
    assert_eq!(num, &251);
}
