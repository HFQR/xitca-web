#![allow(dead_code)]

mod dispatcher;
mod head;
mod headers;
mod hpack;
mod priority;
mod settings;
mod stream_id;

pub(crate) use dispatcher::Dispatcher;

use core::{convert::Infallible, future::pending, mem};

use std::io;

use xitca_io::{
    bytes::{Buf, BufMut, Bytes, BytesMut},
    io::{AsyncIo, Interest, Ready},
};
use xitca_service::Service;
use xitca_unsafe_collection::futures::{Select, SelectOutput};

use crate::{
    http::{Request, RequestExt, Response, Version},
    util::{
        buffered_io::{self, BufInterest, BufWrite, ListWriteBuf, PagedBytesMut},
        futures::Queue,
    },
};

use super::body::RequestBodyV2;

use self::settings::Settings;

const HEADER_LEN: usize = 9;

const PREFACE: &[u8; 24] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

type BufferedIo<'i, Io, W> = buffered_io::BufferedIo<'i, Io, W, { 1024 * 1024 }>;

struct Context {
    decoder: hpack::Decoder,
    encoder: hpack::Encoder,
    // next_frame_len == 0 is used as maker for waiting for new frame.
    next_frame_len: usize,
}

impl Default for Context {
    fn default() -> Self {
        Self::new()
    }
}

impl Context {
    fn new() -> Self {
        Self {
            decoder: hpack::Decoder::new(settings::DEFAULT_SETTINGS_HEADER_TABLE_SIZE),
            encoder: hpack::Encoder::new(65535, 4096),
            next_frame_len: 0,
        }
    }

    fn try_decode<F>(&mut self, buf: &mut PagedBytesMut, mut on_msg: F)
    where
        F: FnMut(Request<RequestExt<RequestBodyV2>>),
    {
        loop {
            if self.next_frame_len == 0 {
                if buf.len() < 3 {
                    return;
                }
                self.next_frame_len = (buf.get_uint(3) + 6) as _;
            }

            if buf.len() < self.next_frame_len {
                return;
            }

            let len = mem::replace(&mut self.next_frame_len, 0);
            let mut frame = buf.split_to(len);
            let head = head::Head::parse(&frame);

            match dbg!(head.kind()) {
                head::Kind::Settings => {
                    // let _setting = Settings::load(head, &frame);
                }
                head::Kind::Headers => {
                    // TODO: Make Head::parse auto advance the frame?
                    frame.advance(6);

                    let (mut headers, mut frame) = headers::Headers::load(head, frame).unwrap();

                    headers.load_hpack(&mut frame, 4096, &mut self.decoder).unwrap();
                    let (_pseudo, headers) = headers.into_parts();

                    let (body, _) = RequestBodyV2::new_pair();
                    let mut req = Request::new(RequestExt::default()).map(|ext| ext.map_body(|_: ()| body));
                    *req.version_mut() = Version::HTTP_2;
                    *req.headers_mut() = headers;

                    on_msg(req);
                }
                head::Kind::Data => {}
                _ => {}
            }
        }
    }
}

/// Experimental h2 http layer.
pub async fn run<Io, S>(mut io: Io, service: S) -> io::Result<()>
where
    Io: AsyncIo,
    S: Service<Request<RequestExt<RequestBodyV2>>, Response = Response<()>, Error = Infallible>,
{
    let write_buf = ListWriteBuf::<Bytes, 32>::default();
    let mut io = BufferedIo::new(&mut io, write_buf);

    io.prefix_check().await?;

    let settings = Settings::default();

    settings.encode(&mut io.write_buf.buf);
    let buf = io.write_buf.buf.split().freeze();
    io.write_buf.buffer(buf);

    io.drain_write().await?;

    let mut ctx = Context::new();
    let mut queue = Queue::new();

    loop {
        match io.ready().select(queue.next()).await {
            SelectOutput::A(ready) => {
                let ready = ready?;
                if ready.is_writable() {
                    io.try_write()?;
                }
                if ready.is_readable() {
                    io.try_read()?;
                    ctx.try_decode(&mut io.read_buf, |req| {
                        queue.push(service.call(req));
                    });
                }
            }
            SelectOutput::B(res) => {
                let (parts, _) = res.unwrap().into_parts();
                let pseudo = headers::Pseudo::response(parts.status);
                let headers = headers::Headers::new(1.into(), pseudo, parts.headers);
                let mut buf = (&mut io.write_buf.buf).limit(4096);
                headers.encode(&mut ctx.encoder, &mut buf);
                let buf = io.write_buf.buf.split().freeze();
                io.write_buf.buffer(buf);
                break;
            }
        }
    }

    io.drain_write().await?;
    io.shutdown().await
}

impl<'a, Io, W> BufferedIo<'a, Io, W>
where
    Io: AsyncIo,
    W: BufWrite,
{
    #[cold]
    #[inline(never)]
    async fn prefix_check(&mut self) -> io::Result<()> {
        while self.read_buf.len() < PREFACE.len() {
            self.read().await?;
        }

        if &self.read_buf[..PREFACE.len()] == PREFACE {
            self.read_buf.advance(PREFACE.len());
        } else {
            todo!()
        }

        Ok(())
    }

    async fn ready(&mut self) -> io::Result<Ready> {
        let interest = match (self.read_buf.want_buf(), self.write_buf.want_write()) {
            (true, true) => Interest::READABLE | Interest::WRITABLE,
            (true, false) => Interest::READABLE,
            (false, true) => Interest::WRITABLE,
            _ => pending().await,
        };
        self.io.ready(interest).await
    }

    async fn recv_frame(&mut self) -> io::Result<BytesMut> {
        while self.read_buf.len() < 3 {
            self.read().await?;
        }

        let len = (self.read_buf.get_uint(3) + 6) as usize;

        while self.read_buf.len() < len {
            self.read().await?;
        }

        Ok(self.read_buf.split_to(len))
    }
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
