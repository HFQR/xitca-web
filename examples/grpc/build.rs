fn main() {
    prost_build::compile_protos(&["protobuf/helloworld.proto"], &["protobuf/"]).unwrap();
}
