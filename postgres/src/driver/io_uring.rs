use core::{async_iter::AsyncIterator, future::poll_fn, mem, pin::pin};

use std::{io, net::Shutdown};

use xitca_io::{
    bytes::BytesMut,
    io::{AsyncBufRead, AsyncBufWrite, BoundedBuf},
    net::io_uring::TcpStream,
};
use xitca_unsafe_collection::futures::{Select, SelectOutput};

use crate::{error::Error, protocol::message::backend};

use super::generic::{DriverRx, GenericDriver, WriteState};

pub type UringDriver = GenericUringDriver<TcpStream>;

impl UringDriver {
    pub(crate) fn from_tcp(drv: GenericDriver<xitca_io::net::TcpStream>) -> Self {
        let GenericDriver { io, read_buf, rx, .. } = drv;
        Self {
            io: TcpStream::from_std(io.into_std().unwrap()),
            read_buf: read_buf.into_inner(),
            rx,
            read_state: State::Running,
            write_state: State::Running,
        }
    }
}

pub struct GenericUringDriver<Io> {
    io: Io,
    read_buf: BytesMut,
    rx: DriverRx,
    read_state: State,
    write_state: State,
}

pub(super) enum State {
    Running,
    Closed(Option<io::Error>),
}

// postgres has no half close state so read and write half are closed in pair. the shared state
// stops accepting new request while message already received are still dispatched.
//
// it takes fields instead of &mut self because the read and write future borrow self for the
// whole driver loop.
fn on_close(rx: &DriverRx, read_state: &mut State, write_state: &mut State) {
    rx.close();
    if !matches!(read_state, State::Closed(_)) {
        *read_state = State::Closed(None);
    }
    if !matches!(write_state, State::Closed(_)) {
        *write_state = State::Closed(None);
    }
}

impl<Io> GenericUringDriver<Io>
where
    Io: AsyncBufRead + AsyncBufWrite + 'static,
{
    pub fn into_iter(mut self) -> impl AsyncIterator<Item = Result<backend::Message, Error>> + use<Io> {
        let mut read_buf = mem::take(&mut self.read_buf);

        async gen move {
            let res = {
                let write = || async {
                    loop {
                        match self.rx.wait().await {
                            WriteState::WantWrite => {
                                let buf = self.rx.guarded.lock().unwrap().buf.split();
                                self.io.write_all(buf).await.0?;
                            }
                            _ => return Ok(()),
                        }
                    }
                };

                let read = || async gen {
                    loop {
                        match self.rx.try_decode(&mut read_buf) {
                            Ok(Some(msg)) => {
                                yield Ok(msg);
                                continue;
                            }
                            Err(e) => {
                                yield Err(SelectOutput::A(e));
                                return;
                            }
                            Ok(None) => {}
                        }

                        let len = read_buf.len();

                        read_buf.reserve(4096);

                        let (res, b) = self.io.read(read_buf.slice(len..)).await;

                        read_buf = b.into_inner();

                        match res {
                            Ok(0) => return,
                            Ok(_) => {}
                            Err(e) => {
                                yield Err(SelectOutput::B(e));
                                return;
                            }
                        }
                    }
                };

                let mut read = pin!(read());
                let mut write = pin!(write());

                loop {
                    let res = match (&mut self.write_state, &mut self.read_state) {
                        (State::Running, State::Running) => {
                            write.as_mut().select(poll_fn(|cx| read.as_mut().poll_next(cx))).await
                        }
                        (State::Running, _) => SelectOutput::A(write.as_mut().await),
                        (_, State::Running) => SelectOutput::B(poll_fn(|cx| read.as_mut().poll_next(cx)).await),
                        (State::Closed(None), State::Closed(None)) => break Ok(()),
                        (State::Closed(err_w), State::Closed(err_r)) => {
                            break Err(Error::driver_io(err_r.take(), err_w.take()));
                        }
                    };

                    match res {
                        // client asked for shutdown. keep draining read until remote closes.
                        SelectOutput::A(Ok(_)) => self.write_state = State::Closed(None),
                        SelectOutput::A(Err(e)) => {
                            self.write_state = State::Closed(Some(e));
                            on_close(&self.rx, &mut self.read_state, &mut self.write_state);
                        }
                        SelectOutput::B(Some(Ok(msg))) => yield Ok(msg),
                        SelectOutput::B(Some(Err(e))) => match e {
                            SelectOutput::A(e) => {
                                self.rx.close();
                                break Err(e);
                            }
                            SelectOutput::B(e) => {
                                self.read_state = State::Closed(Some(e));
                                on_close(&self.rx, &mut self.read_state, &mut self.write_state);
                            }
                        },
                        SelectOutput::B(None) => {
                            self.read_state = State::Closed(None);
                            on_close(&self.rx, &mut self.read_state, &mut self.write_state);
                        }
                    }
                }
            };

            self.rx.shutdown();

            let _ = self.io.shutdown(Shutdown::Both).await;

            if let Err(err) = res {
                yield Err(err);
            }
        }
    }
}

