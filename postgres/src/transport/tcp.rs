//! tcp socket client.

mod io;
mod request;
mod response;

pub use self::{io::BufferedIo, request::Request, response::Response};

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use xitca_io::{bytes::BytesMut, net::TcpStream};

use crate::{
    client::Client,
    config::{Config, Host},
    error::Error,
};

type ResponseSender = UnboundedSender<BytesMut>;

type ResponseReceiver = UnboundedReceiver<BytesMut>;

pub(crate) fn new_pair(msg: BytesMut) -> (Request, Response) {
    let (tx, rx) = unbounded_channel();
    (Request::new(Some(tx), msg), Response::new(rx))
}

pub type ClientTx = UnboundedSender<Request>;

pub(crate) async fn connect2(
    cfg: Config,
) -> Result<(Client, impl std::future::Future<Output = Result<(), Error>> + Send), Error> {
    let io = connect(&cfg).await?;

    let (tx, rx) = unbounded_channel();
    let cli = Client::new(tx);
    let handle = BufferedIo::new(io, rx).spawn();

    let ret = cli.authenticate(cfg).await;
    // retrieve io regardless of authentication outcome.
    let io = handle.into_inner().await;

    ret.map(|_| (cli, io.run()))
}

#[cold]
#[inline(never)]
pub(crate) async fn connect(cfg: &Config) -> Result<TcpStream, Error> {
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
