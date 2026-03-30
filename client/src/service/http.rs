use std::time::Duration;

use crate::{
    Service, ServiceRequest,
    connect::Connect,
    error::Error,
    http::{HeaderName, Version},
    pool::{exclusive, shared},
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

            // temporary version to record possible version downgrade/upgrade happens when making connections.
            // alpn protocol and alt-svc header are possible source of version change.
            #[allow(unused_mut)]
            let mut version = req.version();

            let mut connect = Connect::new(uri);

            let _date = client.date_service.handle();

            loop {
                match version {
                    Version::HTTP_2 | Version::HTTP_3 => match client.shared_pool.acquire(&connect.uri).await {
                        shared::AcquireOutput::Conn(mut _conn) => {
                            let mut _timer = Box::pin(tokio::time::sleep(timeout));
                            *req.version_mut() = version;
                            #[allow(unreachable_code)]
                            return match _conn.conn {
                                #[cfg(feature = "http2")]
                                crate::connection::ConnectionShared::H2(ref mut conn) => {
                                    match crate::h2::proto::send(conn, _date, core::mem::take(req))
                                        .timeout(_timer.as_mut())
                                        .await
                                    {
                                        Ok(Ok(res)) => {
                                            let timeout = client.timeout_config.response_timeout;
                                            Ok(Response::new(res, _timer, timeout))
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
                                #[cfg(feature = "http3")]
                                crate::connection::ConnectionShared::H3(ref mut conn) => {
                                    let res = crate::h3::proto::send(conn, _date, core::mem::take(req))
                                        .timeout(_timer.as_mut())
                                        .await
                                        .map_err(|_| TimeoutError::Request)??;

                                    let timeout = client.timeout_config.response_timeout;
                                    Ok(Response::new(res, _timer, timeout))
                                }
                            };
                        }
                        shared::AcquireOutput::Spawner(_spawner) => match version {
                            Version::HTTP_3 => {
                                #[cfg(feature = "http3")]
                                {
                                    let mut timer = Box::pin(tokio::time::sleep(client.timeout_config.resolve_timeout));

                                    Service::call(&client.resolver, &mut connect)
                                        .timeout(timer.as_mut())
                                        .await
                                        .map_err(|_| TimeoutError::Resolve)??;
                                    timer
                                        .as_mut()
                                        .reset(tokio::time::Instant::now() + client.timeout_config.connect_timeout);

                                    if let Ok(Ok(conn)) = crate::h3::proto::connect(
                                        &client.h3_client,
                                        connect.addrs(),
                                        connect.hostname(),
                                    )
                                    .timeout(timer.as_mut())
                                    .await
                                    {
                                        _spawner.spawned(conn.into());
                                    } else {
                                        #[cfg(feature = "http2")]
                                        {
                                            version = Version::HTTP_2;
                                        }

                                        #[cfg(not(feature = "http2"))]
                                        {
                                            version = Version::HTTP_11;
                                        }
                                    }
                                }

                                #[cfg(not(feature = "http3"))]
                                {
                                    return Err(crate::error::FeatureError::Http3NotEnabled.into());
                                }
                            }
                            Version::HTTP_2 => {
                                #[cfg(feature = "http2")]
                                {
                                    let mut timer = Box::pin(tokio::time::sleep(client.timeout_config.resolve_timeout));
                                    let (conn, alpn_version) =
                                        client.make_exclusive(&mut connect, &mut timer, Version::HTTP_2).await?;

                                    if alpn_version == Version::HTTP_2 {
                                        let conn = crate::h2::proto::handshake(conn).await?;
                                        _spawner.spawned(conn.into());
                                    } else {
                                        #[cfg(not(feature = "http1"))]
                                        {
                                            return Err(crate::error::FeatureError::Http1NotEnabled.into());
                                        }

                                        #[cfg(feature = "http1")]
                                        {
                                            client.exclusive_pool.try_add(&connect.uri, conn);
                                            // downgrade request version to what alpn protocol suggested from make_exclusive.
                                            version = alpn_version;
                                        }
                                    }
                                }

                                #[cfg(not(feature = "http2"))]
                                {
                                    return Err(crate::error::FeatureError::Http2NotEnabled.into());
                                }
                            }
                            _ => unreachable!("outer match didn't  handle version correctly."),
                        },
                    },
                    version => match client.exclusive_pool.acquire(&connect.uri).await {
                        exclusive::AcquireOutput::Conn(mut _conn) => {
                            *req.version_mut() = version;

                            #[cfg(feature = "http1")]
                            {
                                let mut timer = Box::pin(tokio::time::sleep(timeout));
                                let res = crate::h1::proto::send(&mut *_conn, _date, req)
                                    .timeout(timer.as_mut())
                                    .await;

                                return match res {
                                    Ok(Ok((res, buf, decoder, is_close))) => {
                                        if is_close {
                                            _conn.destroy_on_drop();
                                        } else {
                                            let (timeout, max) = parse_keep_alive(&res);
                                            _conn.keep_alive_hint(timeout, max);
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
                                };
                            }

                            #[cfg(not(feature = "http1"))]
                            {
                                return Err(crate::error::FeatureError::Http1NotEnabled.into());
                            }
                        }
                        exclusive::AcquireOutput::Spawner(_spawner) => {
                            let mut timer = Box::pin(tokio::time::sleep(client.timeout_config.resolve_timeout));
                            let (conn, _) = client.make_exclusive(&mut connect, &mut timer, version).await?;
                            _spawner.spawned(conn);
                        }
                    },
                }
            }
        }
    }

    Box::new(HttpService)
}

const KEEP_ALIVE: HeaderName = HeaderName::from_static("keep-alive");

fn parse_keep_alive<B>(res: &crate::http::Response<B>) -> (Option<Duration>, Option<usize>) {
    let header = match res.headers().get(KEEP_ALIVE).map(|h| h.to_str()) {
        Some(Ok(header)) => header,
        _ => return (None, None),
    };

    let mut timeout = None;
    let mut max = None;

    for (key, value) in header.split(',').map(|item| {
        let mut kv = item.splitn(2, '=');

        (
            kv.next().map(|s| s.trim()).unwrap_or_default(),
            kv.next().map(|s| s.trim()).unwrap_or_default(),
        )
    }) {
        match key.to_lowercase().as_str() {
            "timeout" => {
                timeout = value.parse::<u64>().ok().map(Duration::from_secs);
            }
            "max" => {
                max = value.parse().ok();
            }
            _ => {}
        }
    }

    (timeout, max)
}

#[cfg(test)]
mod test {
    use crate::{body::ResponseBody, http};

    use super::*;

    #[test]
    fn test_parse_timeout_and_max() {
        let res = http::Response::builder()
            .header("keep-alive", "timeout=100, max=10")
            .body(ResponseBody::Eof)
            .unwrap();

        let (timeout, max) = parse_keep_alive(&res);

        assert_eq!(timeout, Some(Duration::from_secs(100)));
        assert_eq!(max, Some(10));
    }
}
