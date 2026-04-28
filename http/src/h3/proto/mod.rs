mod coding;
mod dispatcher;
mod dispatcher_v2;
mod headers;
mod push;
mod qpack;
mod stream;
mod varint;

pub(super) mod frame;

pub(super) use headers::Header;
pub(super) use qpack::decode_stateless;

/// Upper bound on a single QPACK-encoded header (or trailer) block we're
/// willing to decode. Prevents an unbounded allocator hit from a malicious
/// peer. Should be aligned with our advertised MAX_HEADER_LIST_SIZE setting.
pub(super) const MAX_HEADER_BLOCK_BYTES: u64 = 64 * 1024;

pub(crate) use dispatcher::Dispatcher;
pub(crate) use dispatcher_v2::Dispatcher as DispatcherV2;
