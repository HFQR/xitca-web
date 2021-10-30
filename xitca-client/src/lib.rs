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
    async fn get_string() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client = crate::Client::builder().openssl().finish();

        let string = client
            .get("https://www.rust-lang.org")?
            .send()
            .await?
            .limit::<{ 1024 * 1024 }>()
            .string()
            .await?;

        println!("{:?}", string);

        Ok(())
    }

    #[cfg(feature = "http3")]
    #[tokio::test]
    async fn get_string_h3() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client = crate::Client::builder().finish();

        let string = client
            .get("https://cloudflare-quic.com/")?
            .send()
            .await?
            .string()
            .await?;

        println!("{:?}", string);

        Ok(())
    }
}
