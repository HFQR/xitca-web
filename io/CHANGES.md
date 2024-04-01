# unreleased 0.3.0
## Change
- rename `http3` feature to `quic`
- rename `H3ServerConfig` to `QuicConfig`
- rename `Udp` prefixed types to `Quic` prefixed
- `AsyncIo::ready` and `AsyncIoDyn::ready` receive `&mut self`. Allowing more arbitrary types implement these traits.
- `AsyncIo::poll_ready` and `AsyncIoDyn::poll_ready` receive `&mut self`. For the same reason as above change.

# 0.2.1
## Add
- `io::AsyncIoDyn` for object safe version of `io::AsyncIo`.

# 0.2.0
## Add
- `io::PollIoAdapter` as adaptor between `io::AsyncIo` and `io::{AsyncRead, AsyncWrite}` traits.

## Remove
- `io::AsyncRead` and `io::AsyncWrite` traits impl for `net::TcpStream` and `net::UnixStream`. Please use `PollIoAdapter` when these traits are needed for any `AsyncIo` type.
