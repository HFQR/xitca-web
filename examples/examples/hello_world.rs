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
use actix_web_alt::{App, WebRequest};
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
    acceptor.set_single_cert(cert_chain, keys.remove(0)).unwrap();

    let protos = vec!["h2".to_string().into(), "http/1.1".to_string().into()];
    acceptor.set_protocols(&protos);

    let acceptor = std::sync::Arc::new(acceptor);

    actix_server_alt::Builder::new()
        .bind("hell_world", addr, move || {
            let app = App::with_current_thread_state(String::from("AppState")).service(H2Factory);

            let http_builder = HttpServiceBuilder::h2(app).rustls(acceptor.clone());

            ErrorLoggerFactory::new(http_builder)
        })?
        .build()
        .await
}

struct H2Factory;

impl<State> ServiceFactory<WebRequest<'_, State>> for H2Factory
where
    State: std::fmt::Debug,
{
    type Response = HttpResponse;
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

struct H2Service {
    state: String,
}

impl<'r, State> Service<WebRequest<'r, State>> for H2Service
where
    State: std::fmt::Debug,
{
    type Response = HttpResponse;
    type Error = Box<dyn std::error::Error>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call<'c>(&'c self, mut req: WebRequest<'r, State>) -> Self::Future<'c>
    where
        'r: 'c,
    {
        async move {
            // borrow service state
            log::info!("Service state: {:?}", self.state);

            // borrow app state from request
            log::info!("App state: {:?}", req.state());

            // borrow http request from web request.
            let req = req.request_mut();

            log::info!("Request header: {:?}", req.headers());

            // collect body and print as string.
            let mut collect = BytesMut::new();

            while let Some(chunk) = req.body_mut().next().await {
                let chunk = chunk?;
                collect.extend_from_slice(&chunk);
            }

            log::info!("Request body as String: {:?}", String::from_utf8_lossy(&collect));

            let res = HttpResponse::builder()
                .status(200)
                .header("Content-Type", "text/plain")
                .body(Bytes::from_static(b"Hello World!").into())?;

            Ok(res)
        }
    }
}
