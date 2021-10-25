mod body;
mod builder;
mod client;
mod connect;
mod connection;
mod pool;
mod request;
mod resolver;
mod timeout;
mod tls;
mod uri;

mod h1;

pub mod error;

pub use self::builder::ClientBuilder;
pub use self::client::Client;
pub use self::resolver::Resolve;

pub use self::tls::{connector::TlsConnect, stream::Io};

// re-export http crate.
pub use http;

/// Protocol version of HTTP.
#[non_exhaustive]
pub enum Protocol {
    HTTP1,
    HTTP2,
    HTTP3,
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn get() {
        let client = Client::new();

        let _ = client.get("http://google.com").unwrap().send().await;
    }
}
