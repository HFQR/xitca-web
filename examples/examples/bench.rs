//! A Plain text bench for Http/1.1

use std::io;

use actix_http::HttpServiceBuilder;
use actix_service::map_config;
use actix_web::{dev::AppConfig, get, App};

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    std::env::set_var("RUST_LOG", "actix=trace, info");
    env_logger::init();

    actix_server_alt::Builder::new()
        .bind("bench", "127.0.0.1:8080", || {
            HttpServiceBuilder::new()
                .h1(map_config(App::new().service(index), |_| AppConfig::default()))
                .tcp()
        })?
        .build()
        .await
}

#[get("/")]
async fn index() -> &'static str {
    "Hello World!"
}
