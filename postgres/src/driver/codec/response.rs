use core::{future::Future, pin::Pin};

use fallible_iterator::FallibleIterator;
use postgres_protocol::message::backend;

use crate::{
    column::Column,
    error::Error,
    prepare::Prepare,
    query::{RowSimpleStream, RowStream, RowStreamGuarded},
    statement::Statement,
};

use super::{sealed, Response};

/// trait for generic over how to construct an async stream rows
pub trait IntoResponse: sealed::Sealed + Sized {
    type Response<'r>
    where
        Self: 'r;

    fn into_response<'r>(self, res: Response) -> Self::Response<'r>
    where
        Self: 'r;
}

impl sealed::Sealed for &[Column] {}

impl IntoResponse for &[Column] {
    type Response<'r>
        = RowStream<'r>
    where
        Self: 'r;

    #[inline]
    fn into_response<'r>(self, res: Response) -> Self::Response<'r>
    where
        Self: 'r,
    {
        RowStream::new(res, self)
    }
}

impl sealed::Sealed for Vec<Column> {}

impl IntoResponse for Vec<Column> {
    type Response<'r>
        = RowSimpleStream
    where
        Self: 'r;

    #[inline]
    fn into_response<'r>(self, res: Response) -> Self::Response<'r>
    where
        Self: 'r,
    {
        RowSimpleStream::new(res, self)
    }
}

pub struct IntoRowStreamGuard<'a, C>(pub &'a C);

impl<C> sealed::Sealed for IntoRowStreamGuard<'_, C> {}

impl<C> IntoResponse for IntoRowStreamGuard<'_, C>
where
    C: Prepare,
{
    type Response<'r>
        = RowStreamGuarded<'r, C>
    where
        Self: 'r;

    #[inline]
    fn into_response<'r>(self, res: Response) -> Self::Response<'r>
    where
        Self: 'r,
    {
        RowStreamGuarded::new(res, self.0)
    }
}

/// type for case where no row stream can be created.
/// the api caller should never call into_stream method from this type.
pub struct NoOpIntoRowStream;

impl sealed::Sealed for NoOpIntoRowStream {}

impl IntoResponse for NoOpIntoRowStream {
    type Response<'r>
        = RowStream<'r>
    where
        Self: 'r;

    fn into_response<'r>(self, _: Response) -> Self::Response<'r>
    where
        Self: 'r,
    {
        unreachable!("no row stream can be generated from no op row stream constructor")
    }
}

pub struct StatementCreateResponse<'a, C> {
    pub(super) name: String,
    pub(super) cli: &'a C,
}

impl<C> sealed::Sealed for StatementCreateResponse<'_, C> {}

impl<C> IntoResponse for StatementCreateResponse<'_, C>
where
    C: Prepare,
{
    type Response<'r>
        = Pin<Box<dyn Future<Output = Result<Statement, Error>> + Send + 'r>>
    where
        Self: 'r;

    fn into_response<'r>(self, mut res: Response) -> Self::Response<'r>
    where
        Self: 'r,
    {
        Box::pin(async move {
            let Self { name, cli } = self;

            match res.recv().await? {
                backend::Message::ParseComplete => {}
                _ => return Err(Error::unexpected()),
            }

            let parameter_description = match res.recv().await? {
                backend::Message::ParameterDescription(body) => body,
                _ => return Err(Error::unexpected()),
            };

            let row_description = match res.recv().await? {
                backend::Message::RowDescription(body) => Some(body),
                backend::Message::NoData => None,
                _ => return Err(Error::unexpected()),
            };

            let mut params = Vec::new();
            let mut it = parameter_description.parameters();
            while let Some(oid) = it.next()? {
                let ty = cli._get_type(oid).await?;
                params.push(ty);
            }

            let mut columns = Vec::new();
            if let Some(row_description) = row_description {
                let mut it = row_description.fields();
                while let Some(field) = it.next()? {
                    let type_ = cli._get_type(field.type_oid()).await?;
                    let column = Column::new(field.name(), type_);
                    columns.push(column);
                }
            }

            Ok(Statement::new(name, params, columns))
        })
    }
}

pub struct StatementCreateResponseBlocking<'a, C> {
    pub(super) name: String,
    pub(super) cli: &'a C,
}

impl<C> sealed::Sealed for StatementCreateResponseBlocking<'_, C> {}

impl<C> IntoResponse for StatementCreateResponseBlocking<'_, C>
where
    C: Prepare,
{
    type Response<'r>
        = Result<Statement, Error>
    where
        Self: 'r;

    fn into_response<'r>(self, mut res: Response) -> Self::Response<'r>
    where
        Self: 'r,
    {
        let Self { name, cli } = self;

        match res.blocking_recv()? {
            backend::Message::ParseComplete => {}
            _ => return Err(Error::unexpected()),
        }

        let parameter_description = match res.blocking_recv()? {
            backend::Message::ParameterDescription(body) => body,
            _ => return Err(Error::unexpected()),
        };

        let row_description = match res.blocking_recv()? {
            backend::Message::RowDescription(body) => Some(body),
            backend::Message::NoData => None,
            _ => return Err(Error::unexpected()),
        };

        let mut params = Vec::new();
        let mut it = parameter_description.parameters();
        while let Some(oid) = it.next()? {
            let ty = cli._get_type_blocking(oid)?;
            params.push(ty);
        }

        let mut columns = Vec::new();
        if let Some(row_description) = row_description {
            let mut it = row_description.fields();
            while let Some(field) = it.next()? {
                let type_ = cli._get_type_blocking(field.type_oid())?;
                let column = Column::new(field.name(), type_);
                columns.push(column);
            }
        }

        Ok(Statement::new(name, params, columns))
    }
}
