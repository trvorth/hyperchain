use aes_gcm::aead::{Aead, AeadCore, OsRng};
use aes_gcm::{aead::generic_array, Aes256Gcm, Key, KeyInit};
use anyhow::{Context, Result};
use argon2::{
    password_hash::{PasswordHash, PasswordVerifier, SaltString},
    Argon2, PasswordHasher,
};
use bip39::Mnemonic;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use rand::Rng;
use secrecy::{ExposeSecret, Secret, SecretVec};
use serde::{Deserialize, Serialize};
use sha2::digest::typenum::Unsigned;
use std::convert::TryInto;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use zeroize::{Zeroize, ZeroizeOnDrop};

const WALLET_FILE_VERSION: u8 = 2;

#[derive(thiserror::Error, Debug)]
pub enum WalletError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] bincode::Error),
    #[error("Encryption/decryption error: {0}")]
    Encryption(String),
    #[error("Passphrase hashing or verification error: {0}")]
    Passphrase(String),
    #[error("Cryptographic operation failed: {0}")]
    Crypto(#[from] ed25519_dalek::SignatureError),
    #[error("Invalid wallet file format or version")]
    InvalidFormat,
    #[error("Invalid private key length or format")]
    InvalidKeyLength,
    #[error("Mnemonic generation or parsing error: {0}")]
    Mnemonic(#[from] bip39::Error),
    #[error("Failed to convert slice to array: {0}")]
    SliceToArrayError(#[from] std::array::TryFromSliceError),
    #[error("Hex decoding error: {0}")]
    Hex(#[from] hex::FromHexError),
}

#[derive(Serialize, Deserialize)]
struct PlainWalletData {
    signing_key: Vec<u8>,
    verifying_key: Vec<u8>,
    mnemonic: String,
}

struct WalletData {
    signing_key: SecretVec<u8>,
    verifying_key: Vec<u8>,
    mnemonic: Secret<String>,
}

impl Serialize for WalletData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        PlainWalletData {
            signing_key: self.signing_key.expose_secret().clone(),
            verifying_key: self.verifying_key.clone(),
            mnemonic: self.mnemonic.expose_secret().clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for WalletData {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let plain = PlainWalletData::deserialize(deserializer)?;
        Ok(WalletData {
            signing_key: SecretVec::new(plain.signing_key),
            verifying_key: plain.verifying_key,
            mnemonic: Secret::new(plain.mnemonic),
        })
    }
}

impl Drop for WalletData {
    fn drop(&mut self) {
        self.zeroize();
    }
}

impl Zeroize for WalletData {
    fn zeroize(&mut self) {
        self.verifying_key.zeroize();
    }
}

#[derive(ZeroizeOnDrop)]
pub struct Wallet {
    data: WalletData,
}

impl Wallet {
    pub fn new() -> Result<Self, WalletError> {
        let mut entropy = [0u8; 32];
        rand::thread_rng().fill(&mut entropy);
        let mnemonic = Mnemonic::from_entropy(&entropy)?;
        Self::from_mnemonic(mnemonic.to_string().as_str())
    }

    pub fn from_private_key(private_key_hex: &str) -> Result<Self, WalletError> {
        let key_bytes = hex::decode(private_key_hex)?;
        if key_bytes.len() != 32 {
            return Err(WalletError::InvalidKeyLength);
        }
        let signing_key = SigningKey::from_bytes(&key_bytes.try_into().unwrap());
        Ok(Self {
            data: WalletData {
                signing_key: SecretVec::new(signing_key.to_bytes().to_vec()),
                verifying_key: signing_key.verifying_key().to_bytes().to_vec(),
                mnemonic: Secret::new("".to_string()),
            },
        })
    }

    pub fn from_mnemonic(mnemonic_phrase: &str) -> Result<Self, WalletError> {
        let mnemonic = Mnemonic::parse(mnemonic_phrase)?;
        let seed = mnemonic.to_seed("");
        let signing_key = SigningKey::from_bytes(
            seed.as_ref()[..32]
                .try_into()
                .map_err(|_| WalletError::InvalidKeyLength)?,
        );

        Ok(Self {
            data: WalletData {
                signing_key: SecretVec::new(signing_key.to_bytes().to_vec()),
                verifying_key: signing_key.verifying_key().to_bytes().to_vec(),
                mnemonic: Secret::new(mnemonic.to_string()),
            },
        })
    }

    pub fn get_signing_key(&self) -> Result<SigningKey, WalletError> {
        let signing_key_bytes: &[u8] = self.data.signing_key.expose_secret();
        let signing_key = SigningKey::from_bytes(signing_key_bytes.try_into()?);
        Ok(signing_key)
    }

    pub fn sign(&self, message: &[u8]) -> Result<ed25519_dalek::Signature, WalletError> {
        let signing_key = self.get_signing_key()?;
        Ok(signing_key.sign(message))
    }

    pub fn verify(
        &self,
        message: &[u8],
        signature: &ed25519_dalek::Signature,
    ) -> Result<(), WalletError> {
        let verifying_key =
            VerifyingKey::from_bytes(self.data.verifying_key.as_slice().try_into()?)?;
        verifying_key
            .verify_strict(message, signature)
            .map_err(WalletError::from)
    }

    pub fn address(&self) -> String {
        hex::encode(&self.data.verifying_key)
    }

    pub fn mnemonic(&self) -> &Secret<String> {
        &self.data.mnemonic
    }

    pub fn save_to_file<P: AsRef<Path>>(
        &self,
        path: P,
        passphrase: &Secret<String>,
    ) -> Result<(), WalletError> {
        let plaintext = bincode::serialize(&self.data)?;
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2
            .hash_password(passphrase.expose_secret().as_bytes(), &salt)
            .map_err(|e| WalletError::Passphrase(e.to_string()))?;
        let key_bytes = password_hash
            .hash
            .context("Argon2 hash is missing")
            .unwrap();
        let key = Key::<Aes256Gcm>::from_slice(key_bytes.as_bytes());
        let cipher = Aes256Gcm::new(key);
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, plaintext.as_slice())
            .map_err(|e| WalletError::Encryption(e.to_string()))?;
        let salt_string = salt.as_str();
        let salt_len = (salt_string.len() as u32).to_le_bytes();
        let mut file_contents = Vec::new();
        file_contents.push(WALLET_FILE_VERSION);
        file_contents.extend_from_slice(&salt_len);
        file_contents.extend_from_slice(salt_string.as_bytes());
        file_contents.extend_from_slice(nonce.as_slice());
        file_contents.extend_from_slice(&ciphertext);
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path.as_ref())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            file.set_permissions(perms)?;
        }
        file.write_all(&file_contents)?;
        Ok(())
    }

    pub fn from_file<P: AsRef<Path>>(
        path: P,
        passphrase: &Secret<String>,
    ) -> Result<Self, WalletError> {
        let file_contents = fs::read(path)?;
        if file_contents.is_empty() || file_contents[0] != WALLET_FILE_VERSION {
            return Err(WalletError::InvalidFormat);
        }
        if file_contents.len() < 5 {
            return Err(WalletError::InvalidFormat);
        }
        let salt_len = u32::from_le_bytes(file_contents[1..5].try_into().unwrap()) as usize;
        let nonce_len = <Aes256Gcm as AeadCore>::NonceSize::to_usize();
        let salt_start = 5;
        let salt_end = salt_start + salt_len;
        let nonce_end = salt_end + nonce_len;
        if file_contents.len() <= nonce_end {
            return Err(WalletError::InvalidFormat);
        }
        let salt_str = std::str::from_utf8(&file_contents[salt_start..salt_end])
            .map_err(|_| WalletError::InvalidFormat)?;
        let nonce = generic_array::GenericArray::from_slice(&file_contents[salt_end..nonce_end]);
        let ciphertext = &file_contents[nonce_end..];
        let password_hash =
            PasswordHash::new(salt_str).map_err(|e| WalletError::Passphrase(e.to_string()))?;
        Argon2::default()
            .verify_password(passphrase.expose_secret().as_bytes(), &password_hash)
            .map_err(|e| WalletError::Passphrase(e.to_string()))?;
        let key_bytes = password_hash
            .hash
            .context("Argon2 hash is missing")
            .unwrap();
        let key = Key::<Aes256Gcm>::from_slice(key_bytes.as_bytes());
        let cipher = Aes256Gcm::new(key);
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| WalletError::Encryption(e.to_string()))?;
        let data: WalletData = bincode::deserialize(&plaintext)?;
        Ok(Self { data })
    }
}
