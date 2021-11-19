#![feature(generic_associated_types, type_alias_impl_trait)]

use std::convert::Infallible;

use xitca_service::{Service, ServiceFactory};

struct Test;

#[derive(Clone)]
struct TestFactory;

#[xitca_http_codegen::service_impl]
impl Test {
    async fn new_service(_: &TestFactory, cfg: String) -> Result<Self, Infallible> {
        assert_eq!(cfg.as_str(), "996");
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

#[tokio::test]
async fn http_codegen() {
    let factory = TestFactory;
    let cfg = String::from("996");
    let test = ServiceFactory::new_service(&factory, cfg).await.unwrap();

    Service::ready(&test).await.err().unwrap();
    let res = Service::call(&test, String::from("007")).await.unwrap();
    assert_eq!(res, 233);
}
