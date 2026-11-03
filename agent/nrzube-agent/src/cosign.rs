//! Offline verification of cosign "simple signing" bundles.
//!
//! The bundle format is the Sigstore bundle v0.3 JSON produced by
//! `cosign sign-blob --key <key> --bundle <out>`:
//!
//! ```json
//! {
//!   "mediaType": "application/vnd.dev.sigstore.bundle.v0.3+json",
//!   "verificationMaterial": { "publicKey": { "hint": "<base64 sha256 of the DER key>" } },
//!   "messageSignature": {
//!     "messageDigest": { "algorithm": "SHA2_256", "digest": "<base64 sha256 of the payload>" },
//!     "signature": "<base64 DER ECDSA signature>"
//!   }
//! }
//! ```
//!
//! The public key is *not* carried by the bundle when the signature is made
//! with a key pair: only a hint (the digest of the DER public key) is. The
//! trusted key is the one embedded in the image — this is the whole trust
//! chain (embedded key -> signature of the manifest -> SHA-256 of the
//! components).
//!
//! Nothing here reaches the network: the verification is fully offline.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use p256::pkcs8::{DecodePublicKey, EncodePublicKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Media type of the bundle produced by cosign v3 for a key pair signature.
pub const BUNDLE_MEDIA_TYPE: &str = "application/vnd.dev.sigstore.bundle.v0.3+json";

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("malformed bundle: {0}")]
    Malformed(String),
    #[error("unexpected bundle media type: {0}")]
    MediaType(String),
    #[error("the bundle does not carry a message signature")]
    NoSignature,
    #[error("unsupported digest algorithm: {0}")]
    DigestAlgorithm(String),
    #[error("the digest of the payload does not match the signed digest")]
    DigestMismatch,
    #[error("invalid signature: {0}")]
    InvalidSignature(String),
    #[error("invalid public key: {0}")]
    InvalidPublicKey(String),
    #[error("no trusted key: the image does not embed a cosign public key")]
    NoTrustedKey,
}

/// A parsed cosign bundle.
#[derive(Debug, Deserialize)]
pub struct Bundle {
    #[serde(rename = "mediaType")]
    pub media_type: Option<String>,
    #[serde(rename = "verificationMaterial", default)]
    pub verification_material: VerificationMaterial,
    #[serde(rename = "messageSignature", default)]
    pub message_signature: Option<MessageSignature>,
}

#[derive(Debug, Default, Deserialize)]
pub struct VerificationMaterial {
    #[serde(rename = "publicKey", default)]
    pub public_key: Option<PublicKeyHint>,
}

