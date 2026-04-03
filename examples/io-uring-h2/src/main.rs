//! A Http/2 server returns Hello World String as Response.
//!
//! *. io_uring is a linux OS feature.
//! *. random self signed cert is used for tls certification.

#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::{convert::Infallible, io};

use xitca_http::{
    HttpServiceBuilder,
    body::Full,
    bytes::Bytes,
    h2::RequestBody,
    http::{Request, RequestExt, Response, const_header_value::TEXT_UTF8, header::CONTENT_TYPE},
};
use xitca_service::{ServiceExt, fn_service};

fn main() -> io::Result<()> {
    xitca_server::Builder::new()
        .bind(
            "http/2",
            "127.0.0.1:8080",
            fn_service(handler).enclosed(
                HttpServiceBuilder::h2().io_uring().openssl(tls_config()?), // specify io_uring flavor of http service.
            ),
        )?
        .build()
        .wait()
}

async fn handler(_: Request<RequestExt<RequestBody>>) -> Result<Response<Full<Bytes>>, Infallible> {
    Ok(Response::builder()
        .header(CONTENT_TYPE, TEXT_UTF8)
        .body(Full::new(Bytes::from_static(b"Hello, World!")))
        .unwrap())
}

// async fn handler(_: Request<RequestExt<xitca_http::h2::RequestBody>>) -> Result<Response<xitca_http::body::Once<Bytes>>, Infallible> {
//     Ok(Response::builder()
//         .header(CONTENT_TYPE, TEXT_UTF8)
//         .body(xitca_http::body::Once::new(Bytes::from_static(b"Hello World!")))
//         .unwrap())
// }

// // rustls configuration.
// fn tls_config() -> std::sync::Arc<rustls::ServerConfig> {
//     let subject_alt_names = vec!["127.0.0.1".to_string(), "localhost".to_string()];

//     let cert = rcgen::generate_simple_self_signed(subject_alt_names).unwrap();

//     let mut config = rustls::ServerConfig::builder()
//         .with_no_client_auth()
//         .with_single_cert(
//             vec![cert.cert.into()],
//             cert.signing_key.serialize_der().try_into().unwrap(),
//         )
//         .unwrap();

//     config.alpn_protocols = vec![b"h2".to_vec()];

//     std::sync::Arc::new(config)
// }

use openssl::ssl::{AlpnError, SslAcceptor, SslFiletype, SslMethod};
fn tls_config() -> io::Result<SslAcceptor> {
    // set up openssl and alpn protocol.
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls())?;
    builder.set_private_key_file("../cert/key.pem", SslFiletype::PEM)?;
    builder.set_certificate_chain_file("../cert/cert.pem")?;

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

    Ok(builder.build())
}
