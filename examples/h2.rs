//! A Http/2 server returns Hello World String as Response.

#![allow(incomplete_features)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]

use std::fs::File;
use std::future::Future;
use std::io;
use std::io::BufReader;
use std::sync::Arc;
use std::task::{Context, Poll};

use actix_http_alt::{
    h2::RequestBody,
    http::{Request, Response},
    util::ErrorLoggerFactory,
    HttpServiceBuilder, ResponseBody,
};
use actix_web_alt::{
    dev::{Service, ServiceFactory},
    HttpServer,
};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use log::info;
use rustls::{
    internal::pemfile::{certs, pkcs8_private_keys},
    NoClientAuth, ServerConfig,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    std::env::set_var("RUST_LOG", "actix=trace, info");
    env_logger::init();

    // configure rustls.
    let acceptor = rustls_acceptor()?;

    // construct http server
    HttpServer::new(move || {
        let builder = HttpServiceBuilder::h2(H2Factory).rustls(acceptor.clone());
        ErrorLoggerFactory::new(builder)
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}

// a http/2 factory that can produce stateful service.
struct H2Factory;

impl ServiceFactory<Request<RequestBody>> for H2Factory {
    type Response = Response<ResponseBody>;
    type Error = Box<dyn std::error::Error>;
    type Config = ();
    type Service = H2Service;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        async {
            Ok(H2Service {
                state: String::from("H2ServiceState"),
            })
        }
    }
}

// a http/2 service that can lend it's state to service call future.
struct H2Service {
    state: String,
}

impl Service<Request<RequestBody>> for H2Service {
    type Response = Response<ResponseBody>;
    type Error = Box<dyn std::error::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call<'c>(&'c self, mut req: Request<RequestBody>) -> Self::Future<'c>
    where
        Request<RequestBody>: 'c,
    {
        async move {
            // borrow service state
            info!("Service state: {:?}", self.state);

            info!("Request header: {:?}", req.headers());

            // collect body and print as string.
            let mut collect = BytesMut::new();

            while let Some(chunk) = req.body_mut().next().await {
                let chunk = chunk?;
                collect.extend_from_slice(&chunk);
            }

            info!("Request body as String: {:?}", String::from_utf8_lossy(&collect));

            let res = Response::builder()
                .status(200)
                .header("Content-Type", "text/plain")
                .body(Bytes::from_static(b"Hello World!").into())?;

            Ok(res)
        }
    }
}

fn rustls_acceptor() -> io::Result<Arc<ServerConfig>> {
    let mut acceptor = ServerConfig::new(NoClientAuth::new());
    let cert_file = &mut BufReader::new(File::open("./cert/cert.pem")?);
    let key_file = &mut BufReader::new(File::open("./cert/key.pem")?);
    let cert_chain = certs(cert_file).unwrap();
    let mut keys = pkcs8_private_keys(key_file).unwrap();

    acceptor.set_single_cert(cert_chain, keys.remove(0)).unwrap();
    let protos = vec!["h2".to_string().into(), "http/1.1".to_string().into()];
    acceptor.set_protocols(&protos);

    Ok(Arc::new(acceptor))
}
