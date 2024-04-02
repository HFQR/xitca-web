use std::{collections::HashSet, error, fs, net::SocketAddr, path::Path, sync::Arc};

use quinn::{Connecting, Endpoint, ServerConfig};
use rustls_0dot21::{Certificate, PrivateKey};
use tracing::error;
use xitca_io::net::TcpStream;
use xitca_unsafe_collection::futures::{Select, SelectOutput};

use super::driver::{
    generic::GenericDriver,
    quic::{QuicStream, QUIC_ALPN},
    Drive,
};

pub type Error = Box<dyn error::Error + Send + Sync>;

/// proxy for translating multiplexed quic traffic to plain tcp traffic for postgres protocol.
pub struct Proxy {
    cfg: Result<ServerConfig, Error>,
    upstream_addr: SocketAddr,
    listen_addr: SocketAddr,
    white_list: Option<HashSet<SocketAddr>>,
}

impl Proxy {
    fn new(cfg: Result<ServerConfig, Error>) -> Self {
        Self {
            cfg,
            upstream_addr: SocketAddr::from(([127, 0, 0, 1], 5432)),
            listen_addr: SocketAddr::from(([0, 0, 0, 0], 5433)),
            white_list: None,
        }
    }

    /// construct a proxy with given single key/cert pair.
    /// key/cert must be pem format.
    pub fn with_cert(cert: impl AsRef<Path>, key: impl AsRef<Path>) -> Self {
        Self::new(cfg_from_cert(cert, key))
    }

    /// construct a proxy with given [quinn::ServerConfig]
    pub fn with_config(cfg: ServerConfig) -> Self {
        Self::new(Ok(cfg))
    }

    /// set upstream postgres server the proxy would connect to.
    pub fn upstream_addr(mut self, addr: SocketAddr) -> Self {
        self.upstream_addr = addr;
        self
    }

    /// set the address proxy used for listening incoming request.
    pub fn listen_addr(mut self, addr: SocketAddr) -> Self {
        self.listen_addr = addr;
        self
    }

    /// set address that belong to proxy's whitelist. once set address do not belong in the list
    /// would be reject from proxy.
    pub fn white_list(mut self, addrs: impl IntoIterator<Item = SocketAddr>) -> Self {
        for addr in addrs.into_iter() {
            self.white_list.get_or_insert_with(HashSet::new).insert(addr);
        }
        self
    }

    /// start the proxy.
    pub async fn run(self) -> Result<(), Error> {
        let cfg = self.cfg?;

        let listener = Endpoint::server(cfg, self.listen_addr)?;

        let addr = self.upstream_addr;
        while let Some(conn) = listener.accept().await {
            if let Some(list) = self.white_list.as_ref() {
                if !list.contains(&conn.remote_address()) {
                    continue;
                }
            }
            tokio::spawn(async move {
                if let Err(e) = listen_task(conn, addr).await {
                    error!("Proxy listen error: {e}");
                }
            });
        }

        Ok(())
    }
}

fn cfg_from_cert(cert: impl AsRef<Path>, key: impl AsRef<Path>) -> Result<ServerConfig, Error> {
    let cert = fs::read(cert)?;
    let key = fs::read(key)?;
    let key = rustls_pemfile::pkcs8_private_keys(&mut &*key).unwrap().remove(0);
    let key = PrivateKey(key);

    let cert = rustls_pemfile::certs(&mut &*cert)?
        .into_iter()
        .map(Certificate)
        .collect();

    let mut config = rustls_0dot21::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(cert, key)?;

    config.alpn_protocols = vec![QUIC_ALPN.to_vec()];

    Ok(ServerConfig::with_crypto(Arc::new(config)))
}

async fn listen_task(conn: Connecting, addr: SocketAddr) -> Result<(), Error> {
    let conn = conn.await?;
    let upstream = TcpStream::connect(addr).await?;

    let (mut drv, _tx) = GenericDriver::new(upstream);

    let (tx, rx) = conn.accept_bi().await?;
    let (mut quic_drv, _quic_tx) = GenericDriver::new(QuicStream::new(tx, rx));

    loop {
        match quic_drv
            .recv_with(|buf| (!buf.is_empty()).then(|| Ok(buf.split())))
            .select(drv.recv_with(|buf| (!buf.is_empty()).then(|| Ok(buf.split()))))
            .await
        {
            SelectOutput::A(res) => {
                let msg = res?;
                drv.send(msg).await?;
            }
            SelectOutput::B(res) => {
                let msg = res?;
                quic_drv.send(msg).await?;
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn construct() {
        let addr = "127.0.0.1:0".parse().unwrap();
        let _ = Proxy::with_cert("", "")
            .upstream_addr(addr)
            .listen_addr(addr)
            .white_list(vec![addr])
            .white_list([addr]);
    }
}
