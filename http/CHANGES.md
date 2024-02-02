# unreleased

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
