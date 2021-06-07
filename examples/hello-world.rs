//! A Http server returns Hello World String as Response.

#![allow(incomplete_features)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]

use std::io;

use actix_http_alt::{
    http::{Request, Response},
    util::ErrorLoggerFactory,
    HttpServiceBuilder, HttpServiceConfig, RequestBody, ResponseBody,
};
use actix_service_alt::fn_service;
use bytes::Bytes;
use h3_quinn::quinn::generic::ServerConfig;
use h3_quinn::quinn::{crypto::rustls::TlsSession, CertificateChain, PrivateKey, ServerConfigBuilder};
use openssl::ssl::{AlpnError, SslAcceptor, SslFiletype, SslMethod};

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    std::env::set_var("RUST_LOG", "actix=trace, info");
    env_logger::init();

    // set up openssl and alpn protocol.
    let acceptor = openssl_config()?;

    // construct http3 quic server config
    let config = h3_config()?;

    // construct server
    actix_server_alt::Builder::new()
        // bind to both tcp and udp addresses where a single service would handle http/1/2/3 traffic.
        .bind_all("hello-world", "127.0.0.1:8080", config, move || {
            // enable pipeline mode for a better micro bench result.
            // in real world this should be left as disabled.(which is by default).
            let config = HttpServiceConfig::new().enable_http1_pipeline();
            let builder = HttpServiceBuilder::new(fn_service(handler)).config(config);

            let builder = HttpServiceBuilder::<_, RequestBody, _, _, _>::openssl(builder, acceptor.clone());

            ErrorLoggerFactory::new(builder)
        })?
        .build()
        .await
}

async fn handler(_: Request<RequestBody>) -> Result<Response<ResponseBody>, Box<dyn std::error::Error>> {
    let res = Response::builder()
        .status(200)
        .header("Content-Type", "text/plain; charset=utf-8")
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

fn openssl_config() -> io::Result<SslAcceptor> {
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls())?;

    builder.set_private_key_file("./cert/key.pem", SslFiletype::PEM)?;
    builder.set_certificate_chain_file("./cert/cert.pem")?;

    const H11: &[u8] = b"\x08http/1.1";
    const H2: &[u8] = b"\x02h2";

    builder.set_alpn_select_callback(|_, protocols| {
        if protocols.windows(3).any(|window| window == H2) {
            Ok(b"h2")
        } else if protocols.windows(9).any(|window| window == H11) {
            Ok(b"http/1.1")
        } else {
            Err(AlpnError::NOACK)
        }
    });

    let protos = H11.iter().chain(H2).cloned().collect::<Vec<_>>();

    builder.set_alpn_protos(&protos)?;

    Ok(builder.build())
}
