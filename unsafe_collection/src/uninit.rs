//! Collection for uninit slices.

use core::mem::MaybeUninit;

/// A shortcut for create array of uninit type.
#[inline(always)]
pub const fn uninit_array<T, const N: usize>() -> [MaybeUninit<T>; N] {
    // SAFETY: An uninitialized `[MaybeUninit<_>; LEN]` is valid.
    unsafe { MaybeUninit::uninit().assume_init() }
}

// SAFETY:
//
// It is up to the caller to guarantee that the `MaybeUninit<T>` elements
// really are in an initialized state.
// Calling this when the content is not yet fully initialized causes undefined behavior.
pub(crate) unsafe fn slice_assume_init_mut<T>(slice: &mut [MaybeUninit<T>]) -> &mut [T] {
    &mut *(slice as *mut [MaybeUninit<T>] as *mut [T])
}

// SAFETY:
//
// The same rule as slice_assume_init_mut function
pub(crate) unsafe fn slice_assume_init<T>(slice: &[MaybeUninit<T>]) -> &[T] {
    &*(slice as *const [MaybeUninit<T>] as *const [T])
}

mod sealed {
    pub trait Sealed {}
}

impl<T> sealed::Sealed for &mut [MaybeUninit<T>] {}

/// Trait for safely initialize an un initialized slice.
pub trait PartialInit: sealed::Sealed + Sized {
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

// T must be Copy so the initializer don't worry about dropping potential already initialized value.
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
    /// A closure used to construct the initialized slice reference.
    #[inline]
    pub fn into_init_with<F>(self, func: F) -> &'a [T]
    where
        F: Fn(I::Item) -> T,
    {
        self.into_init_mut_with(func)
    }

    /// A closure used to construct the initialized slice mutable reference.
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
        unsafe { slice_assume_init_mut(&mut uninit[..len]) }
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
