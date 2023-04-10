//! udp socket with quic protocol as client transport layer.

mod response;

pub use self::response::Response;

use core::{future::Future, pin::Pin};

use postgres_protocol::message::backend;
use quinn::{ClientConfig, Connection, Endpoint, RecvStream, SendStream};
use xitca_io::bytes::BytesMut;

use crate::{
    client::Client,
    config::{Config, Host},
    error::{unexpected_eof_err, Error},
};

use super::{tls::dangerous_config, MessageIo};

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

type Task = impl Future<Output = Result<(), Error>> + Send;
type Ret = Result<(Client, Task), Error>;

pub(crate) async fn connect(mut cfg: Config) -> Ret {
    super::try_connect_multi(&mut cfg, _connect).await
}

#[cold]
#[inline(never)]
pub(crate) async fn _connect(host: Host, cfg: &mut Config) -> Ret {
    match host {
        Host::Udp(ref host) => {
            let tx = connect_quic(host, cfg.get_ports()).await?;
            let mut cli = Client::new(tx);

            let mut io = Io::try_new(&cli).await?;
            cli.prepare_session(&mut io, cfg).await?;
            io.finish().await;

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

// an arbitrary io type for unified MessageIo trait impl for raw tcp/unix connections.
pub(super) struct Io {
    tx: SendStream,
    rx: RecvStream,
    buf: BytesMut,
}

impl Io {
    async fn try_new(cli: &Client) -> Result<Self, Error> {
        let (tx, rx) = cli.tx.inner.open_bi().await.unwrap();
        Ok(Self {
            tx,
            rx,
            buf: BytesMut::new(),
        })
    }

    async fn finish(mut self) {
        let _ = self.tx.finish().await;
    }
}

impl MessageIo for Io {
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
