#![feature(generic_associated_types, type_alias_impl_trait)]

use xitca_http::{
    body::{RequestBody, ResponseBody},
    http::{Request, Response},
};

struct Test;

#[derive(Clone)]
struct TestFactory;

#[xitca_http_codegen::service_impl]
impl Test {
    async fn new_service(factory: &TestFactory, _: ()) -> Result<Self, ()> {
        Ok(Test)
    }

    async fn ready(&self) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    async fn call(&self, req: Request<RequestBody>) -> Result<Response<ResponseBody>, Box<dyn std::error::Error>> {
        todo!()
    }
}
