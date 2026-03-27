// HTTP/2 conformance tests using h2spec (https://github.com/summerwind/h2spec).
//
// Requires the `h2spec` binary on PATH. In CI it is installed by the
// `h2spec` workflow job. Locally:
//   curl -sSL https://github.com/summerwind/h2spec/releases/download/v2.6.0/h2spec_linux_amd64.tar.gz \
//     | tar xz -C ~/.local/bin
//
// Run with:
//   cargo test --package xitca-test --features io-uring h2spec -- --nocapture

#[cfg(feature = "io-uring")]
mod inner {
    use core::{
        convert::Infallible,
        net::SocketAddr,
        pin::Pin,
        task::{Context, Poll},
    };

    use std::{net::TcpListener, process::Command};

    use futures_util::Stream;
    use xitca_http::{
        bytes::Bytes,
        h2,
        http::{Request, RequestExt, Response},
    };
    use xitca_io::net::io_uring::TcpStream;
    use xitca_service::{Service, fn_service};
    use xitca_unsafe_collection::futures::NowOrPanic;

    struct Once(Option<h2::Frame>);

    impl Once {
        fn new() -> Self {
            Self(Some(h2::Frame::Data(Bytes::new())))
        }
    }

    impl Stream for Once {
        type Item = Result<h2::Frame, Infallible>;

        fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Ready(self.get_mut().0.take().map(Ok))
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            (0, Some(0))
        }
    }

    async fn handler(_req: Request<RequestExt<h2::RequestBodyV2>>) -> Result<Response<Once>, Infallible> {
        Ok(Response::new(Once::new()))
    }

    /// Bind to port 0 and immediately release it to get a free port number.
    /// There is a small TOCTOU window, but it is acceptable for CI where
    /// port collisions are rare.
    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    #[test]
    fn h2spec() {
        // Skip gracefully when h2spec is not installed locally.
        // In CI the binary is installed before this test runs.
        if Command::new("h2spec").arg("--version").output().is_err() {
            eprintln!("h2spec not found on PATH — skipping (see top-of-file comment to install)");
            return;
        }

        let port = free_port();
        let addr = format!("127.0.0.1:{port}");

        let (tx, rx) = std::sync::mpsc::sync_channel(1);

        std::thread::spawn(move || {
            let service = fn_service(move |(stream, _): (TcpStream, SocketAddr)| async move {
                // Set max_concurrent_streams=1 so h2spec's 5.1.2 test only
                // needs to open 2 simultaneous streams to trigger the limit.
                let mut settings = h2::Settings::default();
                settings.set_max_concurrent_streams(Some(1));
                h2::run(stream, &fn_service(handler).call(()).now_or_panic().unwrap(), settings).await
            });

            let server = xitca_server::Builder::new()
                .bind("h2spec", addr, service)
                .unwrap()
                .build();

            tx.send(()).unwrap();
            server.wait()
        });

        // Wait until the server thread has called .build() and is listening.
        rx.recv().unwrap();

        let status = Command::new("h2spec")
            .args(["-p", &port.to_string(), "-h", "127.0.0.1", "--timeout", "10"])
            .status()
            .expect("h2spec binary not found — see top-of-file comment for install instructions");

        assert!(status.success(), "h2spec reported conformance failures");
    }
}
