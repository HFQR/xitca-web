use alloc::vec::{self, Vec};

use xitca_unsafe_collection::{bound_queue::stack::StackQueue, bytes::BytesStr};

/// A single URL parameter, consisting of a key and a value.
#[derive(Debug, PartialEq, Eq, Clone, Hash)]
struct Param {
    key: BytesStr,
    value: BytesStr,
}

impl Param {
    fn key_str(&self) -> &str {
        self.key.as_str()
    }

    fn value_str(&self) -> &str {
        self.value.as_str()
    }
}

type Inline = StackQueue<Param, 2>;

#[derive(Debug)]
pub struct Params {
    kind: ParamsKind<Inline, Vec<Param>>,
}

impl Clone for Params {
    fn clone(&self) -> Self {
        let kind = match self.kind {
            ParamsKind::Inline(ref q) => {
                // TODO: this impl is not good. StackQueue should be able to drain params or offer
                // internal clone.
                let mut q2 = Inline::new();
                for p in q.iter() {
                    let _ = q2.push_back(p.clone());
                }
                ParamsKind::Inline(q2)
            }
            ParamsKind::Heap(ref q) => ParamsKind::Heap(q.clone()),
        };
        Self { kind }
    }
}

#[derive(Debug)]
enum ParamsKind<I, P> {
    Inline(I),
    Heap(P),
}

impl Params {
    pub(crate) const fn new() -> Self {
        Self {
            kind: ParamsKind::Inline(Inline::new()),
        }
    }

    /// Returns the number of parameters.
    pub fn len(&self) -> usize {
        match self.kind {
            ParamsKind::Inline(ref q) => q.len(),
            ParamsKind::Heap(ref vec) => vec.len(),
        }
    }

    pub(crate) fn truncate(&mut self, n: usize) {
        match self.kind {
            ParamsKind::Inline(ref mut q) => q.truncate(n),
            ParamsKind::Heap(ref mut vec) => vec.truncate(n),
        }
    }

    /// Returns the value of the first parameter registered under the given key.
    pub fn get(&self, key: impl AsRef<str>) -> Option<&str> {
        let key = key.as_ref();

        match self.kind {
            ParamsKind::Inline(ref q) => q.iter().find(|param| param.key_str() == key).map(Param::value_str),
            ParamsKind::Heap(ref q) => q.iter().find(|param| param.key_str() == key).map(Param::value_str),
        }
    }

    /// Returns `true` if there are no parameters in the list.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Inserts a key value parameter pair into the list.
    pub(crate) fn push(&mut self, key: BytesStr, value: &[u8]) {
        #[cold]
        #[inline(never)]
        fn drain_to_vec<T, const LEN: usize>(value: T, q: &mut StackQueue<T, LEN>) -> Vec<T> {
            // respect vector's exponential growth practice.
            let mut v = Vec::with_capacity(LEN * 2);
            while let Some(value) = q.pop_front() {
                v.push(value);
            }
            v.push(value);
            v
        }

        let param = Param {
            key,
            value: BytesStr::try_from(value).unwrap(),
        };
        match self.kind {
            ParamsKind::Inline(ref mut q) => {
                if let Err(e) = q.push_back(param) {
                    self.kind = ParamsKind::Heap(drain_to_vec(e.into_inner(), q));
                }
            }
            ParamsKind::Heap(ref mut q) => q.push(param),
        }
    }
}

impl IntoIterator for Params {
    type Item = (BytesStr, BytesStr);
    type IntoIter = ParamsIntoIter;

    fn into_iter(self) -> Self::IntoIter {
        let kind = match self.kind {
            ParamsKind::Inline(q) => ParamsKind::Inline(q),
            ParamsKind::Heap(q) => ParamsKind::Heap(q.into_iter()),
        };

        ParamsIntoIter { kind }
    }
}

pub struct ParamsIntoIter {
    kind: ParamsKind<Inline, vec::IntoIter<Param>>,
}

impl Iterator for ParamsIntoIter {
    type Item = (BytesStr, BytesStr);

    fn next(&mut self) -> Option<Self::Item> {
        match self.kind {
            ParamsKind::Inline(ref mut q) => q.pop_front().map(|p| (p.key, p.value)),
            ParamsKind::Heap(ref mut iter) => iter.next().map(|p| (p.key, p.value)),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.kind {
            ParamsKind::Inline(ref q) => {
                let len = q.len();
                (len, Some(len))
            }
            ParamsKind::Heap(ref q) => q.size_hint(),
        }
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
    fn heap_alloc() {
        let vec = vec![
            ("hello", "hello"),
            ("world", "world"),
            ("foo", "foo"),
            ("bar", "bar"),
            ("baz", "baz"),
        ];

        let mut params = Params::new();
        for (key, value) in vec.clone() {
            params.push(key.into(), value.as_bytes());
            assert_eq!(params.get(key), Some(value));
        }

        match params.kind {
            ParamsKind::Heap(..) => {}
            _ => panic!(),
        }

        assert!(params
            .into_iter()
            .eq(vec.iter().map(|(k, v)| ((*k).into(), (*v).into()))));
    }

    #[test]
    fn stack_alloc() {
        let vec = vec![("hello", "hello"), ("world", "world")];

        let mut params = Params::new();
        for (key, value) in vec.clone() {
            params.push(key.into(), value.as_bytes());
            assert_eq!(params.get(key), Some(value));
        }

        match params.kind {
            ParamsKind::Inline(..) => {}
            _ => panic!(),
        }

        assert!(params
            .into_iter()
            .eq(vec.iter().map(|(k, v)| ((*k).into(), (*v).into()))));
    }

    #[test]
    fn ignore_array_default() {
        let params = Params::new();
        assert!(params.get("").is_none());
    }
}
