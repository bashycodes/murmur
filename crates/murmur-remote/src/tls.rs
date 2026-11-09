//! `wss://` (feature `tls`): rustls with Mozilla roots, or a pinned leaf
//! certificate (SHA-256 of its DER) for self-signed servers.

use std::sync::Arc;

use flow_router::{BackendError, Failure, ServerConfig};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

/// Accepts exactly one leaf certificate, by SHA-256 of its DER; the
/// handshake signature is still verified against that certificate.
#[derive(Debug)]
struct PinVerifier {
    pin: Vec<u8>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if Sha256::digest(end_entity.as_ref()).as_slice() == self.pin.as_slice() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("certificate pin mismatch".into()))
        }
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
            &self.provider.signature_verification_algorithms,
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
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

pub fn client_config(cfg: &ServerConfig) -> Result<rustls::ClientConfig, BackendError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| BackendError::new(Failure::ConnectFailed, e.to_string()))?;
    let c = match cfg.pin_hex() {
        Some(hex) => {
            let pin = hex_decode(&hex)
                .filter(|p| p.len() == 32)
                .ok_or_else(|| BackendError::new(Failure::ConnectFailed, "bad tls_pin_sha256"))?;
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(PinVerifier { pin, provider }))
                .with_no_client_auth()
        }
        None => {
            let roots = rustls::RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            };
            builder.with_root_certificates(roots).with_no_client_auth()
        }
    };
    Ok(c)
}

pub async fn connect(
    cfg: &ServerConfig,
    host: &str,
    tcp: TcpStream,
) -> Result<TlsStream<TcpStream>, BackendError> {
    let c = client_config(cfg)?;
    let name = ServerName::try_from(host.to_string())
        .map_err(|e| BackendError::new(Failure::ConnectFailed, e.to_string()))?;
    tokio_rustls::TlsConnector::from(Arc::new(c))
        .connect(name, tcp)
        .await
        .map_err(|e| BackendError::new(Failure::ConnectFailed, format!("TLS: {e}")))
}
