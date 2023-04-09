use alloc::sync::Arc;

use std::{
    error, fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use quinn::{Connecting, Connection, Endpoint, RecvStream, SendStream};
use rustls::{Certificate, PrivateKey};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tracing::error;
use xitca_io::{bytes::BytesMut, net::TcpStream};
use xitca_unsafe_collection::futures::{Select, SelectOutput};

use super::{
    iter::AsyncIterator,
    transport::{self, codec::Request, QUIC_ALPN},
};

pub type Error = Box<dyn error::Error + Send + Sync>;

/// proxy for translating multiplexed quic traffic to plain tcp traffic for postgres protocol.
pub struct Proxy {
    cert: PathBuf,
    key: PathBuf,
    upstream_addr: SocketAddr,
    listen_addr: SocketAddr,
}

impl Proxy {
    pub fn with_cert(cert: impl AsRef<Path>, key: impl AsRef<Path>) -> Self {
        Self {
            cert: cert.as_ref().into(),
            key: key.as_ref().into(),
            upstream_addr: SocketAddr::from(([127, 0, 0, 1], 5432)),
            listen_addr: SocketAddr::from(([0, 0, 0, 0], 5433)),
        }
    }

    pub fn upstream_addr(mut self, addr: SocketAddr) -> Self {
        self.upstream_addr = addr;
        self
    }

    pub fn listen_addr(mut self, addr: SocketAddr) -> Self {
        self.listen_addr = addr;
        self
    }

    pub async fn run(self) -> Result<(), Error> {
        let cert = fs::read(self.cert)?;
        let key = fs::read(self.key)?;
        let key = rustls_pemfile::pkcs8_private_keys(&mut &*key).unwrap().remove(0);
        let key = PrivateKey(key);

        let cert = rustls_pemfile::certs(&mut &*cert)?
            .into_iter()
            .map(Certificate)
            .collect();

        let mut config = rustls::ServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(cert, key)?;

        config.alpn_protocols = vec![QUIC_ALPN.to_vec()];

        let config = quinn::ServerConfig::with_crypto(Arc::new(config));

        let listener = Endpoint::server(config, self.listen_addr)?;

        let addr = self.upstream_addr;
        while let Some(conn) = listener.accept().await {
            tokio::spawn(async move {
                if let Err(e) = listen_task(conn, addr).await {
                    error!("Proxy listen error: {e}");
                }
            });
        }

        Ok(())
    }
}

async fn listen_task(conn: Connecting, addr: SocketAddr) -> Result<(), Error> {
    let conn = conn.await?;
    let upstream = TcpStream::connect(addr).await?;
    let (tx, rx) = unbounded_channel();
    let conn2 = conn.clone();
    tokio::spawn(async move {
        if let Err(e) = upstream_task(upstream, rx, conn2).await {
            error!("Proxy upstream error: {e}");
        }
    });

    loop {
        match conn.accept_bi().select(conn.accept_uni()).await {
            SelectOutput::A(Ok((stream_tx, rx))) => handler(Some(stream_tx), &tx, rx),
            SelectOutput::B(Ok(rx)) => handler(None, &tx, rx),
            SelectOutput::A(Err(e)) | SelectOutput::B(Err(e)) => return Err(e.into()),
        }
    }
}

fn handler(stream_tx: Option<SendStream>, tx: &UnboundedSender<Request>, rx: RecvStream) {
    let tx = tx.clone();
    tokio::spawn(async move {
        if let Err(e) = _handler(stream_tx, tx, rx).await {
            error!("connection error: {e}");
        }
    });
}

async fn _handler(
    stream_tx: Option<SendStream>,
    tx: UnboundedSender<Request>,
    mut rx: RecvStream,
) -> Result<(), Error> {
    let mut bytes = BytesMut::new();
    while let Some(c) = rx.read_chunk(usize::MAX, true).await? {
        bytes.extend_from_slice(&c.bytes);
    }

    let (res, msg) = match stream_tx {
        Some(stream_tx) => {
            let (recv_tx, recv_rx) = unbounded_channel();
            (Some((recv_rx, stream_tx)), Request::new(Some(recv_tx), bytes))
        }
        None => (None, Request::new(None, bytes)),
    };

    tx.send(msg)?;

    if let Some((mut rx, mut stream_tx)) = res {
        while let Some(bytes) = rx.recv().await {
            stream_tx.write_chunk(bytes.freeze()).await?;
        }
        stream_tx.finish().await?;
    }

    Ok(())
}

async fn upstream_task(upstream: TcpStream, rx: UnboundedReceiver<Request>, _: Connection) -> Result<(), Error> {
    let mut io = transport::io::new(upstream, rx);
    while let Some(_) = io.next().await.transpose()? {}
    Ok(())
}
