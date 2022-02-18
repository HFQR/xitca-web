#![feature(generic_associated_types, type_alias_impl_trait)]

use xitca_service::{ready::ReadyService, Service, ServiceFactory, ServiceFactoryExt};

struct Test;

#[derive(Clone)]
struct TestFactory;

#[xitca_http_codegen::service_impl]
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

#[xitca_http_codegen::middleware_impl]
impl<S> TestMiddlewareService<S>
where
    S: ReadyService<String, Error = Box<dyn std::error::Error>, Response = usize>,
{
    async fn new_service(_m: &TestMiddleware, service: S) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(TestMiddlewareService(service))
    }

    async fn ready(&self) -> Result<S::Ready, Box<dyn std::error::Error>> {
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

#[xitca_http_codegen::service_impl]
impl TestWithoutReady {
    async fn new_service(_: &TestWithoutReadyFactory, mut cfg123: String) -> Result<Self, Box<dyn std::error::Error>> {
        cfg123.push_str("+da_gong_ren");
        assert_eq!(cfg123.as_str(), "996+da_gong_ren");
        Ok(TestWithoutReady)
    }

    async fn call(&self, req: String) -> Result<usize, Box<dyn std::error::Error>> {
        assert_eq!(req.as_str(), "007");

        Ok(007)
    }
}

#[tokio::test]
async fn http_codegen() {
    let factory = TestFactory;
    let cfg = String::from("996");
    let service = ServiceFactory::new_service(&factory, cfg.clone()).await.unwrap();

    ReadyService::ready(&service).await.err().unwrap();
    let res = Service::call(&service, String::from("007")).await.unwrap();
    assert_eq!(res, 233);

    let transform = factory.transform(TestMiddleware);
    let middlware = ServiceFactory::new_service(&transform, cfg.clone()).await.unwrap();

    let res = Service::call(&middlware, String::from("007")).await.unwrap();
    assert_eq!(res, 233);

    let factory = TestWithoutReadyFactory;
    let service = ServiceFactory::new_service(&factory, cfg).await.unwrap();
    let res = Service::call(&service, String::from("007")).await.unwrap();
    assert_eq!(res, 007);
}
