//! gRPC interop-style tests.
//!
//! Tests exercise the xitca-web gRPC server implementation against the gRPC spec
//! using xitca-client as a raw HTTP/2 client with manual gRPC framing.

use core::future::poll_fn;
use std::{net::TcpListener, pin::pin};

use prost::Message;
use xitca_client::Client;
use xitca_http::{
    body::Body,
    bytes::{BufMut, Bytes, BytesMut},
    http::Version,
};
use xitca_web::{
    App, WebContext,
    error::{GrpcError, GrpcStatus},
    handler::{
        grpc::{Grpc, GrpcStreamResponse},
        handler_service,
    },
    http::header::{CONTENT_TYPE, HeaderMap},
    middleware::grpc_timeout::GrpcTimeout,
};

type Error = Box<dyn std::error::Error + Send + Sync>;

mod hello_world {
    include!(concat!(env!("OUT_DIR"), "/helloworld.rs"));
}

use hello_world::{Hello, HelloReply, HelloRequest};

// -- gRPC framing helpers --

fn grpc_encode<M: Message>(msg: &M) -> Bytes {
    let len = msg.encoded_len();
    let mut buf = BytesMut::with_capacity(5 + len);
    buf.put_u8(0);
    buf.put_u32(len as u32);
    msg.encode(&mut buf).unwrap();
    buf.freeze()
}

fn grpc_decode<M: Message + Default>(data: &[u8]) -> M {
    assert!(data.len() >= 5, "grpc frame too short");
    let len = u32::from_be_bytes(data[1..5].try_into().unwrap()) as usize;
    assert_eq!(data.len() - 5, len, "grpc frame length mismatch");
    M::decode(&data[5..]).unwrap()
}

// -- handlers --

async fn say_hello(Grpc(req): Grpc<HelloRequest>) -> Result<Grpc<HelloReply>, GrpcError> {
    let response = req.request;
    if response.is_none() {
        return Err(GrpcError::new(GrpcStatus::InvalidArgument, "missing request field"));
    }
    Ok(Grpc(HelloReply { response }))
}

async fn error_with_message(Grpc(_req): Grpc<HelloRequest>) -> Result<Grpc<HelloReply>, GrpcError> {
    Err(GrpcError::new(GrpcStatus::Internal, "test status message"))
}

async fn special_status(Grpc(_req): Grpc<HelloRequest>) -> Result<Grpc<HelloReply>, GrpcError> {
    Err(GrpcError::new(GrpcStatus::Unknown, "réponse spéciale ☕"))
}

async fn slow_handler(Grpc(_req): Grpc<HelloRequest>) -> Result<Grpc<HelloReply>, GrpcError> {
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    Ok(Grpc(HelloReply { response: None }))
}

async fn echo_metadata(
    ctx: &WebContext<'_>,
    Grpc(req): Grpc<HelloRequest>,
) -> Result<GrpcStreamResponse<HelloReply>, GrpcError> {
    let mut initial = HeaderMap::new();
    let mut trailing = HeaderMap::new();

    if let Some(v) = ctx.req().headers().get("x-custom-header") {
        initial.insert("x-custom-header", v.clone());
    }
    if let Some(v) = ctx.req().headers().get("x-custom-trailer-bin") {
        trailing.insert("x-custom-trailer-bin", v.clone());
    }

    let response = HelloReply { response: req.request };
    Ok(GrpcStreamResponse::once(response)
        .initial_metadata(initial)
        .trailing_metadata(trailing))
}

// -- test server --

fn test_grpc_server() -> Result<(String, xitca_server::ServerFuture), Error> {
    let lst = TcpListener::bind("127.0.0.1:0")?;
    let addr = lst.local_addr()?;
    let url_base = format!("http://{}:{}", addr.ip(), addr.port());

    let server = App::new()
        .at("/helloworld.Greeter/SayHello", handler_service(say_hello))
        .at(
            "/helloworld.Greeter/ErrorWithMessage",
            handler_service(error_with_message),
        )
        .at("/helloworld.Greeter/SpecialStatus", handler_service(special_status))
        .at("/helloworld.Greeter/SlowHandler", handler_service(slow_handler))
        .at("/helloworld.Greeter/EchoMetadata", handler_service(echo_metadata))
        .enclosed(GrpcTimeout)
        .serve()
        .h2c_prior_knowledge()
        .listen(lst)?
        .run();

    Ok((url_base, server))
}

