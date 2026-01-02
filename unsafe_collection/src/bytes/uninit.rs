use std::{io::IoSlice, mem::MaybeUninit};

use bytes_crate::{Buf, Bytes, buf::Chain};

use crate::uninit;

use super::buf_list::{BufList, EitherBuf};

mod sealed {
    pub trait Sealed {}
}

/// [Buf::chunks_vectored] method but take slice of `MaybeUninit<T>` instead of `T`
pub trait ChunkVectoredUninit: sealed::Sealed {
    /// # Safety
    ///
    /// MUST write to dst slice continuously start from 0 index.
    /// MUST NOT skip to N + 1 index without writing initialized item to N index.
    /// MAY write to certain index multiple times.
    /// MUST return the total amount of unique items(multiple write to one index count as one)
    /// that are initialized.
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
        unsafe {
            let mut n = self.first_ref().chunks_vectored_uninit(dst);
            n += self.last_ref().chunks_vectored_uninit(&mut dst[n..]);
            n
        }
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
            vecs += unsafe { buf.chunks_vectored_uninit(&mut dst[vecs..]) };
            if vecs == dst.len() {
                break;
            }
        }

        vecs
    }
}

impl<B: ChunkVectoredUninit, const LEN: usize> BufList<B, LEN> {
    /// Write the item inside BufList in front-end order to given uninitialized slice and return
    /// the initialized part as a new slice.
    ///
    /// # Example:
    /// ```rust
    /// let mut lst = xitca_unsafe_collection::bytes::BufList::<&[u8], 6>::new();
    /// lst.push(&b"996"[..]);
    /// lst.push(&b"007"[..]);
    /// let mut dst = [const { core::mem::MaybeUninit::uninit() }; 8];
    /// let init_slice = lst.chunks_vectored_uninit_into_init(&mut dst);
    /// assert_eq!(init_slice.len(), 2);
    /// assert_eq!(&*init_slice[0], &b"996"[..]);
    /// assert_eq!(&*init_slice[1], &b"007"[..]);
    /// ```
    pub fn chunks_vectored_uninit_into_init<'a, 's>(
        &'a self,
        dst: &'s mut [MaybeUninit<IoSlice<'a>>],
    ) -> &'s mut [IoSlice<'a>] {
        // SAFETY:
        //
        // ChunkVectoredUninit is only implemented for types inside this module.
        // Every implementation MUST make sure to follow the safety rule.
        unsafe {
            let len = self.chunks_vectored_uninit(dst);
            uninit::slice_assume_init_mut(&mut dst[..len])
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
        unsafe {
            match *self {
                Self::Left(ref buf) => buf.chunks_vectored_uninit(dst),
                Self::Right(ref buf) => buf.chunks_vectored_uninit(dst),
            }
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

    #[test]
    fn either_buf() {
        let mut lst = BufList::<_, 2>::new();

        let mut buf = [const { MaybeUninit::uninit() }; 4];

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

        let mut buf = [const { MaybeUninit::uninit() }; 5];

        lst.push(EitherBuf::Left((&b"1"[..]).chain(&b"2"[..])));
        lst.push(EitherBuf::Right(&b"3"[..]));

        let slice = lst.chunks_vectored_uninit_into_init(&mut buf);

        assert_eq!(slice.len(), 3);
        assert_eq!(slice[0].as_ref(), b"1");
        assert_eq!(slice[1].as_ref(), b"2");
        assert_eq!(slice[2].as_ref(), b"3");
    }
}
