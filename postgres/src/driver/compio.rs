use core::{async_iter::AsyncIterator, future::poll_fn, pin::pin};

use std::io;

use compio::{
    buf::{BufResult, IntoInner, IoBuf},
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};
use postgres_protocol::message::backend;
use xitca_io::bytes::BytesMut;
use xitca_unsafe_collection::futures::{Select, SelectOutput};

use crate::error::Error;

use super::generic::{DriverRx, GenericDriver, WriteState};

/// postgres driver with compio support enabled
///
/// See [`CompIoDriver::from_tcp`] for usage
pub struct CompIoDriver {
    io: TcpStream,
    read_buf: BytesMut,
    rx: DriverRx,
    read_state: State,
    write_state: State,
}

pub(super) enum State {
    Running,
    Closed(Option<io::Error>),
}

impl CompIoDriver {
    /// convert an existing driver backed by tokio runtime to an driver that can run in compio runtime
    ///
    /// # Examples
    /// ```rust
    /// #![feature(async_iterator)]
    ///
    /// # use xitca_postgres::{iter::AsyncLendingIterator, Execute, ExecuteBlocking};
    /// # type Err = Box<dyn core::error::Error + Send + Sync>;
    /// # fn convert() -> Result<(), Err> {
    /// // start the client and driver in tokio runtime and after that the tokio runtime is safe to be dropped
    /// let (cli, drv) = tokio::runtime::Builder::new_current_thread()
    ///     .enable_all()
    ///     .build()?
    ///     .block_on(xitca_postgres::Postgres::new("postgres://postgres:postgres@localhost:5432").connect())?;
    ///
    /// // start a compio runtime
    /// compio::runtime::RuntimeBuilder::new().build().unwrap().block_on(async move {
    ///     // spawn a task for driver
    ///     let handle = compio::runtime::spawn(async {
    ///         // try to convert the tokio postgres driver to compio uring driver. only work with plain tcp connection for now
    ///         let drv = drv.try_into_tcp().unwrap();
    ///         let drv = xitca_postgres::CompIoDriver::from_tcp(drv)?;
    ///
    ///         // convert driver to async iterator. this step needs nightly rust
    ///         let mut drv = std::pin::pin!(drv.into_async_iter());
    ///
    ///         // poll the async iterator to keep driver running
    ///         use std::async_iter::AsyncIterator;
    ///         while let Some(notify) = std::future::poll_fn(|cx| drv.as_mut().poll_next(cx)).await {
    ///             // handle server side notify
    ///             let _notify = notify;
    ///         }
    ///         std::io::Result::Ok(())
    ///     });
    ///
    ///     // from here client can be used in any async runtime or non async runtime
    ///     {
    ///         let stmt = xitca_postgres::Statement::named("SELECT 1", &[]).execute(&cli).await?;
    ///         let mut res = stmt.query(&cli).await?;
    ///         let row = res.try_next().await?.unwrap();
    ///         let num = row.get::<i32>(0);
    ///         assert_eq!(num, 1);
    ///     }
    ///
    ///     // when client is dropped the driver task would end right after
    ///     drop(cli);
    ///     handle.await.unwrap();
    /// # Ok::<_, Err>(())
    /// })
    /// # }
    ///
    /// ```
    pub fn from_tcp(drv: GenericDriver<xitca_io::net::TcpStream>) -> io::Result<Self> {
        let GenericDriver { io, read_buf, rx, .. } = drv;
        Ok(Self {
            io: TcpStream::from_std(io.into_std()?)?,
            read_buf: read_buf.into_inner(),
            rx,
            read_state: State::Running,
            write_state: State::Running,
        })
    }

    pub fn into_async_iter(mut self) -> impl AsyncIterator<Item = Result<backend::Message, Error>> + use<> {
        let mut read_buf = self.read_buf;

        async gen move {
            let write = || async {
                loop {
                    match self.rx.wait().await {
                        WriteState::WantWrite => {
                            let buf = self.rx.guarded.lock().unwrap().buf.split();
                            let BufResult(res, _) = (&self.io).write_all(buf).await;
                            res?;
                            (&self.io).flush().await?;
                        }
                        _ => return Ok::<_, io::Error>(()),
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

                    let BufResult(res, b) = (&self.io).read(read_buf.slice(len..)).await;

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
                    (State::Closed(None), State::Closed(None)) => {
                        if let Err(e) = (&self.io).shutdown().await {
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
    async fn compio() {
        let (conn, drv) = Postgres::new("postgres://postgres:postgres@localhost:5432")
            .connect()
            .await
            .unwrap();

        let handle = std::thread::spawn(move || {
            compio::runtime::RuntimeBuilder::new().build().unwrap().block_on(async {
                compio::runtime::spawn(async {
                    let drv = drv.try_into_tcp().unwrap();
                    let drv = CompIoDriver::from_tcp(drv).unwrap();
                    let mut drv = pin!(drv.into_async_iter());
                    while poll_fn(|cx| drv.as_mut().poll_next(cx)).await.is_some() {}
                })
                .await
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

        handle.join().unwrap().unwrap();
    }
}
