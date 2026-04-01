# unreleased 0.2.0
## Fix
- `BoundedBuf::put_slice` now extend to it's uninit part. Multiple calls to it would result in accumulation of bytes and not overwritting

## Change
- `FixedBuf` would be cleared on check out to buffer pool (By setting is initialized count to zero)
- remove runtime from default feature
- perf improvement

## Add
- add `BoundedBuf::chunk` to get inited part as byte slice
- add runtime feature for io-uring runtime

# 0.1.1
- fix MSRV

# 0.1.0
- initial release
