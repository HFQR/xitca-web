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

    let (param_formats, params): (Vec<_>, Vec<_>) = params
        .zip(stmt.params())
        .map(|(p, ty)| (p.borrow_to_sql().encode_format(ty) as i16, (p, ty)))
        .unzip();

    let mut error_idx = 0;
    let r = frontend::bind(
        portal,
        stmt.name(),
        param_formats,
        params.into_iter().enumerate(),
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
        Err(frontend::BindError::Conversion(e)) => Err(Error::from(e)),
        Err(frontend::BindError::Serialization(e)) => Err(Error::from(e)),
    }
}
