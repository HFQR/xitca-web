use postgres_protocol::message::frontend;
use postgres_types::{BorrowToSql, IsNull};
use xitca_io::bytes::BytesMut;

use crate::{
    error::{Error, InvalidParamCount},
    statement::Statement,
};

pub(crate) fn encode<I>(buf: &mut BytesMut, stmt: &Statement, params: I) -> Result<(), Error>
where
    I: ExactSizeIterator,
    I::Item: BorrowToSql,
{
    encode_maybe_sync::<I, true>(buf, stmt, params)
}

pub(crate) fn encode_maybe_sync<I, const SYNC_MODE: bool>(
    buf: &mut BytesMut,
    stmt: &Statement,
    params: I,
) -> Result<(), Error>
where
    I: ExactSizeIterator,
    I::Item: BorrowToSql,
{
    encode_bind(stmt, params, "", buf)?;
    frontend::execute("", 0, buf)?;
    if SYNC_MODE {
        frontend::sync(buf);
    }
    Ok(())
}

fn encode_bind<I>(stmt: &Statement, params: I, portal: &str, buf: &mut BytesMut) -> Result<(), Error>
where
    I: ExactSizeIterator,
    I::Item: BorrowToSql,
{
    if params.len() != stmt.params().len() {
        return Err(Error::from(InvalidParamCount {
            expected: params.len(),
            params: stmt.params().len(),
        }));
    }

    let params = params.zip(stmt.params()).collect::<Vec<_>>();

    frontend::bind(
        portal,
        stmt.name(),
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
