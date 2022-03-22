use postgres_protocol::message::{backend, frontend};
use xitca_io::{io::AsyncIo, net::TcpStream};

use super::{config::Config, error::Error, io::BufferedIo};

pub(crate) async fn connect(cfg: &Config) -> Result<TcpStream, Error> {
    let host = cfg.get_hosts().first().unwrap();
    let port = cfg.get_ports().first().unwrap();

    match host {
        crate::config::Host::Tcp(host) => {
            use std::net::ToSocketAddrs;

            // use blocking dns resolve as it's not performance critical for connecting to db.
            let addr = (host.as_str(), *port).to_socket_addrs().unwrap().next().unwrap();

            Ok(TcpStream::connect(addr).await?)
        }
    }
}

pub(crate) async fn authenticate<Io, const BATCH_LIMIT: usize>(
    io: &mut BufferedIo<Io, BATCH_LIMIT>,
    cfg: Config,
) -> Result<(), Error>
where
    Io: AsyncIo,
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

    let mut res = io
        .linear_request(|buf| frontend::startup_message(params, buf).map_err(|_| Error::ToDo))
        .await?;

    match res.recv().await? {
        backend::Message::AuthenticationOk => {}
        backend::Message::AuthenticationCleartextPassword => {
            let res = io
                .linear_request(|buf| {
                    frontend::password_message(cfg.get_password().unwrap(), buf).map_err(|_| Error::ToDo)
                })
                .await?;
        }
        backend::Message::AuthenticationMd5Password(_body) => {}
        _ => {}
    };

    Ok(())
}
