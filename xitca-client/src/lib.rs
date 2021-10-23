mod builder;
mod client;
mod connect;
mod resolver;
mod tls;

pub mod error;

pub use self::builder::ClientBuilder;
pub use self::client::Client;

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn get() {
        let client = Client::new();

        let _ = client.get("google.com").await.unwrap();
    }
}
