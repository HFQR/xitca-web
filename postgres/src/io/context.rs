use std::collections::VecDeque;

use xitca_io::bytes::BytesMut;

use crate::{
    error::Error,
    request::Request,
    response::{ResponseMessage, ResponseSender},
};

pub(super) struct Context {
    concurrent_res: VecDeque<ResponseSender>,
    exclusive_res: Option<ResponseSender>,
    pub req_buf: BytesMut,
    pub res_buf: BytesMut,
}

impl Context {
    pub(super) fn new() -> Self {
        Self {
            concurrent_res: VecDeque::new(),
            exclusive_res: None,
            req_buf: BytesMut::new(),
            res_buf: BytesMut::new(),
        }
    }

    pub(super) fn throttled(&self) -> bool {
        self.exclusive_res.is_some()
    }

    pub(super) fn req_is_empty(&self) -> bool {
        self.req_buf.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.concurrent_res.clear();
        self.exclusive_res = None;
        self.req_buf.clear();
        self.res_buf.clear();
    }

    pub(super) fn push_concurrent_req(&mut self, Request { msg, tx }: Request) {
        debug_assert!(
            self.exclusive_res.is_none(),
            "concurrent request MUST NOT be pushed to Context when there is in process exclusive request"
        );
        self.req_buf.unsplit(msg);
        if let Some(tx) = tx {
            self.concurrent_res.push_back(tx);
        }
    }

    pub(super) fn handle_response(&mut self) -> Result<(), Error> {
        while let Some(res) = ResponseMessage::try_from_buf(&mut self.res_buf)? {
            match res {
                ResponseMessage::Normal { buf, complete } => {
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
                ResponseMessage::Async(_) => {}
            }
        }

        Ok(())
    }
}
