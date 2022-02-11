use std::{collections::VecDeque, io};

use xitca_io::{bytes::Buf, io::AsyncIo};

use super::{
    error::Error,
    message::{Request, Response},
};

pub(crate) struct Context<const SIZE: usize> {
    req: VecDeque<Request>,
    res: VecDeque<Response>,
}

impl<const SIZE: usize> Context<SIZE> {
    pub(crate) fn new() -> Self {
        Self {
            req: VecDeque::with_capacity(SIZE),
            res: VecDeque::with_capacity(SIZE),
        }
    }

    pub(crate) fn req_len(&self) -> usize {
        self.req.len()
    }

    pub(crate) fn push_req(&mut self, req: Request) {
        self.req.push_back(req)
    }

    pub(crate) fn try_write_io<Io>(&mut self, io: &mut Io) -> Result<(), Error>
    where
        Io: AsyncIo,
    {
        while self.req.len() > 0 {
            let mut iovs = [io::IoSlice::new(&[]); SIZE];
            let len = self.chunks_vectored(&mut iovs);
            match io.try_write_vectored(&iovs[..len]) {
                Ok(0) => return Err(Error::ConnectionClosed),
                Ok(n) => {
                    // self.reqs.advance(n)
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }

        Ok(())
    }

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
}
