# unreleased 0.2.0
## Change
- update `rustls` to `0.23.0`

## Remove
- `AsyncRead` and `AsyncWrite` trait impl has been removed from `rustls::TlsStream`. Please use `xitca_io::io::AsyncReadWrite` adaptor as alternative when these traits are needed for `TlsStream`.
