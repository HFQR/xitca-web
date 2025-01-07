//! http tunnel handling.

use super::{
    error::{Error, ErrorResponse},
    http::{response::Parts, StatusCode},
    http_tunnel::HttpTunnel,
    request::RequestBuilder,
    tunnel::Tunnel,
};

pub type UpgradeRequest<'a> = RequestBuilder<'a, marker::Upgrade>;

mod marker {
    pub struct Upgrade;
}

pub struct UpgradeResponse {
    pub parts: Parts,
    pub tunnel: Tunnel<HttpTunnel>,
}

impl UpgradeRequest<'_> {
    /// Send the request and wait for response asynchronously.
    pub async fn send(self) -> Result<UpgradeResponse, Error> {
        let res = self._send().await?;

        let status = res.status();
        let expect_status = StatusCode::SWITCHING_PROTOCOLS;
        if status != expect_status {
            return Err(Error::from(ErrorResponse {
                expect_status,
                status,
                description: "connect tunnel can't be established",
            }));
        }

        let (parts, body) = res.into_parts();

        Ok(UpgradeResponse {
            parts,
            tunnel: Tunnel::new(HttpTunnel {
                body,
                io: Default::default(),
            }),
        })
    }
}
