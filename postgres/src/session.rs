//! session handling after server connection is established with authentication and credential info.

use core::net::SocketAddr;

use fallible_iterator::FallibleIterator;
use postgres_protocol::{
    authentication::{self, sasl},
    message::{backend, frontend},
};
use xitca_io::{bytes::BytesMut, io::AsyncIo};

use super::{
    config::{Config, SslMode, SslNegotiation},
    driver::generic::GenericDriver,
    error::{ConfigError, Error},
};

/// Properties required of a session.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TargetSessionAttrs {
    /// No special properties are required.
    Any,
    /// The session must allow writes.
    ReadWrite,
    /// The session only allows read.
    ReadOnly,
}

/// information about session. used for canceling query
#[derive(Clone)]
pub struct Session {
    pub(crate) id: i32,
    pub(crate) key: i32,
    pub(crate) info: ConnectInfo,
}

#[derive(Clone, Default)]
pub(crate) struct ConnectInfo {
    pub(crate) addr: Addr,
    pub(crate) ssl_mode: SslMode,
    pub(crate) ssl_negotiation: SslNegotiation,
}

impl ConnectInfo {
    pub(crate) fn new(addr: Addr, ssl_mode: SslMode, ssl_negotiation: SslNegotiation) -> Self {
        Self {
            addr,
            ssl_mode,
            ssl_negotiation,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) enum Addr {
    Tcp(Box<str>, SocketAddr),
    #[cfg(unix)]
    Unix(Box<str>, std::path::PathBuf),
    #[cfg(feature = "quic")]
    Quic(Box<str>, SocketAddr),
    // case for where io is supplied by user and no connectivity can be done from this crate
    #[default]
    None,
}

impl Session {
    fn new(info: ConnectInfo) -> Self {
        Self { id: 0, key: 0, info }
    }
}

impl Session {
    #[allow(clippy::needless_pass_by_ref_mut)] // dumb clippy
    #[cold]
    #[inline(never)]
    pub(super) async fn prepare_session<Io>(
        info: ConnectInfo,
        drv: &mut GenericDriver<Io>,
        cfg: &Config,
    ) -> Result<Self, Error>
    where
        Io: AsyncIo + Send,
    {
        let mut buf = BytesMut::new();

        auth(drv, cfg, &mut buf).await?;

        let mut session = Session::new(info);

        loop {
            match drv.recv().await? {
                backend::Message::ReadyForQuery(_) => break,
                backend::Message::BackendKeyData(body) => {
                    session.id = body.process_id();
                    session.key = body.secret_key();
                }
                backend::Message::ParameterStatus(body) => {
                    // TODO: handling params?
                    let _name = body.name()?;
                    let _value = body.value()?;
                }
                backend::Message::ErrorResponse(body) => return Err(Error::db(body.fields())),
                backend::Message::NoticeResponse(_) => {
                    // TODO: collect notice and let Driver emit it when polled?
                }
                _ => return Err(Error::unexpected()),
            }
        }

        if !matches!(cfg.get_target_session_attrs(), TargetSessionAttrs::Any) {
            frontend::query("SHOW transaction_read_only", &mut buf)?;
            let msg = buf.split();
            drv.send(msg).await?;
            // TODO: use RowSimple for parsing?
            loop {
                match drv.recv().await? {
                    backend::Message::DataRow(body) => {
                        let range = body.ranges().next()?.flatten().ok_or(Error::todo())?;
                        let slice = &body.buffer()[range.start..range.end];
                        match (slice, cfg.get_target_session_attrs()) {
                            (b"on", TargetSessionAttrs::ReadWrite) => return Err(Error::todo()),
                            (b"off", TargetSessionAttrs::ReadOnly) => return Err(Error::todo()),
                            _ => {}
                        }
                    }
                    backend::Message::RowDescription(_) | backend::Message::CommandComplete(_) => {}
                    backend::Message::EmptyQueryResponse | backend::Message::ReadyForQuery(_) => break,
                    _ => return Err(Error::unexpected()),
                }
            }
        }

        Ok(session)
    }
}

#[cold]
#[inline(never)]
async fn auth<Io>(drv: &mut GenericDriver<Io>, cfg: &Config, buf: &mut BytesMut) -> Result<(), Error>
where
    Io: AsyncIo + Send,
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
                let pass = cfg.get_password().ok_or(ConfigError::MissingPassWord)?;
                send_pass(drv, pass, buf).await?;
            }
            backend::Message::AuthenticationMd5Password(body) => {
                let pass = cfg.get_password().ok_or(ConfigError::MissingPassWord)?;
                let user = cfg.get_user().ok_or(ConfigError::MissingUserName)?.as_bytes();
                let pass = authentication::md5_hash(user, pass, body.salt());
                send_pass(drv, pass, buf).await?;
            }
            backend::Message::AuthenticationSasl(body) => {
                let pass = cfg.get_password().ok_or(ConfigError::MissingPassWord)?;

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
                        match (buf, is_scram) {
                            (Some(buf), _) => (
                                sasl::ChannelBinding::tls_server_end_point(buf.to_owned()),
                                sasl::SCRAM_SHA_256_PLUS,
                            ),
                            (None, true) => (sasl::ChannelBinding::unrequested(), sasl::SCRAM_SHA_256),
                            // server ask for channel binding but no tls_server_end_point can be
                            // found.
                            _ => return Err(Error::todo()),
                        }
                    }
                    (false, true) => (sasl::ChannelBinding::unrequested(), sasl::SCRAM_SHA_256),
                    // TODO: return "unsupported SASL mechanism" error.
                    (false, false) => return Err(Error::todo()),
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
                    _ => return Err(Error::todo()),
                }

                match drv.recv().await? {
                    backend::Message::AuthenticationSaslFinal(body) => scram.finish(body.data())?,
                    _ => return Err(Error::todo()),
                }
            }
            backend::Message::ErrorResponse(_) => return Err(Error::from(ConfigError::WrongPassWord)),
            _ => {}
        }
    }
}

async fn send_pass<Io>(drv: &mut GenericDriver<Io>, pass: impl AsRef<[u8]>, buf: &mut BytesMut) -> Result<(), Error>
where
    Io: AsyncIo + Send,
{
    frontend::password_message(pass.as_ref(), buf)?;
    let msg = buf.split();
    drv.send(msg).await
}
