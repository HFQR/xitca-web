//! lazy type extractor.

use core::marker::PhantomData;

// lazy deserialize type that wrap around other extractor type like `Json` and `Form`
pub struct Lazy<T> {
    pub(super) buf: Vec<u8>,
    _type: PhantomData<T>,
}

impl<T> Lazy<T> {
    pub(super) fn new(buf: Vec<u8>) -> Self {
        Self {
            buf,
            _type: PhantomData,
        }
    }
}
