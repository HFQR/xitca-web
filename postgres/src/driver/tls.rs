use std::sync::Arc;

use sha2::{Digest, Sha256};
use xitca_io::io::AsyncIo;
use xitca_tls::rustls::{
    self,
    client::danger::HandshakeSignatureValid,
    crypto::{verify_tls12_signature, verify_tls13_signature},
    pki_types::{CertificateDer, ServerName, UnixTime},
    ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore, TlsStream,
};

use crate::{config::Config, error::Error};

pub(super) async fn connect_tls<Io>(
    io: Io,
    host: &str,
    cfg: &mut Config,
) -> Result<TlsStream<ClientConnection, Io>, Error>
where
    Io: AsyncIo,
{
    let name = ServerName::try_from(host).map_err(|_| Error::todo())?.to_owned();
    let config = dangerous_config(Vec::new());
    let config = Arc::new(config);
    let session = ClientConnection::new(config, name).map_err(|_| Error::todo())?;

    let stream = TlsStream::handshake(io, session).await?;

    if let Some(sha256) = stream
        .session()
        .peer_certificates()
        .and_then(|certs| certs.first())
        .map(|cert| Sha256::digest(cert.as_ref()).to_vec())
    {
        cfg.tls_server_end_point(sha256);
    }

    Ok(stream)
}

pub(crate) fn dangerous_config(alpn: Vec<Vec<u8>>) -> xitca_tls::rustls::ClientConfig {
    let mut root_store = RootCertStore::empty();

    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let mut cfg = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    cfg.alpn_protocols = alpn;

    cfg.dangerous().set_certificate_verifier(SkipServerVerification::new());

    cfg
}

#[derive(Debug)]
pub(crate) struct SkipServerVerification;

impl SkipServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
