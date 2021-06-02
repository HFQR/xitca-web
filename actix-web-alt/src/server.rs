use std::net::ToSocketAddrs;

use actix_server_alt::net::TcpStream;
use actix_server_alt::{Builder, ServerFuture};
use actix_service_alt::ServiceFactory;

pub struct HttpServer<F> {
    factory: F,
    builder: Builder,
}

impl<F, I> HttpServer<F>
where
    F: Fn() -> I + Send + Clone + 'static,
{
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            builder: Builder::new(),
        }
    }

    pub fn server_threads(mut self, num: usize) -> Self {
        self.builder = self.builder.server_threads(num);
        self
    }

    pub fn worker_threads(mut self, num: usize) -> Self {
        self.builder = self.builder.worker_threads(num);
        self
    }

    pub fn worker_max_blocking_threads(mut self, num: usize) -> Self {
        self.builder = self.builder.worker_max_blocking_threads(num);
        self
    }

    pub fn backlog(mut self, num: u32) -> Self {
        self.builder = self.builder.backlog(num);
        self
    }

    pub fn connection_limit(mut self, num: usize) -> Self {
        self.builder = self.builder.connection_limit(num);

        self
    }

    pub fn bind<A: ToSocketAddrs>(mut self, addr: A) -> std::io::Result<Self>
    where
        I: ServiceFactory<TcpStream, Config = ()>,
    {
        self.builder = self.builder.bind("actix-web-alt", addr, self.factory.clone())?;

        Ok(self)
    }

    #[cfg(unix)]
    pub fn bind_unix<P: AsRef<std::path::Path>>(mut self, path: P) -> std::io::Result<Self>
    where
        I: ServiceFactory<actix_server_alt::net::UnixStream, Config = ()>,
    {
        self.builder = self.builder.bind_unix("actix-web-alt", path, self.factory.clone())?;

        Ok(self)
    }

    pub fn run(self) -> ServerFuture {
        self.builder.build()
    }
}
