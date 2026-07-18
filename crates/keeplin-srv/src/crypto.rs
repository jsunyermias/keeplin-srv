// md:Overview
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Key, Nonce};
use base64::Engine as _;

use crate::error::AppError;

// md:Constants
pub const ENC_PREFIX: &str = "enc:v1:";
const TAG: &str = ENC_PREFIX;
const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;

// md:Cipher
#[derive(Clone)]
pub struct Cipher {
    cipher: Option<Aes256Gcm>,
}

// md:impl Cipher
impl Cipher {
    // md:impl Cipher > fn from_key
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

    // md:impl Cipher > fn enabled
    pub fn enabled(&self) -> bool {
        self.cipher.is_some()
    }

    // md:impl Cipher > fn encrypt
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

    // md:impl Cipher > fn decrypt
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

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;

    // md:mod tests > fn test_key
    fn test_key() -> String {
        B64.encode([7u8; 32])
    }

    // md:mod tests > fn disabled_is_passthrough
    #[test]
    fn disabled_is_passthrough() {
        let c = Cipher::from_key(None).unwrap();
        assert!(!c.enabled());
        assert_eq!(c.encrypt("hello").unwrap(), "hello");
        assert_eq!(c.decrypt("hello").unwrap(), "hello");
    }

    // md:mod tests > fn round_trips_and_tags
    #[test]
    fn round_trips_and_tags() {
        let c = Cipher::from_key(Some(&test_key())).unwrap();
        let ct = c.encrypt("secret note line").unwrap();
        assert!(ct.starts_with(TAG), "stored value must be tagged");
        assert_ne!(ct, "secret note line");
        assert_eq!(c.decrypt(&ct).unwrap(), "secret note line");
    }

    // md:mod tests > fn nonce_is_random_per_value
    #[test]
    fn nonce_is_random_per_value() {
        let c = Cipher::from_key(Some(&test_key())).unwrap();
        assert_ne!(c.encrypt("x").unwrap(), c.encrypt("x").unwrap());
    }

    // md:mod tests > fn reads_legacy_plaintext_when_enabled
    #[test]
    fn reads_legacy_plaintext_when_enabled() {
        let c = Cipher::from_key(Some(&test_key())).unwrap();
        assert_eq!(
            c.decrypt("old plaintext title").unwrap(),
            "old plaintext title"
        );
    }

    // md:mod tests > fn wrong_key_fails_loudly
    #[test]
    fn wrong_key_fails_loudly() {
        let a = Cipher::from_key(Some(&B64.encode([1u8; 32]))).unwrap();
        let b = Cipher::from_key(Some(&B64.encode([2u8; 32]))).unwrap();
        let ct = a.encrypt("data").unwrap();
        assert!(b.decrypt(&ct).is_err());
    }

    // md:mod tests > fn bad_key_length_rejected
    #[test]
    fn bad_key_length_rejected() {
        assert!(Cipher::from_key(Some(&B64.encode([0u8; 16]))).is_err());
        assert!(Cipher::from_key(Some("not base64!!!")).is_err());
    }
}
