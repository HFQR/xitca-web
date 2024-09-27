use postgres_protocol::message::frontend;
use xitca_io::bytes::BytesMut;

use crate::{
    column::Column,
    error::{Error, InvalidParamCount},
    prepare::Prepare,
    statement::{Statement, StatementCreate, StatementCreateBlocking, StatementQuery, StatementUnnamedQuery},
    types::{BorrowToSql, IsNull, Type},
};

use super::{
    response::{
        IntoResponse, IntoRowStreamGuard, NoOpIntoRowStream, StatementCreateResponse, StatementCreateResponseBlocking,
    },
    sealed, AsParams, DriverTx, Response,
};

/// trait for generic over how to encode a query.
/// currently this trait can not be implement by library user.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not impl Encode trait",
    label = "query statement argument must be types implement Encode trait",
    note = "consider using the types listed below that implementing Encode trait"
)]
pub trait Encode: sealed::Sealed + Sized {
    /// output type defines how a potential async row streaming type should be constructed.
    /// certain state from the encode type may need to be passed for constructing the stream
    type Output<'o>: IntoResponse
    where
        Self: 'o;

    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o;
}

impl sealed::Sealed for &str {}

impl Encode for &str {
    type Output<'o>
        = Vec<Column>
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        frontend::query(self, buf)?;
        Ok(Vec::new())
    }
}

impl sealed::Sealed for &Statement {}

impl Encode for &Statement {
    type Output<'o>
        = &'o [Column]
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        encode_bind(self.name(), self.params(), [] as [i32; 0], "", buf)?;
        frontend::execute("", 0, buf)?;
        if SYNC_MODE {
            frontend::sync(buf);
        }
        Ok(self.columns())
    }
}

impl<C> sealed::Sealed for StatementCreate<'_, C> {}

impl<C> Encode for StatementCreate<'_, C>
where
    C: Prepare,
{
    type Output<'o>
        = StatementCreateResponse<'o, C>
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        let Self { name, stmt, types, cli } = self;
        encode_statement_create(&name, stmt, types, buf).map(|_| StatementCreateResponse { name, cli })
    }
}

impl<C> sealed::Sealed for StatementCreateBlocking<'_, C> {}

impl<C> Encode for StatementCreateBlocking<'_, C>
where
    C: Prepare,
{
    type Output<'o>
        = StatementCreateResponseBlocking<'o, C>
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        let Self { name, stmt, types, cli } = self;
        encode_statement_create(&name, stmt, types, buf).map(|_| StatementCreateResponseBlocking { name, cli })
    }
}

fn encode_statement_create(name: &str, stmt: &str, types: &[Type], buf: &mut BytesMut) -> Result<(), Error> {
    frontend::parse(name, stmt, types.iter().map(Type::oid), buf)?;
    frontend::describe(b'S', name, buf)?;
    frontend::sync(buf);
    Ok(())
}

pub(crate) struct StatementCancel<'a> {
    pub(crate) name: &'a str,
}

impl sealed::Sealed for StatementCancel<'_> {}

impl Encode for StatementCancel<'_> {
    type Output<'o>
        = NoOpIntoRowStream
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        let Self { name } = self;
        frontend::close(b'S', name, buf)?;
        frontend::sync(buf);
        Ok(NoOpIntoRowStream)
    }
}

impl<P> sealed::Sealed for StatementQuery<'_, P> {}

impl<'a, P> Encode for StatementQuery<'a, P>
where
    P: AsParams,
{
    type Output<'o>
        = &'o [Column]
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        let StatementQuery { stmt, params } = self;
        encode_bind(stmt.name(), stmt.params(), params, "", buf)?;
        frontend::execute("", 0, buf)?;
        if SYNC_MODE {
            frontend::sync(buf);
        }
        Ok(stmt.columns())
    }
}

impl<C, P> sealed::Sealed for StatementUnnamedQuery<'_, P, C> {}

impl<C, P> Encode for StatementUnnamedQuery<'_, P, C>
where
    C: Prepare,
    P: AsParams,
{
    type Output<'o>
        = IntoRowStreamGuard<'o, C>
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        let Self {
            stmt,
            types,
            cli,
            params,
        } = self;
        frontend::parse("", stmt, types.iter().map(Type::oid), buf)?;
        encode_bind("", types, params, "", buf)?;
        frontend::describe(b'S', "", buf)?;
        frontend::execute("", 0, buf)?;
        if SYNC_MODE {
            frontend::sync(buf);
        }
        Ok(IntoRowStreamGuard(cli))
    }
}

pub(crate) struct PortalCreate<'a, P> {
    pub(crate) name: &'a str,
    pub(crate) stmt: &'a str,
    pub(crate) types: &'a [Type],
    pub(crate) params: P,
}

impl<P> sealed::Sealed for PortalCreate<'_, P> {}

impl<P> Encode for PortalCreate<'_, P>
where
    P: AsParams,
{
    type Output<'o>
        = NoOpIntoRowStream
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        let PortalCreate {
            name,
            stmt,
            types,
            params,
        } = self;
        encode_bind(stmt, types, params, name, buf)?;
        frontend::sync(buf);
        Ok(NoOpIntoRowStream)
    }
}

pub(crate) struct PortalCancel<'a> {
    pub(crate) name: &'a str,
}

impl sealed::Sealed for PortalCancel<'_> {}

impl Encode for PortalCancel<'_> {
    type Output<'o>
        = NoOpIntoRowStream
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        frontend::close(b'P', self.name, buf)?;
        frontend::sync(buf);
        Ok(NoOpIntoRowStream)
    }
}

pub struct PortalQuery<'a> {
    pub(crate) name: &'a str,
    pub(crate) columns: &'a [Column],
    pub(crate) max_rows: i32,
}

impl sealed::Sealed for PortalQuery<'_> {}

impl Encode for PortalQuery<'_> {
    type Output<'o>
        = &'o [Column]
    where
        Self: 'o;

    #[inline]
    fn encode<'o, const SYNC_MODE: bool>(self, buf: &mut BytesMut) -> Result<Self::Output<'o>, Error>
    where
        Self: 'o,
    {
        let Self {
            name,
            max_rows,
            columns,
        } = self;
        frontend::execute(name, max_rows, buf)?;
        frontend::sync(buf);
        Ok(columns)
    }
}

pub(crate) fn send_encode_query<'a, S>(tx: &DriverTx, stmt: S) -> Result<(S::Output<'a>, Response), Error>
where
    S: Encode + 'a,
{
    tx.send(|buf| stmt.encode::<true>(buf))
}

fn encode_bind<P>(stmt: &str, types: &[Type], params: P, portal: &str, buf: &mut BytesMut) -> Result<(), Error>
where
    P: AsParams,
{
    let params = params.into_iter();
    if params.len() != types.len() {
        return Err(Error::from(InvalidParamCount {
            expected: types.len(),
            params: params.len(),
        }));
    }

    let params = params.zip(types).collect::<Vec<_>>();

    frontend::bind(
        portal,
        stmt,
        params.iter().map(|(p, ty)| p.borrow_to_sql().encode_format(ty) as _),
        params.iter(),
        |(param, ty), buf| {
            param
                .borrow_to_sql()
                .to_sql_checked(ty, buf)
                .map(|is_null| match is_null {
                    IsNull::No => postgres_protocol::IsNull::No,
                    IsNull::Yes => postgres_protocol::IsNull::Yes,
                })
        },
        Some(1),
        buf,
    )
    .map_err(|e| match e {
        frontend::BindError::Conversion(e) => Error::from(e),
        frontend::BindError::Serialization(e) => Error::from(e),
    })
}
