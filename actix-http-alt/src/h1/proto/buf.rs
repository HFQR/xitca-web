//! Copied from `hyper::proto::h1::io`.
//! A write buffer that use vectored buf list.

use std::{fmt, io};

use bytes::{Buf, Bytes, BytesMut};

use crate::util::buf_list::BufList;

pub(super) struct ReadBuf<const READ_BUF_LIMIT: usize> {
    advanced: bool,
    buf: BytesMut,
}

impl<const READ_BUF_LIMIT: usize> ReadBuf<READ_BUF_LIMIT> {
    pub(super) fn new() -> Self {
        Self {
            advanced: false,
            buf: BytesMut::new(),
        }
    }

    #[inline(always)]
    pub(super) fn len(&self) -> usize {
        self.buf.len()
    }

    #[inline(always)]
    pub(super) fn backpressure(&self) -> bool {
        self.buf.len() >= READ_BUF_LIMIT
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

pub(super) enum WriteBuf<const WRITE_BUF_LIMIT: usize> {
    Flat(BytesMut),
    List(WriteListBuf<EncodedBuf<Bytes>>),
}

impl<const WRITE_BUF_LIMIT: usize> WriteBuf<WRITE_BUF_LIMIT> {
    pub(super) fn new(is_vectored: bool) -> Self {
        if is_vectored {
            Self::List(WriteListBuf::new())
        } else {
            Self::Flat(BytesMut::new())
        }
    }

    #[inline(always)]
    pub(super) fn backpressure(&self) -> bool {
        match *self {
            Self::Flat(ref buf) => buf.len() >= WRITE_BUF_LIMIT,
            Self::List(ref list) => {
                // When buffering buf must be empty.
                // (Whoever write into it must split it afterwards)
                debug_assert!(!list.buf.has_remaining());
                // TODO: figure out a suitable backpressure point for list length.
                list.list.remaining() >= WRITE_BUF_LIMIT
            }
        }
    }

    #[inline(always)]
    pub(super) fn empty(&self) -> bool {
        match *self {
            Self::Flat(ref buf) => buf.is_empty(),
            Self::List(ref list) => {
                // When buffering buf must be empty.
                // (Whoever write into it must split it afterwards)
                debug_assert!(!list.buf.has_remaining());
                list.list.remaining() == 0
            }
        }
    }
}

// an internal buffer to collect writes before flushes
pub(super) struct WriteListBuf<B> {
    /// Re-usable buffer that holds response head.
    /// After head writing finished it's split and pushed to list.
    buf: BytesMut,
    /// Deque of user buffers if strategy is Queue
    list: BufList<B>,
}

impl<B: Buf> WriteListBuf<B> {
    fn new() -> Self {
        Self {
            buf: BytesMut::new(),
            list: BufList::new(),
        }
    }
}

impl<B: Buf> WriteListBuf<B> {
    pub(super) fn buffer<BB: Buf + Into<B>>(&mut self, buf: BB) {
        debug_assert!(buf.has_remaining());
        self.list.push(buf.into());
    }

    #[inline(always)]
    pub(super) fn buf_mut(&mut self) -> &mut BytesMut {
        &mut self.buf
    }

    #[inline(always)]
    pub(super) fn list_mut(&mut self) -> &mut BufList<B> {
        &mut self.list
    }
}

impl<B: Buf> fmt::Debug for WriteListBuf<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteBuf")
            .field("remaining", &self.list.remaining())
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
