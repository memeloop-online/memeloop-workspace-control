use std::fmt;

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use thiserror::Error;
use zeroize::Zeroizing;

const KEY_LENGTH: usize = 32;
const NONCE_LENGTH: usize = 12;

#[derive(Clone)]
pub struct EnvelopeCipher {
    key_encryption_key: Zeroizing<[u8; KEY_LENGTH]>,
}

impl fmt::Debug for EnvelopeCipher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvelopeCipher")
            .field("key_encryption_key", &"[REDACTED]")
            .finish()
    }
}

impl EnvelopeCipher {
    pub fn from_base64(encoded_key: &str) -> Result<Self, CryptoError> {
        let bytes = STANDARD
            .decode(encoded_key)
            .map_err(|_| CryptoError::InvalidMasterKey)?;
        let key_encryption_key = <[u8; KEY_LENGTH]>::try_from(bytes.as_slice())
            .map_err(|_| CryptoError::InvalidMasterKey)?;
        Ok(Self {
            key_encryption_key: Zeroizing::new(key_encryption_key),
        })
    }

    pub fn generate_base64_key() -> Result<String, CryptoError> {
        let mut key = Zeroizing::new([0_u8; KEY_LENGTH]);
        getrandom::fill(key.as_mut()).map_err(|_| CryptoError::RandomSource)?;
        Ok(STANDARD.encode(key.as_ref()))
    }

    pub fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> Result<EncryptedEnvelope, CryptoError> {
        let mut data_key = Zeroizing::new([0_u8; KEY_LENGTH]);
        let mut value_nonce = [0_u8; NONCE_LENGTH];
        let mut key_nonce = [0_u8; NONCE_LENGTH];
        getrandom::fill(data_key.as_mut()).map_err(|_| CryptoError::RandomSource)?;
        getrandom::fill(&mut value_nonce).map_err(|_| CryptoError::RandomSource)?;
        getrandom::fill(&mut key_nonce).map_err(|_| CryptoError::RandomSource)?;

        let value_cipher =
            Aes256Gcm::new_from_slice(data_key.as_ref()).map_err(|_| CryptoError::Encryption)?;
        let ciphertext = value_cipher
            .encrypt(
                &Nonce::from(value_nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| CryptoError::Encryption)?;
        let key_cipher = Aes256Gcm::new_from_slice(self.key_encryption_key.as_ref())
            .map_err(|_| CryptoError::Encryption)?;
        let wrapped_data_key = key_cipher
            .encrypt(
                &Nonce::from(key_nonce),
                Payload {
                    msg: data_key.as_ref(),
                    aad,
                },
            )
            .map_err(|_| CryptoError::Encryption)?;

        Ok(EncryptedEnvelope {
            ciphertext,
            value_nonce,
            wrapped_data_key,
            key_nonce,
        })
    }

    pub fn decrypt(
        &self,
        envelope: &EncryptedEnvelope,
        aad: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        let key_cipher = Aes256Gcm::new_from_slice(self.key_encryption_key.as_ref())
            .map_err(|_| CryptoError::Decryption)?;
        let data_key = Zeroizing::new(
            key_cipher
                .decrypt(
                    &Nonce::from(envelope.key_nonce),
                    Payload {
                        msg: &envelope.wrapped_data_key,
                        aad,
                    },
                )
                .map_err(|_| CryptoError::Decryption)?,
        );
        if data_key.len() != KEY_LENGTH {
            return Err(CryptoError::Decryption);
        }
        let value_cipher =
            Aes256Gcm::new_from_slice(data_key.as_ref()).map_err(|_| CryptoError::Decryption)?;
        let plaintext = value_cipher
            .decrypt(
                &Nonce::from(envelope.value_nonce),
                Payload {
                    msg: &envelope.ciphertext,
                    aad,
                },
            )
            .map_err(|_| CryptoError::Decryption)?;
        Ok(Zeroizing::new(plaintext))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedEnvelope {
    pub ciphertext: Vec<u8>,
    pub value_nonce: [u8; NONCE_LENGTH],
    pub wrapped_data_key: Vec<u8>,
    pub key_nonce: [u8; NONCE_LENGTH],
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("master encryption key must be valid Base64 containing exactly 32 bytes")]
    InvalidMasterKey,
    #[error("operating system random source failed")]
    RandomSource,
    #[error("envelope encryption failed")]
    Encryption,
    #[error("envelope authentication or decryption failed")]
    Decryption,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cipher() -> EnvelopeCipher {
        EnvelopeCipher::from_base64(&STANDARD.encode([7_u8; KEY_LENGTH])).unwrap()
    }

    #[test]
    fn envelope_round_trip_preserves_binary_and_multiline_values() {
        let plaintext = b"first\n\n  indented\n\0binary\n";
        let envelope = cipher().encrypt(plaintext, b"org:one:key:1").unwrap();
        assert!(
            !envelope
                .ciphertext
                .windows(plaintext.len())
                .any(|part| part == plaintext)
        );
        assert_eq!(
            cipher()
                .decrypt(&envelope, b"org:one:key:1")
                .unwrap()
                .as_slice(),
            plaintext
        );
    }

    #[test]
    fn aad_and_master_key_are_authenticated() {
        let envelope = cipher().encrypt(b"secret", b"scope-a").unwrap();
        assert_eq!(
            cipher().decrypt(&envelope, b"scope-b"),
            Err(CryptoError::Decryption)
        );
        let other = EnvelopeCipher::from_base64(&STANDARD.encode([8_u8; KEY_LENGTH])).unwrap();
        assert_eq!(
            other.decrypt(&envelope, b"scope-a"),
            Err(CryptoError::Decryption)
        );
    }

    #[test]
    fn every_encryption_uses_fresh_keys_and_nonces() {
        let first = cipher().encrypt(b"same", b"same-aad").unwrap();
        let second = cipher().encrypt(b"same", b"same-aad").unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn debug_output_never_contains_master_key() {
        let cipher = cipher();
        let debug = format!("{cipher:?}");
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains("BwcH"));
    }
}
