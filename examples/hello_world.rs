//! A Http/2 server returns Hello World String as Response and always close connection afterwards.

#![allow(incomplete_features)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]

use std::future::Future;
use std::io;
use std::task::{Context, Poll};

use actix_server_alt::http::{h2::RequestBody, HttpRequest, HttpServiceBuilder};
use actix_server_alt::{Service, ServiceFactory};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use http::Response;

#[actix_web::main]
async fn main() -> io::Result<()> {
    std::env::set_var("RUST_LOG", "actix=trace, info");
    env_logger::init();

    let addr = "127.0.0.1:8080";

    actix_server_alt::Builder::new()
        .bind("test", addr, || {
            use openssl::pkey::PKey;
            use openssl::ssl::{AlpnError, SslAcceptor, SslMethod};
            use openssl::x509::X509;

            let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
            let cert_file = cert.serialize_pem().unwrap();
            let key_file = cert.serialize_private_key_pem();
            let cert = X509::from_pem(cert_file.as_bytes()).unwrap();
            let key = PKey::private_key_from_pem(key_file.as_bytes()).unwrap();

            let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
            builder.set_certificate(&cert).unwrap();
            builder.set_private_key(&key).unwrap();

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

            builder.set_alpn_protos(b"\x08http/1.1\x02h2").unwrap();

            let acceptor = builder.build();

            HttpServiceBuilder::new(MyServiceFactor).openssl(acceptor)
        })?
        .build()
        .await
}

struct MyServiceFactor;

impl ServiceFactory<HttpRequest<RequestBody>> for MyServiceFactor {
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

// a parent service that hold string state and a child service.
struct MyService {
    name: String,
    child: ChildService,
}

impl Service for MyService {
    type Request<'r> = HttpRequest<RequestBody>;
    type Response = Response<Bytes>;
    type Error = Box<dyn std::error::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.child.poll_ready(cx)
    }

    fn call<'s, 'r, 'f>(&'s self, req: Self::Request<'r>) -> Self::Future<'f>
    where
        's: 'f,
        'r: 'f,
    {
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
    type Response = Response<Bytes>;
    type Error = Box<dyn std::error::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call<'s, 'r, 'f>(&'s self, (_, req): Self::Request<'r>) -> Self::Future<'f>
    where
        's: 'f,
        'r: 'f,
    {
        async move {
            // split request into head and body
            let (parts, mut body) = req.into_parts();

            println!("Request head: {:?}", parts);

            // collect body and print as string.
            let mut collect = BytesMut::new();

            while let Some(chunk) = body.next().await {
                let chunk = chunk?;
                collect.extend_from_slice(&chunk);
            }

            println!(
                "Request body as String: {:?}",
                String::from_utf8_lossy(&collect)
            );

            let res = Response::builder()
                .status(200)
                .body(Bytes::from_static(b"Hello World!"))?;

            Ok(res)
        }
    }
}

struct BorrowState<'a>(&'a str);