/// Read all frames from a response body, returning (data bytes, trailers).
async fn read_grpc_response(
    res: xitca_http::http::Response<impl Body<Data = Bytes> + Unpin>,
) -> (BytesMut, Option<HeaderMap>) {
    use xitca_http::body::Frame;

    let (_, body) = res.into_parts();
    let mut body = pin!(body);
    let mut data = BytesMut::new();
    let mut trailers = None;

    loop {
        match poll_fn(|cx| Body::poll_frame(body.as_mut(), cx)).await {
            Some(Ok(Frame::Data(bytes))) => data.extend_from_slice(&bytes),
            Some(Ok(Frame::Trailers(map))) => {
                trailers = Some(map);
                break;
            }
            Some(Err(_)) => break,
            None => break,
        }
    }

    (data, trailers)
}

fn make_hello(name: &str) -> HelloRequest {
    HelloRequest {
        request: Some(Hello {
            name: name.to_string(),
            ..Default::default()
        }),
    }
}

fn grpc_request<'a>(c: &'a Client, url: &str, msg: &impl Message) -> xitca_client::RequestBuilder<'a> {
    let body = grpc_encode(msg);
    let mut req = c.post(url).version(Version::HTTP_2);
    req.headers_mut()
        .insert(CONTENT_TYPE, "application/grpc".parse().unwrap());
    req.bytes(body)
}

// -- tests --

#[tokio::test]
async fn grpc_empty_unary() -> Result<(), Error> {
    let (base, mut server) = test_grpc_server()?;
    let handle = server.handle()?;
    tokio::spawn(server);

    let c = Client::new();
    let req = HelloRequest {
        request: Some(Hello::default()),
    };
    let res = grpc_request(&c, &format!("{base}/helloworld.Greeter/SayHello"), &req)
        .send()
        .await?;

    assert_eq!(res.status().as_u16(), 200);
    assert!(
        res.headers()
            .get(CONTENT_TYPE)
            .unwrap()
            .to_str()?
            .starts_with("application/grpc")
    );

    let (data, trailers) = read_grpc_response(res.into_inner()).await;
    let reply: HelloReply = grpc_decode(&data);
    assert!(reply.response.is_some());
    assert_eq!(trailers.unwrap().get("grpc-status").unwrap().to_str()?, "0");

    handle.stop(false);
    Ok(())
}

#[tokio::test]
async fn grpc_large_unary() -> Result<(), Error> {
    let (base, mut server) = test_grpc_server()?;
    let handle = server.handle()?;
    tokio::spawn(server);

    let c = Client::new();
    let large_name = "x".repeat(1024 * 64 * 32);
    println!("payload_length :{}", large_name.len());
    let req = make_hello(&large_name);
    let res = grpc_request(&c, &format!("{base}/helloworld.Greeter/SayHello"), &req)
        .send()
        .await?;

    assert_eq!(res.status().as_u16(), 200);
    let (data, trailers) = read_grpc_response(res.into_inner()).await;
    let reply: HelloReply = grpc_decode(&data);
    assert_eq!(reply.response.unwrap().name, large_name);
    assert_eq!(trailers.unwrap().get("grpc-status").unwrap().to_str()?, "0");

    handle.stop(false);
    Ok(())
}

#[tokio::test]
async fn grpc_status_code_and_message() -> Result<(), Error> {
    let (base, mut server) = test_grpc_server()?;
    let handle = server.handle()?;
    tokio::spawn(server);

    let c = Client::new();
    let req = make_hello("test");
    let res = grpc_request(&c, &format!("{base}/helloworld.Greeter/ErrorWithMessage"), &req)
        .send()
        .await?;

    assert_eq!(res.status().as_u16(), 200);
    // Trailers-only response: grpc-status is in response headers
    assert_eq!(res.headers().get("grpc-status").unwrap().to_str()?, "13");
    assert_eq!(
        res.headers().get("grpc-message").unwrap().to_str()?,
        "test%20status%20message"
    );

    handle.stop(false);
    Ok(())
}

