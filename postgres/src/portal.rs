use core::sync::atomic::Ordering;

use postgres_protocol::message::backend;

use super::{
    driver::codec::{AsParams, Response},
    error::Error,
    query::RowStream,
    statement::Statement,
};

pub trait PortalTrait {
    fn _send_encode_portal_create<I>(&self, name: &str, stmt: &Statement, params: I) -> Result<Response, Error>
    where
        I: AsParams;

    fn _send_encode_portal_query(&self, name: &str, max_rows: i32) -> Result<Response, Error>;

    fn _send_encode_portal_cancel(&self, name: &str);
}

/// A portal.
///
/// Portals can only be used with the connection that created them, and only exist for the duration of the transaction
/// in which they were created.
pub struct Portal<'a, C>
where
    C: PortalTrait,
{
    cli: &'a C,
    name: String,
    stmt: &'a Statement,
}

impl<C> Drop for Portal<'_, C>
where
    C: PortalTrait,
{
    fn drop(&mut self) {
        PortalTrait::_send_encode_portal_cancel(self.cli, &self.name);
    }
}

impl<C> Portal<'_, C>
where
    C: PortalTrait,
{
    pub(crate) async fn new<'p, I>(cli: &'p C, stmt: &'p Statement, params: I) -> Result<Portal<'p, C>, Error>
    where
        I: AsParams,
    {
        let name = format!("p{}", crate::NEXT_ID.fetch_add(1, Ordering::Relaxed));

        let mut res = cli._send_encode_portal_create(&name, stmt, params)?;

        match res.recv().await? {
            backend::Message::BindComplete => {}
            _ => return Err(Error::unexpected()),
        }

        Ok(Portal { cli, name, stmt })
    }

    pub fn query_portal(&self, max_rows: i32) -> Result<RowStream<'_>, Error> {
        self.cli
            ._send_encode_portal_query(&self.name, max_rows)
            .map(|res| RowStream {
                res,
                col: self.stmt.columns(),
                ranges: Vec::new(),
            })
    }
}
