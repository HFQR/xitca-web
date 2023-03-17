use postgres_protocol::message::backend;

use crate::error::Error;

pub struct Response;

impl Response {
    pub(crate) async fn recv(&mut self) -> Result<backend::Message, Error> {
        todo!()
    }
}
