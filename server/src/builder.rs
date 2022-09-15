use std::{collections::HashMap, future::Future, io, net, pin::Pin, time::Duration};

#[cfg(not(target_family = "wasm"))]
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use xitca_io::net::Stream;

use crate::{
    net::AsListener,
    server::{BuildServiceFn, Factory, Server, ServerFuture, _BuildService},
};

pub struct Builder {
    pub(crate) server_threads: usize,
    pub(crate) worker_threads: usize,
    pub(crate) worker_max_blocking_threads: usize,
    pub(crate) listeners: HashMap<String, Vec<Box<dyn AsListener>>>,
    pub(crate) factories: HashMap<String, Box<dyn _BuildService>>,
    pub(crate) enable_signal: bool,
    pub(crate) shutdown_timeout: Duration,
    pub(crate) on_worker_start: Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>,
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
            worker_threads: std::thread::available_parallelism().map(|size| size.get()).unwrap_or(1),
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
    /// Server would only be shutdown from [ServerHandle](crate::server::ServerHandle)
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
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future + Send + 'static,
    {
        self.on_worker_start = Box::new(move || {
            let fut = on_start();
            Box::pin(async {
                fut.await;
            })
        });

        self
    }

    pub fn listen<N, F, St>(self, name: N, listener: net::TcpListener, factory: F) -> io::Result<Self>
    where
        N: AsRef<str>,
        F: BuildServiceFn<St>,
        St: From<Stream> + Send + 'static,
    {
        self._listen(name, Some(listener), factory)
    }

    pub fn build(self) -> ServerFuture {
        let enable_signal = self.enable_signal;
        match Server::new(self) {
            Ok(server) => ServerFuture::Init { server, enable_signal },
            Err(e) => ServerFuture::Error(e),
        }
    }

    fn _listen<N, L, F, St>(mut self, name: N, listener: L, factory: F) -> io::Result<Self>
    where
        N: AsRef<str>,
        L: AsListener + 'static,
        F: BuildServiceFn<St>,
        St: From<Stream> + Send + 'static,
    {
        self.listeners
            .entry(name.as_ref().to_string())
            .or_insert_with(Vec::new)
            .push(Box::new(listener));

        let factory = Factory::new_boxed(factory);

        self.factories.insert(name.as_ref().to_string(), factory);

        Ok(self)
    }
}

#[cfg(not(target_family = "wasm"))]
impl Builder {
    pub fn bind<N, A, F, St>(self, name: N, addr: A, factory: F) -> io::Result<Self>
    where
        N: AsRef<str>,
        A: net::ToSocketAddrs,
        F: BuildServiceFn<St>,
        St: From<Stream> + Send + 'static,
    {
        let addr = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "Can not parse SocketAddr"))?;

        self._bind(name, addr, factory)
    }

    fn _bind<N, F, St>(self, name: N, addr: net::SocketAddr, factory: F) -> io::Result<Self>
    where
        N: AsRef<str>,
        F: BuildServiceFn<St>,
        St: From<Stream> + Send + 'static,
    {
        let socket = if addr.is_ipv4() {
            Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?
        } else {
            Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?
        };
        socket.set_nonblocking(true)?;
        socket.set_reuse_address(true)?;
        socket.bind(&SockAddr::from(addr))?;
        socket.listen(self.backlog as _)?;

        let listener = socket.into();

        self.listen(name, listener, factory)
    }
}

#[cfg(unix)]
impl Builder {
    pub fn bind_unix<N, P, F, St>(self, name: N, path: P, factory: F) -> io::Result<Self>
    where
        N: AsRef<str>,
        P: AsRef<std::path::Path>,
        F: BuildServiceFn<St>,
        St: From<Stream> + Send + 'static,
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
        self,
        name: N,
        listener: std::os::unix::net::UnixListener,
        factory: F,
    ) -> io::Result<Self>
    where
        N: AsRef<str>,
        F: BuildServiceFn<St>,
        St: From<Stream> + Send + 'static,
    {
        self._listen(name, Some(listener), factory)
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
        F: BuildServiceFn<Stream>,
    {
        let addr = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "Can not parse SocketAddr"))?;

        self = self._bind(name.as_ref(), addr, factory)?;

        let builder = xitca_io::net::UdpListenerBuilder::new(addr, config).backlog(self.backlog);

        self.listeners
            .get_mut(name.as_ref())
            .unwrap()
            .push(Box::new(Some(builder)));

        Ok(self)
    }

    pub fn bind_h3<N, A, F>(
        self,
        name: N,
        addr: A,
        config: xitca_io::net::H3ServerConfig,
        factory: F,
    ) -> io::Result<Self>
    where
        N: AsRef<str>,
        A: net::ToSocketAddrs,
        F: BuildServiceFn<xitca_io::net::UdpStream>,
    {
        let addr = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "Can not parse SocketAddr"))?;

        let builder = xitca_io::net::UdpListenerBuilder::new(addr, config).backlog(self.backlog);

        self._listen(name, Some(builder), factory)
    }
}
