//! Self-issued TLS certificates identify devices by SHA-256(SPKI).
//! Trust is a local outgoing policy. TLS verifies proof of private-key possession.
//! Persistence belongs to the platform adapter; only the device key is kept.
use rustls::{
    ClientConfig, DigitallySignedStruct, DistinguishedName, Error, ServerConfig, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    server::danger::{ClientCertVerified, ClientCertVerifier},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shuttli_model::sync::DeviceId;
use std::sync::Arc;
use x509_parser::prelude::FromDer;
pub type Result<T> = std::result::Result<T, String>;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityFile {
    cert: Vec<u8>,
    key: Vec<u8>,
}
pub struct Identity {
    pub id: DeviceId,
    pub client: Arc<ClientConfig>,
    pub server: Arc<ServerConfig>,
}
pub fn device_id(cert: &[u8]) -> Result<DeviceId> {
    if cert.len() > 8192 {
        return Err("oversized certificate".into());
    }
    let (rest, c) = x509_parser::certificate::X509Certificate::from_der(cert)
        .map_err(|_| "malformed certificate")?;
    if !rest.is_empty() || !c.validity().is_valid() {
        return Err("invalid certificate validity".into());
    }
    Ok(Sha256::digest(c.public_key().raw).into())
}
impl Identity {
    pub fn generate() -> Result<(Self, Vec<u8>)> {
        let key = rcgen::KeyPair::generate().map_err(|e| e.to_string())?;
        let params = rcgen::CertificateParams::new(vec!["shuttli.local".into()])
            .map_err(|e| e.to_string())?;
        let cert = params.self_signed(&key).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec(&IdentityFile {
            cert: cert.der().to_vec(),
            key: key.serialize_der(),
        })
        .map_err(|e| e.to_string())?;
        Ok((Self::from_bytes(&bytes)?, bytes))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 16 * 1024 {
            return Err("oversized identity".into());
        }
        let file: IdentityFile = serde_json::from_slice(bytes)
            .map_err(|_| "corrupt identity; refusing silent key replacement")?;
        let id = device_id(&file.cert)?;
        let cert = CertificateDer::from(file.cert);
        let key = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(file.key));
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = Arc::new(DeviceVerifier {
            provider: provider.clone(),
        });
        let client = ClientConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| e.to_string())?
            .dangerous()
            .with_custom_certificate_verifier(verifier.clone())
            .with_client_auth_cert(vec![cert.clone()], key.clone_key())
            .map_err(|e| e.to_string())?;
        let server = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|e| e.to_string())?
            .with_client_cert_verifier(verifier)
            .with_single_cert(vec![cert], key)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            id,
            client: Arc::new(client),
            server: Arc::new(server),
        })
    }
}
#[derive(Debug)]
struct DeviceVerifier {
    provider: Arc<rustls::crypto::CryptoProvider>,
}
impl DeviceVerifier {
    fn certificate(
        &self,
        cert: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
    ) -> std::result::Result<(), Error> {
        if !intermediates.is_empty() {
            return Err(Error::General(
                "device certificates must be self-issued leaves".into(),
            ));
        }
        device_id(cert.as_ref()).map_err(Error::General)?;
        Ok(())
    }
    fn signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(
            m,
            c,
            d,
            &self.provider.signature_verification_algorithms,
        )
    }
}
impl ServerCertVerifier for DeviceVerifier {
    fn verify_server_cert(
        &self,
        c: &CertificateDer<'_>,
        i: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> std::result::Result<ServerCertVerified, Error> {
        self.certificate(c, i)?;
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        Err(Error::General("TLS 1.2 disabled".into()))
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        self.signature(m, c, d)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
impl ClientCertVerifier for DeviceVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(
        &self,
        c: &CertificateDer<'_>,
        i: &[CertificateDer<'_>],
        _: UnixTime,
    ) -> std::result::Result<ClientCertVerified, Error> {
        self.certificate(c, i)?;
        Ok(ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        Err(Error::General("TLS 1.2 disabled".into()))
    }
    fn verify_tls13_signature(
        &self,
        m: &[u8],
        c: &CertificateDer<'_>,
        d: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, Error> {
        self.signature(m, c, d)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialized_identity_keeps_public_id_and_rejects_corruption() {
        let (first, bytes) = Identity::generate().unwrap();
        let restored = Identity::from_bytes(&bytes).unwrap();
        assert_eq!(first.id, restored.id);
        assert_ne!(first.id, Identity::generate().unwrap().0.id);
        assert!(Identity::from_bytes(&vec![0; 16 * 1024 + 1]).is_err());
        assert!(Identity::from_bytes(b"{}").is_err());
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["extra"] = true.into();
        assert!(Identity::from_bytes(&serde_json::to_vec(&value).unwrap()).is_err());
        let other: serde_json::Value =
            serde_json::from_slice(&Identity::generate().unwrap().1).unwrap();
        value.as_object_mut().unwrap().remove("extra");
        value["key"] = other["key"].clone();
        assert!(Identity::from_bytes(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}
