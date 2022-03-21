use std::{collections::VecDeque, io};

use xitca_io::{
    bytes::{Buf, BytesMut},
    io::AsyncIo,
};

use super::{
    error::Error,
    request::Request,
    response::{ResponseMessage, ResponseSender},
};

pub(crate) struct Context<const LIMIT: usize> {
    req: VecDeque<Request>,
    res: VecDeque<ResponseSender>,
    pub(crate) buf: BytesMut,
}

impl<const LIMIT: usize> Context<LIMIT> {
    pub(crate) fn new() -> Self {
        Self {
            req: VecDeque::with_capacity(LIMIT),
            res: VecDeque::with_capacity(LIMIT),
            buf: BytesMut::new(),
        }
    }

    pub(crate) fn req_is_full(&self) -> bool {
        self.req.len() != LIMIT
    }

    pub(crate) fn req_is_empty(&self) -> bool {
        self.req.is_empty()
    }

    pub(crate) fn res_is_empty(&self) -> bool {
        self.res.is_empty()
    }

    pub(crate) fn push_req(&mut self, req: Request) {
        self.req.push_back(req)
    }

    // try read async io until connection error/closed/blocked.
    pub(crate) fn try_read_io<Io>(&mut self, io: &mut Io) -> Result<(), Error>
    where
        Io: AsyncIo,
    {
        assert!(
            !self.res_is_empty(),
            "If server return anything their must at least one response waiting."
        );

        loop {
            match io.try_read_buf(&mut self.buf) {
                Ok(0) => return Err(Error::ConnectionClosed),
                Ok(_) => continue,
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    }

    pub(crate) fn try_response(&mut self) -> Result<(), Error> {
        while let Some(res) = ResponseMessage::try_from_buf(&mut self.buf)? {
            match res {
                ResponseMessage::Normal { buf, complete } => {
                    let _ = self.res[0].send(buf);

                    if complete {
                        let _ = self.res.pop_front();
                    }
                }
                ResponseMessage::Async(_) => todo!(),
            }
        }

        Ok(())
    }

    // try write to async io with vectored write enabled.
    pub(crate) fn try_write_io<Io>(&mut self, io: &mut Io) -> Result<(), Error>
    where
        Io: AsyncIo,
    {
        while !self.req.is_empty() {
            let mut iovs = [io::IoSlice::new(&[]); LIMIT];
            let len = self.chunks_vectored(&mut iovs);
            match io.try_write_vectored(&iovs[..len]) {
                Ok(0) => return Err(Error::ConnectionClosed),
                Ok(n) => self.advance(n),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }

        Ok(())
    }

    // fill given &mut [IoSlice] with Request's msg bytes. and return the total number of bytes.
    fn chunks_vectored<'a>(&'a self, dst: &mut [io::IoSlice<'a>]) -> usize {
        assert!(!dst.is_empty());
        let mut vecs = 0;
        for req in &self.req {
            vecs += req.msg.chunks_vectored(&mut dst[vecs..]);
            if vecs == dst.len() {
                break;
            }
        }
        vecs
    }

    // remove requests that are sent and move the their tx fields to res queue.
    fn advance(&mut self, mut cnt: usize) {
        while cnt > 0 {
            {
                let front = &mut self.req[0];
                let rem = front.msg.remaining();

                if rem > cnt {
                    // partial message sent. advance and return.
                    front.msg.advance(cnt);
                    return;
                } else {
                    cnt -= rem;
                }
            }

            // whole message written. pop the request. drop message and move tx to res queue.
            let req = self.req.pop_front().unwrap();

            self.res.push_back(req.tx);
        }
    }
}
