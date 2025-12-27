use core::{async_iter::AsyncIterator, future::poll_fn, mem, pin::pin};

use std::{io, net::Shutdown};

use postgres_protocol::message::backend;
use xitca_io::{bytes::BytesMut, io_uring::write_all, net::io_uring::TcpStream};
use xitca_unsafe_collection::futures::{Select, SelectOutput};

use crate::error::Error;

use super::generic::{DriverRx, GenericDriver, WaitState};

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

impl GenericUringDriver<xitca_io::net::io_uring::TcpStream> {
    pub fn into_iter(mut self) -> impl AsyncIterator<Item = Result<backend::Message, Error>> {
        let read_buf = mem::take(&mut self.read_buf);

        async gen move {
            let write = || async {
                loop {
                    match self.rx.wait().await {
                        WaitState::WantWrite => {
                            let buf = self.rx.guarded.lock().unwrap().buf.split();
                            let (res, _) = write_all(&self.io, buf).await;
                            res?;
                        }
                        WaitState::WantClose => return Ok::<_, io::Error>(()),
                    }
                }
            };

            let read = || async gen {
                let reg = tokio_uring_xitca::buf::fixed::FixedBufRegistry::new(Some(Vec::with_capacity(4096 * 4)));

                if let Err(e) = reg.register() {
                    yield Err(SelectOutput::A(e.into()));
                    return;
                }

                let mut read_buf = read_buf;
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

                    let buf = reg.check_out(0).unwrap();

                    let (res, b) = self.io.read_fixed(buf).await;

                    match res {
                        Ok(0) => return,
                        Ok(n) => read_buf.extend_from_slice(&b[..n]),
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
                    (State::Closed(None), State::Closed(None)) => {
                        if let Err(e) = self.io.shutdown(Shutdown::Both) {
                            yield Err(e.into());
                        }
                        return;
                    }
                    (State::Closed(err_w), State::Closed(err_r)) => {
                        yield Err(Error::driver_io(err_r.take(), err_w.take()));
                        return;
                    }
                };

                match res {
                    SelectOutput::A(Ok(_)) => self.write_state = State::Closed(None),
                    SelectOutput::A(Err(e)) => self.write_state = State::Closed(Some(e)),
                    SelectOutput::B(Some(Ok(msg))) => {
                        yield Ok(msg);
                    }
                    SelectOutput::B(Some(Err(e))) => match e {
                        SelectOutput::A(e) => {
                            yield Err(e);
                            return;
                        }
                        SelectOutput::B(e) => {
                            self.read_state = State::Closed(Some(e));
                        }
                    },
                    SelectOutput::B(None) => {
                        self.read_state = State::Closed(None);
                    }
                }
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
