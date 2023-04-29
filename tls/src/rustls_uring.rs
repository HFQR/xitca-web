use core::{
    any::Any,
    cell::RefCell,
    future::Future,
    ops::{Deref, DerefMut},
    slice,
};

use std::{io, net::Shutdown};

use rustls::{ConnectionCommon, SideData};
use xitca_io::{
    bytes::BytesMut,
    io_uring::{AsyncBufRead, AsyncBufWrite, IoBuf, IoBufMut, Slice},
};

use self::buf::{ReadBuf, WriteBuf};

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
    read_buf: Option<ReadBuf>,
    write_buf: Option<WriteBuf>,
}

impl<C, S> Session<C>
where
    C: DerefMut + Deref<Target = ConnectionCommon<S>>,
    S: SideData,
{
    fn read_tls(&mut self, res: io::Result<usize>, bytes: &mut Slice<BytesMut>) -> io::Result<()> {
        if res? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }

        let mut slice = io_ref_slice(bytes);

        // have to drain Slices<BytesMut>'s data as later it will be cleared and
        // reused for writing plaintext bytes decrypted by rustls.
        // this can fail due to buffer upper limit of rustls' internal buffer which
        // can be configured.
        while !slice.is_empty() {
            match self.session.read_tls(&mut slice) {
                Ok(0) => unreachable!("an empty slice can not be used for reading tls"),
                Ok(_) => match self.session.process_new_packets() {
                    Ok(state) => {
                        if state.peer_has_closed() && self.session.is_handshaking() {
                            return Err(io::ErrorKind::UnexpectedEof.into());
                        }
                    }
                    Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
                },
                Err(e) => return Err(e),
            }
        }

        bytes.get_mut().clear();

        Ok(())
    }
}

impl<C, S, Io> TlsStream<C, Io>
where
    C: DerefMut + Deref<Target = ConnectionCommon<S>>,
    S: SideData,
    Io: AsyncBufRead,
{
    async fn read_io(&self) -> io::Result<usize> {
        let mut borrow = self.session.borrow_mut();

        loop {
            let Session { session, read_buf, .. } = &mut *borrow;
            match session.read_tls(read_buf.as_mut().expect(POLL_TO_COMPLETE)) {
                Ok(n) => {
                    let state = session
                        .process_new_packets()
                        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

                    return if state.peer_has_closed() && session.is_handshaking() {
                        Err(io::ErrorKind::UnexpectedEof.into())
                    } else {
                        Ok(n)
                    };
                }
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => {
                    let read_buf = read_buf.take().expect(POLL_TO_COMPLETE);
                    drop(borrow);

                    let read_buf = read_buf.do_io(&self.io).await;

                    borrow = self.session.borrow_mut();

                    borrow.read_buf.replace(read_buf);
                }
                Err(err) => return Err(err),
            }
        }
    }
}

