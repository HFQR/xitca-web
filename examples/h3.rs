//! A Http/3 Server runs on top of [h3](https://github.com/hyperium/h3) and [quinn](https://github.com/quinn-rs/quinn)

#![allow(incomplete_features)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]

use std::io;

use actix_http_alt::util::ErrorLoggerFactory;
use actix_http_alt::{
    h3::{H3ServiceBuilder, RequestBody},
    http::{Request, Response},
    ResponseBody,
};
use actix_service_alt::fn_service;
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use h3_quinn::quinn::generic::ServerConfig;
use h3_quinn::quinn::{crypto::rustls::TlsSession, CertificateChain, PrivateKey, ServerConfigBuilder};
use http::Version;
use log::info;

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    std::env::set_var("RUST_LOG", "actix=trace, info");
    env_logger::init();

    // construct server config
    let config = h3_config()?;

    actix_server_alt::Builder::new()
        .bind_h3("test_h3", "127.0.0.1:8080", config, move || {
            ErrorLoggerFactory::new(H3ServiceBuilder::new(fn_service(handler)))
        })?
        .build()
        .await
}

async fn handler(req: Request<RequestBody>) -> Result<Response<ResponseBody>, Box<dyn std::error::Error>> {
    // split request into head and body
    let (parts, mut body) = req.into_parts();

    info!("Request head: {:?}", parts);

    // collect body and log as String.
    let mut buf = BytesMut::new();
    while let Some(chunk) = body.next().await {
        let chunk = chunk?;
        buf.extend_from_slice(&chunk);
    }

    info!("Request body as String: {:?}", String::from_utf8_lossy(&buf));

    let res = Response::builder()
        .status(200)
        .version(Version::HTTP_3)
        .header("Content-Type", "text/plain")
        .body(Bytes::from_static(b"Hello World!").into())?;

    Ok(res)
}

fn h3_config() -> io::Result<ServerConfig<TlsSession>> {
    let mut config = ServerConfigBuilder::default();
    config.protocols(&[b"h3-29", b"h3-28", b"h3-27"]);

    let key = std::fs::read("./cert/key.pem")?;
    let key = PrivateKey::from_pem(&key).unwrap();

    let cert = std::fs::read("./cert/cert.pem")?;
    let cert = CertificateChain::from_pem(&cert).unwrap();

    config.certificate(cert, key).unwrap();

    Ok(config.build())
}
