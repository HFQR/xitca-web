use core::ops::Range;

use fallible_iterator::FallibleIterator;
use postgres_protocol::message::backend;

use crate::{
    column::Column,
    driver::codec::Response,
    error::Error,
    iter::AsyncLendingIterator,
    prepare::Prepare,
    row::{Row, RowSimple},
    types::Type,
};

#[derive(Debug)]
pub struct GenericRowStream<C> {
    pub(crate) res: Response,
    pub(crate) col: C,
    pub(crate) ranges: Vec<Range<usize>>,
}

impl<C> GenericRowStream<C> {
    pub(crate) fn new(res: Response, col: C) -> Self {
        Self {
            res,
            col,
            ranges: Vec::new(),
        }
    }
}

/// A stream of table rows.
pub type RowStream<'a> = GenericRowStream<&'a [Column]>;

impl<'a> AsyncLendingIterator for RowStream<'a> {
    type Ok<'i>
        = Row<'i>
    where
        Self: 'i;
    type Err = Error;

    async fn try_next(&mut self) -> Result<Option<Self::Ok<'_>>, Self::Err> {
        loop {
            match self.res.recv().await? {
                backend::Message::DataRow(body) => return Row::try_new(self.col, body, &mut self.ranges).map(Some),
                backend::Message::BindComplete
                | backend::Message::EmptyQueryResponse
                | backend::Message::CommandComplete(_)
                | backend::Message::PortalSuspended => {}
                backend::Message::ReadyForQuery(_) => return Ok(None),
                _ => return Err(Error::unexpected()),
            }
        }
    }
}

/// A stream of simple query results.
pub type RowSimpleStream = GenericRowStream<Vec<Column>>;

impl AsyncLendingIterator for RowSimpleStream {
    type Ok<'i>
        = RowSimple<'i>
    where
        Self: 'i;
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

pub struct RowStreamGuarded<'a, C> {
    pub(crate) res: Response,
    pub(crate) col: Vec<Column>,
    pub(crate) ranges: Vec<Range<usize>>,
    pub(crate) cli: &'a C,
}

impl<'a, C> RowStreamGuarded<'a, C> {
    pub(crate) fn new(res: Response, cli: &'a C) -> Self {
        Self {
            res,
            col: Vec::new(),
            ranges: Vec::new(),
            cli,
        }
    }
}

impl<C> AsyncLendingIterator for RowStreamGuarded<'_, C>
where
    C: Prepare + Sync,
{
    type Ok<'i>
        = Row<'i>
    where
        Self: 'i;
    type Err = Error;

    async fn try_next(&mut self) -> Result<Option<Self::Ok<'_>>, Self::Err> {
        loop {
            match self.res.recv().await? {
                backend::Message::RowDescription(body) => {
                    let mut it = body.fields();
                    while let Some(field) = it.next()? {
                        let ty = self.cli._get_type(field.type_oid()).await?;
                        self.col.push(Column::new(field.name(), ty));
                    }
                }
                backend::Message::DataRow(body) => return Row::try_new(&self.col, body, &mut self.ranges).map(Some),
                backend::Message::ParseComplete
                | backend::Message::BindComplete
                | backend::Message::ParameterDescription(_)
                | backend::Message::EmptyQueryResponse
                | backend::Message::CommandComplete(_)
                | backend::Message::PortalSuspended => {}
                backend::Message::NoData | backend::Message::ReadyForQuery(_) => return Ok(None),
                _ => return Err(Error::unexpected()),
            }
        }
    }
}
