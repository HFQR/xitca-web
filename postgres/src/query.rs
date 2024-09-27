mod stream;

#[cfg(feature = "compat")]
pub(crate) mod compat;

pub use stream::{RowAffected, RowSimpleStream, RowStream, RowStreamGuarded, RowStreamOwned};

use super::{
    driver::codec::{encode::Encode, response::IntoResponse, Response},
    error::Error,
};

/// trait generic over api used for querying with typed prepared statement.
///
/// types like [Transaction] and [CopyIn] accept generic client type and they are able to use user supplied
/// client new type to operate and therefore reduce less new types and methods boilerplate.
///
/// [Transaction]: crate::transaction::Transaction
/// [CopyIn]: crate::copy::CopyIn
pub trait Query {
    /// query with statement and dynamic typed params.
    ///
    /// statement must be a type impl [`Encode`] trait
    #[inline]
    fn _query<'a, S>(&self, stmt: S) -> Result<<S::Output<'a> as IntoResponse>::Response<'a>, Error>
    where
        S: Encode + 'a,
    {
        self._send_encode_query(stmt)
            .map(|(stream, res)| stream.into_response(res))
    }

    /// encode statement and params and send it to client driver
    fn _send_encode_query<'a, S>(&self, stmt: S) -> Result<(S::Output<'a>, Response), Error>
    where
        S: Encode + 'a;
}
