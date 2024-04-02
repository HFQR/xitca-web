//! udp socket with quic protocol as client transport layer.

use core::{
    cmp,
    future::Future,
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use std::{io, sync::Arc};

use quinn::{ClientConfig, Endpoint, RecvStream, SendStream};
use xitca_io::{
    bytes::{Buf, Bytes},
    io::{AsyncIo, Interest, Ready},
};

use crate::error::Error;

pub(crate) const QUIC_ALPN: &[u8] = b"quic";

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

pub struct QuicStream {
    writer: Writer,
    reader: Reader,
}

enum Writer {
    Tx(SendStream),
    Error(io::Error),
    InFlight(BoxFuture<io::Result<SendStream>>),
    Closed,
}

impl Writer {
    fn poll_ready(&mut self, interest: Interest, ready: &mut Ready, cx: &mut Context<'_>) {
        match self {
            Self::InFlight(ref mut fut) => {
                if let Poll::Ready(res) = fut.as_mut().poll(cx) {
                    if interest.is_writable() {
                        *ready |= Ready::WRITABLE;
                    }
                    match res {
                        Ok(tx) => *self = Self::Tx(tx),
                        Err(e) => *self = Self::Error(e),
                    }
                }
            }
            _ => {
                if interest.is_writable() {
                    *ready |= Ready::WRITABLE;
                }
            }
        }
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Error(_) => {
                let Self::Error(e) = mem::replace(self, Self::Closed) else {
                    unreachable!()
                };
                Err(e)
            }
            Self::Tx(_) => {
                let Self::Tx(mut tx) = mem::replace(self, Self::Closed) else {
                    unreachable!()
                };
                let bytes = Bytes::copy_from_slice(buf);
                *self = Self::InFlight(Box::pin(async move {
                    tx.write_chunk(bytes).await?;
                    Ok(tx)
                }));

                Ok(buf.len())
            }
            Self::InFlight(_) => Err(io::ErrorKind::WouldBlock.into()),
            Self::Closed => unreachable!(),
        }
    }

    fn poll_shutdown(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self {
            Self::Tx(tx) => tx.poll_finish(cx).map_err(io::Error::other),
            // TODO: handle in flight write.
            _ => Poll::Ready(Ok(())),
        }
    }
}

enum Reader {
    Buffered((Bytes, RecvStream)),
    InFlight(BoxFuture<io::Result<Option<(Bytes, RecvStream)>>>),
    Error(io::Error),
    Closed,
}

impl Reader {
    fn in_flight(mut rx: RecvStream) -> Self {
        Self::InFlight(Box::pin(async move {
            let chunk = rx.read_chunk(4096, true).await?;
            Ok(chunk.map(|c| (c.bytes, rx)))
        }))
    }

    fn poll_ready_once(&mut self, cx: &mut Context<'_>, ready: &mut Ready) {
        match self {
            Self::Buffered((ref bytes, _)) => {
                if !bytes.is_empty() {
                    *ready |= Ready::READABLE;
                    return;
                }

                let Self::Buffered((_, rx)) = mem::replace(self, Self::Closed) else {
                    unreachable!()
                };

                *self = Self::in_flight(rx);

                self.poll_ready_once(cx, ready);
            }
            Self::InFlight(ref mut fut) => {
                if let Poll::Ready(res) = fut.as_mut().poll(cx) {
                    *ready |= Ready::READABLE;
                    match res {
                        Ok(Some(res)) => *self = Self::Buffered(res),
                        Ok(None) => *self = Self::Closed,
                        Err(e) => *self = Self::Error(e),
                    }
                }
            }
            Self::Error(_) => *ready |= Ready::READABLE,
            Self::Closed => {}
        }
    }

    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Buffered((bytes, _)) => {
                let len = cmp::min(buf.len(), bytes.len());
                buf[..len].copy_from_slice(&bytes[..len]);
                bytes.advance(len);
                Ok(len)
            }
            Self::Error(_) => {
                let Self::Error(e) = mem::replace(self, Self::Closed) else {
                    unreachable!()
                };
                Err(e)
            }
            Self::Closed => Ok(0),
            _ => Err(io::ErrorKind::WouldBlock.into()),
        }
    }
}

impl QuicStream {
    pub(crate) fn new(tx: SendStream, rx: RecvStream) -> Self {
        Self {
            writer: Writer::Tx(tx),
            reader: Reader::in_flight(rx),
        }
    }
}

impl AsyncIo for QuicStream {
    async fn ready(&mut self, interest: Interest) -> io::Result<Ready> {
        core::future::poll_fn(|cx| self.poll_ready(interest, cx)).await
    }

    fn poll_ready(&mut self, interest: Interest, cx: &mut Context<'_>) -> Poll<io::Result<Ready>> {
        let mut ready = Ready::EMPTY;

        if interest.is_readable() {
            self.reader.poll_ready_once(cx, &mut ready);
        }

        self.writer.poll_ready(interest, &mut ready, cx);

        if ready.is_empty() {
            Poll::Pending
        } else {
            Poll::Ready(Ok(ready))
        }
    }

    fn is_vectored_write(&self) -> bool {
        false
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.get_mut().writer.poll_shutdown(cx)
    }
}

impl io::Read for QuicStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buf)
    }
}

impl io::Write for QuicStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writer.write(buf)
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
    let addrs = super::connect::resolve(host, ports).await?;
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
