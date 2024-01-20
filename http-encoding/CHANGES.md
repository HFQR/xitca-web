# unreleased

# 0.2.0
## Change
- `try_decoder` function expect `&HeaderMap` instead of `impl Borrow<Request<()>>`. This enables client side decompress where headers are provided by `Response` type.
