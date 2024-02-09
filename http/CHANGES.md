# unreleased 0.3.0
## Change
- `util::middleware::Logger` does not expect `tracing::Span` anymore. it would make new span per request. `Logger` requires the service type it enclosed with it's `Service::Error` type bound to `std::error::Error` trait instead of `std::fmt::Debug`.

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
