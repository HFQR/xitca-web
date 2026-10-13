use crate::{
    bytes::{BufMut, BytesMut},
    http::header::{HeaderName, HeaderValue},
};

use super::{
    header::Header,
    huffman,
    table::{Index, Table},
};

#[derive(Debug)]
pub struct Encoder {
    table: Table,
    size_update: Option<SizeUpdate>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum SizeUpdate {
    One(usize),
    Two(usize, usize), // min, max
}

impl Encoder {
    pub fn new(max_size: usize, capacity: usize) -> Encoder {
        Encoder {
            table: Table::new(max_size, capacity),
            size_update: None,
        }
    }

    /// Queues a max size update.
    ///
    /// The next call to `encode` will include a dynamic size update frame.
    pub fn update_max_size(&mut self, val: usize) {
        let size = match self.size_update {
            Some(SizeUpdate::One(old)) if val > old => {
                if old > self.table.max_size() {
                    SizeUpdate::One(val)
                } else {
                    SizeUpdate::Two(old, val)
                }
            }
            Some(SizeUpdate::One(_)) => SizeUpdate::One(val),
            Some(SizeUpdate::Two(min, _)) if val < min => SizeUpdate::One(val),
            Some(SizeUpdate::Two(min, _)) => SizeUpdate::Two(min, val),
            // Don't bother writing a frame if the value already matches
            // the table's max size.
            None if val != self.table.max_size() => SizeUpdate::One(val),
            None => return,
        };

        self.size_update = Some(size);
    }

    /// Encode a set of headers into the provide buffer
    pub fn encode<I>(&mut self, headers: I, dst: &mut BytesMut)
    where
        I: IntoIterator<Item = Header<Option<HeaderName>>>,
    {
        self.encode_size_updates(dst);

        let mut last_index = None;

        for header in headers {
            match header.reify() {
                // The header has an associated name. In which case, try to
                // index it in the table.
                Ok(header) => {
                    let index = self.table.index(header);
                    self.encode_header(&index, dst);

                    last_index = Some(index);
                }
                // The header does not have an associated name. This means that
                // the name is the same as the previously yielded header. In
                // which case, we skip table lookup and just use the same index
                // as the previous entry.
                Err(value) => {
                    self.encode_header_without_name(
                        last_index
                            .as_ref()
                            .expect("encoding header without name, but no previous index to use for name"),
                        &value,
                        dst,
                    );
                }
            }
        }
    }

    fn encode_size_updates(&mut self, dst: &mut BytesMut) {
        if let Some(size) = self.size_update.take() {
            match size {
                SizeUpdate::One(val) => {
                    self.table.resize(val);
                    encode_size_update(val, dst);
                }
                SizeUpdate::Two(min, max) => {
                    self.table.resize(min);
                    self.table.resize(max);
                    encode_size_update(min, dst);
                    encode_size_update(max, dst);
                }
            }
        }
    }

    fn encode_header(&mut self, index: &Index, dst: &mut BytesMut) {
        match *index {
            Index::Indexed(idx, _) => {
                encode_int(idx, 7, 0x80, dst);
            }
            Index::Name(idx, _) => {
                let header = self.table.resolve(index);

                encode_not_indexed(idx, header.value_slice(), header.is_sensitive(), dst);
            }
            Index::Inserted(_) => {
                let header = self.table.resolve(index);

                assert!(!header.is_sensitive());

                dst.put_u8(0b0100_0000);

                encode_str(header.name().as_slice(), dst);
                encode_str(header.value_slice(), dst);
            }
            Index::InsertedValue(idx, _) => {
                let header = self.table.resolve(index);

                assert!(!header.is_sensitive());

                encode_int(idx, 6, 0b0100_0000, dst);
                encode_str(header.value_slice(), dst);
            }
            Index::NotIndexed(_) => {
                let header = self.table.resolve(index);

                encode_not_indexed2(
                    header.name().as_slice(),
                    header.value_slice(),
                    header.is_sensitive(),
                    dst,
                );
            }
        }
    }

