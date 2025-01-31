# unreleased 0.7.0
## Change
- bump MSRV to `1.85` and Rust edition 2024
- update `xitca-service` to `0.3.0`

# 0.6.0
## Change
- bump MSRV to `1.79`
- update `h3` to `0.0.6`
- update `h3-quinn` to `0.0.7`
- update `xitca-io` to `0.4.0`
- update `xitca-router` to `0.3.0`
- update `xitca-service` to `0.2.0`
- update `xitca-tls` to `0.4.0`
- update `xitca-unsafe-collection` to `0.2.0`
- update `tokio-uring` to `0.5.0`

# 0.5.0
## Change
- update `xitca-io` to `0.3.0`
- update `xitca-tls` to `0.3.0`
- update `h3` to `0.0.5`
- update `h3-quinn` to `0.0.6`

# 0.4.1
## Fix
- fix panic when using `rustls/ring` feature together with `xitca-http/rustls`
- fix panic when using `rustls/ring` feature together with `xitca-http/rustls-uring`

## Change
- update `xitca-io` to `0.2.1`
- update `xitca-tls` to `0.2.3`

# 0.4.0
## Add
- `util::service::router::PathGen` and `util::service::router::RouteObject` for advanced routing behavior. enabling more complex routing like multiple layer of router nesting. Example:
    ```rust
    // this pattern is now valid
    Router::new()
        .insert("/api", Router::new()
            .insert("/v2", Router::new().insert("/login", fn_service(..)))
        );
    
    // this pattern is also valid
    Router::new().insert("/api/v2", Router::new().insert("/login", fn_service(..)));
    ```

## Change
- `util::service::router::RouterGen` is renamed to `RouteGen`. It's API is shrunk to generating route service only. For route path generating please reference `util::service::router::PathGen`.
- `body::Either` doesn't expose it's enum variants in public API anymore.
- relax `Stream::Item` associated type when impl on `body::BoxBody::new` and `body::ResponseBody::boxed_stream` types. Instead of requiring the stream to yield `Ok<Bytes>` it now accepts types `Ok<impl Into<Bytes>>`.
- update `xitca-io` to `0.2.0`.
- update `xitca-tls` to `0.2.0`.

# 0.3.0
## Add
- add `util::middleware::catch_unwind`. A middleware catches panic and output it as error.

## Change
- `body::ResponseBody` doesn't expose it's enum variants in public API anymore.
- `util::middleware::Logger` does not expect `tracing::Span` anymore instead it wants `tracing::Level` for defining the verbosity of span. it would make new span per request with given `Level`. `Logger` middleware requires the service type it enclosed with it's `Service::Error` type bound to `std::error::Error` trait instead of only `std::fmt::Debug`. providing a better tracing output.
- update `xitca-unsafe-collection` to `0.1.1`.

# 0.2.2
## Change
- `set-cookie` header is not folded into single line of header value anymore.

# 0.2.1
## Change
- update `h3` to `0.0.4`.
- update `h3-quinn` to `0.0.5`.

# 0.2.0
## Change
- `h1::proto::context::Context::encode_headers` does not want `Extensions` type argument anymore. It also wants `&mut HeaderMap` instead of `HeaderMap` to avoid consuming ownership of it.
- update `xitca-router` to `0.2.0`
