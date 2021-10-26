use httparse::Header;

#[derive(Clone, Copy)]
pub struct HeaderIndex {
    pub name: (usize, usize),
    pub value: (usize, usize),
}

impl HeaderIndex {
    #[inline]
    pub const fn new_array<const MAX_HEADERS: usize>() -> [Self; MAX_HEADERS] {
        [Self {
            name: (0, 0),
            value: (0, 0),
        }; MAX_HEADERS]
    }

    pub fn record(bytes: &[u8], headers: &[Header<'_>], indices: &mut [Self]) {
        let bytes_ptr = bytes.as_ptr() as usize;

        headers.iter().zip(indices.iter_mut()).for_each(|(header, indice)| {
            let name_start = header.name.as_ptr() as usize - bytes_ptr;
            let value_start = header.value.as_ptr() as usize - bytes_ptr;

            let name_end = name_start + header.name.len();
            let value_end = value_start + header.value.len();

            indice.name = (name_start, name_end);
            indice.value = (value_start, value_end);
        });
    }
}
