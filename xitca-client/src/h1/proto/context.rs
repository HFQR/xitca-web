use bytes::BytesMut;

pub(crate) struct Context {
    buf: BytesMut,
}

impl Context {
    pub(crate) fn with_capacity(cap: usize) -> Self {
        Self {
            buf: BytesMut::with_capacity(cap),
        }
    }

    pub(crate) fn buf(&self) -> &[u8] {
        &self.buf
    }
}
