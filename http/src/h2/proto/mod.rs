#![allow(dead_code)]

mod dispatcher;
mod head;
mod hpack;
mod settings;
mod stream_id;

pub(crate) use dispatcher::Dispatcher;

pub const HEADER_LEN: usize = 9;

use std::io::{self};

use xitca_io::{
    bytes::{Buf, BytesMut},
    io::{AsyncIo, Interest},
};
use xitca_unsafe_collection::bytes::read_buf;

use self::settings::Settings;

const PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

pub async fn handshake<Io>(mut io: Io) -> io::Result<()>
where
    Io: AsyncIo,
{
    let mut write_buf = BytesMut::new();
    let mut r_buf = BytesMut::new();

    let settings = Settings::default();

    settings.encode(&mut write_buf);

    while !write_buf.is_empty() {
        match io.write(&write_buf) {
            Ok(0) => todo!(),
            Ok(n) => write_buf.advance(n),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
        io.ready(Interest::WRITABLE).await?;
    }

    loop {
        io.ready(Interest::READABLE).await?;
        match read_buf(&mut io, &mut r_buf) {
            Ok(0) => todo!(),
            Ok(_) => {
                if r_buf.len() >= PREFACE.len() {
                    if &r_buf[..PREFACE.len()] == PREFACE {
                        break;
                    } else {
                        todo!()
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

/// A helper macro that unpacks a sequence of 4 bytes found in the buffer with
/// the given identifier, starting at the given offset, into the given integer
/// type. Obviously, the integer type should be able to support at least 4
/// bytes.
///
/// # Examples
///
/// ```ignore
/// # // We ignore this doctest because the macro is not exported.
/// let buf: [u8; 4] = [0, 0, 0, 1];
/// assert_eq!(1u32, unpack_octets_4!(buf, 0, u32));
/// ```
macro_rules! unpack_octets_4 {
    // TODO: Get rid of this macro
    ($buf:expr, $offset:expr, $tip:ty) => {
        (($buf[$offset + 0] as $tip) << 24)
            | (($buf[$offset + 1] as $tip) << 16)
            | (($buf[$offset + 2] as $tip) << 8)
            | (($buf[$offset + 3] as $tip) << 0)
    };
}

use unpack_octets_4;

#[cfg(test)]
mod tests {
    #[test]
    fn test_unpack_octets_4() {
        let buf: [u8; 4] = [0, 0, 0, 1];
        assert_eq!(1u32, unpack_octets_4!(buf, 0, u32));
    }
}
