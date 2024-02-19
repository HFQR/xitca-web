#![allow(clippy::await_holding_refcell_ref)] // clippy is dumb

use core::{
    cell::RefCell,
    ops::{Deref, DerefMut},
    slice,
};

use std::{io, net::Shutdown};

use rustls::{ConnectionCommon, SideData};
use xitca_io::{
    bytes::{Buf, BytesMut},
    io_uring::{AsyncBufRead, AsyncBufWrite, IoBuf, IoBufMut},
};

use self::buf::WriteBuf;

/// A tls stream type enable concurrent async read/write through [AsyncBufRead] and [AsyncBufWrite]
/// traits.
///
/// # Panics
/// For now due to design limitation TlsStream offers concurrency with [AsyncBufRead::read] and
/// [AsyncBufWrite::write] but in either case the async function must run to completion and cancel
/// it prematurely would cause panic.
/// ```rust(no_run)
/// # async fn complete(stream: TlsStream<ServerConnection, TcpStream>) {
///     let _ = stream.read(vec![0; 128]).await;
///     let _ = stream.read(vec![0; 128]).await; // serialize read to complete is ok.
///
///     let _ = stream.read(vec![0; 128]);
///     let _ = stream.write(vec![0; 128]); // concurrent read and write is ok.
///
///     let read = stream.read(vec![0; 128]);
///     drop(read); // drop read without completion.
///     let read = stream.read(vec![0; 128]).await; // this line would cause panic.
///
///     let read = stream.read(vec![0; 128]); // making two concurrent read future.
///     let read = stream.read(vec![0; 128]); // this line would cause panic.
/// }
/// ```
pub struct TlsStream<C, Io> {
    io: Io,
    session: RefCell<Session<C>>,
}

struct Session<C> {
    session: C,
    read_buf: Option<BytesMut>,
    write_buf: Option<WriteBuf>,
}

impl<C, S> Session<C>
where
    C: DerefMut + Deref<Target = ConnectionCommon<S>>,
{
    fn read_plain(&mut self, buf: &mut impl IoBufMut) -> io::Result<usize> {
        io::Read::read(&mut self.session.reader(), io_ref_mut_slice(buf)).map(|n| {
            // SAFETY
            // required by IoBufMut trait. when n bytes is write into buffer this method
            // must be called to advance the initialized part of it.
            unsafe { buf.set_init(n) };
            n
        })
    }

    fn write_plain(&mut self, buf: &impl IoBuf) -> io::Result<usize> {
        let writer = &mut self.session.writer();
        let n = io::Write::write(writer, io_ref_slice(buf))?;
        // keep this no op in case rustls change it's behavior.
        io::Write::flush(writer).expect("<rustls::conn::Writer as std::io::Write>::flush should be no op");
        Ok(n)
    }
}

impl<C, S, Io> TlsStream<C, Io>
where
    C: DerefMut + Deref<Target = ConnectionCommon<S>>,
    S: SideData,
    Io: AsyncBufRead,
{
    async fn read_tls(&self) -> io::Result<usize> {
        let mut session = self.session.borrow_mut();

        let mut buf = session.read_buf.take().expect(POLL_TO_COMPLETE);

        if buf.is_empty() {
            drop(session);

            let rem = buf.capacity() - buf.len();
            if rem < 4096 {
                buf.reserve(4096 - rem);
            }

            let (res, b) = self.io.read(buf).await;
            buf = b;

            session = self.session.borrow_mut();

            if res? == 0 {
                session.read_buf.replace(buf);
                return Ok(0);
            }
        }

        let res = session.session.read_tls(&mut buf.as_ref()).map(|n| {
            buf.advance(n);
            n
        });

        session.read_buf.replace(buf);

        let n = res?;

        let state = session
            .session
            .process_new_packets()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        if state.peer_has_closed() && session.session.is_handshaking() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }

        Ok(n)
    }
}

impl<C, S, Io> TlsStream<C, Io>
where
    C: DerefMut + Deref<Target = ConnectionCommon<S>>,
    S: SideData,
    Io: AsyncBufWrite,
{
    async fn write_tls(&self) -> io::Result<usize> {
        let mut session = self.session.borrow_mut();

        let mut write_buf = session.write_buf.take().expect(POLL_TO_COMPLETE);

        let n = match session.session.write_tls(&mut write_buf) {
            Ok(n) => n,
            Err(e) => {
                session.write_buf.replace(write_buf);
                return Err(e);
            }
        };

        drop(session);

        let (res, write_buf) = write_buf.write_io(&self.io).await;

        session = self.session.borrow_mut();
        session.write_buf.replace(write_buf);

        match res {
            Ok(0) => Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(_) => Ok(n),
            Err(e) => Err(e),
        }
    }
}

