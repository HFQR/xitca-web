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
- [http](https://crates.io/crates/http) 
- [futures](https://crates.io/crates/futures) for http types and async streaming interaction
