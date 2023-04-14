//! udp socket with quic protocol as client transport layer.

mod response;

pub use self::response::Response;

use core::{future::Future, pin::Pin};

use postgres_protocol::message::backend;
use quinn::{ClientConfig, Connection, Endpoint, RecvStream, SendStream};
use xitca_io::bytes::{Bytes, BytesMut};

use crate::{
    client::Client,
    config::{Config, Host},
    error::{unexpected_eof_err, Error},
    iter::AsyncIterator,
};

use super::{tls::dangerous_config, Drive, Driver};

pub(crate) const QUIC_ALPN: &[u8] = b"quic";

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

    pub(crate) fn do_send(&self, msg: BytesMut) {
        let this = self.clone();
        tokio::spawn(async move {
            let _ = this.send(msg).await;
        });
    }
}

type Ret = Result<(Client, Driver), Error>;

pub(crate) async fn connect(mut cfg: Config) -> Ret {
    super::try_connect_multi(&mut cfg, _connect).await
}

#[cold]
#[inline(never)]
pub(crate) async fn _connect(host: Host, cfg: &mut Config) -> Ret {
    match host {
        Host::Udp(ref host) => {
            let tx = connect_quic(host, cfg.get_ports()).await?;
            let mut drv = QuicDriver::try_new(&tx.inner).await?;
            let mut cli = Client::new(tx);
            cli.prepare_session(&mut drv, cfg).await?;
            drv.close_tx().await;
            Ok((cli, Driver::quic(drv)))
        }
        _ => unreachable!(),
    }
}

#[cold]
#[inline(never)]
async fn connect_quic(host: &str, ports: &[u16]) -> Result<ClientTx, Error> {
    let addrs = super::resolve(host, ports).await?;
    let mut endpoint = Endpoint::client("0.0.0.0:0".parse().unwrap())?;

    let cfg = dangerous_config(vec![QUIC_ALPN.to_vec()]);
    endpoint.set_default_client_config(ClientConfig::new(cfg));

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

// an arbitrary driver type for unified Drive trait with transport::driver::Driver type.
// *. QuicDriver does not act as actual driver and real IO is handled by `quinn` crate.
pub(crate) struct QuicDriver {
    tx: SendStream,
    rx: RecvStream,
    buf: BytesMut,
}

impl QuicDriver {
    pub(crate) async fn try_new(conn: &Connection) -> Result<Self, Error> {
        let (tx, rx) = conn.open_bi().await.unwrap();
        Ok(Self {
            tx,
            rx,
            buf: BytesMut::new(),
        })
    }

    pub(crate) async fn run(mut self) -> Result<(), Error> {
        while let Some(res) = self.next().await {
            res?;
        }
        Ok(())
    }

    pub(crate) async fn recv_raw(&mut self) -> Option<Result<Bytes, Error>> {
        self.rx
            .read_chunk(4096, true)
            .await
            .map(|c| c.map(|c| c.bytes))
            .map_err(|_| Error::ToDo)
            .transpose()
    }

    async fn close_tx(&mut self) {
        let _ = self.tx.finish().await;
    }
}

impl AsyncIterator for QuicDriver {
    type Future<'f> = impl Future<Output = Option<Self::Item<'f>>> + Send + 'f where Self: 'f;
    type Item<'i> = Result<backend::Message, Error> where Self: 'i;

    fn next(&mut self) -> Self::Future<'_> {
        async {
            loop {
                if let Some(res) = backend::Message::parse(&mut self.buf).transpose() {
                    return Some(res.map_err(Error::from));
                }

                let res = self.rx.read_chunk(4096, true).await.transpose()?;

                match res {
                    Ok(chunk) => self.buf.extend_from_slice(&chunk.bytes),
                    Err(_) => return Some(Err(Error::ToDo)),
                }
            }
        }
    }
}

impl Drive for QuicDriver {
    fn send(&mut self, msg: BytesMut) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + '_>> {
        Box::pin(async move {
            self.tx.write_all(&msg).await.unwrap();
            Ok(())
        })
    }

    fn recv(&mut self) -> Pin<Box<dyn Future<Output = Result<backend::Message, Error>> + Send + '_>> {
        Box::pin(async move {
            loop {
                if let Some(msg) = backend::Message::parse(&mut self.buf)? {
                    return Ok(msg);
                }

                let chunk = self
                    .rx
                    .read_chunk(4096, true)
                    .await
                    .unwrap()
                    .ok_or_else(unexpected_eof_err)?;

                self.buf.extend_from_slice(&chunk.bytes);
            }
        })
    }
}
