//! tcp socket client.

mod io;
mod request;
mod response;

pub use self::{io::BufferedIo, response::Response};

use core::future::Future;

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

pub(crate) async fn connect(cfg: Config) -> Result<(Client, impl Future<Output = Result<(), Error>> + Send), Error> {
    let io = _connect(&cfg).await?;

    let (tx, rx) = unbounded_channel();
    let cli = Client::new(ClientTx(tx));
    let handle = BufferedIo::new(io, rx).spawn();

    let ret = cli.authenticate(cfg).await;
    // retrieve io regardless of authentication outcome.
    let io = handle.into_inner().await;

    ret.map(|_| (cli, io.run()))
}

#[cold]
#[inline(never)]
pub(crate) async fn _connect(cfg: &Config) -> Result<TcpStream, Error> {
    let hosts = cfg.get_hosts();
    let ports = cfg.get_ports();

    let mut err = None;

    for host in hosts {
        match host {
            Host::Tcp(host) => match connect_tcp(host, ports).await {
                Ok(stream) => return Ok(stream),
                Err(e) => err = Some(e),
            },
        }
    }

    Err(err.unwrap())
}

async fn connect_tcp(host: &str, ports: &[u16]) -> Result<TcpStream, Error> {
    let mut err = None;

    for port in ports {
        match tokio::net::lookup_host((host, *port)).await {
            Ok(addrs) => {
                for addr in addrs {
                    match TcpStream::connect(addr).await {
                        Ok(stream) => {
                            let _ = stream.set_nodelay(true);
                            return Ok(stream);
                        }
                        Err(e) => err = Some(e),
                    }
                }
            }
            Err(e) => err = Some(e),
        }
    }

    Err(err.unwrap().into())
}
