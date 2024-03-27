# unreleased

# 0.2.1
## Add
- `io::AsyncIoDyn` for object safe version of `io::AsyncIo`.

# 0.2.0
## Add
- `io::PollIoAdapter` as adaptor between `io::AsyncIo` and `io::{AsyncRead, AsyncWrite}` traits.

## Remove
- `io::AsyncRead` and `io::AsyncWrite` traits impl for `net::TcpStream` and `net::UnixStream`. Please use `PollIoAdapter` when these traits are needed for any `AsyncIo` type.
