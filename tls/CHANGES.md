# unreleased

# 0.6.0
## Remove
- removed `rustls-uring` feature
- removed `Clone` impl from all TlsStream types

## Add
- add `native-tls` feature

## Change
- Cargo feature name rework
    
    `<tls>` -> completion based aysnc IO impl
    
    `<tls>-poll` -> poll based async IO impl
    
    `rustls` -> the same constraint to above convension. with additonal constraint that no crypto provide is enabled
    
    `rustls-poll-<crypto>` -> specific crypto provider enabled
- internal change to reduce memory copy when `io-uring` feature enabled

# 0.5.1
## Add
- add `Clone` impl for `rustls_uring::TlsStream`

# 0.5.0
## Change
- update to Rust edition 2024
- update `xitca-io` to `0.5.0`

# 0.4.1
## Fix
- fix possible io hanging when utilizing `rustls` feature 

# 0.4.0
## Change
- bump MSRV to `1.79`
- update `xitca-io` to `0.4.0`
- update `tokio-uring` to `0.5.0`

# 0.3.0
- update `xitca-io` to `0.3.0`

# 0.2.3
## Add
- `rustls-no-crypto` feature
- `rustls-uring-no-crypto` feature

# 0.2.2
## Add
- `rustls-ring-crypto` feature

# 0.2.1
## Add
- `openssl` feature

# 0.2.0
## Change
- update `rustls` to `0.23.0`
- update `xitca-io` to `0.2.0`

## Remove
- `AsyncRead` and `AsyncWrite` traits impl has been removed from `rustls::TlsStream`. Please use `xitca_io::io::PollIoAdapter` as alternative when these traits are needed.
