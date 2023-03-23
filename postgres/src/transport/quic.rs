//! udp socket with quic protocol as client transport layer.

mod codec;
mod response;

pub use self::response::Response;

use core::future::Future;

use alloc::sync::Arc;

use quinn::{ClientConfig, Connection, Endpoint};
use rustls::{OwnedTrustAnchor, RootCertStore};
use webpki_roots::TLS_SERVER_ROOTS;
use xitca_io::bytes::BytesMut;

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

    let mut root_certs = RootCertStore::empty();

    let certs = TLS_SERVER_ROOTS.0.iter().map(|cert| {
        OwnedTrustAnchor::from_subject_spki_name_constraints(cert.subject, cert.spki, cert.name_constraints)
    });

    root_certs.add_server_trust_anchors(certs);

    let mut crypto = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_root_certificates(root_certs)
        .with_no_client_auth();

    crypto.alpn_protocols = vec![b"quic".to_vec()];

    let config = ClientConfig::new(Arc::new(crypto));

    let mut endpoint = Endpoint::client("0.0.0.0:0".parse().unwrap())?;

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
