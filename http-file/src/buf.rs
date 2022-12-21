use core::fmt;

use bytes::{Bytes, BytesMut};

pub(super) struct BytesMutWriter(BytesMut);

impl BytesMutWriter {
    pub(super) fn with_capacity(cap: usize) -> Self {
        Self(BytesMut::with_capacity(cap))
    }

    pub(super) fn freeze(self) -> Bytes {
        self.0.freeze()
    }
}

impl fmt::Write for BytesMutWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

macro_rules! buf_write_header {
    ($cap:expr, $($arg:tt)*) => {{
        let mut buf = crate::buf::BytesMutWriter::with_capacity($cap);
        use ::core::fmt::Write;
        write!(&mut buf, $($arg)*).unwrap();
        ::http::HeaderValue::from_maybe_shared(buf.freeze()).unwrap()
    }};
}

pub(super) use buf_write_header;
