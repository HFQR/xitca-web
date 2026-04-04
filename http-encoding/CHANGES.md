# unreleased 0.3.0
## Change
- use `http-body-alt` as streaming interface to enable trailer support

## Remove
- remove `futures::Stream` implementation

## Fix
- `try_decoder` would try all possible encoding before eargerly yield with feature error

# 0.2.1
## Fix
- attach `transfer-encoding` header only to HTTP/1.1 response type.

# 0.2.0
## Change
- `try_decoder` function expect `&HeaderMap` instead of `impl Borrow<Request<()>>`. This enables client side decompress where headers are provided by `Response` type.
