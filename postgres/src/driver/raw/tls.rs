use sha2::{Digest, Sha256};
use xitca_io::io::AsyncIo;
use xitca_tls::rustls::{pki_types::ServerName, ClientConnection, TlsStream};

use crate::{config::Config, driver::tls::dangerous_config, error::Error};

pub(super) async fn connect<Io>(io: Io, host: &str, cfg: &mut Config) -> Result<TlsStream<ClientConnection, Io>, Error>
where
    Io: AsyncIo,
{
    let name = ServerName::try_from(host).map_err(|_| Error::ToDo)?.to_owned();
    let config = dangerous_config(Vec::new());
    let session = ClientConnection::new(config, name).map_err(|_| Error::ToDo)?;

    let stream = TlsStream::handshake(io, session).await?;

    if let Some(sha256) = stream
        .session()
        .peer_certificates()
        .and_then(|certs| certs.first())
        .map(|cert| Sha256::digest(cert.as_ref()).to_vec())
    {
        cfg.tls_server_end_point(sha256);
    }

    Ok(stream)
}
