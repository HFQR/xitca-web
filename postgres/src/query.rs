use std::{
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures_core::stream::Stream;
use postgres_protocol::message::{backend, frontend};
use postgres_types::{BorrowToSql, IsNull, ToSql};
use xitca_io::bytes::BytesMut;

use super::{client::Client, error::Error, response::Response, row::Row, slice_iter, statement::Statement};

impl Client {
    #[inline]
    pub async fn query<'a>(&self, stmt: &'a Statement, params: &[&(dyn ToSql + Sync)]) -> Result<RowStream<'a>, Error> {
        self.query_raw(stmt, slice_iter(params)).await
    }

    pub async fn query_raw<'a, I>(&self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        let buf = encode(self, stmt, params.into_iter())?;
        let mut res = self.send(buf)?;

        match res.recv().await? {
            backend::Message::BindComplete => {}
            _ => return Err(Error::ToDo),
        }

        Ok(RowStream::new(stmt, res))
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

/// A stream of table rows.
pub struct RowStream<'a> {
    stmt: &'a Statement,
    res: Response,
}

impl<'a> RowStream<'a> {
    fn new(stmt: &'a Statement, res: Response) -> Self {
        Self { stmt, res }
    }
}

impl<'a> Stream for RowStream<'a> {
    type Item = Result<Row<'a>, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            match ready!(this.res.poll_recv(cx)?) {
                backend::Message::DataRow(body) => return Poll::Ready(Some(Ok(Row::try_new(this.stmt, body)?))),
                backend::Message::EmptyQueryResponse
                | backend::Message::CommandComplete(_)
                | backend::Message::PortalSuspended => {}
                backend::Message::ReadyForQuery(_) => return Poll::Ready(None),
                _ => return Poll::Ready(Some(Err(Error::ToDo))),
            }
        }
    }
}
