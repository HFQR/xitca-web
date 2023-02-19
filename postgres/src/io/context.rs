use std::collections::VecDeque;

use xitca_io::bytes::BytesMut;

use crate::{
    error::Error,
    response::{ResponseMessage, ResponseSender},
};

pub(super) struct Context {
    concurrent_res: VecDeque<ResponseSender>,
    exclusive_res: Option<ResponseSender>,
}

impl Context {
    pub(super) fn new() -> Self {
        Self {
            concurrent_res: VecDeque::new(),
            exclusive_res: None,
        }
    }

    pub(super) fn throttled(&self) -> bool {
        self.exclusive_res.is_some()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.concurrent_res.is_empty() && self.exclusive_res.is_none()
    }

    pub(super) fn push_concurrent_req(&mut self, tx: ResponseSender) {
        debug_assert!(
            self.exclusive_res.is_none(),
            "concurrent request MUST NOT be pushed to Context when there is in process exclusive request"
        );
        self.concurrent_res.push_back(tx);
    }

    pub(super) fn try_decode(&mut self, buf: &mut BytesMut) -> Result<(), Error> {
        while let Some(res) = ResponseMessage::try_from_buf(buf)? {
            if let ResponseMessage::Normal { buf, complete } = res {
                // TODO: unbounded?
                let _ = self
                    .concurrent_res
                    .front_mut()
                    .expect("Out of bound must not happen")
                    .try_send(buf);

                if complete {
                    let _ = self.concurrent_res.pop_front();
                }
            }
        }

        Ok(())
    }
}
