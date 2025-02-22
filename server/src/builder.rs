use std::{collections::HashMap, future::Future, io, pin::Pin, time::Duration};

#[cfg(not(target_family = "wasm"))]
use std::net;

use xitca_io::net::{Listener, Stream};

use crate::{
    net::IntoListener,
    server::{IntoServiceObj, Server, ServerFuture, ServiceObj},
};

type ListenerFn = Box<dyn FnOnce() -> io::Result<Listener> + Send>;

pub struct Builder {
    pub(crate) server_threads: usize,
    pub(crate) worker_threads: usize,
    pub(crate) worker_max_blocking_threads: usize,
    pub(crate) listeners: HashMap<String, Vec<ListenerFn>>,
    pub(crate) factories: HashMap<String, ServiceObj>,
    pub(crate) enable_signal: bool,
    pub(crate) shutdown_timeout: Duration,
    pub(crate) on_worker_start: Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>,
    pub(crate) on_worker_stop: Box<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>,
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
            on_worker_stop: Box::new(|| Box::pin(async {})),
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

    #[doc(hidden)]
    /// Async callback called when worker thread is being shut down.
    ///
    /// *. This API is subject to change with no stable guarantee.
    pub fn on_worker_stop<F, Fut>(mut self, on_stop: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future + Send + 'static,
    {
        self.on_worker_stop = Box::new(move || {
            let fut = on_stop();
            Box::pin(async {
                fut.await;
            })
        });

        self
    }

    pub fn listen<N, L, F, St>(mut self, name: N, listener: L, service: F) -> Self
    where
        N: AsRef<str>,
        F: IntoServiceObj<St>,
        St: TryFrom<Stream> + 'static,
        L: IntoListener + 'static,
    {
        self.listeners
            .entry(name.as_ref().to_string())
            .or_default()
            .push(Box::new(|| listener.into_listener()));

        self.factories.insert(name.as_ref().to_string(), service.into_object());

        self
    }

    pub fn build(self) -> ServerFuture {
        let enable_signal = self.enable_signal;
        match Server::new(self) {
            Ok(server) => ServerFuture::Init { server, enable_signal },
            Err(e) => ServerFuture::Error(e),
        }
    }
}

#[cfg(not(target_family = "wasm"))]
impl Builder {
    pub fn bind<N, A, F, St>(self, name: N, addr: A, service: F) -> io::Result<Self>
    where
        N: AsRef<str>,
        A: net::ToSocketAddrs,
        F: IntoServiceObj<St>,
        St: TryFrom<Stream> + 'static,
    {
        let addr = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "Can not parse SocketAddr"))?;

        self._bind(name, addr, service)
    }

    fn _bind<N, F, St>(self, name: N, addr: net::SocketAddr, service: F) -> io::Result<Self>
    where
        N: AsRef<str>,
        F: IntoServiceObj<St>,
        St: TryFrom<Stream> + 'static,
    {
        let listener = net::TcpListener::bind(addr)?;

        let socket = socket2::SockRef::from(&listener);
        socket.set_reuse_address(true)?;
        socket.listen(self.backlog as _)?;

        Ok(self.listen(name, listener, service))
    }
}

#[cfg(unix)]
impl Builder {
    pub fn bind_unix<N, P, F, St>(self, name: N, path: P, service: F) -> io::Result<Self>
    where
        N: AsRef<str>,
        P: AsRef<std::path::Path>,
        F: IntoServiceObj<St>,
        St: TryFrom<Stream> + 'static,
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

        Ok(self.listen(name, listener, service))
    }
}

#[cfg(feature = "quic")]
impl Builder {
    /// Bind to both Tcp and Udp of the same address to enable http/1/2/3 handling
    /// with single service.
    pub fn bind_all<N, A, F>(
        mut self,
        name: N,
        addr: A,
        config: xitca_io::net::QuicConfig,
        service: F,
    ) -> io::Result<Self>
    where
        N: AsRef<str>,
        A: net::ToSocketAddrs,
        F: IntoServiceObj<Stream>,
    {
        let addr = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "Can not parse SocketAddr"))?;

        self = self._bind(name.as_ref(), addr, service)?;

        let builder = xitca_io::net::QuicListenerBuilder::new(addr, config).backlog(self.backlog);

        self.listeners
            .get_mut(name.as_ref())
            .unwrap()
            .push(Box::new(|| builder.into_listener()));

        Ok(self)
    }

    pub fn bind_h3<N, A, F, St>(
        self,
        name: N,
        addr: A,
        config: xitca_io::net::QuicConfig,
        service: F,
    ) -> io::Result<Self>
    where
        N: AsRef<str>,
        A: net::ToSocketAddrs,
        F: IntoServiceObj<St>,
        St: TryFrom<Stream> + 'static,
    {
        let addr = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "Can not parse SocketAddr"))?;

        let listener = xitca_io::net::QuicListenerBuilder::new(addr, config).backlog(self.backlog);

        Ok(self.listen(name, listener, service))
    }
}
