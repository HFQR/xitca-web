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
        async {
            Ok(MyService {
                name: String::from("MyService"),
                child: ChildService,
            })
        }
    }
}

struct MyService {
    name: String,
    child: ChildService,
}

impl Service for MyService {
    type Request<'r> = (Parts, RecvStream);
    type Response = Response<Bytes>;
    type Error = Box<dyn std::error::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call<'s, 'r, 'f>(&'s self, (req, body): Self::Request<'r>) -> Self::Future<'f>
    where
        's: 'f,
        'r: 'f,
    {
        async move {
            let state = BorrowState(self.name.as_str());

            self.child.call((state, req, body)).await
        }
    }
}

struct ChildService;

impl Service for ChildService {
    type Request<'r> = (BorrowState<'r>, Parts, RecvStream);
    type Response = Response<Bytes>;
    type Error = Box<dyn std::error::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call<'s, 'r, 'f>(&'s self, (state, req, _): Self::Request<'r>) -> Self::Future<'f>
    where
        's: 'f,
        'r: 'f,
    {
        async move {
            println!("{}, got req: {:?}", state.0, req);

            let res = Response::builder()
                .status(200)
                .body(Bytes::from_static(b"Hello World!"))?;

            Ok(res)
        }
    }
}

struct BorrowState<'a>(&'a str);