#[tokio::test]
async fn grpc_special_status_message() -> Result<(), Error> {
    let (base, mut server) = test_grpc_server()?;
    let handle = server.handle()?;
    tokio::spawn(server);

    let c = Client::new();
    let req = make_hello("test");
    let res = grpc_request(&c, &format!("{base}/helloworld.Greeter/SpecialStatus"), &req)
        .send()
        .await?;

    // Trailers-only response: grpc-status is in response headers
    assert_eq!(res.headers().get("grpc-status").unwrap().to_str()?, "2");

    let encoded_msg = res.headers().get("grpc-message").unwrap().to_str()?;
    let decoded = percent_encoding::percent_decode_str(encoded_msg).decode_utf8()?;
    assert_eq!(decoded, "réponse spéciale ☕");

    handle.stop(false);
    Ok(())
}

#[tokio::test]
async fn grpc_unimplemented_method() -> Result<(), Error> {
    let (base, mut server) = test_grpc_server()?;
    let handle = server.handle()?;
    tokio::spawn(server);

    let c = Client::new();
    let req = make_hello("test");
    let res = grpc_request(&c, &format!("{base}/helloworld.Greeter/NonExistentMethod"), &req)
        .send()
        .await?;

    // Trailers-only response: grpc-status is in response headers
    assert_eq!(res.headers().get("grpc-status").unwrap().to_str()?, "12");

    handle.stop(false);
    Ok(())
}

#[tokio::test]
async fn grpc_unimplemented_service() -> Result<(), Error> {
    let (base, mut server) = test_grpc_server()?;
    let handle = server.handle()?;
    tokio::spawn(server);

    let c = Client::new();
    let req = make_hello("test");
    let res = grpc_request(&c, &format!("{base}/nonexistent.Service/Method"), &req)
        .send()
        .await?;

    // Trailers-only response: grpc-status is in response headers
    assert_eq!(res.headers().get("grpc-status").unwrap().to_str()?, "12");

    handle.stop(false);
    Ok(())
}

#[tokio::test]
async fn grpc_timeout_on_sleeping_server() -> Result<(), Error> {
    let (base, mut server) = test_grpc_server()?;
    let handle = server.handle()?;
    tokio::spawn(server);

    let c = Client::new();
    let req = make_hello("test");
    let body = grpc_encode(&req);
    let mut builder = c
        .post(&format!("{base}/helloworld.Greeter/SlowHandler"))
        .version(Version::HTTP_2);
    builder
        .headers_mut()
        .insert(CONTENT_TYPE, "application/grpc".parse().unwrap());
    builder.headers_mut().insert("grpc-timeout", "100m".parse().unwrap());
    let res = builder.bytes(body).send().await?;

    // Trailers-only response: grpc-status is in response headers
    assert_eq!(res.headers().get("grpc-status").unwrap().to_str()?, "4");

    handle.stop(false);
    Ok(())
}

#[tokio::test]
async fn grpc_custom_metadata() -> Result<(), Error> {
    let (base, mut server) = test_grpc_server()?;
    let handle = server.handle()?;
    tokio::spawn(server);

    let c = Client::new();
    let req = make_hello("test");
    let body = grpc_encode(&req);
    let mut builder = c
        .post(&format!("{base}/helloworld.Greeter/EchoMetadata"))
        .version(Version::HTTP_2);
    builder
        .headers_mut()
        .insert(CONTENT_TYPE, "application/grpc".parse().unwrap());
    builder
        .headers_mut()
        .insert("x-custom-header", "custom-value".parse().unwrap());
    builder
        .headers_mut()
        .insert("x-custom-trailer-bin", "dHJhaWxlcg".parse().unwrap());
    let res = builder.bytes(body).send().await?;

    // Check initial metadata
    assert_eq!(res.headers().get("x-custom-header").unwrap().to_str()?, "custom-value");

    let (data, trailers) = read_grpc_response(res.into_inner()).await;
    let reply: HelloReply = grpc_decode(&data);
    assert!(reply.response.is_some());

    let trailers = trailers.unwrap();
    assert_eq!(trailers.get("grpc-status").unwrap().to_str()?, "0");
    assert_eq!(trailers.get("x-custom-trailer-bin").unwrap().to_str()?, "dHJhaWxlcg");

    handle.stop(false);
    Ok(())
}
