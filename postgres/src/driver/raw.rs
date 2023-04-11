//! tcp socket client.

mod response;
#[cfg(feature = "tls")]
mod tls;

pub use self::response::Response;

use std::io;

use postgres_protocol::message::frontend;
use tokio::sync::mpsc::unbounded_channel;
use xitca_io::{
    bytes::{Buf, BytesMut},
    io::{AsyncIo, Interest},
    net::TcpStream,
};

use crate::{
    client::Client,
    config::{Config, Host, SslMode},
    error::{unexpected_eof_err, write_zero_err, Error},
};

use super::{codec::Request, generic::GenericDriverTx, Driver};

#[derive(Debug)]
pub(crate) struct ClientTx(GenericDriverTx);

impl ClientTx {
    pub(crate) fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    pub(crate) async fn send(&self, msg: BytesMut) -> Result<Response, Error> {
        let (tx, rx) = unbounded_channel();
        self.0.send(Request::new(tx, msg))?;
        Ok(Response::new(rx))
    }

    pub(crate) fn do_send(&self, msg: BytesMut) {
        let (tx, _) = unbounded_channel();
        let _ = self.0.send(Request::new(tx, msg));
    }
}

type Ret = Result<(Client, Driver), Error>;

pub(crate) async fn connect(mut cfg: Config) -> Ret {
    super::try_connect_multi(&mut cfg, _connect).await
}

#[cold]
#[inline(never)]
async fn _connect(host: Host, cfg: &mut Config) -> Ret {
    // this block have repeated code due to HRTB limitation.
    // namely for <'_> AsyncIo::Future<'_>: Send bound can not be expressed correctly.
    match host {
        Host::Tcp(ref host) => {
            let mut io = connect_tcp(host, cfg.get_ports()).await?;
            if should_connect_tls(&mut io, cfg).await? {
                #[cfg(feature = "tls")]
                {
                    let io = tls::connect(io, host, cfg).await?;
                    let (mut drv, tx) = super::generic::new(io);
                    let mut cli = Client::new(ClientTx(tx));
                    cli.prepare_session(&mut drv, cfg).await?;
                    Ok((cli, Driver::tls(drv)))
                }
                #[cfg(not(feature = "tls"))]
                {
                    Err(Error::ToDo)
                }
            } else {
                let (mut drv, tx) = super::generic::new(io);
                let mut cli = Client::new(ClientTx(tx));
                cli.prepare_session(&mut drv, cfg).await?;
                Ok((cli, Driver::tcp(drv)))
            }
        }
        #[cfg(unix)]
        Host::Unix(ref host) => {
            let mut io = xitca_io::net::UnixStream::connect(host).await?;
            if should_connect_tls(&mut io, cfg).await? {
                #[cfg(feature = "tls")]
                {
                    let host = host.to_string_lossy();
                    let io = tls::connect(io, host.as_ref(), cfg).await?;
                    let (mut drv, tx) = super::generic::new(io);
                    let mut cli = Client::new(ClientTx(tx));
                    cli.prepare_session(&mut drv, cfg).await?;
                    Ok((cli, Driver::unix(drv)))
                }
                #[cfg(not(feature = "tls"))]
                {
                    Err(Error::ToDo)
                }
            } else {
                let (mut drv, tx) = super::generic::new(io);
                let mut cli = Client::new(ClientTx(tx));
                cli.prepare_session(&mut drv, cfg).await?;
                Ok((cli, Driver::unix_tls(drv)))
            }
        }
        _ => unreachable!(),
    }
}

async fn connect_tcp(host: &str, ports: &[u16]) -> Result<TcpStream, Error> {
    let addrs = super::resolve(host, ports).await?;

    let mut err = None;

    for addr in addrs {
        match TcpStream::connect(addr).await {
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Err(e) => err = Some(e),
        }
    }

    Err(err.unwrap().into())
}

async fn should_connect_tls<Io>(io: &mut Io, cfg: &Config) -> Result<bool, Error>
where
    Io: AsyncIo,
{
    match cfg.get_ssl_mode() {
        SslMode::Disable => Ok(false),
        SslMode::Prefer => _should_connect_tls(io).await.map_err(Into::into),
        SslMode::Require => {
            #[cfg(feature = "tls")]
            {
                if _should_connect_tls(io).await? {
                    return Ok(true);
                }
            }

            Err(Error::ToDo)
        }
    }
}

async fn _should_connect_tls<Io>(io: &mut Io) -> std::io::Result<bool>
where
    Io: AsyncIo,
{
    let mut buf = BytesMut::new();
    frontend::ssl_request(&mut buf);

    while !buf.is_empty() {
        io.ready(Interest::WRITABLE).await?;
        match io.write(&buf) {
            Ok(0) => return Err(write_zero_err()),
            Ok(n) => buf.advance(n),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
    }

    let mut buf = [0];
    loop {
        io.ready(Interest::READABLE).await?;
        match io.read(&mut buf) {
            Ok(0) => return Err(unexpected_eof_err()),
            Ok(_) => return Ok(buf[0] == b'S'),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
    }
}
