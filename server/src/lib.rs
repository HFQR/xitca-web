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
    use tokio_util::sync::CancellationToken;
    use xitca_io::net::TcpStream;
    use xitca_service::fn_service;

    #[test]
    fn test_builder() {
        let listener = std::net::TcpListener::bind("localhost:0").unwrap();
        let _server = crate::builder::Builder::new()
            .listen(
                "test",
                listener,
                fn_service(|(_, _): (TcpStream, CancellationToken)| async { Ok::<_, ()>(()) }),
            )
            .build();
    }
}
