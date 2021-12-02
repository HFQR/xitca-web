use std::{collections::HashMap, future::Future, io, net, pin::Pin, time::Duration};

use xitca_io::net::TcpSocket;

use crate::net::{AsListener, FromStream};
use crate::server::{AsServiceFactoryClone, Factory, Server, ServerFuture, ServerFutureInner, ServiceFactoryClone};

pub struct Builder {
    pub(crate) server_threads: usize,
    pub(crate) worker_threads: usize,
    pub(crate) connection_limit: usize,
    pub(crate) worker_max_blocking_threads: usize,
    pub(crate) listeners: HashMap<String, Vec<Box<dyn AsListener>>>,
    pub(crate) factories: HashMap<String, Box<dyn ServiceFactoryClone>>,
    pub(crate) enable_signal: bool,
    pub(crate) shutdown_timeout: Duration,
    pub(crate) on_worker_start: Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>,
    backlog: u32,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    /// Create new Builder instance
    pub fn new() -> Self {
        Self {
            server_threads: 1,
            worker_threads: num_cpus::get(),
            connection_limit: 25600,
            worker_max_blocking_threads: 512,
            listeners: HashMap::new(),
            factories: HashMap::new(),
            enable_signal: true,
            shutdown_timeout: Duration::from_secs(30),
            on_worker_start: Box::new(|| Box::pin(async {})),
            backlog: 2048,
        }
    }

    /// Set number of threads dedicated to accepting connections.
    ///
    /// Default set to 1.
    ///
    /// # Panics:
    /// When receive 0 as number of server thread.
    pub fn server_threads(mut self, num: usize) -> Self {
        assert_ne!(num, 0, "There must be at least one server thread");
        self.server_threads = num;
        self
    }

    /// Set number of workers to start.
    ///
    /// Default set to available logical cpu as workers count.
    ///
    /// # Panics:
    /// When received 0 as number of worker thread.
    pub fn worker_threads(mut self, num: usize) -> Self {
        assert_ne!(num, 0, "There must be at least one worker thread");

        self.worker_threads = num;
        self
    }

    /// Set limit of connection count for a single worker thread.
    ///
    /// When reaching limit a worker thread would enter backpressure state and stop
    /// accepting new connections until living connections reduces below the limit.
    ///
    /// A worker thread enter backpressure does not prevent other worker threads from
    /// accepting new connections as long as they have not reached their connection
    /// limits.
    ///
    /// Default set to 25_600.
    ///
    /// # Panics:
    /// When received 0 as number of connection limit.
    pub fn connection_limit(mut self, num: usize) -> Self {
        assert_ne!(num, 0, "Connection limit must be higher than 0");

        self.connection_limit = num;
        self
    }

    /// Set max number of threads for each worker's blocking task thread pool.
    ///
    /// One thread pool is set up **per worker**; not shared across workers.
    ///
    /// # Examples:
    /// ```
    /// # use xitca_server::Builder;
    /// let builder = Builder::new()
    ///     .worker_threads(4) // server has 4 worker threads.
    ///     .worker_max_blocking_threads(4); // every worker has 4 max blocking threads.
    /// ```
    ///
    /// See [tokio::runtime::Builder::max_blocking_threads] for behavior reference.
    pub fn worker_max_blocking_threads(mut self, num: usize) -> Self {
        assert_ne!(num, 0, "Blocking threads must be higher than 0");

        self.worker_max_blocking_threads = num;
        self
    }

    /// Disable signal listening.
    ///
    /// `tokio::signal` is used for listening and it only functional in tokio runtime 1.x.
    /// Disabling it would enable server runs in other async runtimes.
    pub fn disable_signal(mut self) -> Self {
        self.enable_signal = false;
        self
    }

    /// Timeout for graceful workers shutdown in seconds.
    ///
    /// After receiving a stop signal, workers have this much time to finish serving requests.
    /// Workers still alive after the timeout are force dropped.
    ///
    /// By default shutdown timeout sets to 30 seconds.
    pub fn shutdown_timeout(mut self, secs: u64) -> Self {
        self.shutdown_timeout = Duration::from_secs(secs);
        self
    }

    #[deprecated(note = "use Builder::backlog instead")]
    pub fn tcp_backlog(mut self, num: u32) -> Self {
        self.backlog = num;
        self
    }

    pub fn backlog(mut self, num: u32) -> Self {
        self.backlog = num;
        self
    }

