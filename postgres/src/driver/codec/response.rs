use fallible_iterator::FallibleIterator;
use postgres_protocol::message::backend;

use crate::{
    column::Column,
    error::Error,
    pipeline::PipelineStream,
    prepare::Prepare,
    query::{RowSimpleStream, RowStream, RowStreamGuarded},
    statement::Statement,
    BoxedFuture,
};

use super::{sealed, Response};

/// trait for generic over how to construct an async stream rows
pub trait IntoResponse: sealed::Sealed + Sized {
    type Response;

    fn into_response(self, res: Response) -> Self::Response;
}

impl sealed::Sealed for &[Column] {}

impl<'c> IntoResponse for &'c [Column] {
    type Response = RowStream<'c>;

    #[inline]
    fn into_response(self, res: Response) -> Self::Response {
        RowStream::new(res, self)
    }
}

impl sealed::Sealed for Vec<Column> {}

impl IntoResponse for Vec<Column> {
    type Response = RowSimpleStream;

    #[inline]
    fn into_response(self, res: Response) -> Self::Response {
        RowSimpleStream::new(res, self)
    }
}

impl sealed::Sealed for Vec<&[Column]> {}

impl<'c> IntoResponse for Vec<&'c [Column]> {
    type Response = PipelineStream<'c>;

    #[inline]
    fn into_response(self, res: Response) -> Self::Response {
        PipelineStream::new(res, self)
    }
}

pub struct IntoRowStreamGuard<'a, C>(pub &'a C);

impl<C> sealed::Sealed for IntoRowStreamGuard<'_, C> {}

impl<'c, C> IntoResponse for IntoRowStreamGuard<'c, C>
where
    C: Prepare,
{
    type Response = RowStreamGuarded<'c, C>;

    #[inline]
    fn into_response(self, res: Response) -> Self::Response {
        RowStreamGuarded::new(res, self.0)
    }
}

/// type for case where no row stream can be created.
/// the api caller should never call into_stream method from this type.
pub struct NoOpIntoRowStream;

impl sealed::Sealed for NoOpIntoRowStream {}

impl IntoResponse for NoOpIntoRowStream {
    type Response = RowStream<'static>;

    fn into_response(self, _: Response) -> Self::Response {
        unreachable!("no row stream can be generated from no op row stream constructor")
    }
}

pub struct StatementCreateResponse<'a, C> {
    pub(super) name: String,
    pub(super) cli: &'a C,
}

impl<C> sealed::Sealed for StatementCreateResponse<'_, C> {}

impl<'s, C> IntoResponse for StatementCreateResponse<'s, C>
where
    C: Prepare,
{
    type Response = BoxedFuture<'s, Result<Statement, Error>>;

    fn into_response(self, mut res: Response) -> Self::Response {
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

            let mut it = parameter_description.parameters();
            let mut params = Vec::with_capacity(it.size_hint().0);

            while let Some(oid) = it.next()? {
                let ty = cli._get_type(oid).await?;
                params.push(ty);
            }

            let mut columns = Vec::new();
            if let Some(row_description) = row_description {
                let mut it = row_description.fields();
                columns.reserve(it.size_hint().0);
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
    type Response = Result<Statement, Error>;

    fn into_response(self, mut res: Response) -> Self::Response {
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

        let mut it = parameter_description.parameters();
        let mut params = Vec::with_capacity(it.size_hint().0);

        while let Some(oid) = it.next()? {
            let ty = cli._get_type_blocking(oid)?;
            params.push(ty);
        }

        let mut columns = Vec::new();
        if let Some(row_description) = row_description {
            let mut it = row_description.fields();
            columns.reserve(it.size_hint().0);
            while let Some(field) = it.next()? {
                let type_ = cli._get_type_blocking(field.type_oid())?;
                let column = Column::new(field.name(), type_);
                columns.push(column);
            }
        }

        Ok(Statement::new(name, params, columns))
    }
}
