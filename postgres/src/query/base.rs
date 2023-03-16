use core::{future::Future, ops::Range};

use postgres_protocol::message::{backend, frontend};
use postgres_types::{BorrowToSql, IsNull};
use xitca_io::bytes::BytesMut;

use crate::{
    client::Client,
    column::Column,
    error::Error,
    iter::{slice_iter, AsyncIterator},
    row::Row,
    statement::Statement,
    transport::Response,
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
    pub async fn query_raw<'a, I>(&self, stmt: &'a Statement, params: I) -> Result<RowStream<'a>, Error>
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: BorrowToSql,
    {
        self.bind(stmt, params).await.map(|res| RowStream {
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

    client.try_encode_with(|buf| {
        encode_bind(stmt, params, "", buf)?;
        frontend::execute("", 0, buf).map_err(|_| Error::ToDo)?;
        frontend::sync(buf);
        Ok(())
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
pub type RowStream<'a> = GenericRowStream<&'a [Column]>;

pub struct GenericRowStream<C> {
    pub(super) res: Response,
    pub(super) col: C,
    pub(super) ranges: Vec<Option<Range<usize>>>,
}

impl<'a> AsyncIterator for RowStream<'a> {
    type Future<'f> = impl Future<Output = Option<Self::Item<'f>>> + Send + 'f where 'a: 'f;
    type Item<'i> = Result<Row<'i>, Error> where 'a: 'i;

    fn next(&mut self) -> Self::Future<'_> {
        async {
            loop {
                match self.res.recv().await {
                    Ok(msg) => match msg {
                        backend::Message::DataRow(body) => return Some(Row::try_new(self.col, body, &mut self.ranges)),
                        backend::Message::EmptyQueryResponse
                        | backend::Message::CommandComplete(_)
                        | backend::Message::PortalSuspended => {}
                        backend::Message::ReadyForQuery(_) => return None,
                        _ => return Some(Err(Error::UnexpectedMessage)),
                    },
                    Err(e) => return Some(Err(e)),
                }
            }
        }
    }
}
