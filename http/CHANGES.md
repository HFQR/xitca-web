# unreleased

# 0.2.0
## Changed
- `h1::proto::context::Context::encode_headers` does not want `Extensions` type argument anymore. It also wants `&mut HeaderMap` instead of `HeaderMap` to avoid consuming ownership of it.
- update `xitca-router` to `0.2.0`
