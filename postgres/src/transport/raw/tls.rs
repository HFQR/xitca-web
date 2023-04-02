use ring::digest;
use rustls::ClientConnection;
use xitca_io::io::AsyncIo;
use xitca_tls::rustls::TlsStream;

use crate::{config::Config, error::Error, transport::tls::dangerous_config};

pub(super) async fn connect<Io>(io: Io, host: &str, cfg: &mut Config) -> Result<TlsStream<ClientConnection, Io>, Error>
where
    Io: AsyncIo,
{
    let name = host.try_into().map_err(|_| Error::ToDo)?;
    let config = dangerous_config(Vec::new());
    let session = ClientConnection::new(config, name).map_err(|_| Error::ToDo)?;

    let stream = TlsStream::handshake(io, session).await?;

    if let Some(sha256) = stream
        .session()
        .peer_certificates()
        .and_then(|certs| certs.get(0))
        .map(|cert| digest::digest(&digest::SHA256, cert.as_ref()))
    {
        cfg.tls_server_end_point(sha256.as_ref().into());
    }

    Ok(stream)
}
