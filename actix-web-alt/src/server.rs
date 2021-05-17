use std::{marker::PhantomData, net::ToSocketAddrs};

use actix_http_alt::HttpRequest;
use actix_server_alt::{Builder, ServerFuture};
use actix_service_alt::ServiceFactory;

pub struct HttpServer<F, S> {
    factory: F,
    builder: Builder,
    _phantom: PhantomData<S>,
}

impl<F, I, S> HttpServer<F, S>
where
    F: Fn() -> I + Send + Clone + 'static,
    I: ServiceFactory<HttpRequest>,
{
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            builder: Builder::new(),
            _phantom: PhantomData,
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

    // pub fn bind<A: ToSocketAddrs>(mut self, addr: A) -> std::io::Result<Self> {
    //     self.builder = self.builder.bind("test", addr, self.factory.clone())?;
    //
    //     Ok(self)
    // }

    pub fn run(self) -> ServerFuture {
        self.builder.build()
    }
}
