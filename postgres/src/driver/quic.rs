//! udp socket with quic protocol as client transport layer.

use core::{
    cmp,
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use std::{io, sync::Arc};

use tokio::sync::Mutex;
use xitca_io::{
    bytes::{Buf, Bytes},
    io::{AsyncIo, Interest, Ready},
};

use quinn::{ClientConfig, Endpoint};

use crate::error::Error;

pub(crate) const QUIC_ALPN: &[u8] = b"quic";

pub struct QuicStream {
    tx: Arc<Mutex<quinn::SendStream>>,
    rx: quinn::RecvStream,
    write_task: Option<Pin<Box<dyn Future<Output = io::Result<()>> + Send>>>,
    write_err: Option<io::Error>,
    read_buf: Option<Bytes>,
    read_err: Option<io::Error>,
    read_closed: bool,
}

impl QuicStream {
    pub(crate) fn new(tx: quinn::SendStream, rx: quinn::RecvStream) -> Self {
        Self {
            tx: Arc::new(Mutex::new(tx)),
            rx,
            write_task: None,
            write_err: None,
            read_buf: None,
            read_err: None,
            read_closed: false,
        }
    }
}

impl AsyncIo for QuicStream {
    async fn ready(&mut self, interest: Interest) -> io::Result<Ready> {
        let mut ready = Ready::EMPTY;
        if interest.is_readable() {
            if self.read_buf.is_some() {
                ready |= Ready::READABLE;
            } else {
                let res = self.rx.read_chunk(4096, true).await;
                ready |= Ready::READABLE;
                match res {
                    Ok(Some(chunk)) => self.read_buf = Some(chunk.bytes),
                    Ok(None) => self.read_closed = true,
                    Err(e) => self.read_err = Some(io::Error::other(e)),
                }
            }
        }

        if interest.is_writable() {
            if let Some(task) = self.write_task.as_mut() {
                if let Err(e) = task.await {
                    self.write_err = Some(e);
                }
                self.write_task = None;
            }

            ready |= Ready::WRITABLE;
        }

        Ok(ready)
    }

    fn poll_ready(&mut self, _: Interest, _: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        unimplemented!("poll_ready is not used!!!!")
    }

    fn is_vectored_write(&self) -> bool {
        false
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let Some(mut task) = this.write_task.take() {
            ready!(task.as_mut().poll(cx))?;
        }

        // lock only exists in write_task and tx fields.
        // write_task is resolved and dropped beforehand and this lock would always succeed.
        this.tx.try_lock().unwrap().poll_finish(cx).map_err(io::Error::other)
    }
}

impl io::Read for QuicStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(mut bytes) = self.read_buf.take() {
            let len = cmp::min(buf.len(), bytes.len());
            buf[..len].copy_from_slice(&bytes[..len]);
            bytes.advance(len);
            if !bytes.is_empty() {
                self.read_buf = Some(bytes);
            }
            return Ok(len);
        };

        if let Some(e) = self.read_err.take() {
            return Err(e);
        }

        if self.read_closed {
            return Ok(0);
        }

        Err(io::ErrorKind::WouldBlock.into())
    }
}

impl io::Write for QuicStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.write_task.is_some() {
            return Err(io::ErrorKind::WouldBlock.into());
        }

        let tx = self.tx.clone();
        let bytes = Bytes::copy_from_slice(buf);
        self.write_task = Some(Box::pin(async move {
            tx.lock().await.write_chunk(bytes).await.map_err(io::Error::other)
        }));

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn dangerous_config_rustls_0dot21(alpn: Vec<Vec<u8>>) -> Arc<rustls_0dot21::ClientConfig> {
    use std::time::SystemTime;

    use rustls_0dot21::{
        client::{ServerCertVerified, ServerCertVerifier},
        Certificate, ClientConfig, ServerName,
    };

    #[derive(Debug)]
    struct SkipServerVerification;

    impl SkipServerVerification {
        fn new() -> Arc<Self> {
            Arc::new(Self)
        }
    }

    impl ServerCertVerifier for SkipServerVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &Certificate,
            _intermediates: &[Certificate],
            _server_name: &ServerName,
            _scts: &mut dyn Iterator<Item = &[u8]>,
            _ocsp_response: &[u8],
            _now: SystemTime,
        ) -> Result<ServerCertVerified, rustls_0dot21::Error> {
            Ok(ServerCertVerified::assertion())
        }
    }

    let mut cfg = ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(SkipServerVerification::new())
        .with_no_client_auth();
    cfg.alpn_protocols = alpn;
    Arc::new(cfg)
}

#[cold]
#[inline(never)]
pub(crate) async fn connect_quic(host: &str, ports: &[u16]) -> Result<QuicStream, Error> {
    let addrs = super::resolve(host, ports).await?;
    let mut endpoint = Endpoint::client("0.0.0.0:0".parse().unwrap())?;

    let cfg = dangerous_config_rustls_0dot21(vec![QUIC_ALPN.to_vec()]);
    endpoint.set_default_client_config(ClientConfig::new(cfg));

    let mut err = None;

    for addr in addrs {
        match endpoint.connect(addr, host) {
            Ok(conn) => match conn.await {
                Ok(inner) => {
                    let (tx, rx) = inner.open_bi().await.unwrap();
                    return Ok(QuicStream::new(tx, rx));
                }
                Err(_) => err = Some(Error::todo()),
            },
            Err(_) => err = Some(Error::todo()),
        }
    }

    Err(err.unwrap())
}
