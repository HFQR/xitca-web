#![forbid(unsafe_code)]

mod body;
mod builder;
mod client;
mod connect;
mod connection;
mod date;
mod h1;
mod pool;
mod request;
mod resolver;
mod response;
mod timeout;
mod tls;
mod uri;
mod ws;

#[cfg(feature = "http2")]
mod h2;

#[cfg(feature = "http3")]
mod h3;

pub mod error;

pub use self::builder::ClientBuilder;
pub use self::client::Client;
pub use self::resolver::Resolve;
pub use self::tls::{connector::TlsConnect, stream::Io};

// re-export http crate.
pub use xitca_http::http;

// re-export bytes crate.
pub use xitca_http::bytes;

#[cfg(test)]
mod test {
    #[cfg(all(feature = "openssl", feature = "http2"))]
    #[tokio::test]
    async fn get() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client = crate::Client::builder().openssl().finish();

        let res = client.get("https://www.rust-lang.org")?.send().await?;

        println!("{:?}", res);

        Ok(())
    }

    #[cfg(feature = "http3")]
    #[tokio::test]
    async fn get_h3() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client = crate::Client::new();

        let res = client.get("https://cloudflare-quic.com/")?.send().await?;

        println!("{:?}", res);

        Ok(())
    }
}
