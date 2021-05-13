use std::{
    cmp,
    fmt::Debug,
    future::Future,
    marker::PhantomData,
    task::{Context, Poll},
};

use bytes::Bytes;
use futures_util::future::poll_fn;
use h2::server::handshake;
use http::{request::Parts, Response};
use openssl::pkey::PKey;
use openssl::ssl::{AlpnError, Ssl, SslAcceptor, SslMethod};
use openssl::x509::X509;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_openssl::SslStream;

use super::service::HttpFlow;

use crate::service::Service;

pub struct Http2Service<S, St> {
    acceptor: SslAcceptor,
    flow: HttpFlow<S>,
    _req: PhantomData<St>,
}

impl<S, St> Http2Service<S, St> {
    /// Construct new Http2Service.
    /// No upgrade/expect services allowed in Http/2.
    pub fn new(service: S) -> Self {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
        let cert_file = cert.serialize_pem().unwrap();
        let key_file = cert.serialize_private_key_pem();
        let cert = X509::from_pem(cert_file.as_bytes()).unwrap();
        let key = PKey::private_key_from_pem(key_file.as_bytes()).unwrap();

        let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
        builder.set_certificate(&cert).unwrap();
        builder.set_private_key(&key).unwrap();

        builder.set_alpn_select_callback(|_, protocols| {
            const H2: &[u8] = b"\x02h2";
            const H11: &[u8] = b"\x08http/1.1";

            if protocols.windows(3).any(|window| window == H2) {
                Ok(b"h2")
            } else if protocols.windows(9).any(|window| window == H11) {
                Ok(b"http/1.1")
            } else {
                Err(AlpnError::NOACK)
            }
        });

        builder.set_alpn_protos(b"\x08http/1.1\x02h2").unwrap();

        Self {
            acceptor: builder.build(),
            flow: HttpFlow::new(service),
            _req: PhantomData,
        }
    }

    // pub fn upgrade(mut self) -> Self {
    //     self.flow.upgrade()
    // }
}

#[rustfmt::skip]
impl<S, St> Service for Http2Service<S, St>
where
    S: for<'r> Service<Request<'r> = (Parts, h2::RecvStream), Response = Response<Bytes>> + 'static,
    S::Error: Debug,
    St: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Request<'r> = St;
    type Response = ();
    type Error = ();
    type Future<'f> = impl Future<Output = Result<Self::Response, Self::Error>> + 'f;

    #[inline]
    fn poll_ready(&self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call<'s, 'r, 'f>(&'s self, req: Self::Request<'r>) -> Self::Future<'f> 
    where
        's: 'f,
        'r: 'f,
    {
        async move {
            let ssl_ctx = self.acceptor.context();
            let ssl = Ssl::new(ssl_ctx).expect("Provided SSL acceptor was invalid.");
            let mut req = SslStream::new(ssl, req).unwrap();

            std::pin::Pin::new(&mut req).accept().await.unwrap();

            let mut conn = match handshake(req).await {
                Ok(conn) => conn,
                Err(e) => panic!("{:?}", e),
            };

            while let Some(Ok((req, mut tx))) = conn.accept().await {
                let (req, body) = req.into_parts();
                let flow = self.flow.clone();
                tokio::task::spawn_local(async move {
                    let res = flow.service.call((req, body)).await.unwrap();

                    let (res, mut body) = res.into_parts();
                    let res = Response::from_parts(res, ());
                    
                    let mut stream = tx.send_response(res, false).unwrap();

                    while !body.is_empty() {
                        stream.reserve_capacity(cmp::min(body.len(), CHUNK_SIZE));

                        match poll_fn(|cx| stream.poll_capacity(cx)).await {
                            // No capacity left. drop body and return.
                            None => return,
                            Some(res) => {
                                // Split chuck to writeable size and send to client.
                                let cap = res.unwrap();
            
                                let len = body.len();
                                let bytes = body.split_to(cmp::min(cap, len));
            
                                stream
                                    .send_data(bytes, false).unwrap();
                            }
                        }
                    }

                    stream.send_data(Bytes::new(), true).unwrap();

                });
            }

            Ok(())
        }
    }
}

const CHUNK_SIZE: usize = 16_384;
