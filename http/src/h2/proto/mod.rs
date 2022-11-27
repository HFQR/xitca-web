#![allow(dead_code)]

mod dispatcher;
mod head;
mod headers;
mod hpack;
mod priority;
mod settings;
mod stream_id;

pub(crate) use dispatcher::Dispatcher;

pub const HEADER_LEN: usize = 9;

use std::io::{self};

use xitca_io::{
    bytes::{Buf, Bytes, BytesMut},
    io::AsyncIo,
};

use crate::util::buffered_io::{self, BufWrite, ListBuf};

use self::settings::Settings;

const PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

type BufferedIo<'i, Io, W> = buffered_io::BufferedIo<'i, Io, W, { 1024 * 1024 }>;

/// Experimental h2 http layer.
pub async fn run<Io>(mut io: Io) -> io::Result<()>
where
    Io: AsyncIo,
{
    let write_buf = ListBuf::<Bytes, 32>::default();
    let mut io = BufferedIo::new(&mut io, write_buf);

    prefix_check(&mut io).await?;

    let settings = Settings::default();

    settings.encode(&mut io.write_buf.buf);
    let buf = io.write_buf.buf.split().freeze();
    io.write_buf.buffer(buf);

    io.drain_write().await?;

    let mut decoder = hpack::Decoder::new(settings::DEFAULT_SETTINGS_HEADER_TABLE_SIZE);

    // settings ack is parsed and ignored for now.
    {
        let frame = recv_frame(&mut io).await?;

        let head = head::Head::parse(&frame);

        let _ = Settings::load(head, &frame).unwrap();
    }

    // naively assume a header frame is gonna come in.
    {
        let frame = recv_frame(&mut io).await?;
        let head = head::Head::parse(&frame);
        assert_eq!(head.kind(), head::Kind::Headers);

        let (mut headers, mut frame) = headers::Headers::load(head, frame).unwrap();

        // TODO: Make Head::parse auto advance the frame?
        frame.advance(6);

        headers.load_hpack(&mut frame, 4096, &mut decoder).unwrap();
        let (pseudo, headers) = headers.into_parts();
        dbg!(pseudo);
        dbg!(headers);
    }

    io.shutdown().await
}

#[cold]
#[inline(never)]
async fn prefix_check<Io, W>(io: &mut BufferedIo<'_, Io, W>) -> io::Result<()>
where
    Io: AsyncIo,
    W: BufWrite,
{
    while io.read_buf.len() < PREFACE.len() {
        io.read().await?;
    }

    if &io.read_buf[..PREFACE.len()] == PREFACE {
        io.read_buf.advance(PREFACE.len());
    } else {
        todo!()
    }

    Ok(())
}

async fn recv_frame<Io, W>(io: &mut BufferedIo<'_, Io, W>) -> io::Result<BytesMut>
where
    Io: AsyncIo,
    W: BufWrite,
{
    while io.read_buf.len() < 3 {
        io.read().await?;
    }

    let len = (io.read_buf.get_uint(3) + 6) as usize;

    while io.read_buf.len() < len {
        io.read().await?;
    }

    Ok(io.read_buf.split_to(len))
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
