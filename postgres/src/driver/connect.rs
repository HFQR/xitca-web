use std::{io, net::SocketAddr};

use postgres_protocol::message::frontend;
use xitca_io::{
    bytes::{Buf, BytesMut},
    io::{AsyncIo, Interest},
    net::TcpStream,
};

use crate::{
    config::{Config, Host, SslMode},
    error::{unexpected_eof_err, Error},
    session::prepare_session,
};

use super::{
    generic::{DriverTx, GenericDriver},
    Driver,
};

#[cold]
#[inline(never)]
pub(super) async fn connect(host: Host, cfg: &mut Config) -> Result<(DriverTx, Driver), Error> {
    // this block have repeated code due to HRTB limitation.
    // namely for <'_> AsyncIo::Future<'_>: Send bound can not be expressed correctly.
    match host {
        Host::Tcp(ref host) => {
            let mut io = connect_tcp(host, cfg.get_ports()).await?;
            if should_connect_tls(&mut io, cfg).await? {
                #[cfg(feature = "tls")]
                {
                    let io = super::tls::connect_tls(io, host, cfg).await?;
                    let (mut drv, tx) = GenericDriver::new(io);
                    prepare_session(&mut drv, cfg).await?;
                    Ok((tx, Driver::tls(drv)))
                }
                #[cfg(not(feature = "tls"))]
                {
                    Err(crate::error::FeatureError::Tls.into())
                }
            } else {
                let (mut drv, tx) = GenericDriver::new(io);
                prepare_session(&mut drv, cfg).await?;
                Ok((tx, Driver::tcp(drv)))
            }
        }
        #[cfg(unix)]
        Host::Unix(ref host) => {
            let mut io = xitca_io::net::UnixStream::connect(host).await?;
            if should_connect_tls(&mut io, cfg).await? {
                #[cfg(feature = "tls")]
                {
                    let host = host.to_string_lossy();
                    let io = super::tls::connect_tls(io, host.as_ref(), cfg).await?;
                    let (mut drv, tx) = GenericDriver::new(io);
                    prepare_session(&mut drv, cfg).await?;
                    Ok((tx, Driver::unix_tls(drv)))
                }
                #[cfg(not(feature = "tls"))]
                {
                    Err(crate::error::FeatureError::Tls.into())
                }
            } else {
                let (mut drv, tx) = GenericDriver::new(io);
                prepare_session(&mut drv, cfg).await?;
                Ok((tx, Driver::unix(drv)))
            }
        }
        Host::Quic(ref _host) => {
            #[cfg(feature = "quic")]
            {
                let io = super::quic::connect_quic(_host, cfg.get_ports()).await?;
                let (mut drv, tx) = GenericDriver::new(io);
                prepare_session(&mut drv, cfg).await?;
                Ok((tx, Driver::quic(drv)))
            }
            #[cfg(not(feature = "quic"))]
            {
                Err(crate::error::FeatureError::Quic.into())
            }
        }
    }
}

#[cold]
#[inline(never)]
pub(super) async fn connect_io<Io>(io: Io, cfg: &mut Config) -> Result<(DriverTx, Driver), Error>
where
    Io: AsyncIo + Send + 'static,
{
    let (mut drv, tx) = GenericDriver::new(Box::new(io) as _);
    prepare_session(&mut drv, cfg).await?;
    Ok((tx, Driver::dynamic(drv)))
}

async fn connect_tcp(host: &str, ports: &[u16]) -> Result<TcpStream, Error> {
    let addrs = resolve(host, ports).await?;

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
        mode => match (_should_connect_tls(io).await?, mode) {
            (false, SslMode::Require) => Err(Error::todo()),
            (bool, _) => Ok(bool),
        },
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
            Ok(0) => return Err(unexpected_eof_err()),
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

pub(super) async fn resolve(host: &str, ports: &[u16]) -> Result<Vec<SocketAddr>, Error> {
    let addrs = tokio::net::lookup_host((host, 0))
        .await?
        .flat_map(|mut addr| {
            ports.iter().map(move |port| {
                addr.set_port(*port);
                addr
            })
        })
        .collect::<Vec<_>>();
    Ok(addrs)
}
