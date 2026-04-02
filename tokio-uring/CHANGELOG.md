# unreleased 0.2.0
## Add
- add `io::{AsyncBufRead, AsyncBufWrte, write_all}`
- add `io::{AsyncBufRead, AsyncBufWrite}` impl for `net::{TcpStream, UnixStream}`

## Fix
- `BoundedBuf::put_slice` now extend to it's uninit part. Multiple calls to it would result in accumulation of bytes and not overwritting

## Change
- `FixedBuf` would be cleared on check out to buffer pool (By setting its initialized size to zero)
- Cargo feature rework

    `default` -> `buf` and `io` module for universal type and trait 
    
    `bytes` -> enable `buf` trait impl for `bytes` crate
    
    `runtime` -> enable `io` trait impl for `tokio` crate type
    
    `runtime-uring` -> enablle `io_uring` on top of `tokio` runtime

- perf improvement

## Add
- add `BoundedBuf::chunk` to get inited part as byte slice
- add runtime feature for io-uring runtime

# 0.1.1
- fix MSRV

# 0.1.0
- initial release
