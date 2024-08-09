# unreleased 0.2.1
## Add
- add `bytes::ChunkVectoredUninit` impl for `BytesMut`
- add `bytes::BufList::clear` method
- add `Send`,`Sync` auto bound for `bytes::BufList`
- add `From` impl for converting `bytes::BufList` to `Bytes` and `BytesMut`

# 0.2.0
## Add
- add `futures::CatchUnwind`

## Change
- bump MSRV to `1.79`

## Remove
- reduce unsafe code block by removing `uninit::uninit_array`
