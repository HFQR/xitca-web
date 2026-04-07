fn main() {
    prost_build::compile_protos(
        &["../examples/grpc/protobuf/helloworld.proto"],
        &["../examples/grpc/protobuf/"],
    )
    .unwrap();
}
