//! Collection for uninit slices.

use core::mem::{self, MaybeUninit};

/// A shortcut for create array of uninit type.
#[inline]
pub const fn uninit_array<T, const N: usize>() -> [MaybeUninit<T>; N] {
    // SAFETY: An uninitialized `[MaybeUninit<_>; LEN]` is valid.
    unsafe { MaybeUninit::uninit().assume_init() }
}

mod sealed {
    pub trait Sealed {}
}

impl<T> sealed::Sealed for &mut [MaybeUninit<T>] {}

/// Trait for safely initialize an unit slice.
pub trait PartialInit: sealed::Sealed + Sized {
    /// Uninitialized slice is coming from input slice.
    fn init_from<I>(self, slice: I) -> PartialInitWith<Self, I>
    where
        I: Iterator,
    {
        PartialInitWith {
            uninit: self,
            init: slice,
        }
    }
}

impl<T: Copy> PartialInit for &mut [MaybeUninit<T>] {}

pub struct PartialInitWith<A, B> {
    uninit: A,
    init: B,
}

impl<'a, T, I> PartialInitWith<&'a mut [MaybeUninit<T>], I>
where
    T: Copy,
    I: Iterator,
{
    /// A closure used to construct the initialized type.
    pub fn into_init_with<F>(self, func: F) -> &'a [T]
    where
        F: Fn(I::Item) -> T,
    {
        self.into_init_mut_with(func)
    }

    /// A closure used to construct the initialized type.
    pub fn into_init_mut_with<F>(self, func: F) -> &'a mut [T]
    where
        F: Fn(I::Item) -> T,
    {
        let Self { uninit, init } = self;

        let len = uninit
            .iter_mut()
            .zip(init)
            .map(|(u, i)| {
                let t = func(i);
                u.write(t);
            })
            .count();

        // SAFETY: The total initialized items are counted by iterator.
        unsafe { mem::transmute(&mut uninit[..len]) }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn init_slice() {
        let mut uninit = [MaybeUninit::<usize>::uninit(); 8];

        let nums = [0, 1, 2, 3];

        let slice = uninit.init_from(nums.iter()).into_init_with(|num| num * 2);

        assert_eq!(slice, &[0, 2, 4, 6]);
    }

    #[test]
    fn init_slice2() {
        let mut uninit = [MaybeUninit::<usize>::uninit(); 3];

        let nums = [0, 1, 2, 3];

        let slice = uninit.init_from(nums.iter()).into_init_with(|num| num * 2);

        assert_eq!(slice, &[0, 2, 4]);
    }
}
