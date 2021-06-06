//! A Http/1 server echos back websocket text message and respond to ping message.

#![allow(incomplete_features)]
#![feature(generic_associated_types, min_type_alias_impl_trait)]

use std::pin::Pin;
use std::task::{Context, Poll};

use actix_http_alt::{
    h1::RequestBody,
    http::{Request, Response},
    util::ErrorLoggerFactory,
    BodyError, HttpServiceBuilder, ResponseBody,
};
use actix_service_alt::fn_service;
use actix_web_alt::HttpServer;
use bytes::Bytes;
use futures_util::Stream;
use http_ws::{ws, EncodeStream, Message};
use log::info;
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod};

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

    let acceptor = builder.build();

    // construct http server
    HttpServer::new(move || {
        let builder = HttpServiceBuilder::h1(fn_service(handler)).openssl(acceptor.clone());
        ErrorLoggerFactory::new(builder)
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}

async fn handler(
    req: Request<RequestBody>,
) -> Result<Response<ResponseBody<MapErrStream>>, Box<dyn std::error::Error>> {
    let (mut decode, res, tx) = ws(req).map_err(|e| format!("{:?}", e))?;

    tokio::task::spawn_local(async move {
        while let Some(msg) = decode.next().await {
            match msg.ok().unwrap() {
                Message::Text(bytes) => {
                    let str = String::from_utf8_lossy(bytes.as_ref());
                    info!("Got text message {:?}", str);
                    tx.send(Message::Text(format!("Echo: {}", str).into())).await.unwrap();
                }
                Message::Ping(bytes) => {
                    info!("Got ping message");
                    tx.send(Message::Pong(bytes)).await.unwrap();
                }
                Message::Close(reason) => {
                    info!("Got close message");
                    tx.send(Message::Close(reason)).await.unwrap();
                    return;
                }
                _ => {}
            }
        }
    });

    let (parts, body) = res.into_parts();
    let body = ResponseBody::stream(MapErrStream(body));
    let res = Response::from_parts(parts, body);

    Ok(res)
}

// a boilerplate stream that map EncodeStream error to actix_http-alt::BodyError.
struct MapErrStream(EncodeStream);

impl Stream for MapErrStream {
    type Item = Result<Bytes, BodyError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().0)
            .poll_next(cx)
            .map_err(|e| BodyError::Std(format!("{:?}", e).into()))
    }
}
