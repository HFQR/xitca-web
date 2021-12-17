//! A postgresql client on top of tokio.

mod futures;

pub mod error;

use std::{collections::VecDeque, future::Future, io};

use tokio::sync::mpsc::{channel, Receiver, Sender};
use xitca_io::{io::Interest, net::TcpStream};

use crate::{
    error::Error,
    futures::{never, Select, SelectOutput},
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

        let (tx, rx) = channel(self.backlog);

        let mut receiver = QueryReceiver {
            rx,
            batch_limit: self.batch_limit,
        };

        let mut batched = VecDeque::with_capacity(self.batch_limit);

        let fut = async move {
            loop {
                match receiver
                    .recv(batched.len())
                    .select(stream.ready(Interest::READABLE | Interest::WRITABLE))
                    .await
                {
                    SelectOutput::A(Some(msg)) => {
                        batched.push_back(msg);
                    }
                    SelectOutput::A(None) => break,
                    SelectOutput::B(ready) => {
                        let ready = ready?;

                        if ready.is_readable() {
                            // decode
                        }
                        if ready.is_writable() {}
                    }
                }
            }

            Ok(())
        };

        Ok((Client { tx }, fut))
    }
}

struct QueryReceiver {
    rx: Receiver<Message>,
    batch_limit: usize,
}

impl QueryReceiver {
    async fn recv(&mut self, batched: usize) -> Option<Message> {
        if batched == self.batch_limit {
            never().await
        } else {
            self.rx.recv().await
        }
    }
}

#[derive(Debug)]
pub struct Client {
    tx: Sender<Message>,
}

impl Client {
    pub async fn prepare(&self) -> Result<(), Error> {
        self.tx.send(Message).await?;
        Ok(())
    }
}

pub struct Message;
