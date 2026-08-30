//! termx-vault
//!
//! Sifrovane ulozeni ulozenych serveru ("hesla k serverum").
//!
//! Format souboru na disku:
//! `MAGIC(8B) | VERSION(1B) | SALT(16B) | NONCE(12B) | CIPHERTEXT+TAG`
//!
//! Klic se odvozuje z hlavniho hesla pres Argon2id (pomale, pametove
//! narocne KDF -> odolne proti brute-force i pri uniku souboru).
//! Sifrovani samotne je AES-256-GCM (autentizovane sifrovani - pri
//! spatnem hesle nebo poskozenem souboru desifrovani selze, misto
//! aby vratilo "tichy" spatny vysledek). Kdo hlavni heslo zapomene,
//! k datum se jiz nedostane - zadny "backdoor" ani reset hesla zde
//! zamerne neni.

mod crypto;
mod error;
mod file;

pub use error::VaultError;

use termx_core::Session;

pub type Result<T> = std::result::Result<T, VaultError>;

/// Obsah trezoru po odemceni - drzet v pameti jen po nezbytnou dobu.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct VaultData {
    pub servers: Vec<Session>,
}

/// Odemceny trezor. Heslo samotne se v `Vault` neuchovava - pri kazdem
/// `save`/`export` je potreba jej predat znovu, aby nezustavalo zbytecne
/// dlouho v pameti procesu.
pub struct Vault {
    path: std::path::PathBuf,
    pub data: VaultData,
}

impl Vault {
    /// Vytvori novy prazdny trezor na danem miste, chraneny `master_password`.
    pub fn create(path: impl Into<std::path::PathBuf>, master_password: &str) -> Result<Self> {
        let path = path.into();
        let data = VaultData::default();
        let vault = Vault { path, data };
        vault.save(master_password)?;
        Ok(vault)
    }

    /// Otevre existujici trezor a pokusi se jej odemknout hlavnim heslem.
    /// Spatne heslo -> [`VaultError::InvalidPasswordOrCorrupt`] (autentizovane
    /// sifrovani nerozlisuje mezi spatnym heslem a poskozenym souborem -
    /// to je zamerne, aby to utocnikovi nic neprozradilo).
    pub fn unlock(path: impl Into<std::path::PathBuf>, master_password: &str) -> Result<Self> {
        let path = path.into();
        let raw = std::fs::read(&path)?;
        let plaintext = file::decrypt_file(&raw, master_password)?;
        let data: VaultData = serde_json::from_slice(&plaintext)?;
        Ok(Vault { path, data })
    }

    /// Znovu zasifruje aktualni obsah a ulozi na puvodni cestu.
    pub fn save(&self, master_password: &str) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let plaintext = serde_json::to_vec(&self.data)?;
        let raw = file::encrypt_file(&plaintext, master_password)?;
        std::fs::write(&self.path, raw)?;
        Ok(())
    }

    /// Vyexportuje aktualni obsah trezoru do samostatneho sifrovaneho
    /// souboru, volitelne chraneneho jinym heslem nez ma hlavni trezor
    /// (napr. pro predani spolupracovnikovi).
    pub fn export(&self, dest_path: impl AsRef<std::path::Path>, export_password: &str) -> Result<()> {
        let plaintext = serde_json::to_vec(&self.data)?;
        let raw = file::encrypt_file(&plaintext, export_password)?;
        std::fs::write(dest_path, raw)?;
        Ok(())
    }

    /// Nacte exportovany soubor a vrati jeho obsah (bez zmeny aktualniho
    /// trezoru) - volajici si rozhodne, jak servery sloucit.
    pub fn import(src_path: impl AsRef<std::path::Path>, import_password: &str) -> Result<VaultData> {
        let raw = std::fs::read(src_path)?;
        let plaintext = file::decrypt_file(&raw, import_password)?;
        let data: VaultData = serde_json::from_slice(&plaintext)?;
        Ok(data)
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}
