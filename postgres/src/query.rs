use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::{ready, stream::Stream};
use postgres_protocol::message::{backend, frontend};
use postgres_types::{BorrowToSql, IsNull};
use xitca_io::bytes::{Bytes, BytesMut};

use super::{client::Client, error::Error, response::Response, row::Row, statement::Statement};

impl Client {
    pub async fn query<'a, P, I>(&self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        P: BorrowToSql,
        I: IntoIterator<Item = P>,
        I::IntoIter: ExactSizeIterator,
    {
        let buf = encode(self, stmt, params)?;
        let mut res = self.send(buf).await?;

        match res.recv().await? {
            backend::Message::BindComplete => {}
            _ => return Err(Error::ToDo),
        }

        Ok(RowStream::new(stmt, res))
    }
}

fn encode<P, I>(client: &Client, stmt: &Statement, params: I) -> Result<Bytes, Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = P>,
    I::IntoIter: ExactSizeIterator,
{
    client.with_buf(|buf| {
        encode_bind(stmt, params, "", buf)?;
        frontend::execute("", 0, buf).map_err(|_| Error::ToDo)?;
        frontend::sync(buf);
        Ok(buf.split().freeze())
    })
}

pub fn encode_bind<P, I>(stmt: &Statement, params: I, portal: &str, buf: &mut BytesMut) -> Result<(), Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = P>,
    I::IntoIter: ExactSizeIterator,
{
    let params = params.into_iter();

    assert_eq!(
        stmt.params().len(),
        params.len(),
        "expected {} parameters but got {}",
        stmt.params().len(),
        params.len()
    );

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
