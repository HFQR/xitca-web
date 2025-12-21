use core::{async_iter::AsyncIterator, future::poll_fn, mem, pin::pin};

use postgres_protocol::message::backend;
use xitca_io::{
    bytes::BytesMut,
    io_uring::{BoundedBuf, write_all},
    net::io_uring::TcpStream,
};
use xitca_unsafe_collection::futures::{Select, SelectOutput};

use crate::error::Error;

use super::{
    codec::ResponseMessage,
    generic::{DriverRx, GenericDriver, WaitState},
};

pub struct UringDriver {
    io: TcpStream,
    read_buf: BytesMut,
    rx: DriverRx,
}

impl UringDriver {
    pub(super) fn from_tcp(drv: GenericDriver<xitca_io::net::TcpStream>) -> Self {
        let GenericDriver { io, read_buf, rx, .. } = drv;
        Self {
            io: TcpStream::from_std(io.into_std().unwrap()),
            read_buf: read_buf.into_inner(),
            rx,
        }
    }

    pub fn into_iter(mut self) -> impl AsyncIterator<Item = Result<backend::Message, Error>> + use<> {
        let mut read_buf = mem::take(&mut self.read_buf);

        async gen move {
            let write = || async {
                loop {
                    match self.rx.wait().await {
                        WaitState::WantWrite => {
                            let buf = self.rx.guarded.lock().unwrap().buf.split();
                            let (res, _) = write_all(&self.io, buf).await;
                            res?;
                        }
                        WaitState::WantClose => return Ok(()),
                    }
                }
            };

            let read = || async gen {
                loop {
                    match try_decode(&self.rx, &mut read_buf) {
                        Ok(Some(msg)) => {
                            yield Ok(msg);
                            continue;
                        }
                        Err(e) => {
                            yield Err(e);
                            return;
                        }
                        Ok(None) => {}
                    }

                    let len = read_buf.len();

                    if len == read_buf.capacity() {
                        read_buf.reserve(4096);
                    }

                    let (res, b) = self.io.read(read_buf.slice(len..)).await;

                    read_buf = b.into_inner();

                    match res {
                        Ok(0) => return,
                        Ok(_) => {}
                        Err(e) => {
                            yield Err(e.into());
                            return;
                        }
                    }
                }
            };

            let mut read = pin!(read());
            let mut write = pin!(write());

            let mut read_err: Option<Error> = None;
            let mut write_err: Option<Error> = None;

            loop {
                match (write.as_mut()).select(poll_fn(|cx| read.as_mut().poll_next(cx))).await {
                    SelectOutput::A(Ok(_)) => break,
                    SelectOutput::A(Err(e)) => {
                        write_err = Some(e);
                        break;
                    }
                    SelectOutput::B(Some(Ok(msg))) => {
                        yield Ok(msg);
                    }
                    SelectOutput::B(Some(Err(e))) => {
                        read_err = Some(e);
                        if let Err(e) = write.await {
                            write_err = Some(e);
                        }
                        break;
                    }
                    SelectOutput::B(None) => break,
                }
            }

            while let Some(res) = poll_fn(|cx| read.as_mut().poll_next(cx)).await {
                match res {
                    Ok(msg) => yield Ok(msg),
                    Err(e) => {
                        read_err = Some(e);
                        break;
                    }
                }
            }

            if read_err.is_some() || write_err.is_some() {
                // yield Err(Error::driver_io(read_err, write_err));
                return;
            }

            if let Err(e) = self.io.shutdown(std::net::Shutdown::Both) {
                yield Err(e.into());
            }
        }
    }
}

fn try_decode(rx: &DriverRx, read_buf: &mut BytesMut) -> Result<Option<backend::Message>, Error> {
    let mut inner = rx.guarded.lock().unwrap();
    while let Some(res) = ResponseMessage::try_from_buf(read_buf)? {
        match res {
            ResponseMessage::Normal(mut msg) => {
                let complete = msg.complete();
                let _ = inner.res.front_mut().ok_or_else(|| msg.parse_error())?.send(msg);
                if complete {
                    inner.res.pop_front();
                }
            }
            ResponseMessage::Async(msg) => return Ok(Some(msg)),
        }
    }
    Ok(None)
}

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
            tokio_uring::start(async move {
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
