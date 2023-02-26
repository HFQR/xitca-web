use core::{future::Future, ops::Range};

use fallible_iterator::FallibleIterator;
use postgres_protocol::message::{backend, frontend};

use crate::{
    client::Client, column::Column, error::Error, iter::AsyncIterator, response::Response, row::RowSimple, Type,
};

impl Client {
    #[inline]
    pub fn query_simple(&self, stmt: &str) -> Result<RowSimpleStream, Error> {
        self.simple(stmt).map(|res| RowSimpleStream {
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
        let buf = self.try_encode_with(|buf| frontend::query(stmt, buf))?;
        self.send(buf)
    }
}

/// A stream of simple query results.
pub struct RowSimpleStream {
    res: Response,
    columns: Option<Vec<Column>>,
    ranges: Vec<Option<Range<usize>>>,
}

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
                                Ok(col) => self.columns = Some(col),
                                Err(e) => return Some(Err(e.into())),
                            }
                        }
                        backend::Message::DataRow(body) => {
                            let res = self
                                .columns
                                .as_ref()
                                .ok_or(Error::UnexpectedMessage)
                                .and_then(|col| RowSimple::try_new(col, body, &mut self.ranges));
                            return Some(res);
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
