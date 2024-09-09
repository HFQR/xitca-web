use core::net::SocketAddr;

use std::io;

use postgres_protocol::message::frontend;
use xitca_io::{
    bytes::{Buf, BytesMut},
    io::{AsyncIo, Interest},
    net::TcpStream,
};

use crate::{
    config::{Config, Host, SslMode},
    error::{unexpected_eof_err, Error},
    session::{Addr, ConnectInfo, Session},
};

use super::{
    dns_resolve,
    generic::{DriverTx, GenericDriver},
    Driver,
};

#[cold]
#[inline(never)]
pub(super) async fn connect_host(host: Host, cfg: &mut Config) -> Result<(DriverTx, Session, Driver), Error> {
    async fn connect_tcp(host: &str, ports: &[u16]) -> Result<(TcpStream, SocketAddr), Error> {
        let addrs = dns_resolve(host, ports).await?;

        let mut err = None;

        for addr in addrs {
            match TcpStream::connect(addr).await {
                Ok(stream) => {
                    let _ = stream.set_nodelay(true);
                    return Ok((stream, addr));
                }
                Err(e) => err = Some(e),
            }
        }

        Err(err.unwrap().into())
    }

    let ssl_mode = cfg.get_ssl_mode();

    match host {
        Host::Tcp(host) => {
            let (mut io, addr) = connect_tcp(&host, cfg.get_ports()).await?;
            if should_connect_tls(&mut io, ssl_mode).await? {
                #[cfg(feature = "tls")]
                {
                    let io = super::tls::connect_tls(io, &host, cfg).await?;
                    let info = ConnectInfo::new(Addr::Tcp(host, addr), ssl_mode);
                    prepare_driver(info, io, cfg)
                        .await
                        .map(|(tx, session, drv)| (tx, session, Driver::Tls(drv)))
                }
                #[cfg(not(feature = "tls"))]
                {
                    Err(crate::error::FeatureError::Tls.into())
                }
            } else {
                let info = ConnectInfo::new(Addr::Tcp(host, addr), ssl_mode);
                prepare_driver(info, io, cfg)
                    .await
                    .map(|(tx, session, drv)| (tx, session, Driver::Tcp(drv)))
            }
        }
        #[cfg(not(unix))]
        Host::Unix(_) => Err(crate::error::SystemError::Unix.into()),
        #[cfg(unix)]
        Host::Unix(host) => {
            let mut io = xitca_io::net::UnixStream::connect(&host).await?;
            let host_str: Box<str> = host.to_string_lossy().into();
            if should_connect_tls(&mut io, ssl_mode).await? {
                #[cfg(feature = "tls")]
                {
                    let io = super::tls::connect_tls(io, host_str.as_ref(), cfg).await?;
                    let info = ConnectInfo::new(Addr::Unix(host_str, host), ssl_mode);
                    prepare_driver(info, io, cfg)
                        .await
                        .map(|(tx, session, drv)| (tx, session, Driver::UnixTls(drv)))
                }
                #[cfg(not(feature = "tls"))]
                {
                    Err(crate::error::FeatureError::Tls.into())
                }
            } else {
                let info = ConnectInfo::new(Addr::Unix(host_str, host), ssl_mode);
                prepare_driver(info, io, cfg)
                    .await
                    .map(|(tx, session, drv)| (tx, session, Driver::Unix(drv)))
            }
        }
        #[cfg(not(feature = "quic"))]
        Host::Quic(_) => Err(crate::error::FeatureError::Quic.into()),
        #[cfg(feature = "quic")]
        Host::Quic(host) => {
            let (io, addr) = super::quic::connect_quic(&host, cfg.get_ports()).await?;
            let info = ConnectInfo::new(Addr::Quic(host, addr), ssl_mode);
            prepare_driver(info, io, cfg)
                .await
                .map(|(tx, session, drv)| (tx, session, Driver::Quic(drv)))
        }
    }
}

