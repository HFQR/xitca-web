//! gRPC interop-style tests.
//!
//! Tests exercise the xitca-web gRPC server implementation against the gRPC spec
//! using xitca-client as the gRPC client.

use std::net::TcpListener;

use futures_util::{SinkExt, StreamExt};
use xitca_client::{Client, grpc::ContentEncoding};
use xitca_web::{
    App, WebContext,
    error::{GrpcError, GrpcStatus},
    handler::{
        grpc::{Grpc, GrpcStreamRequest, GrpcStreamResponse},
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

/// Bidirectional streaming echo: reads HelloRequest messages, echoes back HelloReply for each.
async fn bidi_echo(mut stream: GrpcStreamRequest) -> Result<GrpcStreamResponse<HelloReply>, GrpcError> {
    let (tx, body) = GrpcStreamResponse::channel();
    // Use spawn_local style: read request stream and forward replies via channel sender
    // on a separate non-Send task using the current-thread runtime context.
    tokio::task::spawn_local(async move {
        let mut tx = tx;
        while let Ok(Some(req)) = stream.message::<HelloRequest>().await {
            let reply = HelloReply { response: req.request };
            if tx.send_message(reply).await.is_err() {
                break;
            }
        }
    });
    Ok(body)
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
        .at("/helloworld.Greeter/BidiEcho", handler_service(bidi_echo))
        .enclosed(GrpcTimeout)
        .serve()
        .h2c_prior_knowledge()
        .listen(lst)?
        .run();

    Ok((url_base, server))
}

fn make_hello(name: &str) -> HelloRequest {
    HelloRequest {
        request: Some(Hello {
            name: name.to_string(),
            ..Default::default()
        }),
    }
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
    let res = c.grpc(format!("{base}/helloworld.Greeter/SayHello")).send(req).await?;

    assert_eq!(res.status().as_u16(), 200);
    assert!(
        res.headers()
            .get(CONTENT_TYPE)
            .unwrap()
            .to_str()?
            .starts_with("application/grpc")
    );

    let (reply, trailers): (HelloReply, _) = res.grpc().await?;
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
    let req = make_hello(&large_name);
    let (reply, _): (HelloReply, _) = c
        .grpc(format!("{base}/helloworld.Greeter/SayHello"))
        .send(req)
        .await?
        .grpc()
        .await?;

    assert_eq!(reply.response.unwrap().name, large_name);

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
    let res = c
        .grpc(format!("{base}/helloworld.Greeter/ErrorWithMessage"))
        .send(req)
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
    let res = c
        .grpc(format!("{base}/helloworld.Greeter/SpecialStatus"))
        .send(req)
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
    let res = c
        .grpc(format!("{base}/helloworld.Greeter/NonExistentMethod"))
        .send(req)
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
    let res = c.grpc(format!("{base}/nonexistent.Service/Method")).send(req).await?;

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
    let mut builder = c.grpc(format!("{base}/helloworld.Greeter/SlowHandler"));
    builder.headers_mut().insert("grpc-timeout", "100m".parse().unwrap());
    let res = builder.send(req).await?;

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
    let mut builder = c.grpc(format!("{base}/helloworld.Greeter/EchoMetadata"));
    builder
        .headers_mut()
        .insert("x-custom-header", "custom-value".parse().unwrap());
    builder
        .headers_mut()
        .insert("x-custom-trailer-bin", "dHJhaWxlcg".parse().unwrap());
    let res = builder.send(req).await?;

    // Check initial metadata
    assert_eq!(res.headers().get("x-custom-header").unwrap().to_str()?, "custom-value");

    let (reply, trailers): (HelloReply, _) = res.grpc().await?;
    assert!(reply.response.is_some());

    let trailers = trailers.unwrap();
    assert_eq!(trailers.get("grpc-status").unwrap().to_str()?, "0");
    assert_eq!(trailers.get("x-custom-trailer-bin").unwrap().to_str()?, "dHJhaWxlcg");

    handle.stop(false);
    Ok(())
}

#[tokio::test]
async fn grpc_bidi_stream() -> Result<(), Error> {
    let (base, mut server) = test_grpc_server()?;
    let handle = server.handle()?;
    tokio::spawn(server);

    let c = Client::new();

    let grpc = c
        .grpc_stream(&format!("{base}/helloworld.Greeter/BidiEcho"))
        .send::<HelloReply>()
        .await?;

    // Split into sink + stream for concurrent send/recv.
    let (mut sink, mut reader) = grpc.split();

    // Send multiple messages.
    sink.send(make_hello("one")).await?;
    sink.send(make_hello("two")).await?;

    // Read them back.
    let reply: HelloReply = reader.next().await.expect("expected reply 1")?;
    assert_eq!(reply.response.unwrap().name, "one");

    let reply: HelloReply = reader.next().await.expect("expected reply 2")?;
    assert_eq!(reply.response.unwrap().name, "two");

    // Close the send half.
    SinkExt::<HelloRequest>::close(&mut sink).await?;

    // Stream should end.
    assert!(reader.next().await.is_none());

    handle.stop(false);
    Ok(())
}

#[tokio::test]
async fn grpc_compressed_unary() -> Result<(), Error> {
    let (base, mut server) = test_grpc_server()?;
    let handle = server.handle()?;
    tokio::spawn(server);

    let c = Client::new();
    let req = make_hello("compressed");
    let mut builder = c.grpc(format!("{base}/helloworld.Greeter/SayHello"));
    builder.set_encoding(ContentEncoding::Zstd);
    let res = builder.send(req).await?;

    assert_eq!(res.status().as_u16(), 200);

    // Server should respond with compression (picks from grpc-accept-encoding).
    assert!(res.headers().get("grpc-encoding").is_some());

    let (reply, trailers): (HelloReply, _) = res.grpc().await?;
    assert_eq!(reply.response.unwrap().name, "compressed");
    assert_eq!(trailers.unwrap().get("grpc-status").unwrap().to_str()?, "0");

    handle.stop(false);
    Ok(())
}

#[tokio::test]
async fn grpc_compressed_bidi_stream() -> Result<(), Error> {
    let (base, mut server) = test_grpc_server()?;
    let handle = server.handle()?;
    tokio::spawn(server);

    let c = Client::new();

    let mut builder = c.grpc_stream(&format!("{base}/helloworld.Greeter/BidiEcho"));
    builder.set_encoding(ContentEncoding::Zstd);
    let grpc = builder.send::<HelloReply>().await?;

    let (mut sink, mut reader) = grpc.split();

    sink.send(make_hello("compress-one")).await?;
    sink.send(make_hello("compress-two")).await?;

    let reply: HelloReply = reader.next().await.expect("expected reply 1")?;
    assert_eq!(reply.response.unwrap().name, "compress-one");

    let reply: HelloReply = reader.next().await.expect("expected reply 2")?;
    assert_eq!(reply.response.unwrap().name, "compress-two");

    SinkExt::<HelloRequest>::close(&mut sink).await?;
    assert!(reader.next().await.is_none());

    handle.stop(false);
    Ok(())
}
