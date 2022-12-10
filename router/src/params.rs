use alloc::vec::{self, Vec};

use xitca_unsafe_collection::bound_queue::stack::StackQueue;

use super::BytesStr;

/// A single URL parameter, consisting of a key and a value.
#[derive(Debug, PartialEq, Eq, Clone, Hash)]
struct Param<'v> {
    key: BytesStr,
    value: &'v str,
}

impl<'v> Param<'v> {
    fn key_str(&self) -> &str {
        self.key.as_ref()
    }

    fn value_str(&self) -> &'v str {
        self.value
    }
}

#[derive(Debug)]
pub struct Params<'v> {
    kind: ParamsKind<'v>,
}

#[derive(Debug)]
enum ParamsKind<'v> {
    Inline(StackQueue<Param<'v>, 2>),
    Heap(Vec<Param<'v>>),
}

impl<'v> Params<'v> {
    pub(crate) const fn new() -> Self {
        Self {
            kind: ParamsKind::Inline(StackQueue::new()),
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
    pub fn get(&self, key: impl AsRef<str>) -> Option<&'v str> {
        let key = key.as_ref();

        match self.kind {
            ParamsKind::Inline(ref q) => q.iter().find(|param| param.key_str() == key).map(Param::value_str),
            ParamsKind::Heap(ref q) => q.iter().find(|param| param.key_str() == key).map(Param::value_str),
        }
    }

    /// Returns `true` if there are no parameters in the list.
    pub fn is_empty(&self) -> bool {
        match self.kind {
            ParamsKind::Inline(ref q) => q.is_empty(),
            ParamsKind::Heap(ref q) => q.is_empty(),
        }
    }

    /// Inserts a key value parameter pair into the list.
    pub(crate) fn push(&mut self, key: BytesStr, value: &'v [u8]) {
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
            value: std::str::from_utf8(value).unwrap(),
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

impl<'v> IntoIterator for Params<'v> {
    type Item = (BytesStr, &'v str);
    type IntoIter = ParamsIntoIter<'v>;

    fn into_iter(self) -> Self::IntoIter {
        let kind = match self.kind {
            ParamsKind::Inline(q) => ParamsIntoIterKind::Inline(q),
            ParamsKind::Heap(q) => ParamsIntoIterKind::Heap(q.into_iter()),
        };

        ParamsIntoIter { kind }
    }
}

pub struct ParamsIntoIter<'v> {
    kind: ParamsIntoIterKind<'v>,
}

enum ParamsIntoIterKind<'v> {
    Inline(StackQueue<Param<'v>, 2>),
    Heap(vec::IntoIter<Param<'v>>),
}

impl<'v> Iterator for ParamsIntoIter<'v> {
    type Item = (BytesStr, &'v str);

    fn next(&mut self) -> Option<Self::Item> {
        match self.kind {
            ParamsIntoIterKind::Inline(ref mut q) => q.pop_front().map(|p| (p.key, p.value)),
            ParamsIntoIterKind::Heap(ref mut iter) => iter.next().map(|p| (p.key, p.value)),
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

        assert!(params.into_iter().eq(vec.iter().map(|(k, v)| ((*k).into(), *v))));
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

        assert!(params.into_iter().eq(vec.iter().map(|(k, v)| ((*k).into(), *v))));
    }

    #[test]
    fn ignore_array_default() {
        let params = Params::new();
        assert!(params.get("").is_none());
    }
}
