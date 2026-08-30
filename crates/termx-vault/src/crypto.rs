use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::Argon2;
use rand::RngCore;
use zeroize::Zeroize;

use crate::error::VaultError;

pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

/// Odvodi 256bit klic z hesla a nahodne soli pomoci Argon2id.
/// Parametry Argon2 (pamet/iterace) berou vychozi "sensible" hodnoty
/// crate `argon2`, ktere jsou navrzene tak, aby brute-force utok byl
/// vypocetne a pametove drahy.
fn derive_key(password: &str, salt: &[u8; SALT_LEN]) -> Result<[u8; KEY_LEN], VaultError> {
    let mut key = [0u8; KEY_LEN];
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|_| VaultError::KeyDerivation)?;
    Ok(key)
}

pub struct Encrypted {
    pub salt: [u8; SALT_LEN],
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

pub fn encrypt(plaintext: &[u8], password: &str) -> Result<Encrypted, VaultError> {
    let mut rng = rand::thread_rng();

    let mut salt = [0u8; SALT_LEN];
    rng.fill_bytes(&mut salt);

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rng.fill_bytes(&mut nonce_bytes);

    let mut key = derive_key(password, &salt)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| VaultError::KeyDerivation)?;

    key.zeroize();

    Ok(Encrypted {
        salt,
        nonce: nonce_bytes,
        ciphertext,
    })
}

pub fn decrypt(
    ciphertext: &[u8],
    salt: &[u8; SALT_LEN],
    nonce_bytes: &[u8; NONCE_LEN],
    password: &str,
) -> Result<Vec<u8>, VaultError> {
    let mut key = derive_key(password, salt)?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let nonce = Nonce::from_slice(nonce_bytes);

    let result = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| VaultError::InvalidPasswordOrCorrupt);

    key.zeroize();
    result
}
