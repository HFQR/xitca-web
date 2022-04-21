use std::io::IoSlice;

use bytes_crate::{Buf, BufMut, Bytes, BytesMut};

use crate::array_queue::ArrayQueue;

/// A bounded stack buffer array that can hold up to LEN size items.
/// BufList implement [Buf] trait when it's item is a type implement the same trait.
pub struct BufList<B, const LEN: usize = 8> {
    pub(super) bufs: ArrayQueue<B, LEN>,
    remaining: usize,
}

impl<B: Buf> Default for BufList<B> {
    fn default() -> Self {
        Self::new()
    }
}

impl<B: Buf, const LEN: usize> BufList<B, LEN> {
    #[inline]
    pub const fn new() -> Self {
        Self {
            bufs: ArrayQueue::new(),
            remaining: 0,
        }
    }

    /// # Panic:
    ///
    /// push new item when the list is already full.
    #[inline]
    pub fn push(&mut self, buf: B) {
        debug_assert!(buf.has_remaining());
        self.remaining += buf.remaining();
        self.bufs.push_back(buf).expect("BufList overflown");
    }

    #[inline]
    pub const fn is_full(&self) -> bool {
        self.bufs.is_full()
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.bufs.is_empty()
    }
}

impl<B: Buf, const LEN: usize> Buf for BufList<B, LEN> {
    #[inline]
    fn remaining(&self) -> usize {
        self.remaining
    }

    #[inline]
    fn chunk(&self) -> &[u8] {
        self.bufs.front().map(Buf::chunk).unwrap_or_default()
    }

    #[inline]
    fn chunks_vectored<'a>(&'a self, dst: &mut [IoSlice<'a>]) -> usize {
        assert!(!dst.is_empty());
        let mut vecs = 0;
        for buf in self.bufs.iter() {
            vecs += buf.chunks_vectored(&mut dst[vecs..]);
            if vecs == dst.len() {
                break;
            }
        }
        vecs
    }

    #[inline]
    fn advance(&mut self, mut cnt: usize) {
        debug_assert!(self.remaining >= cnt);
        self.remaining -= cnt;
        while cnt > 0 {
            {
                let front = self
                    .bufs
                    .front_mut()
                    .expect("BufList::advance must be called after BufList::chunks_vectored returns non zero length.");
                let rem = front.remaining();
                if rem > cnt {
                    front.advance(cnt);
                    return;
                } else {
                    front.advance(rem);
                    cnt -= rem;
                }
            }
            self.bufs.pop_front();
        }
    }

    #[inline]
    fn copy_to_bytes(&mut self, len: usize) -> Bytes {
        // Our inner buffer may have an optimized version of copy_to_bytes, and if the whole
        // request can be fulfilled by the front buffer, we can take advantage.
        match self.bufs.front_mut() {
            Some(front) if front.remaining() == len => {
                let b = front.copy_to_bytes(len);
                self.remaining -= len;
                self.bufs.pop_front();
                b
            }
            Some(front) if front.remaining() > len => {
                self.remaining -= len;
                front.copy_to_bytes(len)
            }
            _ => {
                assert!(len <= self.remaining(), "`len` greater than remaining");
                let mut bm = BytesMut::with_capacity(len);
                bm.put(self.take(len));
                bm.freeze()
            }
        }
    }
}

/// An enum implement [Buf] trait when both arms are types implement the same trait.
///
/// *. Enum would implement [super::uninit::ChunkVectoredUninit] trait when both arms are types
/// implement the same trait.
pub enum EitherBuf<L, R> {
    Left(L),
    Right(R),
}

impl<L, R> Buf for EitherBuf<L, R>
where
    L: Buf,
    R: Buf,
{
    #[inline]
    fn remaining(&self) -> usize {
        match *self {
            Self::Left(ref buf) => buf.remaining(),
            Self::Right(ref buf) => buf.remaining(),
        }
    }

    #[inline]
    fn chunk(&self) -> &[u8] {
        match *self {
            Self::Left(ref buf) => buf.chunk(),
            Self::Right(ref buf) => buf.chunk(),
        }
    }

    #[inline]
    fn chunks_vectored<'a>(&'a self, dst: &mut [IoSlice<'a>]) -> usize {
        match *self {
            Self::Left(ref buf) => buf.chunks_vectored(dst),
            Self::Right(ref buf) => buf.chunks_vectored(dst),
        }
    }

    #[inline]
    fn advance(&mut self, cnt: usize) {
        match *self {
            Self::Left(ref mut buf) => buf.advance(cnt),
            Self::Right(ref mut buf) => buf.advance(cnt),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use super::*;

    fn hello_world_buf() -> BufList<Bytes> {
        let mut lst = BufList::default();

        lst.push(Bytes::from("Hello"));
        lst.push(Bytes::from(" "));
        lst.push(Bytes::from("World"));

        lst
    }

    #[test]
    fn to_bytes_shorter() {
        let mut bufs = hello_world_buf();
        let old_ptr = bufs.chunk().as_ptr();
        let start = bufs.copy_to_bytes(4);
        assert_eq!(start, "Hell");
        assert!(ptr::eq(old_ptr, start.as_ptr()));
        assert_eq!(bufs.chunk(), b"o");
        assert!(ptr::eq(old_ptr.wrapping_add(4), bufs.chunk().as_ptr()));
        assert_eq!(bufs.remaining(), 7);
    }

    #[test]
    fn to_bytes_eq() {
        let mut bufs = hello_world_buf();
        let old_ptr = bufs.chunk().as_ptr();
        let start = bufs.copy_to_bytes(5);
        assert_eq!(start, "Hello");
        assert!(ptr::eq(old_ptr, start.as_ptr()));
        assert_eq!(bufs.chunk(), b" ");
        assert_eq!(bufs.remaining(), 6);
    }

    #[test]
    fn to_bytes_longer() {
        let mut bufs = hello_world_buf();
        let start = bufs.copy_to_bytes(7);
        assert_eq!(start, "Hello W");
        assert_eq!(bufs.remaining(), 4);
    }

    #[test]
    fn one_long_buf_to_bytes() {
        let mut buf = BufList::default();
        buf.push(b"Hello World" as &[_]);
        assert_eq!(buf.copy_to_bytes(5), "Hello");
        assert_eq!(buf.chunk(), b" World");
    }

    #[test]
    #[should_panic(expected = "`len` greater than remaining")]
    fn buf_to_bytes_too_many() {
        hello_world_buf().copy_to_bytes(42);
    }
}
