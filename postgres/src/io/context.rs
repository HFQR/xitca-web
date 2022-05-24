use std::{collections::VecDeque, io::IoSlice, mem::MaybeUninit};

use xitca_io::bytes::{Buf, BytesMut};
use xitca_unsafe_collection::{array_queue::ArrayQueue, uninit::PartialInit};

use crate::{
    error::Error,
    request::Request,
    response::{ResponseMessage, ResponseSender},
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

    pub(super) const fn req_is_full(&self) -> bool {
        self.req.is_full()
    }

    pub(super) const fn req_is_empty(&self) -> bool {
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

    pub(super) async fn try_response(&mut self) -> Result<(), Error> {
        while let Some(res) = ResponseMessage::try_from_buf(&mut self.buf)? {
            match res {
                ResponseMessage::Normal { buf, complete } => {
                    // TODO: unbounded or now_or_never?
                    let _ = self
                        .res
                        .front_mut()
                        .expect("Out of bound must not happen")
                        .send(buf)
                        .await;

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
    pub(super) async fn try_response_once(&mut self) -> Result<bool, Error> {
        match ResponseMessage::try_from_buf(&mut self.buf)? {
            Some(ResponseMessage::Normal { buf, complete }) => {
                let _ = self
                    .res
                    .front_mut()
                    .expect("Out of bound must not happen")
                    .send(buf)
                    .await;
                if complete {
                    let _ = self.res.pop_front();
                }
                Ok(true)
            }
            Some(ResponseMessage::Async(_)) => unreachable!("async message handling is not implemented"),
            None => Ok(false),
        }
    }

    // fill given &mut [MaybeUninit<IoSlice>] with Request's msg bytes and return the initialized
    // slice.
    pub(super) fn chunks_vectored<'a>(&'a self, dst: &'a mut [MaybeUninit<IoSlice<'a>>]) -> &[IoSlice<'a>] {
        dst.init_from(self.req.iter())
            .into_init_with(|req| IoSlice::new(req.msg.chunk()))
    }

    // remove requests that are sent and move the their tx fields to res queue.
    pub(super) fn advance(&mut self, mut cnt: usize) {
        while cnt > 0 {
            {
                let front = self
                    .req
                    .front_mut()
                    .expect("Context::advance MUST be called when request is not empty");

                let rem = front.msg.remaining();

                if rem > cnt {
                    // partial message sent. advance and return.
                    front.msg.advance(cnt);
                    return;
                } else {
                    cnt -= rem;
                }
            }

            // SAFETY:
            //
            // This operation is safe. ArrayQueue::front_mut method was called at the beginning.
            // When this line reached there must be at least one item in queue.
            let req = unsafe { self.req.pop_front().unwrap_unchecked() };

            self.res.push_back(req.tx);
        }
    }
}
