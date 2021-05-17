use std::{collections::VecDeque, io};

use actix_server_alt::net::TcpStream;
use bytes::BytesMut;
use tokio::io::Ready;

use crate::flow::HttpFlow;
use crate::h1::{body::RequestBodySender, error::Error};

use super::state::State;

pub(crate) struct Dispatcher<'a, S, X, U> {
    io: TcpStream,
    state: State,
    read_buf: ReadBuffer,
    write_buf: BytesMut,
    queue: VecDeque<usize>,
    body_sender: Option<RequestBodySender>,
    error: Option<Error>,
    flow: &'a HttpFlow<S, X, U>,
}

impl<'a, S, X, U> Dispatcher<'a, S, X, U> {
    pub(crate) fn new(io: TcpStream, flow: &'a HttpFlow<S, X, U>) -> Self {
        Self {
            io,
            state: State::new(),
            read_buf: ReadBuffer::new(),
            write_buf: BytesMut::new(),
            queue: VecDeque::new(),
            body_sender: None,
            error: None,
            flow,
        }
    }

    pub(crate) async fn run(&mut self) -> Result<(), Error> {
        while self.running() {
            let interest = self.state.interest();
            let ready = self.io.ready(interest).await?;

            if ready.is_readable() {
                self.try_read()?;
                self.try_decode()?;
            }
        }

        Ok(())
    }

    fn try_read(&mut self) -> Result<(), Error> {
        self.read_buf.advance(false);

        loop {
            match self.io.try_read_buf(&mut self.read_buf.buf) {
                Ok(0) => {
                    self.set_read_close();
                    return Ok(());
                }
                Ok(_) => self.read_buf.advance(true),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(ref e) if e.kind() == io::ErrorKind::ConnectionReset => {
                    self.set_read_close();
                    return Ok(());
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    fn running(&self) -> bool {
        !(self.state.read_closed() && self.state.write_closed())
    }

    fn try_decode(&mut self) -> Result<(), Error> {
        if self.read_buf.advanced() {}

        Ok(())
    }

    fn set_read_close(&mut self) {
        self.state.set_read_close();
        if let Some(mut body) = self.body_sender.take() {
            body.feed_eof();
        }
    }
}

struct ReadBuffer {
    advanced: bool,
    buf: BytesMut,
}

impl ReadBuffer {
    fn new() -> Self {
        Self {
            advanced: false,
            buf: BytesMut::new(),
        }
    }

    #[inline(always)]
    fn advanced(&self) -> bool {
        self.advanced
    }

    #[inline(always)]
    fn advance(&mut self, advanced: bool) {
        self.advanced = advanced;
    }
}
