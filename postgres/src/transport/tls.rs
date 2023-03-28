use alloc::sync::Arc;

use std::time::SystemTime;

use rustls::{
    client::{ServerCertVerified, ServerCertVerifier},
    Certificate, ClientConfig, ServerName,
};

pub(super) fn dangerous_config(alpn: Vec<Vec<u8>>) -> Arc<ClientConfig> {
    let mut cfg = ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(SkipServerVerification::new())
        .with_no_client_auth();
    cfg.alpn_protocols = alpn;
    Arc::new(cfg)
}

struct SkipServerVerification;

impl SkipServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

impl ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &Certificate,
        _intermediates: &[Certificate],
        _server_name: &ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: SystemTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
}
