//! A Http/2 server handling low level grpc call.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use futures_util::TryStreamExt;
use prost::Message;
use xitca_http::{
    body::Once,
    bytes::{BufMut, Bytes, BytesMut},
    h2::RequestBody,
    http::{
        const_header_value::GRPC,
        header::{HeaderName, HeaderValue, CONTENT_TYPE, TRAILER},
        IntoResponse, Response,
    },
    util::service::{route::post, Router},
    HttpServiceBuilder, Request,
};
use xitca_service::fn_service;

mod hello_world {
    include!(concat!(env!("OUT_DIR"), "/helloworld.rs"));
}

fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("xitca=info,[xitca-logger]=trace")
        .init();

    let factory = || {
        let route = Router::new().insert("/helloworld.Greeter/SayHello", post(fn_service(grpc)));

        HttpServiceBuilder::h2(route)
    };

    xitca_server::Builder::new()
        .bind("http/2", "localhost:50051", factory)?
        .build()
        .wait()
}

async fn grpc(mut req: Request<RequestBody>) -> Result<Response<Once<Bytes>>, anyhow::Error> {
    let body = req.body_mut();

    let mut buf = BytesMut::new();

    while let Some(bytes) = body.try_next().await? {
        buf.extend_from_slice(&*bytes);
    }

    let msg: hello_world::HelloRequest = Message::decode(buf.split_off(5))?;

    Message::encode(&hello_world::HelloReply { response: msg.request }, &mut buf)?;

    let len = buf.len() - 5;

    (&mut buf[1..5]).put_u32(len as u32);

    let mut res = req.into_response(buf.freeze());

    #[allow(clippy::declare_interior_mutable_const)]
    {
        const GRPC_STATUS_NAME: HeaderName = HeaderName::from_static("grpc-status");
        const GRPC_STATUS_VALUE: HeaderValue = HeaderValue::from_static("0");
        const GRPC_STATUS_NAME_AS_VALUE: HeaderValue = HeaderValue::from_static("grpc-status");

        res.headers_mut().insert(CONTENT_TYPE, GRPC);
        res.headers_mut().insert(GRPC_STATUS_NAME, GRPC_STATUS_VALUE);
        res.headers_mut().insert(TRAILER, GRPC_STATUS_NAME_AS_VALUE);
    }

    Ok(res)
}
