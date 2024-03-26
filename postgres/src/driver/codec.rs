use postgres_protocol::message::backend;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use xitca_io::bytes::BytesMut;

use crate::error::Error;

#[derive(Debug)]
pub(crate) struct ResponseSender {
    tx: UnboundedSender<BytesMut>,
    msg_count: usize,
}

impl ResponseSender {
    fn new(tx: UnboundedSender<BytesMut>, msg_count: usize) -> Self {
        Self { tx, msg_count }
    }

    pub(super) fn send(&mut self, msg: BytesMut) {
        let _ = self.tx.send(msg);
    }

    pub(super) fn complete(&mut self, current_msg_complete: bool) -> bool {
        assert!(self.msg_count > 0);

        if current_msg_complete {
            self.msg_count -= 1;
        }

        self.msg_count == 0
    }
}

// TODO: remove this lint.
#[allow(dead_code)]
pub(super) type ResponseReceiver = UnboundedReceiver<BytesMut>;

#[derive(Debug)]
pub struct Request {
    pub(super) tx: ResponseSender,
    pub(crate) msg: BytesMut,
}

impl Request {
    // a request with multiple response messages from database with msg_count as the count of total
    // number of messages.
    pub(crate) fn new(tx: UnboundedSender<BytesMut>, msg_count: usize, msg: BytesMut) -> Self {
        Self {
            tx: ResponseSender::new(tx, msg_count),
            msg,
        }
    }
}

pub enum ResponseMessage {
    Normal { buf: BytesMut, complete: bool },
    Async(backend::Message),
}

impl ResponseMessage {
    pub(crate) fn try_from_buf(buf: &mut BytesMut) -> Result<Option<Self>, Error> {
        let mut idx = 0;
        let mut complete = false;

        loop {
            let slice = &buf[idx..];
            let Some(header) = backend::Header::parse(slice)? else {
                break;
            };
            let len = header.len() as usize + 1;

            if slice.len() < len {
                break;
            }

            match header.tag() {
                backend::NOTICE_RESPONSE_TAG | backend::NOTIFICATION_RESPONSE_TAG | backend::PARAMETER_STATUS_TAG => {
                    if idx == 0 {
                        // TODO:
                        // PagedBytesMut should never expose underlying BytesMut type as reference.
                        // this is needed because postgres-protocol is an external crate.
                        let message = backend::Message::parse(buf)?.unwrap();
                        return Ok(Some(ResponseMessage::Async(message)));
                    }

                    break;
                }
                tag => {
                    idx += len;
                    if matches!(tag, backend::READY_FOR_QUERY_TAG) {
                        complete = true;
                        break;
                    }
                }
            }
        }

        if idx == 0 {
            Ok(None)
        } else {
            Ok(Some(ResponseMessage::Normal {
                buf: buf.split_to(idx),
                complete,
            }))
        }
    }
}
