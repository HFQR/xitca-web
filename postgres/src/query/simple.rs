use fallible_iterator::FallibleIterator;
use postgres_protocol::message::{backend, frontend};
use xitca_io::bytes::BytesMut;

use crate::{
    client::Client, column::Column, driver::codec::Response, error::Error, iter::AsyncLendingIterator, row::RowSimple,
    Type,
};

use super::row_stream::GenericRowStream;

impl Client {
    pub async fn query_simple(&self, stmt: &str) -> Result<RowSimpleStream, Error> {
        let buf = self.try_buf_and_split(|buf| frontend::query(stmt, buf))?;
        self.query_buf_simple(buf).await
    }

    #[inline]
    pub async fn execute_simple(&self, stmt: &str) -> Result<u64, Error> {
        self.encode_send_simple(stmt).await?.try_into_row_affected().await
    }

    pub(crate) async fn encode_send_simple(&self, stmt: &str) -> Result<Response, Error> {
        let buf = self.try_buf_and_split(|buf| frontend::query(stmt, buf))?;
        self.send(buf).await
    }

    pub(crate) async fn query_buf_simple(&self, buf: BytesMut) -> Result<RowSimpleStream, Error> {
        self.send(buf).await.map(|res| RowSimpleStream {
            res,
            col: Vec::new(),
            ranges: Vec::new(),
        })
    }
}

/// A stream of simple query results.
pub type RowSimpleStream = GenericRowStream<Vec<Column>>;

impl AsyncLendingIterator for RowSimpleStream {
    type Ok<'i> = RowSimple<'i> where Self: 'i;
    type Err = Error;

    async fn try_next(&mut self) -> Result<Option<Self::Ok<'_>>, Self::Err> {
        loop {
            match self.res.recv().await? {
                backend::Message::RowDescription(body) => {
                    self.col = body
                        .fields()
                        // text type is used to match RowSimple::try_get's implementation
                        // where column's pg type is always assumed as Option<&str>.
                        // (no runtime pg type check so this does not really matter. it's
                        // better to keep the type consistent though)
                        .map(|f| Ok(Column::new(f.name(), Type::TEXT)))
                        .collect::<Vec<_>>()?;
                }
                backend::Message::DataRow(body) => {
                    return RowSimple::try_new(&self.col, body, &mut self.ranges).map(Some);
                }
                backend::Message::CommandComplete(_)
                | backend::Message::EmptyQueryResponse
                | backend::Message::ReadyForQuery(_) => return Ok(None),
                _ => return Err(Error::unexpected()),
            }
        }
    }
}
