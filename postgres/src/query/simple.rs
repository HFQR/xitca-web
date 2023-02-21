use core::{
    ops::Range,
    pin::Pin,
    task::{ready, Context, Poll},
};

use std::sync::Arc;

use fallible_iterator::FallibleIterator;
use futures_core::stream::Stream;
use postgres_protocol::message::{backend, frontend};

use crate::{
    client::Client,
    column::Column,
    error::Error,
    iter::AsyncIterator,
    response::Response,
    row::{RowSimple, RowSimpleGat},
    Type,
};

impl Client {
    pub fn query_simple(&self, stmt: &str) -> Result<RowSimpleStream, Error> {
        self.simple(stmt).map(|res| RowSimpleStream { res, columns: None })
    }

    pub fn query_simple_gat(&self, stmt: &str) -> Result<RowSimpleStreamGat, Error> {
        self.simple(stmt).map(|res| RowSimpleStreamGat {
            res,
            columns: None,
            ranges: Vec::new(),
        })
    }

    pub async fn execute_simple(&self, stmt: &str) -> Result<u64, Error> {
        let res = self.simple(stmt)?;
        super::base::res_to_row_affected(res).await
    }

    fn simple(&self, stmt: &str) -> Result<Response, Error> {
        let buf = self.with_buf_fallible(|buf| frontend::query(stmt, buf).map(|_| buf.split()))?;
        self.send(buf)
    }
}

/// A stream of simple query results.
pub struct RowSimpleStream {
    res: Response,
    columns: Option<Arc<[Column]>>,
}

impl Stream for RowSimpleStream {
    type Item = Result<RowSimple, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            match ready!(this.res.poll_recv(cx)?) {
                backend::Message::RowDescription(body) => {
                    let columns = body
                        .fields()
                        .map(|f| Ok(Column::new(f.name(), Type::ANY)))
                        .collect::<Vec<_>>()?
                        .into();
                    this.columns = Some(columns);
                }
                backend::Message::DataRow(body) => {
                    let res = this
                        .columns
                        .as_ref()
                        .ok_or(Error::UnexpectedMessage)
                        .and_then(|col| RowSimple::try_new(col.clone(), body));
                    return Poll::Ready(Some(res));
                }
                backend::Message::CommandComplete(_)
                | backend::Message::EmptyQueryResponse
                | backend::Message::ReadyForQuery(_) => return Poll::Ready(None),
                _ => return Poll::Ready(Some(Err(Error::UnexpectedMessage))),
            }
        }
    }
}

/// A stream of simple query results.
pub struct RowSimpleStreamGat {
    res: Response,
    columns: Option<Vec<Column>>,
    ranges: Vec<Option<Range<usize>>>,
}

impl AsyncIterator for RowSimpleStreamGat {
    type Item<'i> = Result<RowSimpleGat<'i>, Error> where Self: 'i;

    fn poll_next<'s>(self: Pin<&'s mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item<'s>>>
    where
        Self: 's,
    {
        let this = self.get_mut();
        loop {
            match ready!(this.res.poll_recv(cx)?) {
                backend::Message::RowDescription(body) => {
                    let columns = body
                        .fields()
                        .map(|f| Ok(Column::new(f.name(), Type::ANY)))
                        .collect::<Vec<_>>()?;
                    this.columns = Some(columns);
                }
                backend::Message::DataRow(body) => {
                    let res = this
                        .columns
                        .as_ref()
                        .ok_or(Error::UnexpectedMessage)
                        .and_then(|col| RowSimpleGat::try_new(col, body, &mut this.ranges));
                    return Poll::Ready(Some(res));
                }
                backend::Message::CommandComplete(_)
                | backend::Message::EmptyQueryResponse
                | backend::Message::ReadyForQuery(_) => return Poll::Ready(None),
                _ => return Poll::Ready(Some(Err(Error::UnexpectedMessage))),
            }
        }
    }
}
