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

pub mod error;

pub use self::builder::ClientBuilder;
pub use self::client::Client;

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn get() {
        let client = Client::new();

        let _ = client.get("https://google.com").unwrap().send().await.unwrap();
    }
}
