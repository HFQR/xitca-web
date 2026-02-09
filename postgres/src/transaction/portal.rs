use core::sync::atomic::Ordering;

use postgres_protocol::message::backend;

use crate::{
    client::ClientBorrow,
    driver::codec::{
        AsParams,
        encode::{Encode, PortalCancel, PortalCreate, PortalQuery},
        response::IntoResponse,
    },
    error::Error,
    statement::Statement,
};

/// A portal.
///
/// Portals can only be used with the connection that created them, and only exist for the duration of the transaction
/// in which they were created.
pub struct Portal<'a, C>
where
    C: ClientBorrow,
{
    cli: &'a C,
    name: String,
    stmt: &'a Statement,
}

impl<C> Drop for Portal<'_, C>
where
    C: ClientBorrow,
{
    fn drop(&mut self) {
        let _ = self.cli.borrow_cli_ref().query_raw(PortalCancel { name: &self.name });
    }
}

impl<C> Portal<'_, C>
where
    C: ClientBorrow,
{
    pub(crate) async fn new<'p, I>(cli: &'p C, stmt: &'p Statement, params: I) -> Result<Portal<'p, C>, Error>
    where
        I: AsParams,
    {
        let name = format!("p{}", crate::NEXT_ID.fetch_add(1, Ordering::Relaxed));

        let (_, mut res) = cli.borrow_cli_ref().query_raw(PortalCreate {
            name: &name,
            stmt: stmt.name(),
            types: stmt.params(),
            params,
        })?;

        match res.recv().await? {
            backend::Message::BindComplete => {}
            _ => return Err(Error::unexpected()),
        }

        Ok(Portal { cli, name, stmt })
    }

    pub fn query_portal(
        &self,
        max_rows: i32,
    ) -> Result<<<PortalQuery<'_> as Encode>::Output as IntoResponse>::Response, Error> {
        self.cli.borrow_cli_ref().query(PortalQuery {
            name: &self.name,
            columns: self.stmt.columns(),
            max_rows,
        })
    }
}
