//! A Http/2 server returns Hello World String as Response and always close connection afterwards.

#![allow(incomplete_features)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]

use std::io;

use actix_server_alt::http::HttpServiceBuilder;
use actix_server_alt::{Service, ServiceFactory};
use bytes::Bytes;
use h2::RecvStream;
use http::{request::Parts, Response};
use std::future::Future;
use std::task::{Context, Poll};

#[actix_web::main]
async fn main() -> io::Result<()> {
    std::env::set_var("RUST_LOG", "actix=trace, info");
    env_logger::init();

    let addr = "127.0.0.1:8080";

    actix_server_alt::Builder::new()
        .bind("test", addr, || {
            HttpServiceBuilder::new(MyServiceFactor).finish()
        })?
        .build()
        .await
}

struct MyServiceFactor;

impl ServiceFactory<(Parts, RecvStream)> for MyServiceFactor {
    type Response = Response<Bytes>;
    type Error = Box<dyn std::error::Error>;
    type Config = ();
    type Service = MyService;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        async { Ok(MyService(String::from("MyService"))) }
    }
}

struct MyService(String);

impl Service for MyService {
    type Request<'r> = (Parts, RecvStream);
    type Response = Response<Bytes>;
    type Error = Box<dyn std::error::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&self, (req, _): Self::Request<'_>) -> Self::Future<'_> {
        async move {
            println!("{}, got req: {:?}", self.0.as_str(), req);

            let res = Response::builder()
                .status(200)
                .body(Bytes::from_static(b"Hello World!"))?;

            Ok(res)
        }
    }
}
