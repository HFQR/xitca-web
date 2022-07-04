use std::{
    io::{self, Read},
    mem::MaybeUninit,
};

use bytes_crate::buf::BufMut;

pub fn read_buf<Io, B>(io: &mut Io, buf: &mut B) -> io::Result<usize>
where
    Io: Read,
    B: BufMut,
{
    let dst = buf.chunk_mut();

    // SAFETY:
    // This is not ideal but it's the only way until zero copy read interface added into Rust.
    // See std::io::Read::read_buf for detail.
    let dst = unsafe { &mut *(dst as *mut _ as *mut [MaybeUninit<u8>] as *mut [u8]) };

    let n = io.read(dst)?;

    // SAFETY:
    // Read should return the bytes that read into dst.
    unsafe {
        buf.advance_mut(n);
    }

    Ok(n)
}
