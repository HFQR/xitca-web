mod stream;

#[cfg(feature = "compat")]
pub(crate) mod compat;

pub use stream::{RowAffected, RowSimpleStream, RowSimpleStreamOwned, RowStream, RowStreamGuarded, RowStreamOwned};

use std::sync::Arc;

use super::{
    client::Client,
    driver::codec::encode,
    driver::codec::{Response, encode::Encode, response::IntoResponse},
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
    fn _query<S>(&self, stmt: S) -> Result<<S::Output as IntoResponse>::Response, Error>
    where
        S: Encode,
    {
        self._send_encode_query(stmt)
            .map(|(stream, res)| stream.into_response(res))
    }

    /// encode statement and params and send it to client driver
    fn _send_encode_query<S>(&self, stmt: S) -> Result<(S::Output, Response), Error>
    where
        S: Encode;
}

impl<T> Query for &T
where
    T: Query,
{
    #[inline]
    fn _send_encode_query<S>(&self, stmt: S) -> Result<(S::Output, Response), Error>
    where
        S: Encode,
    {
        T::_send_encode_query(*self, stmt)
    }
}

impl<T> Query for &mut T
where
    T: Query,
{
    #[inline]
    fn _send_encode_query<S>(&self, stmt: S) -> Result<(S::Output, Response), Error>
    where
        S: Encode,
    {
        T::_send_encode_query(&**self, stmt)
    }
}

impl Query for Client {
    #[inline]
    fn _send_encode_query<S>(&self, stmt: S) -> Result<(S::Output, Response), Error>
    where
        S: Encode,
    {
        encode::send_encode_query(&self.tx, stmt)
    }
}

impl Query for Arc<Client> {
    #[inline]
    fn _send_encode_query<S>(&self, stmt: S) -> Result<(S::Output, Response), Error>
    where
        S: Encode,
    {
        Client::_send_encode_query(&**self, stmt)
    }
}
