//! A synchronous IO bridge for adapting blocking TLS libraries (OpenSSL, native-tls)
//! to completion-based async IO traits.
//!
//! The bridge implements `std::io::Read + Write` using in-memory buffers.
//! TLS libraries read/write to the bridge synchronously, then the caller
//! drains/fills the buffers asynchronously via `AsyncBufRead`/`AsyncBufWrite`.

use std::io;

use xitca_io::{
    bytes::{Buf, BytesMut},
    io::{AsyncBufRead, AsyncBufWrite},
};

/// A synchronous bridge that pairs in-memory buffers with an async IO handle.
///
/// TLS libraries see `Read + Write` backed by `read_buf` and `write_buf`.
/// The caller uses [`fill_read_buf`] and [`drain_write_buf`] to move data
/// between the buffers and the underlying async IO.
pub(crate) struct SyncBridge {
    /// Ciphertext read from the network, consumed by TLS `read`.
    pub read_buf: BytesMut,
    /// Ciphertext produced by TLS `write`, drained to the network.
    pub write_buf: BytesMut,
}

impl SyncBridge {
    pub fn new() -> Self {
        Self {
            read_buf: BytesMut::new(),
            write_buf: BytesMut::new(),
        }
    }
}

impl io::Read for SyncBridge {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.read_buf.is_empty() {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let len = buf.len().min(self.read_buf.len());
        buf[..len].copy_from_slice(&self.read_buf[..len]);
        self.read_buf.advance(len);
        Ok(len)
    }
}

impl io::Write for SyncBridge {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write_buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Read ciphertext from the network into `bridge.read_buf`.
pub(crate) async fn fill_read_buf(io: &impl AsyncBufRead, bridge: &mut SyncBridge) -> io::Result<()> {
    let len = bridge.read_buf.len();
    bridge.read_buf.reserve(4096);

    let (res, b) = io.read(bridge.read_buf.split_off(len)).await;
    let returned = b;

    match res {
        Ok(0) => {
            bridge.read_buf.unsplit(returned);
            Err(io::ErrorKind::UnexpectedEof.into())
        }
        Ok(_) => {
            bridge.read_buf.unsplit(returned);
            Ok(())
        }
        Err(e) => {
            bridge.read_buf.unsplit(returned);
            Err(e)
        }
    }
}

/// Drain all ciphertext from `bridge.write_buf` to the network.
pub(crate) async fn drain_write_buf(io: &impl AsyncBufWrite, bridge: &mut SyncBridge) -> io::Result<()> {
    if bridge.write_buf.is_empty() {
        return Ok(());
    }
    let buf = bridge.write_buf.split();
    let (res, b) = xitca_io::io::write_all(io, buf).await;
    drop(b);
    res
}

/// Drain a pre-split write buffer to the network.
/// Used when the caller has already split the write_buf out of the bridge
/// (e.g. to drop a RefCell borrow before awaiting).
pub(crate) async fn drain_split(io: &impl AsyncBufWrite, buf: BytesMut) -> io::Result<()> {
    if buf.is_empty() {
        return Ok(());
    }
    let (res, _) = xitca_io::io::write_all(io, buf).await;
    res
}

/// Split off a read buffer from the bridge for async filling.
/// Reserves space and returns the tail portion for IO.
/// After the read, unsplit the returned buffer back into `bridge.read_buf`.
pub(crate) fn take_read_buf(bridge: &mut SyncBridge) -> BytesMut {
    let len = bridge.read_buf.len();
    bridge.read_buf.reserve(4096);
    bridge.read_buf.split_off(len)
}

/// Fill a pre-split read buffer from the network.
/// Always returns the buffer (even on error) so the caller can unsplit it back.
pub(crate) async fn fill_split(io: &impl AsyncBufRead, buf: BytesMut) -> (io::Result<()>, BytesMut) {
    let (res, buf) = io.read(buf).await;
    match res {
        Ok(0) => (Err(io::ErrorKind::UnexpectedEof.into()), buf),
        Ok(_) => (Ok(()), buf),
        Err(e) => (Err(e), buf),
    }
}