#[derive(Debug, Deserialize)]
pub struct PublicKeyHint {
    #[serde(default)]
    pub hint: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MessageSignature {
    #[serde(rename = "messageDigest", default)]
    pub message_digest: Option<MessageDigest>,
    #[serde(default)]
    pub signature: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MessageDigest {
    #[serde(default)]
    pub algorithm: Option<String>,
    #[serde(default)]
    pub digest: Option<String>,
}

impl Bundle {
    /// Parse a bundle from its JSON representation.
    pub fn parse(bytes: &[u8]) -> Result<Self, VerifyError> {
        serde_json::from_slice(bytes).map_err(|e| VerifyError::Malformed(e.to_string()))
    }

    /// The digest carried by the bundle, decoded.
    fn signed_digest(&self) -> Result<Vec<u8>, VerifyError> {
        let sig = self
            .message_signature
            .as_ref()
            .ok_or(VerifyError::NoSignature)?;
        let digest = sig
            .message_digest
            .as_ref()
            .ok_or_else(|| VerifyError::Malformed("missing messageDigest".into()))?;

        match digest.algorithm.as_deref() {
            None | Some("SHA2_256") => {}
            Some(other) => return Err(VerifyError::DigestAlgorithm(other.to_string())),
        }

        let raw = digest
            .digest
            .as_deref()
            .ok_or_else(|| VerifyError::Malformed("missing digest".into()))?;
        BASE64
            .decode(raw)
            .map_err(|e| VerifyError::Malformed(format!("invalid digest encoding: {e}")))
    }

    /// The signature carried by the bundle, decoded.
    fn raw_signature(&self) -> Result<Vec<u8>, VerifyError> {
        let sig = self
            .message_signature
            .as_ref()
            .ok_or(VerifyError::NoSignature)?;
        let raw = sig
            .signature
            .as_deref()
            .ok_or_else(|| VerifyError::Malformed("missing signature".into()))?;
        BASE64
            .decode(raw)
            .map_err(|e| VerifyError::Malformed(format!("invalid signature encoding: {e}")))
    }
}

/// A cosign public key (PEM) used to verify the bundles.
#[derive(Debug, Clone)]
pub struct TrustedKey {
    verifying_key: VerifyingKey,
}

impl TrustedKey {
    /// Load a PEM encoded public key.
    pub fn from_pem(pem: &str) -> Result<Self, VerifyError> {
        let trimmed = pem.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            // A placeholder file: no trusted key at all (fail-closed).
            return Err(VerifyError::NoTrustedKey);
        }
        let key = VerifyingKey::from_public_key_pem(trimmed)
            .map_err(|e| VerifyError::InvalidPublicKey(e.to_string()))?;
        Ok(Self { verifying_key: key })
    }

    /// The hint of this key (base64 of the SHA-256 of its DER encoding),
    /// as written in the bundle by cosign.
    pub fn hint(&self) -> String {
        let der = self
            .verifying_key
            .to_public_key_der()
            .expect("a valid verifying key always encodes to DER");
        BASE64.encode(Sha256::digest(der.as_bytes()))
    }

    /// Verify a payload against a bundle.
    ///
    /// The payload is hashed with SHA-256 and the digest is compared with the
    /// digest recorded in the bundle, then the ECDSA P-256 signature is
    /// verified over the payload.
    pub fn verify(&self, payload: &[u8], bundle: &Bundle) -> Result<(), VerifyError> {
        if let Some(media_type) = &bundle.media_type {
            if media_type != BUNDLE_MEDIA_TYPE {
                return Err(VerifyError::MediaType(media_type.clone()));
            }
        }

        let signed = bundle.signed_digest()?;
        let computed: Vec<u8> = Sha256::digest(payload).to_vec();
        if signed != computed {
            return Err(VerifyError::DigestMismatch);
        }
        let signature_bytes = bundle.raw_signature()?;
        let signature = Signature::from_der(&signature_bytes)
            .map_err(|e| VerifyError::InvalidSignature(e.to_string()))?;

        self.verifying_key
            .verify(payload, &signature)
            .map_err(|e| VerifyError::InvalidSignature(e.to_string()))
    }

    /// Verify a payload against a bundle given as raw bytes.
    pub fn verify_bytes(&self, payload: &[u8], bundle_bytes: &[u8]) -> Result<(), VerifyError> {
        let bundle = Bundle::parse(bundle_bytes)?;
        self.verify(payload, &bundle)
    }

    /// Check that the bundle was made with this key (the hint is the digest
    /// of the DER public key).
    pub fn matches_hint(&self, bundle: &Bundle) -> bool {
        match bundle
            .verification_material
            .public_key
            .as_ref()
            .and_then(|k| k.hint.as_deref())
        {
            Some(hint) => hint == self.hint(),
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The public key of the fixture key pair (see scripts/gen-cosign-fixtures.sh).
    const FIXTURE_PUBKEY: &str = include_str!("../tests/fixtures/cosign.pub");
    const FIXTURE_PAYLOAD: &[u8] = include_bytes!("../tests/fixtures/manifest.json");
    const FIXTURE_BUNDLE: &[u8] = include_bytes!("../tests/fixtures/manifest.json.bundle");
    const FIXTURE_TAMPERED: &[u8] = include_bytes!("../tests/fixtures/manifest.json.tampered");
    const FIXTURE_CORRUPT: &[u8] =
        include_bytes!("../tests/fixtures/manifest.json.corrupt.bundle");

    fn fixture_key() -> TrustedKey {
        TrustedKey::from_pem(FIXTURE_PUBKEY).expect("the fixture public key must be valid")
    }

    #[test]
    fn rejects_an_empty_key() {
        assert!(matches!(
            TrustedKey::from_pem("   "),
            Err(VerifyError::NoTrustedKey)
        ));
        assert!(matches!(
            TrustedKey::from_pem("# placeholder\n"),
            Err(VerifyError::NoTrustedKey)
        ));
    }

    #[test]
    fn accepts_a_payload_signed_by_the_trusted_key() {
        fixture_key()
            .verify_bytes(FIXTURE_PAYLOAD, FIXTURE_BUNDLE)
            .expect("the fixture signature must verify");
    }

    #[test]
    fn rejects_a_tampered_payload() {
        let err = fixture_key()
            .verify_bytes(FIXTURE_TAMPERED, FIXTURE_BUNDLE)
            .unwrap_err();
        assert!(matches!(err, VerifyError::DigestMismatch));
    }

    #[test]
    fn rejects_a_corrupted_bundle() {
        let err = fixture_key()
            .verify_bytes(FIXTURE_PAYLOAD, FIXTURE_CORRUPT)
            .unwrap_err();
        assert!(
            matches!(err, VerifyError::InvalidSignature(_)),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_a_bundle_signed_by_another_key() {
        // A key that is not the trusted one must not validate the signature.
        let other = TrustedKey::from_pem(
            "-----BEGIN PUBLIC KEY-----\n\
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAES1m6zVAa4eLNVljLW+YZquzRwvKQ\n\
TJEY34FBM1DLOoTvrFe18pvQcyHQqzSENEUK2ABe/zy9Kxq9lrP9q3wIuQ==\n\
-----END PUBLIC KEY-----\n",
        )
        .expect("valid PEM key");

        let err = other.verify_bytes(FIXTURE_PAYLOAD, FIXTURE_BUNDLE).unwrap_err();
        assert!(
            matches!(err, VerifyError::InvalidSignature(_)),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_a_malformed_bundle() {
        let err = fixture_key()
            .verify_bytes(FIXTURE_PAYLOAD, b"{ not json")
            .unwrap_err();
        assert!(matches!(err, VerifyError::Malformed(_)));
    }

    #[test]
    fn rejects_an_unexpected_media_type() {
        let bundle = r#"{"mediaType":"application/vnd.other","messageSignature":{}}"#;
        let err = fixture_key()
            .verify_bytes(FIXTURE_PAYLOAD, bundle.as_bytes())
            .unwrap_err();
        assert!(matches!(err, VerifyError::MediaType(_)));
    }

    #[test]
    fn the_bundle_hint_matches_the_trusted_key() {
        // cosign stores the digest of the DER public key in the bundle: this
        // checks the hint semantics against a real cosign output.
        let key = fixture_key();
        let bundle = Bundle::parse(FIXTURE_BUNDLE).expect("the fixture bundle must parse");
        assert!(key.matches_hint(&bundle));
        assert_eq!(
            bundle
                .verification_material
                .public_key
                .as_ref()
                .and_then(|k| k.hint.as_deref()),
            Some(key.hint().as_str())
        );
    }
}
