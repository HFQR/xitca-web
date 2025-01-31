# unreleased 0.3.0
## Change
- bump MSRV to 1.85 and Rust edition 2024
- remove `AsyncClosure` trait. use `std::ops::AsyncFn` trait for functional middleware

## Add
- add `middleware::AsyncFn` middleware. `ServiceExt::enclosed_fn(<func>)` is equivalent to `ServiceExt::enclosed(middleware::AsyncFn(<func>))`

# 0.2.0
## Change
- bump MSRV to `1.79`
