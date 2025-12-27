# unreleased 0.3.0
## Add
- add `tokio-uring-xitca` feature

## Remove
- remove `tokio-uring` feature. tokio crate is adding direct support for io_uring and `tokio` feature would eventually cover all it's use case

# 0.2.1
## Change
- forbid unsafe code

# 0.2.0
## Change
- project compile on stable Rust channel with MSRV of 1.79
- update `tokio-uring` to `0.5.0`
