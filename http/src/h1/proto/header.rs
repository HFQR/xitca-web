use std::mem::MaybeUninit;

use xitca_unsafe_collection::uninit::PartialInit;

use httparse::Header;

#[derive(Clone, Copy)]
pub struct HeaderIndex {
    pub name: (usize, usize),
    pub value: (usize, usize),
}

impl HeaderIndex {
    pub fn record<'i, 'b, 'h>(
        indices: &'i mut [MaybeUninit<Self>],
        buf: &'b [u8],
        headers: &'h [Header<'_>],
    ) -> &'i [Self] {
        let bytes_ptr = buf.as_ptr() as usize;

        indices.init_from(headers.iter()).into_init_with(|header| {
            let name_start = header.name.as_ptr() as usize - bytes_ptr;
            let value_start = header.value.as_ptr() as usize - bytes_ptr;

            let name_end = name_start + header.name.len();
            let value_end = value_start + header.value.len();

            Self {
                name: (name_start, name_end),
                value: (value_start, value_end),
            }
        })
    }
}
