use core::{future::Future, marker::PhantomData, ops::Range};

use std::sync::Arc;

use fallible_iterator::FallibleIterator;
use postgres_protocol::message::backend;

use crate::{
    column::Column,
    driver::codec::Response,
    error::Error,
    iter::AsyncLendingIterator,
    prepare::Prepare,
    row::{marker, Row, RowOwned, RowSimple, RowSimpleOwned},
    types::Type,
};

#[derive(Debug)]
pub struct GenericRowStream<C, M> {
    pub(crate) res: Response,
    pub(crate) col: C,
    pub(crate) ranges: Vec<Range<usize>>,
    pub(crate) _marker: PhantomData<M>,
}

impl<C, M> GenericRowStream<C, M> {
    pub(crate) fn new(res: Response, col: C) -> Self {
        Self {
            res,
            col,
            ranges: Vec::new(),
            _marker: PhantomData,
        }
    }
}

/// A stream of table rows.
pub type RowStream<'a> = GenericRowStream<&'a [Column], marker::Typed>;

impl<'a> AsyncLendingIterator for RowStream<'a> {
    type Ok<'i>
        = Row<'i>
    where
        Self: 'i;
    type Err = Error;

    #[inline]
    fn try_next(&mut self) -> impl Future<Output = Result<Option<Self::Ok<'_>>, Self::Err>> + Send {
        try_next(&mut self.res, self.col, &mut self.ranges)
    }
}

async fn try_next<'r>(
    res: &mut Response,
    col: &'r [Column],
    ranges: &'r mut Vec<Range<usize>>,
) -> Result<Option<Row<'r>>, Error> {
    loop {
        match res.recv().await? {
            backend::Message::DataRow(body) => return Row::try_new(col, body, ranges).map(Some),
            backend::Message::BindComplete
            | backend::Message::EmptyQueryResponse
            | backend::Message::CommandComplete(_)
            | backend::Message::PortalSuspended => {}
            backend::Message::ReadyForQuery(_) => return Ok(None),
            _ => return Err(Error::unexpected()),
        }
    }
}

/// [`RowStream`] with static lifetime
///
/// # Usage
/// due to Rust's GAT limitation [`AsyncLendingIterator`] only works well type that have static lifetime.
/// actively converting a [`RowStream`] to [`RowStreamOwned`] will opens up convenient high level APIs at some additional
/// cost (More memory allocation)
///
/// # Examples
/// ```
/// # use xitca_postgres::{iter::{AsyncLendingIterator, AsyncLendingIteratorExt}, Client, Error, Execute, RowStreamOwned};
/// # async fn collect(cli: Client) -> Result<(), Error> {
/// // prepare statement and query for some users from database.
/// let stmt = cli.prepare("SELECT * FROM users", &[]).await?;
/// let mut stream = stmt.query(&cli)?;
///
/// // assuming users contain name column where it can be parsed to string.
/// // then collecting all user name to a collection
/// let mut strings = Vec::new();
/// while let Some(row) = stream.try_next().await? {
///     strings.push(row.get::<String>("name"));
/// }
///
/// // the same operation with owned row stream can be simplified a bit:
/// let stream = stmt.query(&cli)?;
/// // use extended api on top of AsyncIterator to collect user names to collection
/// let strings_2: Vec<String> = RowStreamOwned::from(stream).map_ok(|row| row.get("name")).try_collect().await?;
///
/// assert_eq!(strings, strings_2);
/// # Ok(())
/// # }
/// ```
pub type RowStreamOwned = GenericRowStream<Arc<[Column]>, marker::Typed>;

impl From<RowStream<'_>> for RowStreamOwned {
    fn from(stream: RowStream<'_>) -> Self {
        Self {
            res: stream.res,
            col: Arc::from(stream.col),
            ranges: stream.ranges,
            _marker: PhantomData,
        }
    }
}

impl AsyncLendingIterator for RowStreamOwned {
    type Ok<'i>
        = Row<'i>
    where
        Self: 'i;
    type Err = Error;

    #[inline]
    fn try_next(&mut self) -> impl Future<Output = Result<Option<Self::Ok<'_>>, Self::Err>> + Send {
        try_next(&mut self.res, &self.col, &mut self.ranges)
    }
}

impl IntoIterator for RowStream<'_> {
    type Item = Result<RowOwned, Error>;
    type IntoIter = RowStreamOwned;

    fn into_iter(self) -> Self::IntoIter {
        RowStreamOwned::from(self)
    }
}

