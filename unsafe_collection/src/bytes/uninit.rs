use std::{
    io::IoSlice,
    mem::{self, MaybeUninit},
};

use bytes_crate::{buf::Chain, Buf, Bytes};

use super::buf_list::{BufList, EitherBuf};

mod sealed {
    pub trait Sealed {}
}

/// [Buf::chunks_vectored] method but take slice of `MaybeUninit<T>` instead of `T`
pub trait ChunkVectoredUninit: sealed::Sealed {
    /// # Safety:
    ///
    /// MUST write to dst slice continuously start from index 0.
    /// MUST NOT skip to next index without writing initialized item to previous index.
    /// MAY write to certain index multiple times.
    unsafe fn chunks_vectored_uninit<'a>(&'a self, dst: &mut [MaybeUninit<IoSlice<'a>>]) -> usize;
}

impl<T, U> sealed::Sealed for Chain<T, U>
where
    T: sealed::Sealed,
    U: sealed::Sealed,
{
}

impl<T, U> ChunkVectoredUninit for Chain<T, U>
where
    T: ChunkVectoredUninit,
    U: ChunkVectoredUninit,
{
    // SAFETY:
    // T, U must follow the safety rule of chunks_vectored_uninit method.
    unsafe fn chunks_vectored_uninit<'a>(&'a self, dst: &mut [MaybeUninit<IoSlice<'a>>]) -> usize {
        let mut n = self.first_ref().chunks_vectored_uninit(dst);
        n += self.last_ref().chunks_vectored_uninit(&mut dst[n..]);
        n
    }
}

impl<B, const LEN: usize> sealed::Sealed for BufList<B, LEN> where B: ChunkVectoredUninit {}

impl<B: ChunkVectoredUninit, const LEN: usize> ChunkVectoredUninit for BufList<B, LEN> {
    // SAFETY:
    // B must follow the safety rule of chunks_vectored_uninit method.
    #[inline]
    unsafe fn chunks_vectored_uninit<'a>(&'a self, dst: &mut [MaybeUninit<IoSlice<'a>>]) -> usize {
        assert!(!dst.is_empty());
        let mut vecs = 0;
        for buf in self.bufs.iter() {
            vecs += buf.chunks_vectored_uninit(&mut dst[vecs..]);
            if vecs == dst.len() {
                break;
            }
        }

        vecs
    }
}

impl<B: ChunkVectoredUninit, const LEN: usize> BufList<B, LEN> {
    pub fn chunks_vectored_uninit_into_init<'a>(&'a self, dst: &mut [MaybeUninit<IoSlice<'a>>]) -> &mut [IoSlice<'a>] {
        // SAFETY:
        //
        // ChunkVectoredUninit is only implemented for types inside this module.
        // Every implementation MUST make sure to follow the safety rule.
        unsafe {
            let len = self.chunks_vectored_uninit(dst);
            mem::transmute(&mut dst[..len])
        }
    }
}

impl<L, R> sealed::Sealed for EitherBuf<L, R>
where
    L: ChunkVectoredUninit,
    R: ChunkVectoredUninit,
{
}

impl<L, R> ChunkVectoredUninit for EitherBuf<L, R>
where
    L: ChunkVectoredUninit,
    R: ChunkVectoredUninit,
{
    // SAFETY:
    //
    // See ChunkVectoredUninit trait for safety rule.
    unsafe fn chunks_vectored_uninit<'a>(&'a self, dst: &mut [MaybeUninit<IoSlice<'a>>]) -> usize {
        match *self {
            Self::Left(ref buf) => buf.chunks_vectored_uninit(dst),
            Self::Right(ref buf) => buf.chunks_vectored_uninit(dst),
        }
    }
}

impl sealed::Sealed for Bytes {}

impl ChunkVectoredUninit for Bytes {
    // SAFETY:
    //
    // See ChunkVectoredUninit trait for safety rule.
    unsafe fn chunks_vectored_uninit<'a>(&'a self, dst: &mut [MaybeUninit<IoSlice<'a>>]) -> usize {
        if dst.is_empty() {
            return 0;
        }

        if self.has_remaining() {
            dst[0].write(IoSlice::new(self.chunk()));
            1
        } else {
            0
        }
    }
}

impl sealed::Sealed for &'_ [u8] {}

impl ChunkVectoredUninit for &'_ [u8] {
    // SAFETY:
    //
    // See ChunkVectoredUninit trait for safety rule.
    unsafe fn chunks_vectored_uninit<'a>(&'a self, dst: &mut [MaybeUninit<IoSlice<'a>>]) -> usize {
        if dst.is_empty() {
            return 0;
        }

        if self.has_remaining() {
            dst[0].write(IoSlice::new(self));
            1
        } else {
            0
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use crate::uninit::uninit_array;

    #[test]
    fn either_buf() {
        let mut lst = BufList::<_, 2>::new();

        let mut buf = uninit_array::<_, 4>();

        lst.push(EitherBuf::Left(&b"left"[..]));
        lst.push(EitherBuf::Right(&b"right"[..]));

        let slice = lst.chunks_vectored_uninit_into_init(&mut buf);

        assert_eq!(slice.len(), 2);
        assert_eq!(slice[0].as_ref(), b"left");
        assert_eq!(slice[1].as_ref(), b"right");
    }

    #[test]
    fn either_chain() {
        let mut lst = BufList::<_, 3>::new();

        let mut buf = uninit_array::<_, 5>();

        lst.push(EitherBuf::Left((&b"1"[..]).chain(&b"2"[..])));
        lst.push(EitherBuf::Right(&b"3"[..]));

        let slice = lst.chunks_vectored_uninit_into_init(&mut buf);

        assert_eq!(slice.len(), 3);
        assert_eq!(slice[0].as_ref(), b"1");
        assert_eq!(slice[1].as_ref(), b"2");
        assert_eq!(slice[2].as_ref(), b"3");
    }
}
