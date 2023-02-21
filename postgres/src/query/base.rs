use core::{
    pin::Pin,
    task::{ready, Context, Poll},
};
use std::ops::Range;

use futures_core::stream::Stream;
use postgres_protocol::message::{backend, frontend};
use postgres_types::{BorrowToSql, IsNull};
use xitca_io::bytes::BytesMut;

use crate::{
    client::Client,
    column::Column,
    error::Error,
    iter::{slice_iter, AsyncIterator},
    response::Response,
    row::{Row, RowGat},
    statement::Statement,
    ToSql,
};

impl Client {
    /// Executes a statement, returning a vector of the resulting rows.
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the
    /// parameter of the list provided, 1-indexed.
    ///
    /// If the same statement will be repeatedly executed (perhaps with different query parameters),
    /// consider preparing the statement up front with the [Client::prepare] method.
    ///
    /// # Panics
    ///
    /// Panics if given params slice length does not match the length of [Statement::params].
    #[inline]
    pub async fn query<'a>(&self, stmt: &'a Statement, params: &[&(dyn ToSql + Sync)]) -> Result<RowStream<'a>, Error> {
        self.query_raw(stmt, slice_iter(params)).await
    }

    /// # Panics
    ///
    /// Panics if given params' [ExactSizeIterator::len] does not match the length of [Statement::params].
    #[inline]
    pub async fn query_raw<'a, I>(&self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        self.bind(stmt, params).await.map(|res| RowStream {
            col: stmt.columns(),
            res,
        })
    }

    /// GAT(generic associated type) enabled query row stream.
    pub async fn query_raw_gat<'a, I>(&self, stmt: &'a Statement, params: I) -> Result<RowStreamGat<'a>, Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        self.bind(stmt, params).await.map(|res| RowStreamGat {
            col: stmt.columns(),
            res,
            ranges: Vec::new(),
        })
    }

    /// Executes a statement, returning the number of rows modified.
    ///
    /// A statement may contain parameters, specified by `$n`, where `n` is the index of the parameter of the list
    /// provided, 1-indexed.
    ///
    /// If the same statement will be repeatedly executed (perhaps with different query parameters),
    /// consider preparing the statement up front with the [Client::prepare] method.
    ///
    /// If the statement does not modify any rows (e.g. `SELECT`), 0 is returned.
    ///
    /// # Panics
    ///
    /// Panics if given params' [ExactSizeIterator::len] does not match the length of [Statement::params].
    #[inline]
    pub async fn execute(&self, stmt: &Statement, params: &[&(dyn ToSql + Sync)]) -> Result<u64, Error> {
        self.execute_raw(stmt, slice_iter(params)).await
    }

    /// # Panics
    ///
    /// Panics if given params' [ExactSizeIterator::len] does not match the length of [Statement::params].
    pub async fn execute_raw<I>(&self, stmt: &Statement, params: I) -> Result<u64, Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        let res = self.bind(stmt, params).await?;
        res_to_row_affected(res).await
    }

    async fn bind<I>(&self, stmt: &Statement, params: I) -> Result<Response, Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        let buf = encode(self, stmt, params.into_iter())?;
        let mut res = self.send(buf)?;
        match res.recv().await? {
            backend::Message::BindComplete => Ok(res),
            _ => Err(Error::UnexpectedMessage),
        }
    }
}

fn encode<I>(client: &Client, stmt: &Statement, params: I) -> Result<BytesMut, Error>
where
    I: ExactSizeIterator,
    I::Item: BorrowToSql,
{
    assert_eq!(
        stmt.params().len(),
        params.len(),
        "expected {} parameters but got {}",
        stmt.params().len(),
        params.len()
    );

    client.with_buf_fallible(|buf| {
        encode_bind(stmt, params, "", buf)?;
        frontend::execute("", 0, buf).map_err(|_| Error::ToDo)?;
        frontend::sync(buf);
        Ok(buf.split())
    })
}

fn encode_bind<I>(stmt: &Statement, params: I, portal: &str, buf: &mut BytesMut) -> Result<(), Error>
where
    I: ExactSizeIterator,
    I::Item: BorrowToSql,
{
    let mut error_idx = 0;
    let r = frontend::bind(
        portal,
        stmt.name(),
        Some(1),
        params.zip(stmt.params()).enumerate(),
        |(idx, (param, ty)), buf| match param.borrow_to_sql().to_sql_checked(ty, buf) {
            Ok(IsNull::No) => Ok(postgres_protocol::IsNull::No),
            Ok(IsNull::Yes) => Ok(postgres_protocol::IsNull::Yes),
            Err(e) => {
                error_idx = idx;
                Err(e)
            }
        },
        Some(1),
        buf,
    );

    match r {
        Ok(()) => Ok(()),
        Err(frontend::BindError::Conversion(_)) => Err(Error::ToDo),
        Err(frontend::BindError::Serialization(_)) => Err(Error::ToDo),
    }
}

pub(super) async fn res_to_row_affected(mut res: Response) -> Result<u64, Error> {
    let mut rows = 0;
    loop {
        match res.recv().await? {
            backend::Message::RowDescription(_) | backend::Message::DataRow(_) => {}
            backend::Message::CommandComplete(body) => {
                rows = body_to_affected_rows(&body)?;
            }
            backend::Message::EmptyQueryResponse => rows = 0,
            backend::Message::ReadyForQuery(_) => return Ok(rows),
            _ => return Err(Error::UnexpectedMessage),
        }
    }
}

// Extract the number of rows affected.
fn body_to_affected_rows(body: &backend::CommandCompleteBody) -> Result<u64, Error> {
    body.tag()
        .map_err(|_| Error::ToDo)
        .map(|r| r.rsplit(' ').next().unwrap().parse().unwrap_or(0))
}

/// A stream of table rows.
pub struct RowStream<'a> {
    col: &'a [Column],
    res: Response,
}

impl<'a> Stream for RowStream<'a> {
    type Item = Result<Row<'a>, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            match ready!(this.res.poll_recv(cx))? {
                backend::Message::DataRow(body) => return Poll::Ready(Some(Row::try_new(this.col, body))),
                backend::Message::EmptyQueryResponse
                | backend::Message::CommandComplete(_)
                | backend::Message::PortalSuspended => {}
                backend::Message::ReadyForQuery(_) => return Poll::Ready(None),
                _ => return Poll::Ready(Some(Err(Error::UnexpectedMessage))),
            }
        }
    }
}

/// A stream of table rows.
pub struct RowStreamGat<'a> {
    col: &'a [Column],
    res: Response,
    ranges: Vec<Option<Range<usize>>>,
}

impl<'a> AsyncIterator for RowStreamGat<'a> {
    type Item<'i> = Result<RowGat<'i>, Error> where 'a: 'i;

    fn poll_next<'s>(self: Pin<&'s mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item<'s>>>
    where
        'a: 's,
    {
        let this = self.get_mut();
        loop {
            match ready!(this.res.poll_recv(cx))? {
                backend::Message::DataRow(body) => {
                    return Poll::Ready(Some(RowGat::try_new(this.col, body, &mut this.ranges)))
                }
                backend::Message::EmptyQueryResponse
                | backend::Message::CommandComplete(_)
                | backend::Message::PortalSuspended => {}
                backend::Message::ReadyForQuery(_) => return Poll::Ready(None),
                _ => return Poll::Ready(Some(Err(Error::UnexpectedMessage))),
            }
        }
    }
}
