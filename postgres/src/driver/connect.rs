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

// applied on socket creation. errors are ignored: a platform refusing an option must not fail
// an otherwise healthy connection.
fn set_tcp_opt(stream: &TcpStream, cfg: &Config) {
    let _ = stream.set_nodelay(true);

    let socket = socket2::SockRef::from(stream);

    if cfg.get_keepalives() {
        // every timer is only applied when configured. leaving it unset keeps the system value
        // which is what libpq does.
        let mut keepalive = socket2::TcpKeepalive::new();

        if let Some(idle) = cfg.get_keepalives_idle() {
            keepalive = keepalive.with_time(idle);
        }

        if let Some(interval) = cfg.get_keepalives_interval() {
            keepalive = keepalive.with_interval(interval);
        }

        #[cfg(not(any(target_os = "openbsd", target_os = "redox", target_os = "solaris", windows)))]
        if let Some(retries) = cfg.get_keepalives_retries() {
            keepalive = keepalive.with_retries(retries);
        }

        let _ = socket.set_tcp_keepalive(&keepalive);
    }

    // keepalive can not observe a peer that keeps its receive window closed: it answers the zero
    // window probes so nothing ever times out. TCP_USER_TIMEOUT bounds that case.
    #[cfg(any(target_os = "android", target_os = "fuchsia", target_os = "linux"))]
    if let Some(timeout) = cfg.get_tcp_user_timeout() {
        let _ = socket.set_tcp_user_timeout(Some(timeout));
    }
}

#[cold]
#[inline(never)]
pub(super) async fn connect_host(host: Host, cfg: &mut Config) -> Result<(DriverTx, Session, Driver), Error> {
    async fn connect_tcp(host: &str, ports: &[u16], cfg: &Config) -> Result<(TcpStream, SocketAddr), Error> {
        let addrs = dns_resolve(host, ports).await?;

        let mut err = None;

        for addr in addrs {
            match TcpStream::connect(addr).await {
                Ok(stream) => {
                    set_tcp_opt(&stream, cfg);
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
            let (mut io, addr) = connect_tcp(&host, cfg.get_ports(), cfg).await?;
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

    #[allow(unused_mut)]
    let mut cfg = Config::default();
    let concurrency = cfg.get_max_queued_requests();

    match addr {
        Addr::Tcp(_host, addr) => {
            let mut io = TcpStream::connect(addr).await?;
            let _ = io.set_nodelay(true);

            if should_connect_tls(&mut io, ssl_mode, ssl_negotiation).await? {
                #[cfg(feature = "tls")]
                {
                    let io = super::tls::connect_tls(io, &_host, &mut cfg).await?;
                    let (io, tx) = GenericDriver::new(io, concurrency);
                    Ok((tx, Driver::Tls(io)))
                }
                #[cfg(not(feature = "tls"))]
                {
                    Err(crate::error::FeatureError::Tls.into())
                }
            } else {
                let (io, tx) = GenericDriver::new(io, concurrency);
                Ok((tx, Driver::Tcp(io)))
            }
        }
        #[cfg(unix)]
        Addr::Unix(_host, path) => {
            let mut io = xitca_io::net::UnixStream::connect(path).await?;
            if should_connect_tls(&mut io, ssl_mode, ssl_negotiation).await? {
                #[cfg(feature = "tls")]
                {
                    let io = super::tls::connect_tls(io, &_host, &mut cfg).await?;
                    let (io, tx) = GenericDriver::new(io, concurrency);
                    Ok((tx, Driver::UnixTls(io)))
                }
                #[cfg(not(feature = "tls"))]
                {
                    Err(crate::error::FeatureError::Tls.into())
                }
            } else {
                let (io, tx) = GenericDriver::new(io, concurrency);
                Ok((tx, Driver::Unix(io)))
            }
        }
        #[cfg(feature = "quic")]
        Addr::Quic(host, addr) => {
            let io = super::quic::connect_quic_addr(&host, addr).await?;
            let (io, tx) = GenericDriver::new(io, concurrency);
            Ok((tx, Driver::Quic(io)))
        }
        Addr::None => Err(Error::todo()),
    }
}

#[cfg(test)]
mod test {
    use core::time::Duration;

    use super::*;

    async fn connected_stream() -> TcpStream {
        let lis = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = lis.local_addr().unwrap();
        TcpStream::connect(addr).await.unwrap()
    }

    #[tokio::test]
    async fn tcp_opt_applied_to_socket() {
        let stream = connected_stream().await;

        let mut cfg = Config::new();
        cfg.keepalives_idle(Duration::from_secs(30))
            .keepalives_interval(Duration::from_secs(5))
            .keepalives_retries(3)
            .tcp_user_timeout(Duration::from_millis(10_000));

        set_tcp_opt(&stream, &cfg);

        let socket = socket2::SockRef::from(&stream);
        assert!(socket.keepalive().unwrap());

        #[cfg(target_os = "linux")]
        {
            assert_eq!(socket.tcp_keepalive_time().unwrap(), Duration::from_secs(30));
            assert_eq!(socket.tcp_keepalive_interval().unwrap(), Duration::from_secs(5));
            assert_eq!(socket.tcp_keepalive_retries().unwrap(), 3);
            assert_eq!(socket.tcp_user_timeout().unwrap(), Some(Duration::from_millis(10_000)));
        }
    }

    // libpq enables keepalive but leaves the timers alone. a system wide setting must survive.
    #[tokio::test]
    async fn keepalive_default_keeps_system_timers() {
        let stream = connected_stream().await;

        #[cfg(target_os = "linux")]
        let system = {
            let socket = socket2::SockRef::from(&stream);
            (
                socket.tcp_keepalive_time().unwrap(),
                socket.tcp_keepalive_interval().unwrap(),
                socket.tcp_keepalive_retries().unwrap(),
            )
        };

        set_tcp_opt(&stream, &Config::new());

        let socket = socket2::SockRef::from(&stream);
        assert!(socket.keepalive().unwrap());

        #[cfg(target_os = "linux")]
        {
            assert_eq!(socket.tcp_keepalive_time().unwrap(), system.0);
            assert_eq!(socket.tcp_keepalive_interval().unwrap(), system.1);
            assert_eq!(socket.tcp_keepalive_retries().unwrap(), system.2);
            assert_eq!(socket.tcp_user_timeout().unwrap(), None);
        }
    }

    #[tokio::test]
    async fn keepalive_can_be_disabled() {
        let stream = connected_stream().await;

        let mut cfg = Config::new();
        cfg.keepalives(false);

        set_tcp_opt(&stream, &cfg);

        assert!(!socket2::SockRef::from(&stream).keepalive().unwrap());
    }
}
