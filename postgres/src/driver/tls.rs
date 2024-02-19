#[cfg(not(feature = "quic"))]
#[cfg(feature = "tls")]
pub(super) fn dangerous_config(alpn: Vec<Vec<u8>>) -> std::sync::Arc<xitca_tls::rustls::ClientConfig> {
    use std::sync::Arc;

    use xitca_tls::rustls::{
        self,
        client::danger::HandshakeSignatureValid,
        crypto::{verify_tls12_signature, verify_tls13_signature},
        ClientConfig, DigitallySignedStruct, RootCertStore,
    };

    use rustls_pki_types::{CertificateDer, ServerName, UnixTime};

    #[derive(Debug)]
    struct SkipServerVerification;

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

    let mut root_store = RootCertStore::empty();

    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let mut cfg = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    cfg.alpn_protocols = alpn;

    cfg.dangerous().set_certificate_verifier(SkipServerVerification::new());

    Arc::new(cfg)
}
