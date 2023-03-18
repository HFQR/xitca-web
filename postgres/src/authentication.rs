use fallible_iterator::FallibleIterator;
use postgres_protocol::{
    authentication::{self, sasl},
    message::{backend, frontend},
};

use super::{client::Client, config::Config, error::AuthenticationError, error::Error};

impl Client {
    #[cold]
    #[inline(never)]
    pub(crate) async fn authenticate(&self, cfg: Config) -> Result<(), Error> {
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

        let msg = self.try_encode_with(|buf| frontend::startup_message(params, buf))?;
        let mut res = self.send(msg).await?;

        loop {
            match res.recv().await? {
                backend::Message::AuthenticationOk => return Ok(()),
                backend::Message::AuthenticationCleartextPassword => {
                    let pass = cfg.get_password().ok_or(AuthenticationError::MissingPassWord)?;
                    self.send_pass(pass).await?;
                }
                backend::Message::AuthenticationMd5Password(body) => {
                    let pass = cfg.get_password().ok_or(AuthenticationError::MissingPassWord)?;
                    let user = cfg.get_user().ok_or(AuthenticationError::MissingUserName)?.as_bytes();
                    let pass = authentication::md5_hash(user, pass, body.salt());
                    self.send_pass(pass).await?;
                }
                backend::Message::AuthenticationSasl(body) => {
                    let pass = cfg.get_password().ok_or(AuthenticationError::MissingPassWord)?;

                    let mut is_scram = false;
                    let mut is_scram_plus = false;
                    let mut mechanisms = body.mechanisms();
                    let channel_binding = None;

                    while let Some(mechanism) = mechanisms.next()? {
                        match mechanism {
                            sasl::SCRAM_SHA_256 => is_scram = true,
                            sasl::SCRAM_SHA_256_PLUS => is_scram_plus = true,
                            _ => {}
                        }
                    }

                    let (channel_binding, mechanism) = match (channel_binding, is_scram_plus, is_scram) {
                        // TODO: return "unsupported SASL mechanism" error.
                        (_, false, false) => return Err(Error::ToDo),
                        (Some(binding), true, _) => (binding, sasl::SCRAM_SHA_256_PLUS),
                        (Some(_), false, true) => (sasl::ChannelBinding::unrequested(), sasl::SCRAM_SHA_256),
                        (None, _, _) => (sasl::ChannelBinding::unsupported(), sasl::SCRAM_SHA_256),
                    };

                    let mut scram = sasl::ScramSha256::new(pass, channel_binding);

                    let msg =
                        self.try_encode_with(|buf| frontend::sasl_initial_response(mechanism, scram.message(), buf))?;
                    self.send2(msg).await?;

                    match res.recv().await? {
                        backend::Message::AuthenticationSaslContinue(body) => {
                            scram.update(body.data())?;
                            let msg = self.try_encode_with(|buf| frontend::sasl_response(scram.message(), buf))?;
                            self.send2(msg).await?;
                        }
                        _ => return Err(Error::ToDo),
                    }

                    match res.recv().await? {
                        backend::Message::AuthenticationSaslFinal(body) => scram.finish(body.data())?,
                        _ => return Err(Error::ToDo),
                    }
                }
                backend::Message::ErrorResponse(_) => return Err(Error::from(AuthenticationError::WrongPassWord)),
                _ => {}
            }
        }
    }

    #[cold]
    #[inline(never)]
    async fn send_pass(&self, pass: impl AsRef<[u8]>) -> Result<(), Error> {
        let msg = self.try_encode_with(|buf| frontend::password_message(pass.as_ref(), buf))?;
        self.send2(msg).await
    }
}
