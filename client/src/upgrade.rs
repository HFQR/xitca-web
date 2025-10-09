//! http upgrade handling.

use super::{
    error::{Error, ErrorResponse},
    http::{
        header::{self, HeaderValue},
        response::Parts,
        StatusCode,
    },
    http_tunnel::HttpTunnel,
    request::RequestBuilder,
    tunnel::Tunnel,
};
use std::ops::{Deref, DerefMut};

pub type UpgradeRequest<'a> = RequestBuilder<'a, marker::Upgrade>;
pub type UpgradeRequestWithProtocol<'a> = RequestBuilder<'a, marker::UpgradeWithProtocol>;

mod marker {
    pub struct Upgrade;
    pub struct UpgradeWithProtocol;
}

pub struct UpgradeResponse {
    pub parts: Parts,
    pub tunnel: Tunnel<HttpTunnel>,
}

impl<'a> UpgradeRequest<'a> {
    pub fn protocol<V>(mut self, proto: V) -> UpgradeRequestWithProtocol<'a>
    where
        V: IntoIterator,
        V::Item: TryInto<HeaderValue>,
        <V::Item as TryInto<HeaderValue>>::Error: Into<crate::http::Error>,
    {
        let res = proto
            .into_iter()
            .map(|proto| proto.try_into().map_err(|e| Error::Std(Box::new(e.into()))))
            .collect::<Result<Vec<_>, Error>>();

        match res {
            Ok(proto) => {
                for proto in proto {
                    self.req.headers_mut().append(header::UPGRADE, proto);
                }
            }
            Err(e) => self.push_error(e),
        };

        self.mutate_marker()
    }
}

impl UpgradeRequestWithProtocol<'_> {
    /// Send the request and wait for response asynchronously.
    pub async fn send(mut self) -> Result<UpgradeResponse, Error> {
        self.headers_mut()
            .insert(header::CONNECTION, HeaderValue::from_static("upgrade"));

        let res = self._send().await?;

        let status = res.status();
        let expect_status = StatusCode::SWITCHING_PROTOCOLS;
        if status != expect_status {
            return Err(Error::from(ErrorResponse {
                expect_status,
                res,
                description: "upgrade tunnel can't be established",
            }));
        }

        let (parts, body) = res.into_inner().into_parts();

        Ok(UpgradeResponse {
            parts,
            tunnel: Tunnel::new(HttpTunnel {
                body,
                io: Default::default(),
            }),
        })
    }
}

impl UpgradeResponse {
    #[inline]
    pub fn into_parts(self) -> (Parts, Tunnel<HttpTunnel>) {
        (self.parts, self.tunnel)
    }

    #[inline]
    pub fn tunnel(&mut self) -> &mut Tunnel<HttpTunnel> {
        &mut self.tunnel
    }
}

impl Deref for UpgradeResponse {
    type Target = Parts;

    fn deref(&self) -> &Self::Target {
        &self.parts
    }
}

impl DerefMut for UpgradeResponse {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.parts
    }
}
