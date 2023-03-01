use core::{
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
};

use bytes_crate::{
    buf::{Buf, BufMut, UninitSlice},
    Bytes, BytesMut,
};

/// const generic guarded BytesMut with page size in byte unit at compile time.
/// when internal memory reserve happen it always aim for the page size.
#[derive(Debug)]
pub struct PagedBytesMut<const PAGE_SIZE: usize>(BytesMut);

impl<const PAGE_SIZE: usize> PagedBytesMut<PAGE_SIZE> {
    pub fn new() -> Self {
        Self(BytesMut::new())
    }

    /// same as [BytesMut::split_to]
    #[inline]
    pub fn split_to(&mut self, at: usize) -> BytesMut {
        self.0.split_to(at)
    }

    /// same as [BytesMut::split_off]
    #[inline]
    pub fn split_off(&mut self, at: usize) -> BytesMut {
        self.0.split_off(at)
    }

    /// same as [BytesMut::split]
    #[inline]
    pub fn split(&mut self) -> BytesMut {
        self.0.split()
    }

    #[doc(hidden)]
    #[inline]
    pub fn get_mut(&mut self) -> &mut BytesMut {
        &mut self.0
    }

    /// take ownership of inner bytes mut.
    #[inline]
    pub fn into_inner(self) -> BytesMut {
        self.0
    }
}

impl<T, const PAGE_SIZE: usize> From<T> for PagedBytesMut<PAGE_SIZE>
where
    BytesMut: From<T>,
{
    fn from(t: T) -> Self {
        Self(BytesMut::from(t))
    }
}

impl<const PAGE_SIZE: usize> Default for PagedBytesMut<PAGE_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const PAGE_SIZE: usize> Buf for PagedBytesMut<PAGE_SIZE> {
    #[inline]
    fn remaining(&self) -> usize {
        self.0.remaining()
    }

    #[inline]
    fn chunk(&self) -> &[u8] {
        self.0.chunk()
    }

    #[inline]
    fn advance(&mut self, cnt: usize) {
        self.0.advance(cnt)
    }

    #[inline]
    fn copy_to_bytes(&mut self, len: usize) -> Bytes {
        self.0.copy_to_bytes(len)
    }
}

// a mirrored BufMut impl from BytesMut.
unsafe impl<const PAGE_SIZE: usize> BufMut for PagedBytesMut<PAGE_SIZE> {
    #[inline]
    fn remaining_mut(&self) -> usize {
        self.0.remaining_mut()
    }

    // SAFETY:
    // forward to BytesMut.
    #[inline]
    unsafe fn advance_mut(&mut self, cnt: usize) {
        self.0.advance_mut(cnt)
    }

    #[inline]
    fn chunk_mut(&mut self) -> &mut UninitSlice {
        if self.0.capacity() == self.0.len() {
            self.0.reserve(PAGE_SIZE);
        }
        // SAFETY:
        // copy paste from <BytesMut as BufMut>::chunk_mut method impl.
        unsafe { &mut *(self.0.spare_capacity_mut() as *mut [MaybeUninit<u8>] as *mut UninitSlice) }
    }

    #[inline]
    fn put<T: Buf>(&mut self, src: T)
    where
        Self: Sized,
    {
        self.0.put(src)
    }

    #[inline]
    fn put_slice(&mut self, src: &[u8]) {
        self.0.put_slice(src)
    }

    #[inline]
    fn put_bytes(&mut self, val: u8, cnt: usize) {
        self.0.put_bytes(val, cnt)
    }
}

impl<const PAGE_SIZE: usize> AsRef<[u8]> for PagedBytesMut<PAGE_SIZE> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl<const PAGE_SIZE: usize> AsMut<[u8]> for PagedBytesMut<PAGE_SIZE> {
    #[inline]
    fn as_mut(&mut self) -> &mut [u8] {
        self.0.as_mut()
    }
}

impl<const PAGE_SIZE: usize> Deref for PagedBytesMut<PAGE_SIZE> {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        self.0.deref()
    }
}

impl<const PAGE_SIZE: usize> DerefMut for PagedBytesMut<PAGE_SIZE> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [u8] {
        self.0.deref_mut()
    }
}

#[cfg(test)]
mod test {
    use std::io;

    use crate::bytes::read_buf;

    use super::*;

    #[test]
    fn limit_buf() {
        const PAGE: usize = 4096;

        let mut buf = PagedBytesMut::<PAGE>::new();

        let input = b"hello,world!";
        let n = read_buf(&mut io::Cursor::new(input), &mut buf).unwrap();

        assert_eq!(n, input.len());
        assert_eq!(buf.chunk(), input);
        assert_eq!(buf.chunk_mut().len(), PAGE - input.len());

        let mut io = io::Cursor::new(vec![0; 4096 * 2]);

        let n = read_buf(&mut io, &mut buf).unwrap();
        assert_eq!(n, PAGE - input.len());
        assert_eq!(buf.chunk().len(), PAGE);

        let n = read_buf(&mut io, &mut buf).unwrap();
        assert_eq!(n, PAGE);
        assert_eq!(buf.chunk().len(), PAGE * 2);
    }
}
