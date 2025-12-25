# unreleased

# 0.5.0
## Fix
- relax trait bound of `io_uring::write_all`

## Change
- switch to `tokio-uring-xitca` for io_uring feature
- update `tokio` to `1.48`

# 0.4.1
## Fix
- fix `WriteBuf` not properly removing Io flushing state

# 0.4.0
## Change
- bump MSRV to `1.79`
- update `xitca-unsafe-collection` to `0.2.0`
- update `tokio-uring` to `0.5.0`

# 0.3.0
## Change
- rename `http3` feature to `quic`
- rename `H3ServerConfig` to `QuicConfig`
- rename `Udp` prefixed types to `Quic` prefixed
- `AsyncIo::ready` and `AsyncIoDyn::ready` receive `&mut self`. Allowing more arbitrary types implement these traits.
- `AsyncIo::poll_ready` and `AsyncIoDyn::poll_ready` receive `&mut self`. For the same reason as above change.
- update `quinn` to `0.11`

# 0.2.1
## Add
- `io::AsyncIoDyn` for object safe version of `io::AsyncIo`.

# 0.2.0
## Add
- `io::PollIoAdapter` as adaptor between `io::AsyncIo` and `io::{AsyncRead, AsyncWrite}` traits.

## Remove
- `io::AsyncRead` and `io::AsyncWrite` traits impl for `net::TcpStream` and `net::UnixStream`. Please use `PollIoAdapter` when these traits are needed for any `AsyncIo` type.
