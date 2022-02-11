//! A postgresql client on top of tokio.

mod client;
mod futures;
mod message;
mod prepare;
mod statement;

pub mod error;

pub use statement::Statement;

use std::{collections::VecDeque, future::Future, io};

use tokio::sync::mpsc::{channel, Receiver};
use xitca_io::{
    io::{AsyncIo, Interest},
    net::TcpStream,
};

use crate::{
    client::Client,
    error::Error,
    futures::{never, Select, SelectOutput},
    message::{Request, RequestList},
};

#[derive(Debug)]
pub struct Postgres<'a> {
    url: &'a str,
    backlog: usize,
    batch_limit: usize,
}

impl<'a> Postgres<'a> {
    pub fn new(url: &'a str) -> Self {
        Self {
            url,
            backlog: 128,
            batch_limit: 20,
        }
    }

    pub fn backlog(mut self, num: usize) -> Self {
        self.backlog = num;
        self
    }

    pub fn batch_limit(mut self, num: usize) -> Self {
        self.batch_limit = num;
        self
    }

    pub async fn connect(self) -> io::Result<(Client, impl Future<Output = Result<(), crate::error::Error>>)> {
        let stream = TcpStream::connect(self.url).await?;
        Ok(self.start_with_io(stream))
    }

    #[cfg(unix)]
    pub async fn connect_unix(self) -> io::Result<(Client, impl Future<Output = Result<(), crate::error::Error>>)> {
        let stream = xitca_io::net::UnixStream::connect(self.url).await?;
        Ok(self.start_with_io(stream))
    }

    // Start client and io task with given io type that impl `AsyncIo` trait.
    pub fn start_with_io<S>(self, mut io: S) -> (Client, impl Future<Output = Result<(), crate::error::Error>>)
    where
        S: AsyncIo,
    {
        let (tx, rx) = channel(self.backlog);

        let mut receiver = QueryReceiver {
            rx,
            batch_limit: self.batch_limit,
        };

        let mut reqs = crate::message::RequestList::with_capacity(self.batch_limit);

        // let mut res = VecDeque::with_capacity(self.batch_limit);

        let fut = async move {
            loop {
                match receiver
                    .recv(reqs.len())
                    .select(io.ready(Interest::READABLE | Interest::WRITABLE))
                    .await
                {
                    // batch message and keep polling.
                    SelectOutput::A(Some(msg)) => {
                        reqs.push(msg);
                    }
                    // client is gone.
                    SelectOutput::A(None) => break,
                    SelectOutput::B(ready) => {
                        let ready = ready?;

                        if ready.is_readable() {
                            // decode
                        }
                        if ready.is_writable() {
                            // TODO: make batch size const generic and pass it to try_write_io.
                            try_write_io::<_, 20>(&mut io, &mut reqs)?;
                        }
                    }
                }
            }

            Ok(())
        };

        (Client::new(tx), fut)
    }
}

fn try_write_io<Io: AsyncIo, const LEN: usize>(io: &mut Io, reqs: &mut RequestList) -> Result<(), Error> {
    while reqs.len() > 0 {
        let mut iovs = [io::IoSlice::new(&[]); LEN];
        // let len = reqs.chunks_vectored(&mut iovs);
        // match io.try_write_vectored(&iovs[..len]) {
        //     Ok(0) => return Err(Error::ConnectionClosed),
        //     Ok(n) => reqs.advance(n),
        //     Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
        //     Err(e) => return Err(e.into()),
        // }
    }

    Ok(())
}

struct QueryReceiver {
    rx: Receiver<Request>,
    batch_limit: usize,
}

impl QueryReceiver {
    async fn recv(&mut self, batched: usize) -> Option<Request> {
        if batched == self.batch_limit {
            never().await
        } else {
            self.rx.recv().await
        }
    }
}
