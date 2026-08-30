use crate::crypto::{self, NONCE_LEN, SALT_LEN};
use crate::error::VaultError;

const MAGIC: &[u8; 8] = b"TERMX-V1";

/// Zabali plaintext do sifrovaneho binarniho formatu Term-IX vault souboru.
pub fn encrypt_file(plaintext: &[u8], password: &str) -> Result<Vec<u8>, VaultError> {
    let enc = crypto::encrypt(plaintext, password)?;

    let mut out = Vec::with_capacity(8 + SALT_LEN + NONCE_LEN + enc.ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&enc.salt);
    out.extend_from_slice(&enc.nonce);
    out.extend_from_slice(&enc.ciphertext);
    Ok(out)
}

/// Rozbali a desifruje binarni format Term-IX vault souboru.
pub fn decrypt_file(raw: &[u8], password: &str) -> Result<Vec<u8>, VaultError> {
    let min_len = MAGIC.len() + SALT_LEN + NONCE_LEN;
    if raw.len() < min_len || &raw[..MAGIC.len()] != MAGIC {
        return Err(VaultError::BadFormat);
    }

    let mut offset = MAGIC.len();
    let salt: [u8; SALT_LEN] = raw[offset..offset + SALT_LEN].try_into().unwrap();
    offset += SALT_LEN;
    let nonce: [u8; NONCE_LEN] = raw[offset..offset + NONCE_LEN].try_into().unwrap();
    offset += NONCE_LEN;
    let ciphertext = &raw[offset..];

    crypto::decrypt(ciphertext, &salt, &nonce, password)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ok_with_correct_password() {
        let raw = encrypt_file(b"tajna data", "spravne-heslo").unwrap();
        let plain = decrypt_file(&raw, "spravne-heslo").unwrap();
        assert_eq!(plain, b"tajna data");
    }

    #[test]
    fn wrong_password_fails() {
        let raw = encrypt_file(b"tajna data", "spravne-heslo").unwrap();
        let err = decrypt_file(&raw, "spatne-heslo").unwrap_err();
        assert!(matches!(err, VaultError::InvalidPasswordOrCorrupt));
    }

    #[test]
    fn corrupted_file_is_rejected() {
        let mut raw = encrypt_file(b"tajna data", "heslo").unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xFF;
        let err = decrypt_file(&raw, "heslo").unwrap_err();
        assert!(matches!(err, VaultError::InvalidPasswordOrCorrupt));
    }

    #[test]
    fn bad_magic_is_rejected() {
        let err = decrypt_file(b"not a vault file at all", "heslo").unwrap_err();
        assert!(matches!(err, VaultError::BadFormat));
    }
}
