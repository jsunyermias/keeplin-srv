//! At-rest encryption of the sensitive note fields — line content and note
//! title (issue keeplin#110).
//!
//! Collaborative editing needs the plaintext in the server's memory to merge
//! line ops, so this does **not** make notes end-to-end encrypted; it protects
//! the data **at rest** in PostgreSQL. A database dump, a stolen backup, or SQL
//! read access sees ciphertext, not note contents. It does not defend against a
//! compromised running server or a malicious operator (both hold the key).
//!
//! Design:
//! - AES-256-GCM with a fresh random 96-bit nonce per value.
//! - The key comes from `AT_REST_KEY` (base64, 32 bytes). If unset, encryption
//!   is **disabled** and values are stored as-is — so the feature is opt-in and
//!   an existing deployment keeps working.
//! - A stored value is tagged `enc:v1:<base64(nonce‖ciphertext)>`. Untagged
//!   values are plaintext. Both forms decrypt correctly, so enabling the key on
//!   a running database is safe: old rows stay readable and new writes are
//!   encrypted. The one-off `keeplin-reencrypt` binary (`src/reencrypt.rs`)
//!   migrates the old plaintext rows to `enc:v1:` — see RUNBOOK.md.

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Key, Nonce};
use base64::Engine as _;

use crate::error::AppError;

/// Storage tag prefixing every encrypted value. Public so the re-encrypt pass
/// (`src/reencrypt.rs`) can select the rows that still lack it (`NOT LIKE
/// 'enc:v1:%'`) without duplicating the literal.
pub const ENC_PREFIX: &str = "enc:v1:";
const TAG: &str = ENC_PREFIX;
const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

/// The at-rest cipher. Cheap to clone; holds the parsed key (or nothing, when
/// encryption is disabled).
#[derive(Clone)]
pub struct Cipher {
    cipher: Option<Aes256Gcm>,
}

impl Cipher {
    /// Build from the optional base64 `AT_REST_KEY`. `None`/empty disables
    /// encryption (values pass through). A present-but-invalid key is a
    /// configuration error and is returned as such so the server refuses to
    /// start rather than silently storing plaintext.
    pub fn from_key(key: Option<&str>) -> Result<Self, String> {
        let raw = match key.map(str::trim).filter(|k| !k.is_empty()) {
            None => return Ok(Self { cipher: None }),
            Some(k) => k,
        };
        let bytes = B64
            .decode(raw)
            .map_err(|_| "AT_REST_KEY must be valid base64".to_string())?;
        if bytes.len() != 32 {
            return Err(format!(
                "AT_REST_KEY must decode to 32 bytes (got {})",
                bytes.len()
            ));
        }
        let key = Key::<Aes256Gcm>::from_slice(&bytes);
        Ok(Self {
            cipher: Some(Aes256Gcm::new(key)),
        })
    }

    pub fn enabled(&self) -> bool {
        self.cipher.is_some()
    }

    /// Encrypt a value for storage. When disabled, returns the plaintext
    /// unchanged. When enabled, returns `enc:v1:<base64(nonce‖ciphertext)>`.
    pub fn encrypt(&self, plaintext: &str) -> Result<String, AppError> {
        let Some(cipher) = &self.cipher else {
            return Ok(plaintext.to_string());
        };
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| AppError::Internal("at-rest encryption failed".into()))?;
        let mut blob = nonce.to_vec();
        blob.extend_from_slice(&ct);
        Ok(format!("{TAG}{}", B64.encode(blob)))
    }

    /// Decrypt a stored value. An untagged value is returned as-is (plaintext,
    /// or a value written before the key was enabled). A tagged value is
    /// decrypted; a failure (wrong key, corruption) is an error rather than a
    /// silent wrong answer.
    pub fn decrypt(&self, stored: &str) -> Result<String, AppError> {
        let Some(rest) = stored.strip_prefix(TAG) else {
            return Ok(stored.to_string());
        };
        let cipher = self
            .cipher
            .as_ref()
            .ok_or_else(|| AppError::Internal("encrypted value but AT_REST_KEY is unset".into()))?;
        let blob = B64
            .decode(rest)
            .map_err(|_| AppError::Internal("at-rest ciphertext is not valid base64".into()))?;
        if blob.len() < 12 {
            return Err(AppError::Internal("at-rest ciphertext too short".into()));
        }
        let (nonce_bytes, ct) = blob.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        let pt = cipher
            .decrypt(nonce, ct)
            .map_err(|_| AppError::Internal("at-rest decryption failed (wrong key?)".into()))?;
        String::from_utf8(pt).map_err(|_| AppError::Internal("at-rest plaintext not utf-8".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> String {
        B64.encode([7u8; 32])
    }

    #[test]
    fn disabled_is_passthrough() {
        let c = Cipher::from_key(None).unwrap();
        assert!(!c.enabled());
        assert_eq!(c.encrypt("hello").unwrap(), "hello");
        assert_eq!(c.decrypt("hello").unwrap(), "hello");
    }

    #[test]
    fn round_trips_and_tags() {
        let c = Cipher::from_key(Some(&test_key())).unwrap();
        let ct = c.encrypt("secret note line").unwrap();
        assert!(ct.starts_with(TAG), "stored value must be tagged");
        assert_ne!(ct, "secret note line");
        assert_eq!(c.decrypt(&ct).unwrap(), "secret note line");
    }

    #[test]
    fn nonce_is_random_per_value() {
        let c = Cipher::from_key(Some(&test_key())).unwrap();
        assert_ne!(c.encrypt("x").unwrap(), c.encrypt("x").unwrap());
    }

    #[test]
    fn reads_legacy_plaintext_when_enabled() {
        // Enabling the key on an existing DB must not break old plaintext rows.
        let c = Cipher::from_key(Some(&test_key())).unwrap();
        assert_eq!(
            c.decrypt("old plaintext title").unwrap(),
            "old plaintext title"
        );
    }

    #[test]
    fn wrong_key_fails_loudly() {
        let a = Cipher::from_key(Some(&B64.encode([1u8; 32]))).unwrap();
        let b = Cipher::from_key(Some(&B64.encode([2u8; 32]))).unwrap();
        let ct = a.encrypt("data").unwrap();
        assert!(b.decrypt(&ct).is_err());
    }

    #[test]
    fn bad_key_length_rejected() {
        assert!(Cipher::from_key(Some(&B64.encode([0u8; 16]))).is_err());
        assert!(Cipher::from_key(Some("not base64!!!")).is_err());
    }
}
