use std::io;
use std::{collections::HashMap, net};

use crate::net::{AsListener, TcpSocket, TcpStream};
use crate::server::{
    Factory, IntoServiceFactoryClone, Server, ServerFuture, ServerFutureInner, ServiceFactoryClone,
};

pub struct Builder {
    pub(crate) server_threads: usize,
    pub(crate) worker_threads: usize,
    pub(crate) connection_limit: usize,
    pub(crate) max_blocking_threads: usize,
    pub(crate) listeners: HashMap<String, Vec<Box<dyn AsListener + Send>>>,
    pub(crate) factories: HashMap<String, Box<dyn ServiceFactoryClone>>,
    pub(crate) enable_signal: bool,
    backlog: u32,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    pub fn new() -> Self {
        Self {
            server_threads: 1,
            worker_threads: num_cpus::get(),
            connection_limit: 25600,
            max_blocking_threads: 512,
            listeners: HashMap::new(),
            factories: HashMap::new(),
            enable_signal: true,
            backlog: 2048,
        }
    }

    /// Set the thread count dedicated to accepting connections.
    ///
    /// Default set to 1.
    pub fn server_threads(mut self, num: usize) -> Self {
        assert_ne!(num, 0, "There must be at least one server thread");
        self.server_threads = num;
        self
    }

    /// Set the thread count for handling connections.
    ///
    /// Default set to machine's logic core count.
    pub fn worker_threads(mut self, num: usize) -> Self {
        assert_ne!(num, 0, "There must be at least one worker thread");

        self.worker_threads = num;
        self
    }

    pub fn connection_limit(mut self, num: usize) -> Self {
        assert_ne!(num, 0, "Connection limit must be higher than 0");

        self.connection_limit = num;
        self
    }

    pub fn max_blocking_threads(mut self, num: usize) -> Self {
        assert_ne!(num, 0, "Blocking threads must be higher than 0");

        self.max_blocking_threads = num;
        self
    }

    pub fn disable_signal(mut self) -> Self {
        self.enable_signal = false;
        self
    }

    pub fn backlog(mut self, num: u32) -> Self {
        self.backlog = num;
        self
    }

    pub fn bind<N, A, F>(self, name: N, addr: A, factory: F) -> io::Result<Self>
    where
        N: AsRef<str>,
        A: net::ToSocketAddrs,
        F: IntoServiceFactoryClone<TcpStream>,
    {
        let addr = addr.to_socket_addrs()?.next().ok_or_else(|| {
            io::Error::new(io::ErrorKind::AddrNotAvailable, "Can not parse SocketAddr")
        })?;

        let socket = if addr.is_ipv4() {
            TcpSocket::new_v4()?
        } else {
            TcpSocket::new_v6()?
        };

        socket.set_reuseaddr(true)?;
        socket.bind(addr)?;
        let listener = socket.listen(self.backlog)?.into_std()?;

        self.listen(name, listener, factory)
    }

    pub fn listen<N, F>(
        mut self,
        name: N,
        listener: net::TcpListener,
        factory: F,
    ) -> io::Result<Self>
    where
        N: AsRef<str>,
        F: IntoServiceFactoryClone<TcpStream>,
    {
        self.listeners
            .entry(name.as_ref().to_string())
            .or_insert_with(Vec::new)
            .push(Box::new(Some(listener)));

        let factory = Factory::create(factory);

        self.factories.insert(name.as_ref().to_string(), factory);

        Ok(self)
    }

    pub fn build(self) -> ServerFuture {
        let _enable_signal = self.enable_signal;
        match Server::new(self) {
            Ok(server) => ServerFuture::Server(ServerFutureInner {
                server,
                #[cfg(feature = "signal")]
                signals: if _enable_signal {
                    Some(crate::signals::Signals::start())
                } else {
                    None
                },
            }),
            Err(e) => ServerFuture::Error(e),
        }
    }
}

#[cfg(unix)]
impl Builder {
    pub fn bind_unix<N, P, F>(self, name: N, path: P, factory: F) -> io::Result<Self>
    where
        N: AsRef<str>,
        P: AsRef<std::path::Path>,
        F: IntoServiceFactoryClone<crate::net::UnixStream>,
    {
        // The path must not exist when we try to bind.
        // Try to remove it to avoid bind error.
        if let Err(e) = std::fs::remove_file(path.as_ref()) {
            // NotFound is expected and not an issue. Anything else is.
            if e.kind() != io::ErrorKind::NotFound {
                return Err(e);
            }
        }

        let listener = std::os::unix::net::UnixListener::bind(path)?;

        self.listen_unix(name, listener, factory)
    }

    pub fn listen_unix<N, F>(
        mut self,
        name: N,
        listener: std::os::unix::net::UnixListener,
        factory: F,
    ) -> io::Result<Self>
    where
        N: AsRef<str>,
        F: IntoServiceFactoryClone<crate::net::UnixStream>,
    {
        self.listeners
            .entry(name.as_ref().to_string())
            .or_insert_with(Vec::new)
            .push(Box::new(Some(listener)));

        let factory = Factory::create(factory);

        self.factories.insert(name.as_ref().to_string(), factory);

        Ok(self)
    }
}

#[cfg(feature = "http3")]
impl Builder {
    pub fn bind_h3<N, A, F>(
        mut self,
        name: N,
        addr: A,
        config: crate::net::H3ServerConfig,
        factory: F,
    ) -> io::Result<Self>
    where
        N: AsRef<str>,
        A: net::ToSocketAddrs,
        F: IntoServiceFactoryClone<crate::net::UdpStream>,
    {
        let addr = addr.to_socket_addrs()?.next().ok_or_else(|| {
            io::Error::new(io::ErrorKind::AddrNotAvailable, "Can not parse SocketAddr")
        })?;

        let builder = crate::net::UdpListenerBuilder::new(addr, config);

        self.listeners
            .entry(name.as_ref().to_string())
            .or_insert_with(Vec::new)
            .push(Box::new(Some(builder)));

        let factory = Factory::create(factory);

        self.factories.insert(name.as_ref().to_string(), factory);

        Ok(self)
    }
}
