//! A Http/2 server returns Hello World String as Response and always close connection afterwards.

#![allow(incomplete_features)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]

use std::fs::File;
use std::future::Future;
use std::io;
use std::io::BufReader;
use std::task::{Context, Poll};

use actix_http_alt::util::ErrorLoggerFactory;
use actix_http_alt::{h2::RequestBody, HttpRequest, HttpResponse, HttpServiceBuilder};
use actix_service_alt::{Service, ServiceFactory};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use rustls::{
    internal::pemfile::{certs, pkcs8_private_keys},
    NoClientAuth, ServerConfig,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    std::env::set_var("RUST_LOG", "actix=trace, info");
    env_logger::init();

    let addr = "127.0.0.1:8080";

    let mut acceptor = ServerConfig::new(NoClientAuth::new());
    let cert_file = &mut BufReader::new(File::open("./examples/cert/cert.pem")?);
    let key_file = &mut BufReader::new(File::open("./examples/cert/key.pem")?);
    let cert_chain = certs(cert_file).unwrap();
    let mut keys = pkcs8_private_keys(key_file).unwrap();
    acceptor
        .set_single_cert(cert_chain, keys.remove(0))
        .unwrap();

    let protos = vec!["h2".to_string().into(), "http/1.1".to_string().into()];
    acceptor.set_protocols(&protos);

    let acceptor = std::sync::Arc::new(acceptor);

    actix_server_alt::Builder::new()
        .bind("hell_world", addr, move || {
            let http_builder = HttpServiceBuilder::h2(H2Factory).rustls(acceptor.clone());
            ErrorLoggerFactory::new(http_builder)
        })?
        .build()
        .await
}

struct H2Factory;

impl ServiceFactory<HttpRequest<RequestBody>> for H2Factory {
    type Response = HttpResponse;
    type Error = Box<dyn std::error::Error>;
    type Config = ();
    type Service = H2Service;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        async {
            Ok(H2Service {
                name: String::from("MyService"),
                child: ChildService,
            })
        }
    }
}

// a parent service that hold string state and a child service.
struct H2Service {
    name: String,
    child: ChildService,
}

impl Service for H2Service {
    type Request<'r> = HttpRequest<RequestBody>;
    type Response = HttpResponse;
    type Error = Box<dyn std::error::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.child.poll_ready(cx)
    }

    fn call<'s>(&'s self, req: Self::Request<'s>) -> Self::Future<'s> {
        async move {
            // pass self's name as borrowed state to child service.
            let state = BorrowState(self.name.as_str());

            self.child.call((state, req)).await
        }
    }
}

struct ChildService;

impl Service for ChildService {
    type Request<'r> = (BorrowState<'r>, HttpRequest<RequestBody>);
    type Response = HttpResponse;
    type Error = Box<dyn std::error::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call<'s>(&'s self, (state, req): Self::Request<'s>) -> Self::Future<'s> {
        async move {
            // borrowed state from parent service.
            log::info!("Parent service name: {}", state.0);

            // split request into head and body
            let (parts, mut body) = req.into_parts();

            log::info!("Request head: {:?}", parts);

            // collect body and print as string.
            let mut collect = BytesMut::new();

            while let Some(chunk) = body.next().await {
                let chunk = chunk?;
                collect.extend_from_slice(&chunk);
            }

            log::info!(
                "Request body as String: {:?}",
                String::from_utf8_lossy(&collect)
            );

            let res = HttpResponse::builder()
                .status(200)
                .header("Content-Type", "text/plain")
                .body(Bytes::from_static(b"Hello World!").into())?;

            Ok(res)
        }
    }
}

struct BorrowState<'a>(&'a str);
