//! A Http/1 server decompress request body and compress response body.

use std::io;

use futures_util::{pin_mut, Stream, StreamExt};
use http_encoding::{Coder, ContentDecoder, ContentEncoder, ContentEncoding};
use tracing::info;
use xitca_http::{
    bytes::{Buf, Bytes},
    http::{const_header_value::TEXT_UTF8, header::{CONTENT_TYPE}, Response, self},
    HttpServiceBuilder, Request, ResponseBody,
};
use xitca_service::{fn_service, BuildServiceExt, Service};

fn main() -> io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("xitca=trace,[xitca-logger]=trace")
        .init();

    xitca_server::Builder::new()
        .bind("content-encoding", "127.0.0.1:8080", || {
            let service = fn_service(handler)
                .enclosed_fn(decode_middleware)
                .enclosed_fn(encode_middleware);

            HttpServiceBuilder::h1(service).with_logger()
        })?
        .build()
        .wait()
}

async fn handler<B, E>(req: Request<B>) -> Result<Response<ResponseBody>, Box<dyn std::error::Error>>
where
    B: Stream<Item = Result<Bytes, E>>,
    E: std::error::Error + 'static,
{
    // collect and log request body as string.
    let mut buf = Vec::new();
    let body = req.into_body();
    pin_mut!(body);
    while let Some(res) = body.next().await {
        buf.extend_from_slice(res?.chunk());
    }
    info!("Request body is: {}", std::str::from_utf8(&buf)?);

    let res = Response::builder()
        .status(200)
        .header(CONTENT_TYPE, TEXT_UTF8)
        .body("Hello World!".into())?;
        
    Ok(res)
}

// a simple middleware look into request's `content-encoding` header and apply proper body decoder to it.
async fn decode_middleware<S, B, E>(service: &S, req: Request<B>) -> Result<S::Response, S::Error>
where
    S: Service<Request<Coder<B, ContentDecoder>>>,
    B: Stream<Item = Result<Bytes, E>>,
{
    let (req, body) = req.replace_body(());
    let stream = http_encoding::try_decoder(&*req, body).unwrap();
    let req = req.map_body(|_| stream);
    service.call(req).await
}

// a simple middleware apply br body encoder to compress the response.
async fn encode_middleware<S, Req, B>(service: &S, req: Req) -> Result<Response<Coder<ResponseBody, ContentEncoder>>, S::Error>
where
    S: Service<Req, Response = Response<ResponseBody>>,
    Req: std::borrow::Borrow<http::Request<B>>,
{
    let res = service.call(req).await?;
    // hardcode br as compress encoding. in real world this should be a look up into `accept-encoding` header of request
    // and genarated from it.
    Ok(http_encoding::try_encoder(res, ContentEncoding::Br).unwrap())
}
