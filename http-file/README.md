# an async static file serving crate

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

[^1]: see project `Cargo.toml` for dependency versioning.
