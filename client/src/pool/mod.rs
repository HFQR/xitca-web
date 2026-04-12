#![allow(dead_code)]

// pool for http/1 connections. connection is uniquely owned and ownership is exchanged between
// pool and caller.
pub(crate) mod exclusive;

// pool for http/2 and http/3 connections. connection is shared owned and ownership is reference
// counted between pool and caller.
pub(crate) mod shared;

// pluggable pool service layer.
pub mod service;

/// readiness probe used by [Pool::acquire] to evict dead cached entries before
/// handing them to a caller. implementations must return `Err` when the
/// connection can no longer open new streams.
pub(crate) trait Ready {
    fn ready(&mut self) -> impl Future<Output = Result<(), ()>> + Send;
}
