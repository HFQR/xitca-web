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

    #[cfg(all(feature = "json", feature = "openssl"))]
    #[tokio::test]
    async fn get_json() {
        use std::collections::HashMap;

        let client = Client::builder().set_max_http_version(http::Version::HTTP_11).finish();

        let string = client
            .get("https://www.rust-lang.org")
            .unwrap()
            .send()
            .await
            .unwrap()
            .limit::<{ 1024 * 1024 * 1024 }>()
            .string()
            .await
            .unwrap();

        println!("{:?}", string);
    }
}
