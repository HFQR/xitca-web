//! Multi-threaded server for Tcp/Udp/UnixDomain handling.

#![forbid(unsafe_code)]

mod builder;
mod server;
mod signals;
mod worker;

pub mod net;

pub use builder::Builder;
pub use server::{ServerFuture, ServerHandle};

#[cfg(all(not(target_os = "linux"), feature = "io-uring"))]
compile_error!("io_uring can only be used on linux system");

#[cfg(test)]
mod test {
    use xitca_io::net::TcpStream;
    use xitca_service::fn_service;

    #[test]
    fn test_builder() {
        let listener = std::net::TcpListener::bind("localhost:0").unwrap();
        let _server = crate::builder::Builder::new()
            .listen("test", listener, fn_service(|_: TcpStream| async { Ok::<_, ()>(()) }))
            .build();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_two_phase_shutdown() -> std::io::Result<()> {
        let listener = std::net::TcpListener::bind("localhost:0")?;
        let mut server = crate::builder::Builder::new()
            .disable_signal()
            .listen("test", listener, fn_service(|_: TcpStream| async { Ok::<_, ()>(()) }))
            .build();

        let handle = server.handle()?;
        let waiting = tokio::spawn(async move { server.run_to_shutdown().await });

        handle.stop(true);

        let shutdown = waiting.await.expect("shutdown waiter panicked")?;
        shutdown.await
    }
}
