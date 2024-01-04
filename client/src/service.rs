use core::{future::Future, pin::Pin, time::Duration};

use tokio::time::Instant;

use crate::{
    body::BoxBody, client::Client, connect::Connect, error::Error, http::Request, response::Response, uri::Uri,
};

type BoxFuture<'f, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'f>>;

/// trait for composable http services. Used for middleware,resolver and tls connector.
pub trait Service<Req> {
    type Response;
    type Error;

    fn call(&self, req: Req) -> impl Future<Output = Result<Self::Response, Self::Error>> + Send;
}

pub trait ServiceDyn<Req> {
    type Response;
    type Error;

    fn call<'s>(&'s self, req: Req) -> BoxFuture<'s, Self::Response, Self::Error>
    where
        Req: 's;
}

impl<S, Req> ServiceDyn<Req> for S
where
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = S::Error;

    #[inline]
    fn call<'s>(&'s self, req: Req) -> BoxFuture<'s, Self::Response, Self::Error>
    where
        Req: 's,
    {
        Box::pin(Service::call(self, req))
    }
}

impl<I, Req> Service<Req> for Box<I>
where
    Req: Send,
    I: ServiceDyn<Req> + ?Sized + Send + Sync,
{
    type Response = I::Response;
    type Error = I::Error;

    #[inline]
    async fn call(&self, req: Req) -> Result<Self::Response, Self::Error> {
        ServiceDyn::call(&**self, req).await
    }
}

/// request type for middlewares.
/// It's similar to [RequestBuilder] type but with additional side effect enabled.
///
/// [RequestBuilder]: crate::request::RequestBuilder
pub struct ServiceRequest<'r, 'c> {
    pub req: &'r mut Request<BoxBody>,
    pub client: &'c Client,
    pub timeout: Duration,
}

/// type alias for object safe wrapper of type implement [Service] trait.
pub type HttpService =
    Box<dyn for<'r, 'c> ServiceDyn<ServiceRequest<'r, 'c>, Response = Response<'c>, Error = Error> + Send + Sync>;

pub(crate) fn base_service() -> HttpService {
    struct _HttpService;

    impl<'r, 'c> Service<ServiceRequest<'r, 'c>> for _HttpService {
        type Response = Response<'c>;
        type Error = Error;

        async fn call(&self, req: ServiceRequest<'r, 'c>) -> Result<Self::Response, Self::Error> {
            #[cfg(any(feature = "http1", feature = "http2", feature = "http3"))]
            use crate::{connection::Connection, error::TimeoutError, http::Version, timeout::Timeout};

            let ServiceRequest { req, client, timeout } = req;

            let uri = Uri::try_parse(req.uri())?;

            // Try to grab a connection from pool.
            let mut conn = client.pool.acquire(&uri).await?;

            let conn_is_none = conn.is_none();

            // setup timer according to outcome and timeout configs.
            let dur = if conn_is_none {
                client.timeout_config.resolve_timeout
            } else {
                timeout
            };

            // heap allocate timer so it can be moved to Response type afterwards
            let mut timer = Box::pin(tokio::time::sleep(dur));

            // Nothing in the pool. construct new connection and add it to Conn.
            if conn_is_none {
                let mut connect = Connect::new(uri);
                let c = client.make_connection(&mut connect, &mut timer, req.version()).await?;
                conn.add(c);
            }

            let _date = client.date_service.handle();

            timer
                .as_mut()
                .reset(Instant::now() + client.timeout_config.request_timeout);

            let _res = match *conn {
                #[cfg(feature = "http1")]
                Connection::Tcp(ref mut stream) => {
                    if matches!(req.version(), Version::HTTP_2 | Version::HTTP_3) {
                        *req.version_mut() = Version::HTTP_11
                    }
                    crate::h1::proto::send(stream, _date, req).timeout(timer.as_mut()).await
                }
                #[cfg(feature = "http1")]
                Connection::Tls(ref mut stream) => {
                    if matches!(req.version(), Version::HTTP_2 | Version::HTTP_3) {
                        *req.version_mut() = Version::HTTP_11
                    }
                    crate::h1::proto::send(stream, _date, req).timeout(timer.as_mut()).await
                }
                #[cfg(feature = "http1")]
                #[cfg(unix)]
                Connection::Unix(ref mut stream) => {
                    crate::h1::proto::send(stream, _date, req).timeout(timer.as_mut()).await
                }
                #[cfg(feature = "http2")]
                Connection::H2(ref mut stream) => {
                    *req.version_mut() = Version::HTTP_2;

                    return match crate::h2::proto::send(stream, _date, core::mem::take(req))
                        .timeout(timer.as_mut())
                        .await
                    {
                        Ok(Ok(res)) => {
                            let timeout = client.timeout_config.response_timeout;
                            Ok(Response::new(res, timer, timeout))
                        }
                        Ok(Err(e)) => {
                            conn.destroy_on_drop();
                            Err(e.into())
                        }
                        Err(_) => {
                            conn.destroy_on_drop();
                            Err(TimeoutError::Request.into())
                        }
                    };
                }
                #[cfg(feature = "http3")]
                Connection::H3(ref mut c) => {
                    *req.version_mut() = Version::HTTP_3;

                    return match crate::h3::proto::send(c, _date, core::mem::take(req))
                        .timeout(timer.as_mut())
                        .await
                    {
                        Ok(Ok(res)) => {
                            let timeout = client.timeout_config.response_timeout;
                            Ok(Response::new(res, timer, timeout))
                        }
                        Ok(Err(e)) => {
                            conn.destroy_on_drop();
                            Err(e.into())
                        }
                        Err(_) => {
                            conn.destroy_on_drop();
                            Err(TimeoutError::Request.into())
                        }
                    };
                }
                #[cfg(not(feature = "http1"))]
                _ => panic!("http1 feature is not enabled in Cargo.toml"),
            };

            #[cfg(feature = "http1")]
            match _res {
                Ok(Ok((res, buf, chunk, decoder, is_close))) => {
                    if is_close {
                        conn.destroy_on_drop();
                    }

                    let body = crate::h1::body::ResponseBody::new(conn, buf, chunk, decoder);
                    let res = res.map(|_| crate::body::ResponseBody::H1(body));
                    let timeout = client.timeout_config.response_timeout;

                    Ok(Response::new(res, timer, timeout))
                }
                Ok(Err(e)) => {
                    conn.destroy_on_drop();
                    Err(e.into())
                }
                Err(_) => {
                    conn.destroy_on_drop();
                    Err(TimeoutError::Request.into())
                }
            }
        }
    }

    Box::new(_HttpService)
}