    fn encode_header_without_name(&mut self, last: &Index, value: &HeaderValue, dst: &mut BytesMut) {
        match *last {
            Index::Indexed(..) | Index::Name(..) | Index::Inserted(..) | Index::InsertedValue(..) => {
                let idx = self.table.resolve_idx(last);

                encode_not_indexed(idx, value.as_ref(), value.is_sensitive(), dst);
            }
            Index::NotIndexed(_) => {
                let last = self.table.resolve(last);

                encode_not_indexed2(last.name().as_slice(), value.as_ref(), value.is_sensitive(), dst);
            }
        }
    }
}

impl Default for Encoder {
    fn default() -> Encoder {
        Encoder::new(4096, 0)
    }
}

fn encode_size_update(val: usize, dst: &mut BytesMut) {
    encode_int(val, 5, 0b0010_0000, dst)
}

fn encode_not_indexed(name: usize, value: &[u8], sensitive: bool, dst: &mut BytesMut) {
    let sensitive = sensitive_byte(sensitive);
    encode_int(name, 4, sensitive, dst);
    encode_str(value, dst);
}

fn encode_not_indexed2(name: &[u8], value: &[u8], sensitive: bool, dst: &mut BytesMut) {
    let sensitive = sensitive_byte(sensitive);
    dst.put_u8(sensitive);
    encode_str(name, dst);
    encode_str(value, dst);
}

fn sensitive_byte(sensitive: bool) -> u8 {
    if sensitive { 0b10000 } else { 0 }
}

fn encode_str(val: &[u8], dst: &mut BytesMut) {
    let idx = position(dst);
    // Push a placeholder byte for the length header
    dst.put_u8(0);

    if val.is_empty() {
        return;
    }

    // Encode with huffman
    huffman::encode(val, dst);

    let huff_len = position(dst) - (idx + 1);

    if encode_int_one_byte(huff_len, 7) {
        // Write the string head
        dst[idx] = 0x80 | huff_len as u8;
    } else {
        // Write the head to a placeholder
        const PLACEHOLDER_LEN: usize = 8;
        let mut buf = [0u8; PLACEHOLDER_LEN];

        let head_len = {
            let mut head_dst = &mut buf[..];
            encode_int(huff_len, 7, 0x80, &mut head_dst);
            PLACEHOLDER_LEN - head_dst.remaining_mut()
        };

        // This is just done to reserve space in the destination
        dst.put_slice(&buf[1..head_len]);

        // Shift the header forward
        for i in 0..huff_len {
            let src_i = idx + 1 + (huff_len - (i + 1));
            let dst_i = idx + head_len + (huff_len - (i + 1));
            dst[dst_i] = dst[src_i];
        }

        // Copy in the head
        for i in 0..head_len {
            dst[idx + i] = buf[i];
        }
    }
}

/// Encode an integer into the given destination buffer
fn encode_int<B: BufMut>(
    mut value: usize,   // The integer to encode
    prefix_bits: usize, // The number of bits in the prefix
    first_byte: u8,     // The base upon which to start encoding the int
    dst: &mut B,
) {
    if encode_int_one_byte(value, prefix_bits) {
        dst.put_u8(first_byte | value as u8);
        return;
    }

    let low = (1 << prefix_bits) - 1;

    value -= low;

    dst.put_u8(first_byte | low as u8);

    while value >= 128 {
        dst.put_u8(0b1000_0000 | value as u8);

        value >>= 7;
    }

    dst.put_u8(value as u8);
}

/// Returns true if the in the int can be fully encoded in the first byte.
fn encode_int_one_byte(value: usize, prefix_bits: usize) -> bool {
    value < (1 << prefix_bits) - 1
}

fn position(buf: &BytesMut) -> usize {
    buf.len()
}

#[cfg(test)]
mod tests;
