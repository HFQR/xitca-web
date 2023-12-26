//! lazy type extractor.

use core::marker::PhantomData;

enum Source<'a> {
    Vec(Vec<u8>),
    Slice(&'a [u8]),
    #[cfg(feature = "params")]
    Params(super::params::Params2<'a>),
}

// lazy deserialize type that wrap around other extractor type like `Json`, `Form` and `Query`
pub struct Lazy<'a, T> {
    source: Source<'a>,
    _type: PhantomData<T>,
}

impl<T> Lazy<'_, T> {
    #[cfg(any(feature = "json", feature = "urlencoded"))]
    pub(super) fn as_slice(&self) -> &[u8] {
        match self.source {
            Source::Vec(ref vec) => vec.as_slice(),
            Source::Slice(slice) => slice,
            #[cfg(feature = "params")]
            Source::Params(_) => unreachable!("params must not be sliced"),
        }
    }

    #[cfg(feature = "params")]
    pub(super) fn as_params(&self) -> super::params::Params2<'_> {
        match self.source {
            Source::Params(ref params) => *params,
            _ => unreachable!("vec and slice must not be cast to params"),
        }
    }
}

impl<T> From<Vec<u8>> for Lazy<'static, T> {
    fn from(vec: Vec<u8>) -> Self {
        Self {
            source: Source::Vec(vec),
            _type: PhantomData,
        }
    }
}

impl<'a, T> From<&'a [u8]> for Lazy<'a, T> {
    fn from(slice: &'a [u8]) -> Self {
        Self {
            source: Source::Slice(slice),
            _type: PhantomData,
        }
    }
}

#[cfg(feature = "params")]
impl<'a, T> From<super::params::Params2<'a>> for Lazy<'a, T> {
    fn from(params: super::params::Params2<'a>) -> Self {
        Self {
            source: Source::Params(params),
            _type: PhantomData,
        }
    }
}
