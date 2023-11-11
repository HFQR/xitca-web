//! udp socket with quic protocol as client transport layer.

mod response;

pub use self::response::Response;

use core::{future::Future, pin::Pin};

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use postgres_protocol::message::backend;
use quinn::{ClientConfig, Connection, Endpoint, ReadError, RecvStream, SendStream};
use quinn_proto::ConnectionError;
use xitca_io::bytes::{Bytes, BytesMut};

use crate::{
    client::Client,
    config::{Config, Host},
    error::{unexpected_eof_err, Error},
    iter::AsyncIterator,
};

use super::{tls::dangerous_config, Drive, Driver};

pub(crate) const QUIC_ALPN: &[u8] = b"quic";

pub(crate) struct ClientTx {
    counter: Arc<AtomicUsize>,
    inner: Connection,
}

impl Clone for ClientTx {
    fn clone(&self) -> Self {
        self.counter.fetch_add(1, Ordering::SeqCst);
        Self {
            counter: self.counter.clone(),
            inner: self.inner.clone(),
        }
    }
}

impl Drop for ClientTx {
    fn drop(&mut self) {
        // QuicDriver always hold one stream for receiving server
        // notify. ClientTx must call Connection::close manually
        // so server can observe the event.
        if self.counter.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.inner.close(0u8.into(), &[]);
        }
    }
}

impl ClientTx {
    fn new(inner: Connection) -> Self {
        Self {
            counter: Arc::new(AtomicUsize::new(1)),
            inner,
        }
    }

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

#[cold]
#[inline(never)]
pub(super) async fn _connect(host: Host, cfg: &mut Config) -> Result<(Client, Driver), Error> {
    match host {
        Host::Udp(ref host) => {
            let tx = connect_quic(host, cfg.get_ports()).await?;
            let streams = tx.inner.open_bi().await.unwrap();
            let mut drv = QuicDriver::new(streams);
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
                Ok(inner) => return Ok(ClientTx::new(inner)),
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
    pub(crate) tx: SendStream,
    pub(crate) rx: RecvStream,
    buf: BytesMut,
}

impl QuicDriver {
    pub(crate) fn new((tx, rx): (SendStream, RecvStream)) -> Self {
        Self {
            tx,
            rx,
            buf: BytesMut::new(),
        }
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

    async fn try_next(&mut self) -> Result<Option<backend::Message>, Error> {
        loop {
            if let Some(msg) = backend::Message::parse(&mut self.buf)? {
                return Ok(Some(msg));
            }

            match self.rx.read_chunk(4096, true).await {
                Ok(Some(chunk)) => self.buf.extend_from_slice(&chunk.bytes),
                Ok(None)
                | Err(ReadError::ConnectionLost(ConnectionError::ApplicationClosed(_)))
                | Err(ReadError::ConnectionLost(ConnectionError::LocallyClosed)) => return Ok(None),
                Err(_) => return Err(Error::ToDo),
            }
        }
    }

    async fn close_tx(&mut self) {
        let _ = self.tx.finish().await;
    }
}

impl AsyncIterator for QuicDriver {
    type Item<'i> = Result<backend::Message, Error> where Self: 'i;

    #[inline]
    async fn next(&mut self) -> Option<Self::Item<'_>> {
        self.try_next().await.transpose()
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
        Box::pin(async move { self.try_next().await?.ok_or_else(|| Error::from(unexpected_eof_err())) })
    }
}
