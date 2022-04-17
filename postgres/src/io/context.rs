use std::collections::VecDeque;

use xitca_io::bytes::BytesMut;

use crate::{
    error::Error,
    request::Request,
    response::{ResponseMessage, ResponseSender},
};

pub(super) struct Context {
    res: VecDeque<ResponseSender>,
    pub(super) write_buf: BytesMut,
    pub(super) read_buf: BytesMut,
}

impl Context {
    pub(super) fn new() -> Self {
        Self {
            res: VecDeque::new(),
            write_buf: BytesMut::new(),
            read_buf: BytesMut::new(),
        }
    }

    pub(super) fn req_is_empty(&self) -> bool {
        self.write_buf.is_empty()
    }

    pub(super) fn push_req(&mut self, req: Request) {
        self.write_buf.extend_from_slice(&req.msg);
        self.res.push_back(req.tx);
    }

    pub(super) fn clear(&mut self) {
        self.res.clear();
        self.write_buf.clear();
        self.read_buf.clear();
    }

    pub(super) fn try_response(&mut self) -> Result<(), Error> {
        while let Some(res) = ResponseMessage::try_from_buf(&mut self.read_buf)? {
            match res {
                ResponseMessage::Normal { buf, complete } => {
                    let _ = self.res.front().expect("Out of bound must not happen").send(buf);

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
        match ResponseMessage::try_from_buf(&mut self.read_buf)? {
            Some(ResponseMessage::Normal { buf, complete }) => {
                let _ = self.res.front().expect("Out of bound must not happen").send(buf);
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
