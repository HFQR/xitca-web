//! module for canceling ongoing queries

use postgres_protocol::message::frontend;
use xitca_io::bytes::BytesMut;

use super::{client::Client, error::Error, session::Session};

impl Client {
    /// obtain a token from client. can only be used to cancel queries associated to it
    pub fn cancel_token(&self) -> Session {
        self.session.clone()
    }
}

impl Session {
    pub async fn query_cancel(self) -> Result<(), Error> {
        let Session { id, key, info } = self;
        let mut drv = super::driver::connect_info(info).await?;

        let mut buf = BytesMut::new();

        frontend::cancel_request(id, key, &mut buf);

        drv.send(buf).await?;

        Ok(())
    }
}
