//! tcp socket client.

mod io;
mod request;
mod response;

pub use self::{io::BufferedIo, response::Response};

use core::{future::Future, pin::Pin};

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use xitca_io::{bytes::BytesMut, net::TcpStream};

use crate::{
    client::Client,
    config::{Config, Host},
    error::Error,
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
    let cli = Client::new(ClientTx(tx));

    match *host {
        Host::Tcp(ref host) => {
            let io = connect_tcp(host, cfg.get_ports()).await?;
            let handle = BufferedIo::new(io, rx).spawn();

            let ret = cli.authenticate(cfg).await;
            // retrieve io regardless of authentication outcome.
            let io = handle.into_inner().await;

            ret.map(|_| (cli, Box::pin(io.run()) as _))
        }
        #[cfg(unix)]
        Host::Unix(ref host) => {
            let io = xitca_io::net::UnixStream::connect(host).await?;

            let handle = BufferedIo::new(io, rx).spawn();

            let ret = cli.authenticate(cfg).await;
            let io = handle.into_inner().await;

            ret.map(|_| (cli, Box::pin(io.run()) as _))
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
