//! Copied from `hyper::proto::h1::io`.
//! A write buffer that use vectored buf list.

use std::{fmt, io};

use bytes::{Buf, Bytes, BytesMut};

use crate::util::buf_list::BufList;

/// The default maximum read buffer size. If the buffer gets this big and
/// a message is still not complete, a `TooLarge` error is triggered.
// Note: if this changes, update server::conn::Http::max_buf_size docs.
pub(crate) const DEFAULT_MAX_BUFFER_SIZE: usize = 8192 + 4096 * 100;

/// The maximum number of distinct `Buf`s to hold in a list before requiring
/// a flush. Only affects when the buffer strategy is to queue buffers.
///
/// Note that a flush can happen before reaching the maximum. This simply
/// forces a flush if the queue gets this big.
const MAX_BUF_LIST_BUFFERS: usize = 16;

pub(super) struct ReadBuf {
    advanced: bool,
    buf: BytesMut,
}

impl ReadBuf {
    pub(super) fn new() -> Self {
        Self {
            advanced: false,
            buf: BytesMut::new(),
        }
    }

    #[inline(always)]
    pub(super) fn buf_mut(&mut self) -> &mut BytesMut {
        &mut self.buf
    }

    #[inline(always)]
    pub(super) fn advanced(&self) -> bool {
        self.advanced
    }

    #[inline(always)]
    pub(super) fn advance(&mut self, advanced: bool) {
        self.advanced = advanced;
    }
}

pub(super) enum WriteBuf {
    Flat(BytesMut),
    List(WriteListBuf<EncodedBuf<Bytes>>),
}

impl WriteBuf {
    pub(super) fn new(is_vectored: bool) -> Self {
        if is_vectored {
            Self::List(WriteListBuf::new())
        } else {
            Self::Flat(BytesMut::new())
        }
    }
}

// an internal buffer to collect writes before flushes
pub(super) struct WriteListBuf<B> {
    /// Re-usable buffer that holds message headers
    buf: BytesMut,
    max_buf_size: usize,
    /// Deque of user buffers if strategy is Queue
    queue: BufList<B>,
}

impl<B: Buf> WriteListBuf<B> {
    fn new() -> Self {
        Self {
            buf: BytesMut::new(),
            max_buf_size: DEFAULT_MAX_BUFFER_SIZE,
            queue: BufList::new(),
        }
    }
}

impl<B: Buf> WriteListBuf<B> {
    pub(super) fn buffer<BB: Buf + Into<B>>(&mut self, buf: BB) {
        debug_assert!(buf.has_remaining());
        self.queue.push(buf.into());
    }

    fn can_buffer(&self) -> bool {
        self.queue.bufs_cnt() < MAX_BUF_LIST_BUFFERS && self.queue.remaining() < self.max_buf_size
    }

    #[inline(always)]
    pub(super) fn buf_mut(&mut self) -> &mut BytesMut {
        debug_assert!(!self.queue.has_remaining());
        &mut self.buf
    }

    #[inline(always)]
    pub(super) fn queue_mut(&mut self) -> &mut BufList<B> {
        &mut self.queue
    }
}

impl<B: Buf> fmt::Debug for WriteListBuf<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteBuf")
            .field("remaining", &self.queue.remaining())
            .finish()
    }
}

pub(super) enum EncodedBuf<B> {
    Buf(B),
    Static(&'static [u8]),
}

impl<B: Buf> Buf for EncodedBuf<B> {
    #[inline]
    fn remaining(&self) -> usize {
        match *self {
            Self::Buf(ref buf) => buf.remaining(),
            Self::Static(ref buf) => buf.remaining(),
        }
    }

    #[inline]
    fn chunk(&self) -> &[u8] {
        match *self {
            Self::Buf(ref buf) => buf.chunk(),
            Self::Static(ref buf) => buf.chunk(),
        }
    }

    #[inline]
    fn chunks_vectored<'a>(&'a self, dst: &mut [io::IoSlice<'a>]) -> usize {
        match *self {
            Self::Buf(ref buf) => buf.chunks_vectored(dst),
            Self::Static(ref buf) => buf.chunks_vectored(dst),
        }
    }

    #[inline]
    fn advance(&mut self, cnt: usize) {
        match *self {
            Self::Buf(ref mut buf) => buf.advance(cnt),
            Self::Static(ref mut buf) => buf.advance(cnt),
        }
    }
}
