use alloc::collections::VecDeque;

use crate::{
    error::Error,
    response::{ResponseMessage, ResponseSender},
};

use super::buffered::PagedBytesMut;

pub(super) struct Context {
    concurrent_res: VecDeque<ResponseSender>,
}

impl Context {
    pub(super) fn new() -> Self {
        Self {
            concurrent_res: VecDeque::new(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.concurrent_res.is_empty()
    }

    pub(super) fn push_concurrent_req(&mut self, tx: ResponseSender) {
        self.concurrent_res.push_back(tx);
    }

    pub(super) fn try_decode(&mut self, buf: &mut PagedBytesMut) -> Result<(), Error> {
        while let Some(res) = ResponseMessage::try_from_buf(buf)? {
            if let ResponseMessage::Normal { buf, complete } = res {
                let _ = self
                    .concurrent_res
                    .front_mut()
                    .expect("Out of bound must not happen")
                    .send(buf);

                if complete {
                    let _ = self.concurrent_res.pop_front();
                }
            }
        }

        Ok(())
    }
}
