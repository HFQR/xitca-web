use std::collections::VecDeque;

use xitca_io::bytes::BytesMut;

use crate::{
    error::Error,
    response::{ResponseMessage, ResponseSender},
};

pub(super) struct Context<const LIMIT: usize> {
    res: VecDeque<ResponseSender>,
    buf: BytesMut,
}

impl<const LIMIT: usize> Context<LIMIT> {
    pub(super) fn new() -> Self {
        Self {
            res: VecDeque::with_capacity(LIMIT * 2),
            buf: BytesMut::new(),
        }
    }

    pub(super) fn clear(&mut self) {
        self.res.clear();
        self.buf.clear();
    }

    pub(super) fn buf_mut(&mut self) -> &mut BytesMut {
        &mut self.buf
    }

    pub(super) fn add_pending_res(&mut self, tx: ResponseSender) {
        self.res.push_back(tx);
    }

    pub(super) fn try_response(&mut self) -> Result<(), Error> {
        while let Some(res) = ResponseMessage::try_from_buf(&mut self.buf)? {
            match res {
                ResponseMessage::Normal { buf, complete } => {
                    // TODO: unbounded?
                    self.res
                        .front_mut()
                        .expect("Out of bound must not happen")
                        .try_send(buf)
                        .expect("Response channel is overflown.");

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
                self.res
                    .front_mut()
                    .expect("Out of bound must not happen")
                    .try_send(buf)
                    .expect("Response channel is overflown.");

                if complete {
                    let _ = self.res.pop_front();
                }
                Ok(true)
            }
            Some(ResponseMessage::Async(_)) => unreachable!("async message handling is not implemented"),
            None => Ok(false),
        }
    }
}