impl<C, S, Io> TlsStream<C, Io>
where
    C: DerefMut + Deref<Target = ConnectionCommon<S>>,
    S: SideData,
    Io: AsyncBufRead + AsyncBufWrite,
{
    pub async fn handshake(io: Io, session: C) -> io::Result<Self> {
        let mut stream = TlsStream {
            io,
            session: RefCell::new(Session {
                session,
                read_buf: Some(BytesMut::new()),
                write_buf: Some(WriteBuf::default()),
            }),
        };
        stream._handshake().await?;
        Ok(stream)
    }

    pub(crate) async fn _handshake(&mut self) -> io::Result<(usize, usize)> {
        let mut wrlen = 0;
        let mut rdlen = 0;
        let mut eof = false;

        loop {
            while self.session.get_mut().session.wants_write() && self.session.get_mut().session.is_handshaking() {
                wrlen += self.write_tls().await?;
            }

            while !eof && self.session.get_mut().session.wants_read() && self.session.get_mut().session.is_handshaking()
            {
                let n = self.read_tls().await?;
                rdlen += n;
                if n == 0 {
                    eof = true;
                }
            }

            match (eof, self.session.get_mut().session.is_handshaking()) {
                (true, true) => {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "tls handshake eof"));
                }
                (false, true) => {}
                (_, false) => break,
            };
        }

        while self.session.get_mut().session.wants_write() {
            wrlen += self.write_tls().await?;
        }

        Ok((rdlen, wrlen))
    }
}

impl<C, S, Io> AsyncBufRead for TlsStream<C, Io>
where
    C: DerefMut + Deref<Target = ConnectionCommon<S>>,
    S: SideData,
    Io: AsyncBufRead,
{
    async fn read<B>(&self, mut buf: B) -> (io::Result<usize>, B)
    where
        B: IoBufMut,
    {
        let mut session = self.session.borrow_mut();

        loop {
            match session.read_plain(&mut buf) {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                res => return (res, buf),
            }

            drop(session);

            match self.read_tls().await {
                Ok(0) => return (Err(io::ErrorKind::UnexpectedEof.into()), buf),
                Ok(_) => session = self.session.borrow_mut(),
                e => return (e, buf),
            };
        }
    }
}

impl<C, S, Io> AsyncBufWrite for TlsStream<C, Io>
where
    C: DerefMut + Deref<Target = ConnectionCommon<S>>,
    S: SideData,
    Io: AsyncBufWrite,
{
    async fn write<B>(&self, buf: B) -> (io::Result<usize>, B)
    where
        B: IoBuf,
    {
        let mut session = self.session.borrow_mut();

        let len = match session.write_plain(&buf) {
            Ok(n) => n,
            e => return (e, buf),
        };

        let mut write_buf = session.write_buf.take().expect(POLL_TO_COMPLETE);

        // currently there is no AsyncBufWrite::flush so write must keep flushing io until
        // every bit of tls data is sent. this could be changed in the future for more efficient
        // rustl buffer usage.
        while session.session.wants_write() {
            if let Err(e) = session.session.write_tls(&mut write_buf) {
                session.write_buf.replace(write_buf);
                return (Err(e), buf);
            }

            drop(session);

            let (res, b) = write_buf.write_io(&self.io).await;
            write_buf = b;

            session = self.session.borrow_mut();

            match res {
                Ok(0) => {
                    session.write_buf.replace(write_buf);
                    return (Err(io::ErrorKind::UnexpectedEof.into()), buf);
                }
                Ok(_) => {}
                e => {
                    session.write_buf.replace(write_buf);
                    return (e, buf);
                }
            }
        }

        session.write_buf.replace(write_buf);

        (Ok(len), buf)
    }

    fn shutdown(&self, direction: Shutdown) -> io::Result<()> {
        self.io.shutdown(direction)
    }
}

fn io_ref_slice(buf: &impl IoBuf) -> &[u8] {
    // SAFETY
    // have to trust IoBuf implementor to provide valid pointer and it's length.
    unsafe { slice::from_raw_parts(buf.stable_ptr(), buf.bytes_init()) }
}

fn io_ref_mut_slice(buf: &mut impl IoBufMut) -> &mut [u8] {
    // SAFETY
    // have to trust IoBufMut implementor to provide valid pointer and it's capacity.
    unsafe { slice::from_raw_parts_mut(buf.stable_mut_ptr(), buf.bytes_total()) }
}

const POLL_TO_COMPLETE: &str = "previous call to future dropped before polling to completion";

mod buf {
    use super::*;

    pub(super) struct WriteBuf {
        buf: BytesMut,
    }

    impl Default for WriteBuf {
        fn default() -> Self {
            Self { buf: BytesMut::new() }
        }
    }

    impl WriteBuf {
        pub(super) async fn write_io(mut self, io: &impl AsyncBufWrite) -> (io::Result<usize>, Self) {
            if self.buf.is_empty() {
                return (Ok(0), self);
            }
            let (res, buf) = io.write(self.buf).await;
            self.buf = buf;

            (
                res.map(|n| {
                    self.buf.advance(n);
                    n
                }),
                self,
            )
        }
    }

    impl io::Write for WriteBuf {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.buf.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
