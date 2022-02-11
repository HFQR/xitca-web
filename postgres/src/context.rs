use std::{collections::VecDeque, io};

use tokio::sync::mpsc::UnboundedSender;
use xitca_io::{bytes::Buf, io::AsyncIo};

use super::{error::Error, message::Request};

pub(crate) struct Context<const SIZE: usize> {
    req: VecDeque<Request>,
    res: VecDeque<UnboundedSender<()>>,
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

    // try write to async io with vectored write enabled.
    pub(crate) fn try_write_io<Io>(&mut self, io: &mut Io) -> Result<(), Error>
    where
        Io: AsyncIo,
    {
        while self.req.len() > 0 {
            let mut iovs = [io::IoSlice::new(&[]); SIZE];
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
