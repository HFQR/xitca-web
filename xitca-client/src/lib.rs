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
    use futures_util::{SinkExt, StreamExt};
    use http_ws::Message;
    use xitca_http::bytes::Bytes;

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
        let client = crate::Client::new();

        let res = client.get("https://cloudflare-quic.com/")?.send().await?;

        println!("{:?}", res);

        let string = res.string().await?;

        println!("{:?}", string);

        Ok(())
    }

    // #[tokio::test]
    // async fn ws() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    //     tokio::spawn(async {
    //         let client = crate::Client::new();

    //         let ws = client.ws("ws://127.0.0.1:8080")?.send().await?.ws()?;
    //         let (mut tx, mut rx) = ws.split();

    //         tx.send(Message::Text(Bytes::from("hello world"))).await?;
    //         let msg = rx.next().await.unwrap()?;
    //         assert_eq!(msg, Message::Text(Bytes::from("Echo: hello world")));

    //         Ok(())
    //     }).await.unwrap()
    // }
}
