use std::{collections::VecDeque, io, mem::MaybeUninit};

use xitca_io::bytes::{Buf, BytesMut};

use crate::{
    error::Error,
    request::Request,
    response::{ResponseMessage, ResponseSender},
    util::queue::ArrayQueue,
};

pub(super) struct Context<const LIMIT: usize> {
    req: ArrayQueue<Request, LIMIT>,
    res: VecDeque<ResponseSender>,
    pub buf: BytesMut,
}

impl<const LIMIT: usize> Context<LIMIT> {
    pub(super) fn new() -> Self {
        Self {
            req: ArrayQueue::new(),
            res: VecDeque::with_capacity(LIMIT * 2),
            buf: BytesMut::new(),
        }
    }

    pub(super) fn req_is_full(&self) -> bool {
        self.req.is_full()
    }

    pub(super) fn req_is_empty(&self) -> bool {
        self.req.is_empty()
    }

    pub(super) fn push_req(&mut self, req: Request) {
        self.req.push_back(req).expect("Out of bound must not happen.");
    }

    pub(super) fn clear(&mut self) {
        self.req.clear();
        self.res.clear();
        self.buf.clear();
    }

    pub(super) fn try_response(&mut self) -> Result<(), Error> {
        while let Some(res) = ResponseMessage::try_from_buf(&mut self.buf)? {
            match res {
                ResponseMessage::Normal { buf, complete } => {
                    let _ = self.res.get(0).expect("Out of bound must not happen").send(buf);

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
    pub(super) fn try_response_once(&mut self) -> Result<bool, Error> {
        match ResponseMessage::try_from_buf(&mut self.buf)? {
            Some(ResponseMessage::Normal { buf, complete }) => {
                let _ = self.res.get(0).expect("Out of bound must not happen").send(buf);
                if complete {
                    let _ = self.res.pop_front();
                }
                Ok(true)
            }
            Some(ResponseMessage::Async(_)) => unreachable!("async message handling is not implemented"),
            None => Ok(false),
        }
    }

    // fill given &mut [MaybeUninit<IoSlice>] with Request's msg bytes
    // and return the total length of written slice.
    pub(super) fn chunks_vectored<'a>(&'a self, dst: &mut [MaybeUninit<io::IoSlice<'a>>]) -> usize {
        assert!(
            self.req.len() <= dst.len(),
            "Request queue length must not bigger than IoSlice buffer queue"
        );

        let mut num = 0;

        for req in self.req.iter() {
            if req.msg.has_remaining() {
                dst[num].write(io::IoSlice::new(req.msg.chunk()));
                num += 1;
            }
        }

        num
    }

    // remove requests that are sent and move the their tx fields to res queue.
    pub(super) fn advance(&mut self, mut cnt: usize) {
        while cnt > 0 {
            {
                let front = self.req.get_mut(0).expect("Out of bound must not happen");
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
