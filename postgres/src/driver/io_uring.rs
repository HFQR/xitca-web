use core::{async_iter::AsyncIterator, mem};

use std::io;

use postgres_protocol::message::backend;
use xitca_io::{
    bytes::BytesMut,
    io_uring::{BoundedBuf, Slice, write_all},
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
        async gen move {
            let mut read_buf = Some(mem::take(&mut self.read_buf));

            let mut read_task = None;

            loop {
                if let Some(ref mut task) = read_task {
                    match self.rx.wait().select(task).await {
                        SelectOutput::A(WaitState::WantWrite) => {
                            let buf = self.rx.guarded.lock().unwrap().buf.split();
                            let (res, _) = write_all(&self.io, buf).await;
                            match res {
                                Ok(_) => continue,
                                Err(e) => {
                                    yield Err(e.into());
                                    return;
                                }
                            }
                        }
                        SelectOutput::A(WaitState::WantClose) => return,
                        SelectOutput::B::<_, (io::Result<usize>, Slice<BytesMut>)>((res, r_buf)) => {
                            read_task.take();
                            read_buf = Some(r_buf.into_inner());
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
                }

                match try_decode(&self.rx, read_buf.as_mut().unwrap()) {
                    Ok(Some(msg)) => {
                        yield Ok(msg);
                        continue;
                    }
                    Err(e) => {
                        yield Err(e);
                        return;
                    }
                    Ok(None) => {}
                };

                let mut buf = read_buf.take().unwrap();

                let len = buf.len();

                if len == buf.capacity() {
                    buf.reserve(4096);
                }

                read_task.replace(Box::pin(self.io.read(buf.slice(len..))));
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