#[cfg(not(feature = "tls"))]
#[cfg(test)]
mod test {
    use core::{future::poll_fn, pin::pin};

    use crate::{Execute, Postgres, Statement, iter::AsyncLendingIterator};

    use super::*;

    #[tokio::test]
    async fn io_uring_drv() {
        let (conn, drv) = Postgres::new("postgres://postgres:postgres@localhost:5432")
            .connect()
            .await
            .unwrap();

        let handle = std::thread::spawn(move || {
            tokio_uring_xitca::start(async move {
                let mut drv = pin!(drv.try_into_uring().unwrap().into_iter());
                while poll_fn(|cx| drv.as_mut().poll_next(cx)).await.is_some() {}
            })
        });

        let num = Statement::named("SELECT 1", &[])
            .execute(&conn)
            .await
            .unwrap()
            .query(&conn)
            .await
            .unwrap()
            .try_next()
            .await
            .unwrap()
            .unwrap()
            .get::<i32>(0);

        assert_eq!(num, 1);

        drop(conn);

        handle.join().unwrap()
    }
}

#[cfg(test)]
mod backpressure_test {
    use core::{
        cell::{Cell, RefCell},
        future::Future,
        task::Poll,
    };

    use std::{io::Write, rc::Rc};

    use xitca_io::io::BoundedBufMut;

    use crate::{driver::generic::DriverTx, error::DriverBusy};

    use super::*;

    #[derive(Clone)]
    struct MockIo(Rc<MockState>);

    struct MockState {
        read: tokio::net::TcpStream,
        budget: Cell<usize>,
        failure: Cell<Option<io::ErrorKind>>,
        written: RefCell<Vec<u8>>,
    }

    impl AsyncBufRead for MockIo {
        async fn read<B: BoundedBufMut>(&self, buf: B) -> (io::Result<usize>, B) {
            self.0.read.read(buf).await
        }
    }

    impl AsyncBufWrite for MockIo {
        async fn write<B: BoundedBuf>(&self, buf: B) -> (io::Result<usize>, B) {
            let res = poll_fn(|_| {
                if let Some(kind) = self.0.failure.take() {
                    return Poll::Ready(if kind == io::ErrorKind::WriteZero {
                        Ok(0)
                    } else {
                        Err(kind.into())
                    });
                }
                let n = self.0.budget.get().min(buf.bytes_init());
                if n == 0 {
                    return Poll::Pending;
                }
                self.0.budget.set(self.0.budget.get() - n);
                self.0.written.borrow_mut().extend_from_slice(&buf.chunk()[..n]);
                Poll::Ready(Ok(n))
            })
            .await;
            (res, buf)
        }

        async fn shutdown(self, _: Shutdown) -> io::Result<()> {
            Ok(())
        }
    }

