use super::{client::Client, config::Config, error::Error};
use crate::AsyncIterator;

/// Properties required of a session.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TargetSessionAttrs {
    /// No special properties are required.
    Any,
    /// The session must allow writes.
    ReadWrite,
}

impl Client {
    pub(super) async fn session_attrs(&mut self, cfg: &mut Config) -> Result<(), Error> {
        if matches!(cfg.get_target_session_attrs(), TargetSessionAttrs::ReadWrite) {
            let mut rows = self.query_simple("SHOW transaction_read_only").await?;
            let row = rows.next().await.ok_or(Error::ToDo)??;
            if row.try_get(0)? == Some("on") {
                return Err(Error::ToDo);
            }
        }
        Ok(())
    }
}
