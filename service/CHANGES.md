# unreleased 0.3.0
## Add
- add `middleware::AsyncFn` middleware. `ServiceExt::enclosed_fn(<func>)` is equivalent to `ServiceExt::enclosed(middleware::AsyncFn(<func>))`

## Change
- rename `AsyncClosure` trait to `AsyncFn`

# 0.2.0
## Change
- bump MSRV to `1.79`
