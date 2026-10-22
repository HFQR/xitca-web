# unreleased 0.3.0
## Change
- crate is `no_std` by default and depends on `core` only. Items depending on `alloc` are gated behind new `alloc` feature:
  - `small_str`
- items depending on `std` are gated behind new `std` feature (implies `alloc`):
  - `fake::FakeSend`
  - `futures::CatchUnwind`
- `bytes` feature implies `std`
- bump to Rust 2024 edition and MSRV to `1.85`

## Remove
- remove unused `bound_queue` module
- remove unused `bytes::{BufList, ChunkVectoredUninit, EitherBuf}`
- remove unused `futures::ReusableLocalBoxFuture`
- remove unused `fake::FakeClone`

# 0.2.0
## Add
- add `futures::CatchUnwind`

## Change
- bump MSRV to `1.79`

## Remove
- reduce unsafe code block by removing `uninit::uninit_array`
