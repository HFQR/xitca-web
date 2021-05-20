//! A Http/3 Server runs on top of [h3](https://github.com/hyperium/h3) and [quinn](https://github.com/quinn-rs/quinn)

#![allow(incomplete_features)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]

use std::future::Future;
use std::io;
use std::task::{Context, Poll};

use actix_http_alt::util::ErrorLoggerFactory;
use actix_http_alt::{
    h3::{H3ServiceBuilder, RequestBody},
    HttpRequest, HttpResponse,
};
use actix_service_alt::{Service, ServiceFactory};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use h3_quinn::quinn::{CertificateChain, PrivateKey, ServerConfigBuilder};
use http::Version;

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    std::env::set_var("RUST_LOG", "actix=trace, info");
    env_logger::init();

    let addr = "127.0.0.1:8080";

    // construct server config
    let mut config = ServerConfigBuilder::default();
    config.protocols(&[b"h3-29"]);

    let key = std::fs::read("./examples/cert/key.pem")?;
    let key = PrivateKey::from_pem(&key).unwrap();

    let cert = std::fs::read("./examples/cert/cert.pem")?;
    let cert = CertificateChain::from_pem(&cert).unwrap();

    config.certificate(cert, key).unwrap();

    let config = config.build();

    actix_server_alt::Builder::new()
        .bind_h3("test_h3", addr, config, move || {
            let http_builder = H3ServiceBuilder::new(H3Factory);
            ErrorLoggerFactory::new(http_builder)
        })?
        .build()
        .await
}

struct H3Factory;

impl ServiceFactory<HttpRequest<RequestBody>> for H3Factory {
    type Response = HttpResponse;
    type Error = Box<dyn std::error::Error>;
    type Config = ();
    type Service = H3Service;
    type InitError = ();
    type Future = impl Future<Output = Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: Self::Config) -> Self::Future {
        async { Ok(H3Service) }
    }
}

struct H3Service;

impl Service<HttpRequest<RequestBody>> for H3Service {
    type Response = HttpResponse;
    type Error = Box<dyn std::error::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    #[inline]
    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call<'c>(&'c self, req: HttpRequest<RequestBody>) -> Self::Future<'c>
    where
        HttpRequest<RequestBody>: 'c,
    {
        async move {
            // split request into head and body
            let (parts, mut body) = req.into_parts();

            log::info!("Request head: {:?}", parts);

            // collect body and log as String.
            let mut buf = BytesMut::new();
            while let Some(chunk) = body.next().await {
                let chunk = chunk?;
                buf.extend_from_slice(&chunk);
            }

            log::info!("Request body as String: {:?}", String::from_utf8_lossy(&buf));

            let res = HttpResponse::builder()
                .status(200)
                .version(Version::HTTP_3)
                .header("Content-Type", "text/plain")
                .body(Bytes::from_static(b"Hello World!").into())?;

            Ok(res)
        }
    }
}
