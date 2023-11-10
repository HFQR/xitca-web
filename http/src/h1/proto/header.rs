use core::mem::MaybeUninit;

use xitca_unsafe_collection::uninit::PartialInit;

use httparse::Header;

use super::error::ProtoError;

use crate::http::header::HeaderValue;

#[derive(Clone, Copy)]
pub struct HeaderIndex {
    pub name: (usize, usize),
    pub value: (usize, usize),
}

impl HeaderIndex {
    /// Record indices of pointer offset of give &[Header<'_>] slice from the &[u8] it parse from.
    pub fn record<'i>(indices: &'i mut [MaybeUninit<Self>], buf: &[u8], headers: &[Header<'_>]) -> &'i [Self] {
        let head = buf.as_ptr() as usize;
        indices.init_from(headers.iter()).into_init_with(|header| {
            let name_start = header.name.as_ptr() as usize - head;
            let value_start = header.value.as_ptr() as usize - head;
            let name_end = name_start + header.name.len();
            let value_end = value_start + header.value.len();
            Self {
                name: (name_start, name_end),
                value: (value_start, value_end),
            }
        })
    }
}

pub(super) fn parse_content_length(val: &HeaderValue) -> Result<u64, ProtoError> {
    val.to_str()
        .ok()
        .and_then(|v| v.parse().ok())
        .ok_or(ProtoError::HeaderValue)
}
