use core::{
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures_core::stream::Stream;
use postgres_protocol::message::{backend, frontend};
use postgres_types::{BorrowToSql, IsNull};
use xitca_io::bytes::BytesMut;

use crate::{
    client::Client, column::Column, error::Error, response::Response, row::Row, slice_iter, statement::Statement, ToSql,
};

impl Client {
    #[inline]
    pub async fn query<'a>(&self, stmt: &'a Statement, params: &[&(dyn ToSql + Sync)]) -> Result<RowStream<'a>, Error> {
        self.query_raw(stmt, slice_iter(params)).await
    }

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

    pub async fn execute<I>(&self, stmt: &Statement, params: I) -> Result<u64, Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        let mut res = self.bind(stmt, params).await?;
        let mut rows = 0;
        loop {
            match res.recv().await? {
                backend::Message::DataRow(_) => {}
                backend::Message::CommandComplete(body) => {
                    rows = extract_row_affected(&body)?;
                }
                backend::Message::EmptyQueryResponse => rows = 0,
                backend::Message::ReadyForQuery(_) => return Ok(rows),
                _ => return Err(Error::UnexpectedMessage),
            }
        }
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
            _ => Err(Error::ToDo),
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

/// Extract the number of rows affected from [`CommandCompleteBody`].
pub(super) fn extract_row_affected(body: &backend::CommandCompleteBody) -> Result<u64, Error> {
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
            match ready!(this.res.poll_recv(cx)?) {
                backend::Message::DataRow(body) => return Poll::Ready(Some(Ok(Row::try_new(this.col, body)?))),
                backend::Message::EmptyQueryResponse
                | backend::Message::CommandComplete(_)
                | backend::Message::PortalSuspended => {}
                backend::Message::ReadyForQuery(_) => return Poll::Ready(None),
                _ => return Poll::Ready(Some(Err(Error::UnexpectedMessage))),
            }
        }
    }
}
