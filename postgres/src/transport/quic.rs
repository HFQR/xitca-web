//! udp socket with quic protocol as client transport layer.

mod response;

pub use self::response::Response;

use core::future::Future;

use alloc::sync::Arc;

use std::{collections::VecDeque, net::SocketAddr};

use quinn::{ClientConfig, Connection, Endpoint, SendStream, ServerConfig};
use rustls::{Certificate, OwnedTrustAnchor, PrivateKey, RootCertStore};
use tokio::sync::mpsc::channel;
use webpki_roots::TLS_SERVER_ROOTS;
use xitca_io::{
    bytes::{Buf, Bytes, BytesMut},
    io::{AsyncIo, Interest},
    net::TcpStream,
};
use xitca_unsafe_collection::bytes::{read_buf, PagedBytesMut};

use crate::transport::codec::ResponseMessage;
use crate::{
    client::Client,
    config::{Config, Host},
    error::Error,
};

#[derive(Clone, Debug)]
pub(crate) struct ClientTx {
    inner: Connection,
}

impl ClientTx {
    pub(crate) fn is_closed(&self) -> bool {
        self.inner.close_reason().is_some()
    }

    pub(crate) async fn send(&self, msg: BytesMut) -> Result<Response, Error> {
        let (mut tx, rx) = self.inner.open_bi().await.unwrap();
        tx.write_all(&msg).await.unwrap();
        tx.finish().await.unwrap();
        Ok(Response::new(rx))
    }

    pub(crate) async fn send2(&self, msg: BytesMut) -> Result<(), Error> {
        let mut tx = self.inner.open_uni().await.unwrap();
        tx.write_all(&msg).await.unwrap();
        tx.finish().await.unwrap();
        Ok(())
    }

    pub(crate) fn do_send(&self, msg: BytesMut) {
        let this = self.clone();
        tokio::spawn(async move {
            let _ = this.send(msg).await;
        });
    }
}

type Task = impl Future<Output = Result<(), Error>> + Send;
type Ret = Result<(Client, Task), Error>;

pub(crate) async fn connect(cfg: Config) -> Ret {
    super::try_connect_multi(&cfg, _connect).await
}

#[cold]
#[inline(never)]
pub(crate) async fn _connect(host: &Host, cfg: &Config) -> Ret {
    match *host {
        Host::Udp(ref host) => {
            let tx = connect_quic(host, cfg.get_ports()).await?;
            let mut cli = Client::new(tx);
            cli.authenticate(cfg).await?;
            Ok((cli, async { Ok(()) }))
        }
        _ => unreachable!(),
    }
}

#[cold]
#[inline(never)]
async fn connect_quic(host: &str, ports: &[u16]) -> Result<ClientTx, Error> {
    let addrs = super::resolve(host, ports).await?;

    let mut endpoint = Endpoint::client("0.0.0.0:0".parse().unwrap())?;

    let mut root_certs = RootCertStore::empty();

    root_certs.add_server_trust_anchors(TLS_SERVER_ROOTS.0.iter().map(|cert| {
        OwnedTrustAnchor::from_subject_spki_name_constraints(cert.subject, cert.spki, cert.name_constraints)
    }));

    let mut config = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_certs)
        .with_no_client_auth();

    config.alpn_protocols = vec![b"quic".to_vec()];

    let config = ClientConfig::new(Arc::new(config));

    endpoint.set_default_client_config(config);

    let mut err = None;

    for addr in addrs {
        match endpoint.connect(addr, host) {
            Ok(conn) => match conn.await {
                Ok(inner) => return Ok(ClientTx { inner }),
                Err(_) => err = Some(Error::ToDo),
            },
            Err(_) => err = Some(Error::ToDo),
        }
    }

    Err(err.unwrap())
}

pub async fn proxy() {
    let listen = "0.0.0.0:5440".parse().unwrap();
    let mut upstream = TcpStream::connect("127.0.0.1:5432").await.unwrap();

    let cert = std::fs::read("../cert/cert.pem").unwrap();
    let key = std::fs::read("../cert/key.pem").unwrap();

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
        .with_single_cert(cert, key)
        .unwrap();

    config.alpn_protocols = vec![b"quic".to_vec()];

    let config = ServerConfig::with_crypto(Arc::new(config));

    let listen = Endpoint::server(config, listen).unwrap();

    let (mut tx, mut rx) = channel::<(SendStream, VecDeque<Bytes>)>(128);

    tokio::spawn(async move {
        'out: while let Some((mut tx, mut bytes)) = rx.recv().await {
            while let Some(mut bytes) = bytes.pop_front() {
                use std::io::{Read, Write};

                'inner: loop {
                    match upstream.write(&bytes) {
                        Ok(0) => continue 'out,
                        Ok(n) => {
                            if n < bytes.len() {
                                bytes.advance(n);
                            } else {
                                break 'inner;
                            }
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(_) => continue 'out,
                    }
                    upstream.ready(Interest::WRITABLE).await.unwrap();
                }

                let mut buf = PagedBytesMut::<4096>::new();

                'inner: loop {
                    upstream.ready(Interest::READABLE).await.unwrap();
                    match read_buf(&mut upstream, &mut buf) {
                        Ok(0) => continue 'out,
                        Ok(_) => {
                            if let Some(msg) = ResponseMessage::try_from_buf(&mut buf).unwrap() {
                                match msg {
                                    ResponseMessage::Normal { buf, complete } => {
                                        tx.write_all(&buf).await.unwrap();
                                        if complete {
                                            tx.finish().await.unwrap();
                                            return;
                                        }
                                    }
                                    ResponseMessage::Async(_) => {}
                                }
                            }
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(_) => continue 'out,
                    }
                }
            }
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
}
