use aes_gcm::aead::{Aead, AeadCore, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHasher};
use bip39::{Language, Mnemonic};
use ed25519_dalek::{SigningKey, VerifyingKey};
use hex;
use rand::rngs::OsRng;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use thiserror::Error;
use zeroize::Zeroize;

#[derive(Error, Debug)]
pub enum WalletError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Encryption error: {0}")]
    Encryption(String),
    #[error("Invalid key length: {0}")]
    InvalidLength(String),
    #[error("Invalid key: {0}")]
    InvalidKey(String),
    #[error("Invalid hex: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("Mnemonic error: {0}")]
    Mnemonic(String),
    #[error("Passphrase verification failed")]
    PassphraseVerification,
}

#[derive(Serialize, Deserialize)]
pub struct HyperWallet {
    signing_key: Vec<u8>,
    verifying_key: Vec<u8>,
    #[serde(skip)]
    mnemonic: Option<Mnemonic>,
}

impl HyperWallet {
    pub fn new() -> Result<Self, WalletError> {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let entropy = OsRng.gen::<[u8; 16]>();
        let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy)
            .map_err(|e| WalletError::Mnemonic(e.to_string()))?;
        Ok(HyperWallet {
            signing_key: signing_key.to_bytes().to_vec(),
            verifying_key: verifying_key.to_bytes().to_vec(),
            mnemonic: Some(mnemonic),
        })
    }

    pub fn from_private_key(private_key: &str) -> Result<Self, WalletError> {
        let key_bytes = hex::decode(private_key)?;
        if key_bytes.len() != 32 {
            return Err(WalletError::InvalidLength(format!(
                "Expected 32 bytes, got {}",
                key_bytes.len()
            )));
        }
        let signing_key = SigningKey::from_bytes(
            &key_bytes
                .try_into()
                .map_err(|_| WalletError::InvalidKey("Invalid signing key format".to_string()))?,
        );
        let verifying_key = signing_key.verifying_key();
        Ok(HyperWallet {
            signing_key: signing_key.to_bytes().to_vec(),
            verifying_key: verifying_key.to_bytes().to_vec(),
            mnemonic: None,
        })
    }

    pub fn from_mnemonic(mnemonic: &str) -> Result<Self, WalletError> {
        let mnemonic = Mnemonic::parse_in(Language::English, mnemonic)
            .map_err(|e| WalletError::Mnemonic(e.to_string()))?;
        let seed = mnemonic.to_seed("");
        let signing_key = SigningKey::from_bytes(
            &seed[..32]
                .try_into()
                .map_err(|_| WalletError::InvalidKey("Invalid seed length".to_string()))?,
        );
        let verifying_key = signing_key.verifying_key();
        Ok(HyperWallet {
            signing_key: signing_key.to_bytes().to_vec(),
            verifying_key: verifying_key.to_bytes().to_vec(),
            mnemonic: Some(mnemonic),
        })
    }

    pub fn get_address(&self) -> String {
        hex::encode(&self.verifying_key).to_lowercase()
    }

    pub fn get_public_key(&self) -> Result<VerifyingKey, WalletError> {
        VerifyingKey::from_bytes(
            &self
                .verifying_key
                .clone()
                .try_into()
                .map_err(|_| WalletError::InvalidKey("Invalid verifying key".to_string()))?,
        )
        .map_err(|e| WalletError::InvalidKey(e.to_string()))
    }

    pub fn get_signing_key(&self) -> Result<SigningKey, WalletError> {
        let key_bytes: [u8; 32] = self
            .signing_key
            .clone()
            .try_into()
            .map_err(|_| WalletError::InvalidKey("Invalid signing key".to_string()))?;
        Ok(SigningKey::from_bytes(&key_bytes))
    }

    pub fn get_mnemonic(&self) -> Option<String> {
        self.mnemonic.as_ref().map(|m| m.to_string())
    }

    pub fn from_file(path: &str, passphrase: Option<&str>) -> Result<Self, WalletError> {
        let mut file = File::open(path)?;
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)?;

        if let Some(pass) = passphrase {
            if contents.len() < 12 {
                return Err(WalletError::Encryption("Invalid encrypted data length".to_string()));
            }
            let nonce = Nonce::from_slice(&contents[..12]);
            let ciphertext = &contents[12..];
            let salt = SaltString::generate(&mut OsRng);
            let argon2 = Argon2::default();
            let password_hash = argon2
                .hash_password(pass.as_bytes(), &salt)
                .map_err(|e| WalletError::Encryption(e.to_string()))?;
            let hash = password_hash.hash.unwrap();
            let mut key = [0u8; 32];
            key.copy_from_slice(&hash.as_bytes()[..32]);
            let cipher = Aes256Gcm::new_from_slice(&key)
                .map_err(|e| WalletError::Encryption(e.to_string()))?;
            let plaintext = cipher
                .decrypt(nonce, Payload { msg: ciphertext, aad: b"" })
                .map_err(|e| WalletError::Encryption(e.to_string()))?;
            Ok(serde_json::from_slice(&plaintext)?)
        } else {
            Ok(serde_json::from_slice(&contents)?)
        }
    }

    pub fn save_to_file(&self, path: &str, passphrase: Option<&str>) -> Result<(), WalletError> {
        let serialized = serde_json::to_vec(self)?;
        if let Some(pass) = passphrase {
            let salt = SaltString::generate(&mut OsRng);
            let argon2 = Argon2::default();
            let password_hash = argon2
                .hash_password(pass.as_bytes(), &salt)
                .map_err(|e| WalletError::Encryption(e.to_string()))?;
            let hash = password_hash.hash.unwrap();
            let mut key = [0u8; 32];
            key.copy_from_slice(&hash.as_bytes()[..32]);
            let cipher = Aes256Gcm::new_from_slice(&key)
                .map_err(|e| WalletError::Encryption(e.to_string()))?;
            let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
            let ciphertext = cipher
                .encrypt(&nonce, Payload { msg: &serialized, aad: b"" })
                .map_err(|e| WalletError::Encryption(e.to_string()))?;
            let mut file = File::create(path)?;
            file.write_all(&nonce)?;
            file.write_all(&ciphertext)?;
        } else {
            fs::write(path, serialized)?;
        }
        Ok(())
    }
}

impl From<ed25519_dalek::ed25519::Error> for WalletError {
    fn from(e: ed25519_dalek::ed25519::Error) -> Self {
        WalletError::InvalidKey(e.to_string())
    }
}

impl Drop for HyperWallet {
    fn drop(&mut self) {
        self.signing_key.zeroize();
        if let Some(mnemonic) = self.mnemonic.take() {
            let phrase = mnemonic.to_string();
            let mut bytes = phrase.into_bytes();
            bytes.zeroize();
        }
    }
}