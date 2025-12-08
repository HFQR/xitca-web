use core::net::SocketAddr;

use xitca_io::net::TcpStream;

use crate::{
    config::{Config, Host},
    error::Error,
    session::{Addr, ConnectInfo, Session},
};

use super::{
    Driver, dns_resolve,
    generic::{DriverTx, GenericDriver},
    prepare_driver, should_connect_tls,
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
    let ssl_negotiation = cfg.get_ssl_negotiation();

    match host {
        Host::Tcp(host) => {
            let (mut io, addr) = connect_tcp(&host, cfg.get_ports()).await?;
            if should_connect_tls(&mut io, ssl_mode, ssl_negotiation).await? {
                #[cfg(feature = "tls")]
                {
                    let io = super::tls::connect_tls(io, &host, cfg).await?;
                    let info = ConnectInfo::new(Addr::Tcp(host, addr), ssl_mode, ssl_negotiation);
                    prepare_driver(info, io, cfg)
                        .await
                        .map(|(tx, session, drv)| (tx, session, Driver::Tls(drv)))
                }
                #[cfg(not(feature = "tls"))]
                {
                    Err(crate::error::FeatureError::Tls.into())
                }
            } else {
                let info = ConnectInfo::new(Addr::Tcp(host, addr), ssl_mode, ssl_negotiation);
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
            if should_connect_tls(&mut io, ssl_mode, ssl_negotiation).await? {
                #[cfg(feature = "tls")]
                {
                    let io = super::tls::connect_tls(io, host_str.as_ref(), cfg).await?;
                    let info = ConnectInfo::new(Addr::Unix(host_str, host), ssl_mode, ssl_negotiation);
                    prepare_driver(info, io, cfg)
                        .await
                        .map(|(tx, session, drv)| (tx, session, Driver::UnixTls(drv)))
                }
                #[cfg(not(feature = "tls"))]
                {
                    Err(crate::error::FeatureError::Tls.into())
                }
            } else {
                let info = ConnectInfo::new(Addr::Unix(host_str, host), ssl_mode, ssl_negotiation);
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
            let info = ConnectInfo::new(Addr::Quic(host, addr), ssl_mode, ssl_negotiation);
            prepare_driver(info, io, cfg)
                .await
                .map(|(tx, session, drv)| (tx, session, Driver::Quic(drv)))
        }
    }
}

#[cold]
#[inline(never)]
pub(super) async fn connect_info(info: ConnectInfo) -> Result<(DriverTx, Driver), Error> {
    let ConnectInfo {
        addr,
        ssl_mode,
        ssl_negotiation,
    } = info;
    match addr {
        Addr::Tcp(_host, addr) => {
            let mut io = TcpStream::connect(addr).await?;
            let _ = io.set_nodelay(true);

            if should_connect_tls(&mut io, ssl_mode, ssl_negotiation).await? {
                #[cfg(feature = "tls")]
                {
                    let io = super::tls::connect_tls(io, &_host, &mut Config::default()).await?;
                    let (io, tx) = GenericDriver::new(io);
                    Ok((tx, Driver::Tls(io)))
                }
                #[cfg(not(feature = "tls"))]
                {
                    Err(crate::error::FeatureError::Tls.into())
                }
            } else {
                let (io, tx) = GenericDriver::new(io);
                Ok((tx, Driver::Tcp(io)))
            }
        }
        #[cfg(unix)]
        Addr::Unix(_host, path) => {
            let mut io = xitca_io::net::UnixStream::connect(path).await?;
            if should_connect_tls(&mut io, ssl_mode, ssl_negotiation).await? {
                #[cfg(feature = "tls")]
                {
                    let io = super::tls::connect_tls(io, &_host, &mut Config::default()).await?;
                    let (io, tx) = GenericDriver::new(io);
                    Ok((tx, Driver::UnixTls(io)))
                }
                #[cfg(not(feature = "tls"))]
                {
                    Err(crate::error::FeatureError::Tls.into())
                }
            } else {
                let (io, tx) = GenericDriver::new(io);
                Ok((tx, Driver::Unix(io)))
            }
        }
        #[cfg(feature = "quic")]
        Addr::Quic(host, addr) => {
            let io = super::quic::connect_quic_addr(&host, addr).await?;
            let (io, tx) = GenericDriver::new(io);
            Ok((tx, Driver::Quic(io)))
        }
        Addr::None => Err(Error::todo()),
    }
}
