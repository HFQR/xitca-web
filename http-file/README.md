# a work in progress static file serving crate

```rust
use http::Request;
use http_file::ServeDir;

async fn serve(req: &Request<()>) {
    let dir = ServeDir::new("sample");
    let res = dir.serve(&req).await;
}
```

## Requirement
- nightly Rust
- [http](https://crates.io/crates/http) and [futures](https://crates.io/crates/futures) for http types and async streaming interaction[^1]

## Current state
- serving directory with sized files.
- every feature rather than above is missing. no range support, no content-encoding support. etc

[^1]: see project `Cargo.toml` for dependency versioning.
