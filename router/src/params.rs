use core::{
    borrow::Borrow,
    fmt,
    hash::{Hash, Hasher},
    ops::Deref,
    slice,
};

use crate::{SmallStr, String, Vec};

/// A single URL parameter, consisting of a key and a value.
#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub struct Param {
    key: SmallStr,
    value: SmallStr,
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
/// # router.insert("/users/{id}", true)?;
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

    pub(super) fn push_val(&mut self, value: &str) {
        self.inner.push(Param {
            key: const { SmallStr::new() },
            value: value.into(),
        });
    }

    pub(super) fn push(&mut self, key: &str, value: &str) {
        self.inner.push(Param {
            key: key.into(),
            value: value.into(),
        });
    }

    // Set each param key from the provided slice, zipping with the stored params.
    // Using zip avoids a bounds check on keys[i] that indexed access would require.
    pub(crate) fn apply_keys(&mut self, keys: &[SmallStr]) {
        self.inner
            .iter_mut()
            .zip(keys.iter())
            .for_each(|(param, key)| param.key = key.clone());
    }
}

impl IntoIterator for Params {
    type Item = (ParamStr, ParamStr);
    type IntoIter = IntoIter<<Vec<Param> as IntoIterator>::IntoIter>;

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

pub struct IntoIter<I> {
    inner: I,
}

impl<I> Iterator for IntoIter<I>
where
    I: Iterator<Item = Param>,
{
    type Item = (ParamStr, ParamStr);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|p| (ParamStr(p.key), ParamStr(p.value)))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

/// Owned key or value of a parameter, yielded by [`Params::into_iter`].
///
/// Dereferences to `str`. Short strings are stored inline without heap allocation.
#[derive(Clone, PartialEq, Eq)]
pub struct ParamStr(SmallStr);

impl ParamStr {
    #[inline]
    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

impl Deref for ParamStr {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for ParamStr {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for ParamStr {
    #[inline]
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

// hash as str so the Borrow<str> impl holds for hash based collections.
impl Hash for ParamStr {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state)
    }
}

impl PartialEq<str> for ParamStr {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for ParamStr {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl fmt::Debug for ParamStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for ParamStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<ParamStr> for String {
    #[inline]
    fn from(value: ParamStr) -> Self {
        String::from(value.as_str())
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

    #[test]
    fn into_iter_param_str() {
        let mut params = Params::new();
        params.push("id", "1");
        params.push("name", "a_value_longer_than_inline_capacity");

        let mut iter = params.into_iter();

        let (key, value) = iter.next().unwrap();
        assert_eq!(key, "id");
        assert_eq!(value, "1");

        let (key, value) = iter.next().unwrap();
        assert_eq!(key, "name");
        assert_eq!(String::from(value), "a_value_longer_than_inline_capacity");

        assert!(iter.next().is_none());
    }

    #[test]
    fn param_str_hash_as_str() {
        use core::hash::BuildHasher;

        let hasher = std::hash::RandomState::new();
        let s = ParamStr(SmallStr::from("foo"));
        assert_eq!(hasher.hash_one(&s), hasher.hash_one("foo"));
    }
}
