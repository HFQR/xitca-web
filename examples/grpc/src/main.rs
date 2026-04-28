//! A Http/2 grpc server using xitca-web's grpc extractor and responder.

use xitca_web::{
    App,
    error::{GrpcError, GrpcStatus},
    handler::{grpc::Grpc, handler_service},
    route::post,
};

mod hello_world {
    include!(concat!(env!("OUT_DIR"), "/helloworld.rs"));
}

use hello_world::{HelloReply, HelloRequest};

fn main() -> std::io::Result<()> {
    App::new()
        .at("/helloworld.Greeter/SayHello", post(handler_service(say_hello)))
        .serve()
        .worker_threads(1)
        .h2c_prior_knowledge()
        .bind("localhost:50051")?
        .run()
        .wait()
}

async fn say_hello(Grpc(req): Grpc<HelloRequest>) -> Result<Grpc<HelloReply>, GrpcError> {
    let response = req.request;
    if response.is_none() {
        return Err(GrpcError::new(GrpcStatus::InvalidArgument, "missing request field"));
    }
    Ok(Grpc(HelloReply { response }))
}
