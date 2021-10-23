use crate::client::Client;
use crate::resolver::{Resolve, Resolver};
use crate::tls::connector::Connector;

pub struct ClientBuilder {
    connector: Connector,
    resolver: Resolver,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        ClientBuilder {
            connector: Connector::default(),
            resolver: Resolver::default(),
        }
    }
}

impl ClientBuilder {
    pub fn resolver(mut self, resolver: impl Resolve + 'static) -> Self {
        self.resolver = Resolver::custom(resolver);
        self
    }

    pub fn finish(self) -> Client {
        Client {
            connector: self.connector,
            resolver: self.resolver,
        }
    }
}
