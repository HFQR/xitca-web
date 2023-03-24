//! tcp socket client.

mod io;
mod request;
mod response;
#[cfg(feature = "tls")]
mod tls;

pub use self::response::Response;

use core::{future::Future, pin::Pin};

use postgres_protocol::message::frontend;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use xitca_io::{
    bytes::{Buf, BytesMut},
    io::{AsyncIo, Interest},
    net::TcpStream,
};

use crate::{
    client::Client,
    config::{Config, Host, SslMode},
    error::{unexpected_eof_err, Error},
};

use self::request::Request;

type ResponseSender = UnboundedSender<BytesMut>;

type ResponseReceiver = UnboundedReceiver<BytesMut>;

#[derive(Debug)]
pub(crate) struct ClientTx(UnboundedSender<Request>);

impl ClientTx {
    pub(crate) fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    pub(crate) async fn send(&self, msg: BytesMut) -> Result<Response, Error> {
        let (tx, rx) = unbounded_channel();
        self.0.send(Request::new(Some(tx), msg))?;
        Ok(Response::new(rx))
    }

    pub(crate) async fn send2(&self, msg: BytesMut) -> Result<(), Error> {
        self.0.send(Request::new(None, msg))?;
        Ok(())
    }

    pub(crate) fn do_send(&self, msg: BytesMut) {
        let _ = self.0.send(Request::new(None, msg));
    }
}

type Task = Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>;
type Ret = Result<(Client, Task), Error>;

pub(crate) async fn connect(cfg: Config) -> Ret {
    super::try_connect_multi(&cfg, _connect).await
}

#[cold]
#[inline(never)]
pub(crate) async fn _connect(host: &Host, cfg: &Config) -> Ret {
    let (tx, rx) = unbounded_channel();
    let mut cli = Client::new(ClientTx(tx));

    // this block have repeated code due to HRTB limitation.
    // namely for <'_> AsyncIo::Future<'_>: Send bound can not be expressed correctly.
    match *host {
        Host::Tcp(ref host) => {
            let mut io = connect_tcp(host, cfg.get_ports()).await?;
            if should_connect_tls(&mut io, cfg).await? {
                #[cfg(feature = "tls")]
                {
                    let io = self::tls::TlsStream::connect(io, host).await?;
                    let handle = io::new(io, rx).spawn();
                    let ret = cli.authenticate(cfg).await;
                    let io = handle.into_inner().await;
                    ret.map(|_| (cli, Box::pin(io.run()) as _))
                }
                #[cfg(not(feature = "tls"))]
                {
                    Err(Error::ToDo)
                }
            } else {
                let handle = io::new(io, rx).spawn();
                let ret = cli.authenticate(cfg).await;
                let io = handle.into_inner().await;
                ret.map(|_| (cli, Box::pin(io.run()) as _))
            }
        }
        #[cfg(unix)]
        Host::Unix(ref host) => {
            let mut io = xitca_io::net::UnixStream::connect(host).await?;
            if should_connect_tls(&mut io, cfg).await? {
                #[cfg(feature = "tls")]
                {
                    let io = self::tls::TlsStream::connect(io, host).await?;
                    let handle = io::new(io, rx).spawn();
                    let ret = cli.authenticate(cfg).await;
                    let io = handle.into_inner().await;
                    ret.map(|_| (cli, Box::pin(io.run()) as _))
                }
                #[cfg(not(feature = "tls"))]
                {
                    Err(Error::ToDo)
                }
            } else {
                let handle = io::new(io, rx).spawn();
                let ret = cli.authenticate(cfg).await;
                let io = handle.into_inner().await;
                ret.map(|_| (cli, Box::pin(io.run()) as _))
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
        SslMode::Prefer => _should_connect_tls(io).await,
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

async fn _should_connect_tls<Io>(io: &mut Io) -> Result<bool, Error>
where
    Io: AsyncIo,
{
    let mut buf = BytesMut::new();
    frontend::ssl_request(&mut buf);

    while !buf.is_empty() {
        io.ready(Interest::WRITABLE).await?;
        match io.write(&buf) {
            Ok(n) => buf.advance(n),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e.into()),
        }
    }

    let mut buf = [0];
    loop {
        io.ready(Interest::READABLE).await?;
        match io.read(&mut buf) {
            Ok(0) => return Err(unexpected_eof_err().into()),
            Ok(_) => return if buf[0] == b'S' { Ok(true) } else { Ok(false) },
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e.into()),
        }
    }
}
