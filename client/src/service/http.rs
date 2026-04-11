use crate::{
    Service, ServiceRequest,
    connect::Connect,
    error::Error,
    pool::service::{Lease, PoolRequest},
    response::Response,
    service::ServiceDyn,
    uri::Uri,
};

/// type alias for object safe wrapper of type implement [Service] trait.
pub type HttpService =
    Box<dyn for<'r, 'c> ServiceDyn<ServiceRequest<'r, 'c>, Response = Response, Error = Error> + Send + Sync>;

pub(crate) fn base_service() -> HttpService {
    struct HttpService;

    impl<'r, 'c> Service<ServiceRequest<'r, 'c>> for HttpService {
        type Response = Response;
        type Error = Error;

        async fn call(&self, req: ServiceRequest<'r, 'c>) -> Result<Self::Response, Self::Error> {
            #[cfg(any(feature = "http1", feature = "http2", feature = "http3"))]
            use crate::{error::TimeoutError, timeout::Timeout};

            let ServiceRequest { req, client, timeout } = req;

            let uri = Uri::try_parse(req.uri())?;
            let version = req.version();
            let connect = Connect::new(uri);

            let _date = client.date_service.handle();

            // determine whether the pool is permitted to transparently downgrade
            // a failed h2c handshake to http/1. gRPC callers must disable this.
            #[cfg(all(feature = "grpc", feature = "http2"))]
            let allow_h2c_downgrade = !crate::grpc::is_grpc_request(&*req);
            #[cfg(not(all(feature = "grpc", feature = "http2")))]
            let allow_h2c_downgrade = true;

            let lease = Service::call(
                &client.pool,
                PoolRequest {
                    client,
                    connect,
                    version,
                    allow_h2c_downgrade,
                },
            )
            .await?;

            match lease {
                Lease::Shared { mut conn, version } => {
                    let mut _timer = Box::pin(tokio::time::sleep(timeout));
                    *req.version_mut() = version;
                    #[allow(unreachable_patterns, unreachable_code)]
                    match &mut *conn {
                        #[cfg(feature = "http2")]
                        crate::connection::ConnectionShared::H2(c) => {
                            match crate::h2::proto::send(c, _date, core::mem::take(req))
                                .timeout(_timer.as_mut())
                                .await
                            {
                                Ok(Ok(res)) => {
                                    let timeout = client.timeout_config.response_timeout;
                                    Ok(Response::new(res, _timer, timeout))
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
                        #[cfg(feature = "http3")]
                        crate::connection::ConnectionShared::H3(c) => {
                            let res = crate::h3::proto::send(c, _date, core::mem::take(req))
                                .timeout(_timer.as_mut())
                                .await
                                .map_err(|_| TimeoutError::Request)??;

                            let timeout = client.timeout_config.response_timeout;
                            Ok(Response::new(res, _timer, timeout))
                        }
                        _ => unreachable!("ConnectionShared has no enabled variants"),
                    }
                }
                Lease::Exclusive {
                    conn: mut _conn,
                    version,
                } => {
                    *req.version_mut() = version;

                    #[cfg(feature = "http1")]
                    {
                        let mut timer = Box::pin(tokio::time::sleep(timeout));
                        let res = crate::h1::proto::send(&mut *_conn, _date, req)
                            .timeout(timer.as_mut())
                            .await;

                        match res {
                            Ok(Ok((res, buf, decoder, is_close))) => {
                                if is_close {
                                    _conn.destroy_on_drop();
                                }
                                let body = crate::h1::body::ResponseBody::new(_conn, buf, decoder);
                                let res = res.map(|_| crate::body::ResponseBody::H1(body));
                                let timeout = client.timeout_config.response_timeout;
                                Ok(Response::new(res, timer, timeout))
                            }
                            Ok(Err(e)) => {
                                _conn.destroy_on_drop();
                                Err(e.into())
                            }
                            Err(_) => {
                                _conn.destroy_on_drop();
                                Err(TimeoutError::Request.into())
                            }
                        }
                    }

                    #[cfg(not(feature = "http1"))]
                    {
                        let _ = _conn;
                        Err(crate::error::FeatureError::Http1NotEnabled.into())
                    }
                }
            }
        }
    }

    Box::new(HttpService)
}