#[cold]
#[inline(never)]
pub(super) async fn connect_info(info: ConnectInfo) -> Result<Driver, Error> {
    let ConnectInfo { addr, ssl_mode } = info;
    match addr {
        Addr::Tcp(_host, addr) => {
            let mut io = TcpStream::connect(addr).await?;
            let _ = io.set_nodelay(true);

            if should_connect_tls(&mut io, ssl_mode).await? {
                #[cfg(feature = "tls")]
                {
                    let io = super::tls::connect_tls(io, &_host, &mut Config::default()).await?;
                    Ok(Driver::Tls(GenericDriver::new(io).0))
                }
                #[cfg(not(feature = "tls"))]
                {
                    Err(crate::error::FeatureError::Tls.into())
                }
            } else {
                Ok(Driver::Tcp(GenericDriver::new(io).0))
            }
        }
        #[cfg(unix)]
        Addr::Unix(_host, path) => {
            let mut io = xitca_io::net::UnixStream::connect(path).await?;
            if should_connect_tls(&mut io, ssl_mode).await? {
                #[cfg(feature = "tls")]
                {
                    let io = super::tls::connect_tls(io, &_host, &mut Config::default()).await?;
                    Ok(Driver::UnixTls(GenericDriver::new(io).0))
                }
                #[cfg(not(feature = "tls"))]
                {
                    Err(crate::error::FeatureError::Tls.into())
                }
            } else {
                Ok(Driver::Unix(GenericDriver::new(io).0))
            }
        }
        #[cfg(feature = "quic")]
        Addr::Quic(host, addr) => {
            let io = super::quic::connect_quic_addr(&host, addr).await?;
            Ok(Driver::Quic(GenericDriver::new(io).0))
        }
        Addr::None => Err(Error::todo()),
    }
}

#[cold]
#[inline(never)]
pub(super) async fn connect_io<Io>(io: Io, cfg: &mut Config) -> Result<(DriverTx, Session, Driver), Error>
where
    Io: AsyncIo + Send + 'static,
{
    prepare_driver(ConnectInfo::default(), Box::new(io) as _, cfg)
        .await
        .map(|(tx, session, drv)| (tx, session, Driver::Dynamic(drv)))
}

async fn prepare_driver<Io>(
    info: ConnectInfo,
    io: Io,
    cfg: &mut Config,
) -> Result<(DriverTx, Session, GenericDriver<Io>), Error>
where
    Io: AsyncIo + Send + 'static,
{
    let (mut drv, tx) = GenericDriver::new(io);
    let session = Session::prepare_session(info, &mut drv, cfg).await?;
    Ok((tx, session, drv))
}

async fn should_connect_tls<Io>(io: &mut Io, ssl_mode: SslMode) -> Result<bool, Error>
where
    Io: AsyncIo,
{
    async fn query_tls_availability<Io>(io: &mut Io) -> std::io::Result<bool>
    where
        Io: AsyncIo,
    {
        let mut buf = BytesMut::new();
        frontend::ssl_request(&mut buf);

        while !buf.is_empty() {
            match io.write(&buf) {
                Ok(0) => return Err(unexpected_eof_err()),
                Ok(n) => buf.advance(n),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    io.ready(Interest::WRITABLE).await?;
                }
                Err(e) => return Err(e),
            }
        }

        let mut buf = [0];
        loop {
            match io.read(&mut buf) {
                Ok(0) => return Err(unexpected_eof_err()),
                Ok(_) => return Ok(buf[0] == b'S'),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    io.ready(Interest::READABLE).await?;
                }
                Err(e) => return Err(e),
            }
        }
    }

    match ssl_mode {
        SslMode::Disable => Ok(false),
        mode => match (query_tls_availability(io).await?, mode) {
            (false, SslMode::Require) => Err(Error::todo()),
            (bool, _) => Ok(bool),
        },
    }
}
