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
use xitca_unsafe_collection::{
    bytes::{read_buf, PagedBytesMut},
    futures::{Select, SelectOutput},
};

use crate::{
    error::{unexpected_eof_err, write_zero_err},
    transport::{codec::ResponseMessage, QUIC_ALPN},
};

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

        config.alpn_protocols = vec![QUIC_ALPN.to_vec()];

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
            let tx2 = tx.clone();
            tokio::spawn(async move {
                let c = conn.await.unwrap();
                let c2 = c.clone();
                tokio::spawn(async move {
                    while let Ok((stream_tx, mut rx)) = c.accept_bi().await {
                        let mut bytes = Vec::new();
                        while let Some(c) = rx.read_chunk(usize::MAX, true).await.unwrap() {
                            bytes.push(c.bytes);
                        }
                        tx.send((Some(stream_tx), bytes)).await.unwrap();
                    }
                });

                tokio::spawn(async move {
                    while let Ok(mut rx) = c2.accept_uni().await {
                        let mut bytes = Vec::new();
                        while let Some(c) = rx.read_chunk(usize::MAX, true).await.unwrap() {
                            bytes.push(c.bytes);
                        }
                        tx2.send((None, bytes)).await.unwrap();
                    }
                });
            });
        }

        Ok(())
    }
}

async fn upstream_task(mut upstream: TcpStream, mut rx: Receiver<(Option<SendStream>, Vec<Bytes>)>) -> io::Result<()> {
    let mut buf_read = PagedBytesMut::<4096>::new();

    let mut req_queue = VecDeque::new();
    let mut res_queue = VecDeque::new();

    loop {
        let interest = if !req_queue.is_empty() {
            Interest::READABLE | Interest::WRITABLE
        } else {
            Interest::READABLE
        };

        match rx.recv().select(upstream.ready(interest)).await {
            SelectOutput::A(Some((tx, bytes))) => {
                req_queue.extend(bytes);
                if let Some(tx) = tx {
                    res_queue.push_back(tx);
                }
            }
            SelectOutput::A(None) => break,
            SelectOutput::B(ready) => {
                let ready = ready?;
                if ready.is_writable() {
                    'write: while let Some(bytes) = req_queue.front_mut() {
                        loop {
                            match upstream.write(bytes) {
                                Ok(0) => return Err(write_zero_err()),
                                Ok(n) => {
                                    bytes.advance(n);
                                    if bytes.is_empty() {
                                        req_queue.pop_front();
                                        continue 'write;
                                    }
                                }
                                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break 'write,
                                Err(e) => return Err(e),
                            }
                        }
                    }
                }

                if ready.is_readable() {
                    loop {
                        match read_buf(&mut upstream, &mut buf_read) {
                            Ok(0) => return Err(unexpected_eof_err()),
                            Ok(_) => {
                                while let Some(msg) = ResponseMessage::try_from_buf(&mut buf_read).unwrap() {
                                    match msg {
                                        ResponseMessage::Normal { buf, complete } => {
                                            let tx = res_queue.front_mut().unwrap();

                                            tx.write_chunk(buf.freeze()).await.unwrap();
                                            if complete {
                                                tx.finish().await.unwrap();
                                                res_queue.pop_front();
                                            }
                                        }
                                        ResponseMessage::Async(_) => {}
                                    }
                                }
                            }
                            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                            Err(e) => return Err(e),
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
