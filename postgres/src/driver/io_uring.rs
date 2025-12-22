use core::async_iter::AsyncIterator;

use std::{io, net::Shutdown};

use postgres_protocol::message::backend;
use xitca_io::{
    bytes::BytesMut,
    io_uring::{AsyncBufRead, AsyncBufWrite, BoundedBuf, write_all},
    net::io_uring::TcpStream,
};

use crate::error::Error;

use super::generic::{DriverRx, GenericDriver, WaitState};

pub struct UringDriver<Io = TcpStream> {
    io: Io,
    read_buf: BytesMut,
    rx: DriverRx,
}

impl UringDriver {
    pub(crate) fn from_tcp(drv: GenericDriver<xitca_io::net::TcpStream>) -> Self {
        let GenericDriver { io, read_buf, rx, .. } = drv;
        Self {
            io: TcpStream::from_std(io.into_std().unwrap()),
            read_buf: read_buf.into_inner(),
            rx,
        }
    }
}

impl<Io> UringDriver<Io>
where
    Io: AsyncBufRead + AsyncBufWrite + 'static,
{
    pub fn into_iter(self) -> impl AsyncIterator<Item = Result<backend::Message, Error>> + use<Io> {
        let Self { io, mut read_buf, rx } = self;

        use std::rc::Rc;

        let io = Rc::new(io);
        let rx = Rc::new(rx);

        let io_write = io.clone();
        let rx_write = rx.clone();

        let write_task = tokio::task::spawn_local(async move {
            loop {
                match rx_write.wait().await {
                    WaitState::WantWrite => {
                        let buf = rx_write.guarded.lock().unwrap().buf.split();
                        let (res, _) = write_all(&*io_write, buf).await;
                        res?;
                    }
                    WaitState::WantClose => return Ok::<_, io::Error>(()),
                }
            }
        });

        async gen move {
            let res_read = loop {
                match rx.try_decode(&mut read_buf) {
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

                let (res, b) = io.read(read_buf.slice(len..)).await;

                read_buf = b.into_inner();

                match res {
                    Ok(0) => break Ok(()),
                    Ok(_) => {}
                    Err(e) => break Err(e),
                }
            };

            let res_write = write_task.await.expect("read task must not panic");

            match (res_read.err(), res_write.err()) {
                (None, None) => {
                    if let Err(e) = io.shutdown(Shutdown::Both) {
                        yield Err(e.into());
                    }
                    return;
                }
                (err_r, err_w) => {
                    yield Err(Error::driver_io(err_r, err_w));
                    return;
                }
            }
        }
    }
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
