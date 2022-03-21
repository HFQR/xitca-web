use std::io;

use xitca_io::net::TcpStream;

use super::config::Config;

pub(crate) async fn connect(cfg: Config) -> io::Result<TcpStream> {
    let host = cfg.get_hosts().first().unwrap();
    let port = cfg.get_ports().first().unwrap();

    match host {
        crate::config::Host::Tcp(host) => {
            use std::net::ToSocketAddrs;
            let addr = (host.as_str(), *port).to_socket_addrs().unwrap().next().unwrap();

            TcpStream::connect(addr).await
        }
    }
}
