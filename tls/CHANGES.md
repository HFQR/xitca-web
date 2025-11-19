# unreleased

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