    #[doc(hidden)]
    /// Async callback called when worker thread is spawned.
    ///
    /// *. This API is subject to change with no stable guarantee.
    pub fn on_worker_start<F, Fut>(mut self, on_start: F) -> Self
    where
        F: Fn() -> Fut + Send + Clone + 'static,
        Fut: Future + Send,
    {
        self.on_worker_start = Box::new(move || {
            let on_start = on_start.clone();
            Box::pin(async move {
                let fut = on_start();
                let _ = fut.await;
            })
        });

        self
    }

    pub fn bind<N, A, F, St>(self, name: N, addr: A, factory: F) -> io::Result<Self>
    where
        N: AsRef<str>,
        A: net::ToSocketAddrs,
        F: AsServiceFactoryClone<St>,
        St: FromStream + Send + 'static,
    {
        let addr = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "Can not parse SocketAddr"))?;

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

    pub fn listen<N, F, St>(mut self, name: N, listener: net::TcpListener, factory: F) -> io::Result<Self>
    where
        N: AsRef<str>,
        F: AsServiceFactoryClone<St>,
        St: FromStream + Send + 'static,
    {
        self.listeners
            .entry(name.as_ref().to_string())
            .or_insert_with(Vec::new)
            .push(Box::new(Some(listener)));

        let factory = Factory::new_boxed(factory);

        self.factories.insert(name.as_ref().to_string(), factory);

        Ok(self)
    }

    pub fn build(self) -> ServerFuture {
        let _enable_signal = self.enable_signal;
        match Server::new(self) {
            Ok(server) => ServerFuture::Server(ServerFutureInner {
                server,
                #[cfg(feature = "signal")]
                signals: _enable_signal.then(crate::signals::Signals::start),
            }),
            Err(e) => ServerFuture::Error(e),
        }
    }
}

#[cfg(unix)]
impl Builder {
    pub fn bind_unix<N, P, F, St>(self, name: N, path: P, factory: F) -> io::Result<Self>
    where
        N: AsRef<str>,
        P: AsRef<std::path::Path>,
        F: AsServiceFactoryClone<St>,
        St: FromStream + Send + 'static,
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

    pub fn listen_unix<N, F, St>(
        mut self,
        name: N,
        listener: std::os::unix::net::UnixListener,
        factory: F,
    ) -> io::Result<Self>
    where
        N: AsRef<str>,
        F: AsServiceFactoryClone<St>,
        St: FromStream + Send + 'static,
    {
        self.listeners
            .entry(name.as_ref().to_string())
            .or_insert_with(Vec::new)
            .push(Box::new(Some(listener)));

        let factory = Factory::new_boxed(factory);

        self.factories.insert(name.as_ref().to_string(), factory);

        Ok(self)
    }
}

#[cfg(feature = "http3")]
impl Builder {
    /// Bind to both Tcp and Udp of the same address to enable http/1/2/3 handling
    /// with single service.
    pub fn bind_all<N, A, F>(
        mut self,
        name: N,
        addr: A,
        config: xitca_io::net::H3ServerConfig,
        factory: F,
    ) -> io::Result<Self>
    where
        N: AsRef<str>,
        A: net::ToSocketAddrs,
        F: AsServiceFactoryClone<xitca_io::net::Stream>,
    {
        let addr = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "Can not parse SocketAddr"))?;

        let socket = if addr.is_ipv4() {
            TcpSocket::new_v4()?
        } else {
            TcpSocket::new_v6()?
        };

        socket.set_reuseaddr(true)?;
        socket.bind(addr)?;
        let listener = socket.listen(self.backlog)?.into_std()?;

        self.listeners
            .entry(name.as_ref().to_string())
            .or_insert_with(Vec::new)
            .push(Box::new(Some(listener)));

        let builder = xitca_io::net::UdpListenerBuilder::new(addr, config);

        self.listeners
            .entry(name.as_ref().to_string())
            .or_insert_with(Vec::new)
            .push(Box::new(Some(builder)));

        let factory = Factory::new_boxed(factory);

        self.factories.insert(name.as_ref().to_string(), factory);

        Ok(self)
    }

    pub fn bind_h3<N, A, F>(
        mut self,
        name: N,
        addr: A,
        config: xitca_io::net::H3ServerConfig,
        factory: F,
    ) -> io::Result<Self>
    where
        N: AsRef<str>,
        A: net::ToSocketAddrs,
        F: AsServiceFactoryClone<xitca_io::net::UdpStream>,
    {
        let addr = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "Can not parse SocketAddr"))?;

        let builder = xitca_io::net::UdpListenerBuilder::new(addr, config).backlog(self.backlog);

        self.listeners
            .entry(name.as_ref().to_string())
            .or_insert_with(Vec::new)
            .push(Box::new(Some(builder)));

        let factory = Factory::new_boxed(factory);

        self.factories.insert(name.as_ref().to_string(), factory);

        Ok(self)
    }
}
