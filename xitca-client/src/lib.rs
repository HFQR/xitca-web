mod body;
mod builder;
mod client;
mod connect;
mod connection;
mod h1;
mod pool;
mod request;
mod resolver;
mod timeout;
mod tls;
mod uri;

#[cfg(feature = "http2")]
mod h2;

pub mod error;

pub use self::builder::ClientBuilder;
pub use self::client::Client;
pub use self::resolver::Resolve;
pub use self::tls::{connector::TlsConnect, stream::Io};

// re-export http crate.
pub use xitca_http::http;

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn get() {
        let client = Client::new();

        // let _ = client.get("http://google.com").unwrap().send().await;
    }
}
