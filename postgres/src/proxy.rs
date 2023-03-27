use alloc::{collections::VecDeque, sync::Arc};

use std::{
    error, fs,
    io::{self, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
};

use quinn::{Endpoint, SendStream};
use rustls::{Certificate, PrivateKey};
use tokio::sync::mpsc::{channel, Receiver};
use tracing::error;
use xitca_io::{
    bytes::{Buf, Bytes},
    io::{AsyncIo, Interest},
    net::TcpStream,
};
use xitca_unsafe_collection::bytes::{read_buf, PagedBytesMut};

use crate::{
    error::{unexpected_eof_err, write_zero_err},
    transport::codec::ResponseMessage,
};

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

    pub async fn run(self) -> Result<(), Box<dyn error::Error + Send + Sync>> {
        let cert = fs::read(self.cert)?;
        let key = fs::read(self.key)?;
        let key = rustls_pemfile::pkcs8_private_keys(&mut &*key).unwrap().remove(0);
        let key = PrivateKey(key);

        let cert = rustls_pemfile::certs(&mut &*cert)
            .unwrap()
            .into_iter()
            .map(Certificate)
            .collect();

        let mut config = rustls::ServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(cert, key)?;

        config.alpn_protocols = vec![b"quic".to_vec()];

        let config = quinn::ServerConfig::with_crypto(Arc::new(config));

        let listen = Endpoint::server(config, self.listen_addr)?;

        let upstream = TcpStream::connect(self.upstream_addr).await?;

        let (tx, rx) = channel(128);

        tokio::spawn(async move {
            if let Err(e) = upstream_task(upstream, rx).await {
                error!("Proxy upstream error: {e}");
            }
        });

        while let Some(conn) = listen.accept().await {
            let tx = tx.clone();
            tokio::spawn(async move {
                let c = conn.await.unwrap();
                let (stream_tx, mut rx) = c.accept_bi().await.unwrap();
                let mut bytes = VecDeque::new();
                while let Some(c) = rx.read_chunk(usize::MAX, true).await.unwrap() {
                    bytes.push_back(c.bytes);
                }
                tx.send((stream_tx, bytes)).await.unwrap();
            });
        }

        Ok(())
    }
}

async fn upstream_task(mut upstream: TcpStream, mut rx: Receiver<(SendStream, VecDeque<Bytes>)>) -> io::Result<()> {
    let mut buf = PagedBytesMut::<4096>::new();

    while let Some((mut tx, mut bytes)) = rx.recv().await {
        while let Some(mut bytes) = bytes.pop_front() {
            'write: loop {
                match upstream.write(&bytes) {
                    Ok(0) => return Err(write_zero_err()),
                    Ok(n) => {
                        bytes.advance(n);
                        if bytes.is_empty() {
                            continue 'write;
                        }
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e),
                }
                upstream.ready(Interest::WRITABLE).await?;
            }
        }

        'read: loop {
            upstream.ready(Interest::READABLE).await?;
            match read_buf(&mut upstream, &mut buf) {
                Ok(0) => return Err(unexpected_eof_err()),
                Ok(_) => {
                    while let Some(msg) = ResponseMessage::try_from_buf(&mut buf).unwrap() {
                        match msg {
                            ResponseMessage::Normal { buf, complete } => {
                                tx.write_chunk(buf.freeze()).await.unwrap();
                                if complete {
                                    tx.finish().await.unwrap();
                                    break 'read;
                                }
                            }
                            ResponseMessage::Async(_) => {}
                        }
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e),
            }
        }
    }

    Ok(())
}