impl<C, S, Io> TlsStream<C, Io>
where
    C: DerefMut + Deref<Target = ConnectionCommon<S>>,
    S: SideData,
    Io: AsyncBufWrite,
{
    async fn write_io(&self) -> io::Result<usize> {
        let mut borrow = self.session.borrow_mut();

        loop {
            let Session { session, write_buf, .. } = &mut *borrow;

            let mut write_buf = write_buf.take().expect(POLL_TO_COMPLETE);
            let read_some = match session.write_tls(&mut write_buf) {
                Ok(n) => Some(n),
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => None,
                Err(err) => return Err(err),
            };

            drop(borrow);

            let buf = write_buf.write_io(&self.io).await;

            borrow = self.session.borrow_mut();
            borrow.write_buf.replace(buf);

            if let Some(n) = read_some {
                return Ok(n);
            }
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
                read_buf: Some(ReadBuf::default()),
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
                wrlen += self.write_io().await?;
            }

            while !eof && self.session.get_mut().session.wants_read() && self.session.get_mut().session.is_handshaking()
            {
                let n = self.read_io().await?;
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
            wrlen += self.write_io().await?;
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
    type Future<'f, B> = impl Future<Output = (io::Result<usize>, B)> + 'f
    where
        Self: 'f,
        B: IoBufMut + 'f;

    fn read<B>(&self, mut buf: B) -> Self::Future<'_, B>
    where
        B: IoBufMut,
    {
        async {
            let mut session = self.session.borrow_mut();

            loop {
                match io::Read::read(&mut session.session.reader(), io_ref_mut_slice(&mut buf)) {
                    Ok(n) => {
                        // SAFETY
                        // required by IoBufMut trait. when n bytes is write into buffer this method
                        // must be called to advance the initialized part of it.
                        unsafe { buf.set_init(n) };

                        return (Ok(n), buf);
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => {
                        return (Err(e), buf);
                    }
                }

                drop(session);

                // this is a hack targeting xitca_http which often use Slice<BytesMut> type.
                // the purpose is to reuse B type without touching TlsStream's internal buffer to
                // reduce memory copy.
                // related issue:
                // https://github.com/tokio-rs/tokio-uring/issues/216
                match downcast_to_slice(buf) {
                    Ok(bytes) => {
                        let (res, mut bytes) = self.io.read(bytes).await;
                        session = self.session.borrow_mut();
                        let res = session.read_tls(res, &mut bytes);
                        buf = upcast_to_buf(bytes);
                        if let Err(e) = res {
                            return (Err(e), buf);
                        }
                    }
                    Err(b) => {
                        buf = b;
                        match self.read_io().await {
                            Ok(0) => return (Err(io::ErrorKind::UnexpectedEof.into()), buf),
                            Ok(_) => session = self.session.borrow_mut(),
                            Err(e) => return (Err(e), buf),
                        };
                    }
                }
            }
        }
    }
}

impl<C, S, Io> AsyncBufWrite for TlsStream<C, Io>
where
    C: DerefMut + Deref<Target = ConnectionCommon<S>>,
    S: SideData,
    Io: AsyncBufWrite,
{
    type Future<'f, B> = impl Future<Output = (io::Result<usize>, B)> + 'f
    where
        Self: 'f,
        B: IoBuf + 'f;

    fn write<B>(&self, buf: B) -> Self::Future<'_, B>
    where
        B: IoBuf,
    {
        async {
            let (len, mut want_write) = {
                let mut session = self.session.borrow_mut();

                let slice = io_ref_slice(&buf);
                let len = slice.len();

                let writer = &mut session.session.writer();

                if let Err(e) = io::Write::write_all(writer, slice) {
                    return (Err(e), buf);
                };

                io::Write::flush(writer).expect("<rustls::conn::Writer as std::io::Write>::flush should be no op");

                (len, session.session.wants_write())
            };

            while want_write {
                match self.write_io().await {
                    Ok(0) => break,
                    Ok(_) => want_write = self.session.borrow_mut().session.wants_write(),
                    Err(e) => return (Err(e), buf),
                }
            }

            (Ok(len), buf)
        }
    }

    fn shutdown(&self, direction: Shutdown) -> io::Result<()> {
        self.io.shutdown(direction)
    }
}

fn downcast_to_slice<B>(buf: B) -> Result<Slice<BytesMut>, B>
where
    B: 'static,
{
    let mut opt = Some(buf);
    (&mut opt as &mut dyn Any)
        .downcast_mut::<Option<Slice<BytesMut>>>()
        .map(|opt| opt.take().unwrap())
        .ok_or_else(|| opt.take().unwrap())
}

fn upcast_to_buf<B>(bytes: Slice<BytesMut>) -> B
where
    B: 'static,
{
    (&mut Some(bytes) as &mut dyn Any)
        .downcast_mut::<Option<B>>()
        .unwrap()
        .take()
        .unwrap()
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
    use xitca_io::bytes::{Buf, BytesMut};

    use super::*;

    #[derive(Debug, Default)]
    struct State {
        err: Option<io::Error>,
        is_eof: bool,
    }

    pub(crate) struct ReadBuf {
        buf: BytesMut,
        state: State,
    }

    impl Default for ReadBuf {
        fn default() -> Self {
            Self {
                buf: BytesMut::new(),
                state: State::default(),
            }
        }
    }

    impl ReadBuf {
        pub(super) async fn do_io(mut self, io: &impl AsyncBufRead) -> Self {
            if !self.buf.is_empty() {
                return self;
            }
            let rem = self.buf.capacity() - self.buf.len();
            if rem < 4096 {
                self.buf.reserve(4096 - rem);
            }
            let (res, buf) = io.read(self.buf).await;
            self.buf = buf;
            match res {
                Ok(0) => self.state.is_eof = true,
                Ok(_) => {}
                Err(e) => self.state.err = Some(e),
            }
            self
        }
    }

    impl io::Read for ReadBuf {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }

            if self.buf.is_empty() {
                return if self.state.err.is_some() {
                    Err(self.state.err.take().unwrap())
                } else if self.state.is_eof {
                    Ok(0)
                } else {
                    Err(io::ErrorKind::WouldBlock.into())
                };
            }

            let len = self.buf.len().min(buf.len());

            self.buf.copy_to_slice(&mut buf[..len]);

            Ok(len)
        }
    }

    pub(super) struct WriteBuf {
        buf: BytesMut,
        state: State,
    }

    impl Default for WriteBuf {
        fn default() -> Self {
            Self {
                buf: BytesMut::new(),
                state: State::default(),
            }
        }
    }

    impl WriteBuf {
        pub(super) async fn write_io(mut self, io: &impl AsyncBufWrite) -> Self {
            if self.buf.is_empty() {
                return self;
            }
            let (res, buf) = io.write(self.buf).await;
            self.buf = buf;
            match res {
                Ok(n) => self.buf.advance(n),
                Err(e) => self.state.err = Some(e),
            }
            self
        }
    }

    impl io::Write for WriteBuf {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }

            if self.state.err.is_some() {
                return Err(self.state.err.take().unwrap());
            }

            self.buf.extend_from_slice(buf);

            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            if self.state.err.is_some() {
                return Err(self.state.err.take().unwrap());
            }

            if !self.buf.is_empty() {
                return Err(io::ErrorKind::WouldBlock.into());
            }

            Ok(())
        }
    }
}
