use core::{
    fmt,
    ops::{Deref, Range},
};

use crate::{Box, String, Vec};

/// An unescaped route that keeps track of the position of
/// escaped characters, i.e. '{{' or '}}'.
///
/// Note that this type dereferences to `&[u8]`.
#[derive(Clone, Default)]
pub struct UnescapedRoute {
    // Both fields use boxed slices instead of `Vec` or `String` to omit the
    // unused capacity word, shrinking the struct from 48 → 32 bytes. The
    // mutation methods (`splice`, `append`, `truncate`) reallocate on each
    // call, but those are only invoked during `Router::insert`, which is
    // typically a one-time startup cost.
    inner: Box<str>,
    escaped: Box<[usize]>,
}

impl UnescapedRoute {
    /// Unescapes escaped brackets ('{{' or '}}') in a route.
    pub(crate) fn new(route: String) -> UnescapedRoute {
        let mut inner = route.into_bytes();
        let mut escaped = Vec::new();
        let mut i = 0;

        while let Some(&c) = inner.get(i) {
            if (c == b'{' && inner.get(i + 1) == Some(&b'{')) || (c == b'}' && inner.get(i + 1) == Some(&b'}')) {
                inner.remove(i);
                escaped.push(i);
            }

            i += 1;
        }

        // Routes are always valid UTF-8 (ASCII); the only modifications above
        // were removing duplicate '{' or '}' bytes, preserving UTF-8 validity.
        let inner = String::from_utf8(inner).unwrap().into_boxed_str();
        UnescapedRoute {
            inner,
            escaped: escaped.into_boxed_slice(),
        }
    }

    // Returns true if the character at the given index was escaped.
    #[inline]
    pub(crate) fn is_escaped(&self, i: usize) -> bool {
        self.escaped.contains(&i)
    }

    // Replaces the characters in the given range and returns the removed portion.
    pub(crate) fn splice(&mut self, range: Range<usize>, replace: &str) -> String {
        let offset = (replace.len() as isize) - (range.len() as isize);
        let new_escaped: Vec<usize> = self
            .escaped
            .iter()
            .filter(|&&x| !range.contains(&x))
            .map(|&i| {
                if i > range.end {
                    i.checked_add_signed(offset).unwrap()
                } else {
                    i
                }
            })
            .collect();
        self.escaped = new_escaped.into_boxed_slice();

        let removed = String::from(&self.inner[range.clone()]);
        let mut new_inner = String::with_capacity(self.inner.len() - range.len() + replace.len());
        new_inner.push_str(&self.inner[..range.start]);
        new_inner.push_str(replace);
        new_inner.push_str(&self.inner[range.end..]);
        self.inner = new_inner.into_boxed_str();
        removed
    }

    // Appends another route to the end of this one.
    pub(crate) fn append(&mut self, other: &UnescapedRoute) {
        let self_len = self.inner.len();
        let new_escaped: Vec<usize> = self
            .escaped
            .iter()
            .copied()
            .chain(other.escaped.iter().map(|&i| self_len + i))
            .collect();
        self.escaped = new_escaped.into_boxed_slice();

        let mut new_inner = String::with_capacity(self_len + other.inner.len());
        new_inner.push_str(&self.inner);
        new_inner.push_str(&other.inner);
        self.inner = new_inner.into_boxed_str();
    }

    // Truncates the route to the given length.
    pub(crate) fn truncate(&mut self, to: usize) {
        let new_escaped: Vec<usize> = self.escaped.iter().copied().filter(|&x| x < to).collect();
        self.escaped = new_escaped.into_boxed_slice();
        self.inner = Box::from(&self.inner[..to]);
    }

    #[inline]
    pub(crate) fn as_ref(&self) -> UnescapedRef<'_> {
        UnescapedRef {
            inner: self.inner.as_bytes(),
            escaped: &self.escaped,
            offset: 0,
        }
    }

    #[inline]
    pub(crate) fn unescaped(&self) -> &str {
        &self.inner
    }

    #[inline]
    pub(crate) fn into_unescaped(self) -> String {
        String::from(self.inner)
    }
}

impl Deref for UnescapedRoute {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.inner.as_bytes()
    }
}

impl fmt::Debug for UnescapedRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.inner, f)
    }
}

/// A reference to an `UnescapedRoute`.
#[derive(Copy, Clone)]
pub struct UnescapedRef<'a> {
    inner: &'a [u8],
    escaped: &'a [usize],
    // An offset applied to each escaped index.
    offset: isize,
}

impl<'a> UnescapedRef<'a> {
    // Converts this reference into an owned route.
    pub(crate) fn to_owned(self) -> UnescapedRoute {
        let escaped = self
            .escaped
            .iter()
            .filter_map(|i| i.checked_add_signed(self.offset))
            .filter(|i| *i < self.inner.len())
            .collect();

        UnescapedRoute {
            escaped,
            inner: Box::from(core::str::from_utf8(self.inner).unwrap()),
        }
    }

    // Returns `true` if the character at the given index was escaped.
    pub(crate) fn is_escaped(&self, i: usize) -> bool {
        i.checked_add_signed(-self.offset)
            .is_some_and(|i| self.escaped.contains(&i))
    }

    // Slices the route with `start..`.
    pub(crate) fn slice_off(&mut self, start: usize) -> UnescapedRef<'a> {
        UnescapedRef {
            inner: &self.inner[start..],
            escaped: self.escaped,
            offset: self.offset - (start as isize),
        }
    }

    // Slices the route with `..end`.
    pub(crate) fn slice_until(&self, end: usize) -> UnescapedRef<'a> {
        UnescapedRef {
            inner: &self.inner[..end],
            escaped: self.escaped,
            offset: self.offset,
        }
    }
}

impl Deref for UnescapedRef<'_> {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.inner
    }
}

impl fmt::Debug for UnescapedRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnescapedRef")
            .field("inner", &core::str::from_utf8(self.inner).unwrap())
            .field("escaped", &self.escaped)
            .field("offset", &self.offset)
            .finish()
    }
}