    async fn driver(limit: usize) -> (GenericUringDriver<MockIo>, DriverTx, MockIo, std::net::TcpStream) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let tcp = xitca_io::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (drv, tx) = GenericDriver::new(tcp, limit);
        let (peer, _) = listener.accept().unwrap();
        let GenericDriver { io, read_buf, rx, .. } = drv;
        let io = MockIo(Rc::new(MockState {
            read: tokio::net::TcpStream::from_std(io.into_std().unwrap()).unwrap(),
            budget: Cell::new(0),
            failure: Cell::new(None),
            written: RefCell::new(Vec::new()),
        }));
        let drv = GenericUringDriver {
            io: io.clone(),
            read_buf: read_buf.into_inner(),
            rx,
            read_state: State::Running,
            write_state: State::Running,
        };
        (drv, tx, io, peer)
    }

    fn request(tx: &DriverTx, bytes: &[u8]) -> Result<(), Error> {
        tx.try_send(|buf| {
            buf.extend_from_slice(bytes);
            Ok(())
        })
        .map(|_| ())
    }

    #[tokio::test]
    async fn request_capacity_follows_responses_across_partial_writes() {
        let (drv, tx, io, mut peer) = driver(2).await;
        tx.send_one_way_unbounded(|buf| {
            buf.extend_from_slice(b"cleanup");
            Ok(())
        })
        .unwrap();
        request(&tx, b"first").unwrap();
        request(&tx, b"second").unwrap();

        let mut iter = pin!(drv.into_iter());
        assert!(
            poll_fn(|cx| Poll::Ready(iter.as_mut().poll_next(cx)))
                .await
                .is_pending()
        );

        // Writes retain both request slots, including the fully written first request.
        io.0.budget.set(b"cleanupfirsts".len());
        assert!(
            poll_fn(|cx| Poll::Ready(iter.as_mut().poll_next(cx)))
                .await
                .is_pending()
        );
        assert!(
            request(&tx, b"full")
                .unwrap_err()
                .downcast_ref::<DriverBusy>()
                .is_some()
        );

        io.0.budget.set(b"econd".len());
        assert!(
            poll_fn(|cx| Poll::Ready(iter.as_mut().poll_next(cx)))
                .await
                .is_pending()
        );
        assert!(
            request(&tx, b"full")
                .unwrap_err()
                .downcast_ref::<DriverBusy>()
                .is_some()
        );

        assert_eq!(&*io.0.written.borrow(), b"cleanupfirstsecond");

        // A real read completion routes ReadyForQuery and returns just the first slot.
        peer.write_all(b"Z\0\0\0\x05I").unwrap();
        assert!(
            tokio::time::timeout(
                core::time::Duration::from_millis(100),
                poll_fn(|cx| iter.as_mut().poll_next(cx))
            )
            .await
            .is_err()
        );
        request(&tx, b"third").unwrap();
        assert!(
            request(&tx, b"full")
                .unwrap_err()
                .downcast_ref::<DriverBusy>()
                .is_some()
        );

        io.0.budget.set(b"third".len());
        assert!(
            poll_fn(|cx| Poll::Ready(iter.as_mut().poll_next(cx)))
                .await
                .is_pending()
        );
        peer.write_all(b"Z\0\0\0\x05IZ\0\0\0\x05I").unwrap();
        assert!(
            tokio::time::timeout(
                core::time::Duration::from_millis(100),
                poll_fn(|cx| iter.as_mut().poll_next(cx))
            )
            .await
            .is_err()
        );
        request(&tx, b"fourth").unwrap();
        request(&tx, b"fifth").unwrap();
        assert!(
            request(&tx, b"full")
                .unwrap_err()
                .downcast_ref::<DriverBusy>()
                .is_some()
        );
    }

    #[tokio::test]
    async fn write_failure_releases_full_queue_waiter() {
        for kind in [io::ErrorKind::ConnectionReset, io::ErrorKind::WriteZero] {
            let (drv, tx, io, _peer) = driver(1).await;
            request(&tx, b"request").unwrap();
            let mut waiting = pin!(tx.send(|_| Err::<(), _>(Error::todo())));
            assert!(poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx))).await.is_pending());

            io.0.failure.set(Some(kind));
            let mut iter = pin!(drv.into_iter());
            let Some(Err(err)) = poll_fn(|cx| iter.as_mut().poll_next(cx)).await else {
                panic!("expected a write error");
            };
            assert_eq!(err.downcast_ref::<io::Error>().unwrap().kind(), kind);
            let err = tokio::time::timeout(core::time::Duration::from_secs(5), waiting)
                .await
                .expect("queue waiter remained blocked")
                .unwrap_err();
            assert!(err.is_driver_down());
            assert!(request(&tx, b"closed").unwrap_err().is_driver_down());
        }
    }
}
