use std::{collections::VecDeque, io};

use xitca_io::bytes::{Buf, BytesMut};

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
        self.req.len() == LIMIT
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

    pub(crate) fn clear(&mut self) {
        self.req.clear();
        self.res.clear();
        self.buf.clear();
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
                ResponseMessage::Async(_) => {}
            }
        }

        Ok(())
    }

    // only try parse response for once and return true when success.
    pub(crate) fn try_response_once(&mut self) -> Result<bool, Error> {
        match ResponseMessage::try_from_buf(&mut self.buf)? {
            Some(ResponseMessage::Normal { buf, complete }) => {
                let _ = self.res[0].send(buf);
                if complete {
                    let _ = self.res.pop_front();
                }
                Ok(true)
            }
            Some(ResponseMessage::Async(_)) => unreachable!("async message handling is not implemented"),
            None => Ok(false),
        }
    }

    // fill given &mut [IoSlice] with Request's msg bytes. and return the total number of bytes.
    pub(crate) fn chunks_vectored<'a>(&'a self, dst: &mut [io::IoSlice<'a>]) -> usize {
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
    pub(crate) fn advance(&mut self, mut cnt: usize) {
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
