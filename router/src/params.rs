use core::slice;

use alloc::vec::{self, Vec};

use xitca_unsafe_collection::{bytes::BytesStr, small_str::SmallBoxedStr};

/// A single URL parameter, consisting of a key and a value.
#[derive(Debug, PartialEq, Eq, Clone, Hash)]
struct Param {
    key: BytesStr,
    value: SmallBoxedStr,
}

impl Param {
    fn key_str(&self) -> &str {
        self.key.as_ref()
    }

    fn value_str(&self) -> &str {
        self.value.as_ref()
    }
}

/// A list of parameters returned by a route match.
///
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// # let mut router = xitca_router::Router::new();
/// # router.insert("/users/:id", true)?;
/// let matched = router.at("/users/1")?;
///
/// // you can get a specific value by key
/// let id = matched.params.get("id");
/// assert_eq!(id, Some("1"));
///
/// // or iterate through the keys and values
/// for (key, value) in matched.params.iter() {
///     println!("key: {}, value: {}", key, value);
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Params {
    inner: Vec<Param>,
}

impl Default for Params {
    fn default() -> Self {
        Self::new()
    }
}

impl Params {
    /// Returns the number of parameters.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if there are no parameters in the list.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the value of the first parameter registered under the given key.
    #[inline]
    pub fn get(&self, key: impl AsRef<str>) -> Option<&str> {
        self.inner
            .iter()
            .find(|param| param.key_str() == key.as_ref())
            .map(Param::value_str)
    }

    #[inline]
    pub fn iter(&self) -> Iter<'_> {
        Iter {
            inner: self.inner.iter(),
        }
    }
}

impl Params {
    pub(super) const fn new() -> Self {
        Self { inner: Vec::new() }
    }

    pub(super) fn truncate(&mut self, n: usize) {
        self.inner.truncate(n)
    }

    pub(super) fn push(&mut self, key: BytesStr, value: &str) {
        self.inner.push(Param {
            key,
            value: value.into(),
        });
    }
}

impl IntoIterator for Params {
    type Item = (BytesStr, SmallBoxedStr);
    type IntoIter = IntoIter;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            inner: self.inner.into_iter(),
        }
    }
}

pub struct Iter<'a> {
    inner: slice::Iter<'a, Param>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = (&'a str, &'a str);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|p| (p.key.as_ref(), p.value.as_ref()))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

pub struct IntoIter {
    inner: vec::IntoIter<Param>,
}

impl Iterator for IntoIter {
    type Item = (BytesStr, SmallBoxedStr);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|p| (p.key, p.value))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_alloc() {
        assert!(Params::new().is_empty());
    }

    #[test]
    fn ignore_array_default() {
        let params = Params::new();
        assert!(params.get("").is_none());
    }
}
