use core::{future::Future, pin::pin};

use std::net::SocketAddr;

use futures_core::stream::Stream;
use xitca_io::io::AsyncIo;
use xitca_service::Service;

use crate::{
    bytes::Bytes,
    error::{HttpServiceError, TimeoutError},
    http::{Request, RequestExt, Response},
    service::HttpService,
    util::timer::Timeout,
};

use super::body::RequestBody;

pub type H1Service<St, S, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize> =
    HttpService<St, S, RequestBody, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>;

impl<St, S, B, BE, A, const HEADER_LIMIT: usize, const READ_BUF_LIMIT: usize, const WRITE_BUF_LIMIT: usize>
    Service<(St, SocketAddr)> for H1Service<St, S, A, HEADER_LIMIT, READ_BUF_LIMIT, WRITE_BUF_LIMIT>
where
    S: Service<Request<RequestExt<RequestBody>>, Response = Response<B>>,
    A: Service<St>,
    St: AsyncIo,
    A::Response: AsyncIo + 'static,
    B: Stream<Item = Result<Bytes, BE>>,
    HttpServiceError<S::Error, BE>: From<A::Error>,
{
    type Response = ();
    type Error = HttpServiceError<S::Error, BE>;
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f where Self: 'f;

    fn call<'s>(&'s self, (io, addr): (St, SocketAddr)) -> Self::Future<'s>
    where
        St: 's,
    {
        async move {
            // at this stage keep-alive timer is used to tracks tls accept timeout.
            let mut timer = pin!(self.keep_alive());

            let io = self
                .tls_acceptor
                .call(io)
                .timeout(timer.as_mut())
                .await
                .map_err(|_| HttpServiceError::Timeout(TimeoutError::TlsAccept))??;

            #[cfg(feature = "io-uring")]
            {
                let io = (&mut Some(io) as &mut dyn std::any::Any)
                    .downcast_mut::<Option<xitca_io::net::TcpStream>>()
                    .expect("currently io-uring feature only support TcpStream.")
                    .take()
                    .unwrap();

                let io = io.into_std().unwrap();
                let io = xitca_io::net::io_uring::TcpStream::from_std(io);

                super::dispatcher_uring::Dispatcher::new(io, addr, timer, self.config, &self.service, self.date.get())
                    .run()
                    .await?;
            }

            #[cfg(not(feature = "io-uring"))]
            {
                let mut io = io;
                super::dispatcher::run(&mut io, addr, timer, self.config, &self.service, self.date.get()).await?;
            }

            Ok(())
        }
    }
}
