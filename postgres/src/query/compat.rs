use core::{
    pin::Pin,
    task::{ready, Context, Poll},
};

use std::sync::Arc;

use futures_core::Stream;
use postgres_protocol::message::backend;

use crate::{column::Column, driver::codec::Response, error::Error, row::compat::RowOwned};

use super::RowStream;

pub struct RowStreamOwned {
    pub(crate) res: Response,
    pub(crate) col: Arc<[Column]>,
}

impl From<RowStream<'_>> for RowStreamOwned {
    fn from(RowStream { res, col, .. }: RowStream<'_>) -> Self {
        Self {
            res,
            col: Arc::from(col),
        }
    }
}

impl Stream for RowStreamOwned {
    type Item = Result<RowOwned, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            match ready!(this.res.poll_recv(cx))? {
                backend::Message::DataRow(body) => return Poll::Ready(Some(RowOwned::try_new(this.col.clone(), body))),
                backend::Message::BindComplete
                | backend::Message::EmptyQueryResponse
                | backend::Message::CommandComplete(_)
                | backend::Message::PortalSuspended => {}
                backend::Message::ReadyForQuery(_) => return Poll::Ready(None),
                _ => return Poll::Ready(Some(Err(Error::unexpected()))),
            }
        }
    }
}
