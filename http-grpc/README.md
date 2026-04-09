# a gRPC codec crate for http type.

## Features
- gRPC length-prefixed framing codec with protobuf encode/decode via [prost](https://crates.io/crates/prost)
- request stream decoding and response body encoding over `http_body_alt::Body` trait
- optional compression support (brotli, gzip, deflate, zstd)

## Requirement
- Rust 1.85
- [http](https://crates.io/crates/http) for http types[^1]

[^1]: see project `Cargo.toml` for dependency versioning.
