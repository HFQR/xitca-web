use core::{
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures_core::Stream;
use postgres_protocol::message::backend;

use crate::{error::Error, row::RowOwned};

use super::RowStreamOwned;

impl Stream for RowStreamOwned {
    type Item = Result<RowOwned, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            match ready!(this.res.poll_recv(cx))? {
                backend::Message::DataRow(body) => {
                    return Poll::Ready(Some(RowOwned::try_new(this.col.clone(), body, Vec::new())))
                }
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
