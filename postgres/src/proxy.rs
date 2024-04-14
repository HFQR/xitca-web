//! proxy serves as a sample implementation of server side traffic forwarder
//! between a xitca-postgres Client with `quic` feature enabled and the postgres
//! database server.

use std::{
    collections::HashSet,
    error, fs,
    io::{self, Read, Write},
    net::SocketAddr,
    path::Path,
    sync::Arc,
};

use quinn::{Endpoint, Incoming, ServerConfig};
use rustls_0dot21::{Certificate, PrivateKey};
use tracing::error;
use xitca_io::{
    bytes::Buf,
    io::{AsyncIo, Interest},
    net::TcpStream,
};
use xitca_unsafe_collection::futures::{Select, SelectOutput};

use super::driver::quic::QUIC_ALPN;

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

async fn listen_task(conn: Incoming, addr: SocketAddr) -> Result<(), Error> {
    let conn = conn.await?;

    // bridge quic client connection to tcp connection to database.
    let mut upstream = TcpStream::connect(addr).await?;

    // the proxy does not multiplex over streams from a quic client connection but it's not a hard
    // requirement.
    // an alternative proxy implementation can multiplex tcp socket connection to database with
    // additional bidirectional stream.
    let (mut tx, mut rx) = conn.accept_bi().await?;

    // loop and copy bytes between the quic stream and tcp socket.

    let mut buf = [0; 4096];

    loop {
        match rx
            .read_chunk(4096, true)
            .select(upstream.ready(Interest::READABLE))
            .await
        {
            SelectOutput::A(Ok(Some(chunk))) => {
                let mut bytes = chunk.bytes;
                while !bytes.is_empty() {
                    match upstream.write(bytes.as_ref()) {
                        Ok(0) => return Ok(()),
                        Ok(n) => {
                            bytes.advance(n);
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            upstream.ready(Interest::WRITABLE).await?;
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
            }
            SelectOutput::B(Ok(_)) => 'inner: loop {
                match upstream.read(&mut buf) {
                    Ok(0) => return Ok(()),
                    Ok(n) => tx.write_all(&buf[..n]).await?,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break 'inner,
                    Err(e) => return Err(e.into()),
                }
            },
            SelectOutput::A(Err(e)) => return Err(e.into()),
            SelectOutput::B(Err(e)) => return Err(e.into()),
            _ => return Ok(()),
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
