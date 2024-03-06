//! session handling after server connection is established.

use fallible_iterator::FallibleIterator;
use postgres_protocol::{
    authentication::{self, sasl},
    message::{backend, frontend},
};
use xitca_io::bytes::BytesMut;

use super::{
    config::Config,
    driver::Drive,
    error::{AuthenticationError, Error},
};

/// Properties required of a session.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TargetSessionAttrs {
    /// No special properties are required.
    Any,
    /// The session must allow writes.
    ReadWrite,
}

#[allow(clippy::needless_pass_by_ref_mut)] // dumb clippy
#[cold]
#[inline(never)]
pub(super) async fn prepare_session<D>(drv: &mut D, cfg: &Config) -> Result<(), Error>
where
    D: Drive,
{
    let mut buf = BytesMut::new();

    auth(drv, cfg, &mut buf).await?;

    loop {
        match drv.recv().await? {
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
        frontend::query("SHOW transaction_read_only", &mut buf)?;
        let msg = buf.split();
        drv.send(msg).await?;
        // TODO: use RowSimple for parsing?
        loop {
            match drv.recv().await? {
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

#[cold]
#[inline(never)]
async fn auth<D>(drv: &mut D, cfg: &Config, buf: &mut BytesMut) -> Result<(), Error>
where
    D: Drive,
{
    let mut params = vec![("client_encoding", "UTF8")];
    if let Some(user) = &cfg.user {
        params.push(("user", &**user));
    }
    if let Some(dbname) = &cfg.dbname {
        params.push(("database", &**dbname));
    }
    if let Some(options) = &cfg.options {
        params.push(("options", &**options));
    }
    if let Some(application_name) = &cfg.application_name {
        params.push(("application_name", &**application_name));
    }

    frontend::startup_message(params, buf)?;
    let msg = buf.split();
    drv.send(msg).await?;

    loop {
        match drv.recv().await? {
            backend::Message::AuthenticationOk => return Ok(()),
            backend::Message::AuthenticationCleartextPassword => {
                let pass = cfg.get_password().ok_or(AuthenticationError::MissingPassWord)?;
                send_pass(drv, pass, buf).await?;
            }
            backend::Message::AuthenticationMd5Password(body) => {
                let pass = cfg.get_password().ok_or(AuthenticationError::MissingPassWord)?;
                let user = cfg.get_user().ok_or(AuthenticationError::MissingUserName)?.as_bytes();
                let pass = authentication::md5_hash(user, pass, body.salt());
                send_pass(drv, pass, buf).await?;
            }
            backend::Message::AuthenticationSasl(body) => {
                let pass = cfg.get_password().ok_or(AuthenticationError::MissingPassWord)?;

                let mut is_scram = false;
                let mut is_scram_plus = false;
                let mut mechanisms = body.mechanisms();

                while let Some(mechanism) = mechanisms.next()? {
                    match mechanism {
                        sasl::SCRAM_SHA_256 => is_scram = true,
                        sasl::SCRAM_SHA_256_PLUS => is_scram_plus = true,
                        _ => {}
                    }
                }

                let (channel_binding, mechanism) = match (is_scram_plus, is_scram) {
                    (true, is_scram) => {
                        let buf = cfg.get_tls_server_end_point();
                        if !buf.is_empty() {
                            (
                                sasl::ChannelBinding::tls_server_end_point(buf),
                                sasl::SCRAM_SHA_256_PLUS,
                            )
                        } else if is_scram {
                            (sasl::ChannelBinding::unrequested(), sasl::SCRAM_SHA_256)
                        } else {
                            // server ask for channel binding but no tls_server_end_point can be
                            // found.
                            return Err(Error::ToDo);
                        }
                    }
                    (false, true) => (sasl::ChannelBinding::unrequested(), sasl::SCRAM_SHA_256),
                    // TODO: return "unsupported SASL mechanism" error.
                    (false, false) => return Err(Error::ToDo),
                };

                let mut scram = sasl::ScramSha256::new(pass, channel_binding);

                frontend::sasl_initial_response(mechanism, scram.message(), buf)?;
                let msg = buf.split();
                drv.send(msg).await?;

                match drv.recv().await? {
                    backend::Message::AuthenticationSaslContinue(body) => {
                        scram.update(body.data())?;
                        frontend::sasl_response(scram.message(), buf)?;
                        let msg = buf.split();
                        drv.send(msg).await?;
                    }
                    _ => return Err(Error::ToDo),
                }

                match drv.recv().await? {
                    backend::Message::AuthenticationSaslFinal(body) => scram.finish(body.data())?,
                    _ => return Err(Error::ToDo),
                }
            }
            backend::Message::ErrorResponse(_) => return Err(Error::from(AuthenticationError::WrongPassWord)),
            _ => {}
        }
    }
}

async fn send_pass<D>(drv: &mut D, pass: impl AsRef<[u8]>, buf: &mut BytesMut) -> Result<(), Error>
where
    D: Drive,
{
    frontend::password_message(pass.as_ref(), buf)?;
    let msg = buf.split();
    drv.send(msg).await
}
