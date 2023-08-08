use core::future::Future;

use fallible_iterator::FallibleIterator;
use postgres_protocol::message::{backend, frontend};

use crate::{
    client::Client, column::Column, driver::Response, error::Error, iter::AsyncIterator, row::RowSimple, Type,
};

use super::row_stream::GenericRowStream;

impl Client {
    #[inline]
    pub async fn query_simple(&self, stmt: &str) -> Result<RowSimpleStream, Error> {
        self.simple(stmt).await.map(|res| RowSimpleStream {
            res,
            col: Vec::new(),
            ranges: Vec::new(),
        })
    }

    pub async fn execute_simple(&self, stmt: &str) -> Result<u64, Error> {
        let res = self.simple(stmt).await?;
        super::base::res_to_row_affected(res).await
    }

    async fn simple(&self, stmt: &str) -> Result<Response, Error> {
        let buf = self.try_buf_and_split(|buf| frontend::query(stmt, buf))?;
        self.send(buf).await
    }
}

/// A stream of simple query results.
pub type RowSimpleStream = GenericRowStream<Vec<Column>>;

impl AsyncIterator for RowSimpleStream {
    type Future<'f> = impl Future<Output = Option<Self::Item<'f>>> + Send + 'f where Self: 'f;
    type Item<'i> = Result<RowSimple<'i>, Error> where Self: 'i;

    fn next(&mut self) -> Self::Future<'_> {
        async {
            loop {
                match self.res.recv().await {
                    Ok(msg) => match msg {
                        backend::Message::RowDescription(body) => {
                            match body
                                .fields()
                                // text type is used to match RowSimple::try_get's implementation
                                // where column's pg type is always assumed as Option<&str>.
                                // (no runtime pg type check so this does not really matter. it's
                                // better to keep the type consistent though)
                                .map(|f| Ok(Column::new(f.name(), Type::TEXT)))
                                .collect::<Vec<_>>()
                            {
                                Ok(col) => self.col = col,
                                Err(e) => return Some(Err(e.into())),
                            }
                        }
                        backend::Message::DataRow(body) => {
                            return Some(RowSimple::try_new(&self.col, body, &mut self.ranges));
                        }
                        backend::Message::CommandComplete(_)
                        | backend::Message::EmptyQueryResponse
                        | backend::Message::ReadyForQuery(_) => return None,
                        _ => return Some(Err(Error::UnexpectedMessage)),
                    },
                    Err(e) => return Some(Err(e)),
                }
            }
        }
    }
}
