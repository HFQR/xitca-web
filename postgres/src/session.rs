use fallible_iterator::FallibleIterator;
use postgres_protocol::message::{backend, frontend};

use super::{client::Client, config::Config, error::Error, transport::MessageIo};

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
    pub(super) async fn session<Io>(&mut self, io: &mut Io, cfg: &mut Config) -> Result<(), Error>
    where
        Io: MessageIo,
    {
        loop {
            match io.recv().await? {
                backend::Message::ReadyForQuery(_) => break,
                backend::Message::BackendKeyData(_) => {
                    // TODO: handle process id and secret key.
                }
                backend::Message::ParameterStatus(_) => {
                    // TODO: handle parameters
                }
                _ => {
                    // TODO: other session message handling?
                }
            }
        }

        if matches!(cfg.get_target_session_attrs(), TargetSessionAttrs::ReadWrite) {
            let buf = self.try_encode_with(|buf| frontend::query("SHOW transaction_read_only", buf))?;
            io.send(buf).await?;
            // TODO: use RowSimple for parsing?
            loop {
                match io.recv().await? {
                    backend::Message::DataRow(body) => {
                        let range = body.ranges().next()?.flatten().ok_or(Error::ToDo)?;
                        let slice = &body.buffer()[range.start..range.end];
                        if slice == b"on" {
                            return Err(Error::ToDo);
                        }
                    }
                    backend::Message::RowDescription(_) | backend::Message::CommandComplete(_) => {}
                    backend::Message::EmptyQueryResponse | backend::Message::ReadyForQuery(_) => break,
                    _ => return Err(Error::UnexpectedMessage),
                }
            }
        }
        Ok(())
    }
}
