//! A Http server returns Hello World String as Response.

#![allow(incomplete_features)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]

use actix_http_alt::{
    http::{Request, Response},
    util::ErrorLoggerFactory,
    HttpServiceBuilder, HttpServiceConfig, RequestBody, ResponseBody,
};
use actix_service_alt::fn_service;
use actix_web_alt::HttpServer;
use bytes::Bytes;
use openssl::ssl::{AlpnError, SslAcceptor, SslFiletype, SslMethod};

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix=trace, info");
    env_logger::init();

    // set up openssl and alpn protocol.
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    builder
        .set_private_key_file("./cert/key.pem", SslFiletype::PEM)
        .unwrap();
    builder.set_certificate_chain_file("./cert/cert.pem").unwrap();

    builder.set_alpn_select_callback(|_, protocols| {
        const H2: &[u8] = b"\x02h2";
        const H11: &[u8] = b"\x08http/1.1";

        if protocols.windows(3).any(|window| window == H2) {
            Ok(b"h2")
        } else if protocols.windows(9).any(|window| window == H11) {
            Ok(b"http/1.1")
        } else {
            Err(AlpnError::NOACK)
        }
    });

    builder.set_alpn_protos(b"\x08http/1.1\x02h2")?;

    let acceptor = builder.build();

    // construct http server
    HttpServer::new(move || {
        // enable pipeline mode for a better micro bench result.
        // in real world this should be left as disabled.(which is by default).
        let config = HttpServiceConfig::new().enable_http1_pipeline();
        let builder = HttpServiceBuilder::new(fn_service(handler))
            .config(config)
            .openssl(acceptor.clone());
        ErrorLoggerFactory::new(builder)
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}

async fn handler(_: Request<RequestBody>) -> Result<Response<ResponseBody>, Box<dyn std::error::Error>> {
    let res = Response::builder()
        .status(200)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(Bytes::from_static(b"Hello World!").into())?;

    Ok(res)
}
