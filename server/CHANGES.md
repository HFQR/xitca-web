# unreleased 0.5.0
## Change
- bump MSRV to `1.85` and Rust edition 2024
- rename `net::AsListener` trait to `IntoListener`. improve it's interface and reduce possibility of panicing
- update `xitca-service` to `0.3.0`

# 0.4.0
## Change
- bump MSRV to `1.79`
- update `xitca-io` to `0.4.0`
- update `xitca-service` to `0.2.0`
- update `xitca-unsafe-collection` to `0.2.0`
- update `tokio-uring` to `0.5.0`

# 0.3.0
## Remove
- remove `Builder::listen_unix`

## Change
- rename `http3` feature to `quic`
- `Builder::listen` is able to accept generic socket listener
- update `xitca-io` to `0.3.0`

# 0.2.0
## Change
- update `xitca-io` to `0.2.0`
