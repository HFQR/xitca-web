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
        pin::Pin,
        task::{Context, Poll},
        time::Duration,
    };

    use std::{net::TcpListener, process::Command};

    use futures_util::Stream;
    use xitca_http::{
        HttpServiceBuilder,
        bytes::Bytes,
        config::HttpServiceConfig,
        h2::dispatcher::{Frame, RequestBody},
        http::{Request, RequestExt, Response},
    };
    use xitca_service::{ServiceExt, fn_service};

    struct Once(Option<Frame>);

    impl Once {
        fn new() -> Self {
            Self(Some(Frame::Data(Bytes::new())))
        }
    }

    impl Stream for Once {
        type Item = Result<Frame, Infallible>;

        fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Ready(self.get_mut().0.take().map(Ok))
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            (0, Some(0))
        }
    }

    async fn handler(
        _: Request<RequestExt<RequestBody>>,
    ) -> Result<Response<Once>, Box<dyn std::error::Error + Send + Sync>> {
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
            let service = fn_service(handler).enclosed(
                HttpServiceBuilder::h2().io_uring().config(
                    HttpServiceConfig::new()
                        .request_head_timeout(Duration::from_mins(1))
                        .keep_alive_timeout(Duration::from_mins(1))
                        .h2_max_concurrent_streams(2),
                ),
            );

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