impl Iterator for RowStreamOwned {
    type Item = Result<RowOwned, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.res.blocking_recv() {
                Ok(msg) => match msg {
                    backend::Message::DataRow(body) => {
                        return Some(RowOwned::try_new(self.col.clone(), body, Vec::new()))
                    }
                    backend::Message::BindComplete
                    | backend::Message::EmptyQueryResponse
                    | backend::Message::CommandComplete(_)
                    | backend::Message::PortalSuspended => {}
                    backend::Message::ReadyForQuery(_) => return None,
                    _ => return Some(Err(Error::unexpected())),
                },
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

/// A stream of simple query results.
pub type RowSimpleStream = GenericRowStream<Vec<Column>, marker::NoTyped>;

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

/// [`RowSimpleStreamOwned`] with static lifetime
pub type RowSimpleStreamOwned = GenericRowStream<Arc<[Column]>, marker::NoTyped>;

impl From<RowSimpleStream> for RowSimpleStreamOwned {
    fn from(stream: RowSimpleStream) -> Self {
        Self {
            res: stream.res,
            col: stream.col.into(),
            ranges: stream.ranges,
            _marker: PhantomData,
        }
    }
}

impl IntoIterator for RowSimpleStream {
    type IntoIter = RowSimpleStreamOwned;
    type Item = Result<RowSimpleOwned, Error>;

    fn into_iter(self) -> Self::IntoIter {
        RowSimpleStreamOwned::from(self)
    }
}

impl Iterator for RowSimpleStreamOwned {
    type Item = Result<RowSimpleOwned, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.res.blocking_recv() {
                Ok(msg) => match msg {
                    backend::Message::RowDescription(body) => match body
                        .fields()
                        .map(|f| Ok(Column::new(f.name(), Type::TEXT)))
                        .collect::<Vec<_>>()
                    {
                        Ok(col) => self.col = col.into(),
                        Err(e) => return Some(Err(Error::from(e))),
                    },
                    backend::Message::DataRow(body) => {
                        return Some(RowSimpleOwned::try_new(self.col.clone(), body, Vec::new()));
                    }
                    backend::Message::CommandComplete(_)
                    | backend::Message::EmptyQueryResponse
                    | backend::Message::ReadyForQuery(_) => return None,
                    _ => return Some(Err(Error::unexpected())),
                },
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

/// a stream of table rows where column type looked up and row parsing are bundled together
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

pub struct RowStreamGuardedOwned<'a, C> {
    res: Response,
    col: Arc<[Column]>,
    cli: &'a C,
}

impl<'a, C> From<RowStreamGuarded<'a, C>> for RowStreamGuardedOwned<'a, C> {
    fn from(stream: RowStreamGuarded<'a, C>) -> Self {
        Self {
            res: stream.res,
            col: stream.col.into(),
            cli: stream.cli,
        }
    }
}

impl<'a, C> IntoIterator for RowStreamGuarded<'a, C>
where
    C: Prepare,
{
    type Item = Result<RowOwned, Error>;
    type IntoIter = RowStreamGuardedOwned<'a, C>;

    fn into_iter(self) -> Self::IntoIter {
        RowStreamGuardedOwned::from(self)
    }
}

impl<C> Iterator for RowStreamGuardedOwned<'_, C>
where
    C: Prepare,
{
    type Item = Result<RowOwned, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.res.blocking_recv() {
                Ok(msg) => match msg {
                    backend::Message::RowDescription(body) => {
                        match body
                            .fields()
                            .map_err(Error::from)
                            .map(|f| {
                                let ty = self.cli._get_type_blocking(f.type_oid())?;
                                Ok(Column::new(f.name(), ty))
                            })
                            .collect::<Vec<_>>()
                        {
                            Ok(col) => self.col = col.into(),
                            Err(e) => return Some(Err(e)),
                        }
                    }
                    backend::Message::DataRow(body) => {
                        return Some(RowOwned::try_new(self.col.clone(), body, Vec::new()))
                    }
                    backend::Message::ParseComplete
                    | backend::Message::BindComplete
                    | backend::Message::ParameterDescription(_)
                    | backend::Message::EmptyQueryResponse
                    | backend::Message::CommandComplete(_)
                    | backend::Message::PortalSuspended => {}
                    backend::Message::NoData | backend::Message::ReadyForQuery(_) => return None,
                    _ => return Some(Err(Error::unexpected())),
                },
                Err(e) => return Some(Err(e)),
            }
        }
    }
}
