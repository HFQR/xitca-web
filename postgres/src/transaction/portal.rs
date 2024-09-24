use core::sync::atomic::Ordering;

use postgres_protocol::message::backend;

use crate::{
    driver::codec::{Encode, IntoParams, IntoStream, PortalCancel, PortalCreate, PortalQuery},
    error::Error,
    query::Query,
    statement::Statement,
};

/// A portal.
///
/// Portals can only be used with the connection that created them, and only exist for the duration of the transaction
/// in which they were created.
pub struct Portal<'a, C>
where
    C: Query,
{
    cli: &'a C,
    name: String,
    stmt: &'a Statement,
}

impl<C> Drop for Portal<'_, C>
where
    C: Query,
{
    fn drop(&mut self) {
        let _ = self.cli._send_encode_query(PortalCancel { name: &self.name });
    }
}

impl<C> Portal<'_, C>
where
    C: Query,
{
    pub(crate) async fn new<'p, I>(cli: &'p C, stmt: &'p Statement, params: I) -> Result<Portal<'p, C>, Error>
    where
        I: IntoParams,
    {
        let name = format!("p{}", crate::NEXT_ID.fetch_add(1, Ordering::Relaxed));

        let (_, mut res) = cli._send_encode_query((
            PortalCreate {
                name: &name,
                stmt: stmt.name(),
                types: stmt.params(),
            },
            params,
        ))?;

        match res.recv().await? {
            backend::Message::BindComplete => {}
            _ => return Err(Error::unexpected()),
        }

        Ok(Portal { cli, name, stmt })
    }

    pub fn query_portal(
        &self,
        max_rows: i32,
    ) -> Result<<<PortalQuery<'_> as Encode>::Output<'_> as IntoStream>::RowStream<'_>, Error> {
        self.cli._query(PortalQuery {
            name: &self.name,
            columns: self.stmt.columns(),
            max_rows,
        })
    }
}
