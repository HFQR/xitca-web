use core::{cell::RefCell, future::Future, ops::DerefMut, slice};

use std::io::Write;
use std::{io, net::Shutdown};

use rustls::{ConnectionCommon, SideData};
use xitca_io::io_uring::{AsyncBufRead, AsyncBufWrite, IoBuf, IoBufMut};

pub struct TlsStream<S, Io> {
    session: Session<S>,
    io: Io,
}

struct Session<S>(RefCell<SessionInner<S>>);

struct SessionInner<S> {
    session: S,
    write_buf: Option<Vec<u8>>,
}

impl<S, D> Session<S>
where
    S: DerefMut<Target = ConnectionCommon<D>>,
    D: SideData,
{
    fn read_plain(&self, buf: &mut impl IoBufMut) -> io::Result<usize> {
        let mut inner = self.0.borrow_mut();
        io::Read::read(&mut inner.session.reader(), io_buf_slice_mut(buf)).map(|n| {
            // SAFETY
            // have to trust IoBufMut trait to properly update initialized length.
            unsafe { buf.set_init(n) };
            n
        })
    }

    fn read_tls(&self, buf: &impl IoBuf) -> io::Result<()> {
        let mut inner = self.0.borrow_mut();

        let mut slice = io_buf_slice(buf);
        let len = slice.len();

        let n = inner.session.read_tls(&mut slice)?;
        assert_eq!(n, len, "TlsStream does not do internal read buffering. IoBuf type's initialized bytes must fits into rustls internal buffer.");

        let state = inner
            .session
            .process_new_packets()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        if state.peer_has_closed() && inner.session.is_handshaking() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }

        Ok(())
    }
}

impl<S, D, Io> AsyncBufRead for TlsStream<S, Io>
where
    S: DerefMut<Target = ConnectionCommon<D>>,
    D: SideData,
    Io: AsyncBufRead,
{
    type Future<'f, B> = impl Future<Output = (io::Result<usize>, B)> + 'f where Self: 'f, B: IoBufMut + 'f;

    fn read<B>(&self, mut buf: B) -> Self::Future<'_, B>
    where
        B: IoBufMut,
    {
        async {
            loop {
                match self.session.read_plain(&mut buf) {
                    Ok(n) => return (Ok(n), buf),
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return (Err(e), buf),
                }

                match self.io.read(buf).await {
                    (Ok(0), buf) => return (Ok(0), buf),
                    (Ok(_), b) => match self.session.read_tls(&b) {
                        Ok(_) => buf = b,
                        Err(e) => return (Err(e), b),
                    },
                    e => return e,
                }
            }
        }
    }
}

impl<S, D, Io> AsyncBufWrite for TlsStream<S, Io>
where
    S: DerefMut<Target = ConnectionCommon<D>>,
    D: SideData,
    Io: AsyncBufWrite,
{
    type Future<'f, B> = impl Future<Output = (io::Result<usize>, B)> + 'f where Self: 'f, B: IoBuf + 'f;

    fn write<B>(&self, buf: B) -> Self::Future<'_, B>
    where
        B: IoBuf,
    {
        async {
            let mut inner = self.session.0.borrow_mut();

            let n = match inner.session.writer().write(io_buf_slice(&buf)) {
                Ok(n) => n,
                Err(e) => return (Err(e), buf),
            };

            let mut write_buf = inner
                .write_buf
                .take()
                .expect("<TlsStream as AsyncBufWrite>::write did not polled to completion");

            loop {
                inner
                    .session
                    .write_tls(&mut write_buf)
                    .expect("<Vec<u8> as Write>::write should not have an error path");

                drop(inner);

                let mut slice = write_buf.slice(..);

                while !slice.is_empty() {
                    match self.io.write(slice).await {
                        (Ok(0), s) => {
                            write_buf = s.into_inner();
                            write_buf.clear();
                            self.session.0.borrow_mut().write_buf.replace(write_buf);
                            return (Err(io::ErrorKind::WriteZero.into()), buf);
                        }
                        (Ok(n), s) => slice = s.into_inner().slice(n..),
                        (Err(e), s) => {
                            write_buf = s.into_inner();
                            write_buf.clear();
                            self.session.0.borrow_mut().write_buf.replace(write_buf);
                            return (Err(e), buf);
                        }
                    }
                }

                write_buf = slice.into_inner();

                inner = self.session.0.borrow_mut();

                if !inner.session.wants_write() {
                    break;
                }
            }

            inner.write_buf.replace(write_buf);

            (Ok(n), buf)
        }
    }

    fn shutdown(&self, direction: Shutdown) -> io::Result<()> {
        self.io.shutdown(direction)
    }
}

fn io_buf_slice(buf: &impl IoBuf) -> &[u8] {
    // have to trust IoBuf trait impl to provide valid pointer and initialized length.
    unsafe { slice::from_raw_parts(buf.stable_ptr(), buf.bytes_init()) }
}

fn io_buf_slice_mut(buf: &mut impl IoBufMut) -> &mut [u8] {
    // have to trust IoBufMut trait impl to provide valid pointer and total length.
    unsafe { slice::from_raw_parts_mut(buf.stable_mut_ptr(), buf.bytes_total()) }
}
