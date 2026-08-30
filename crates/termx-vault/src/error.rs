use thiserror::Error;

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("soubor trezoru nema platny format Term-IX vault")]
    BadFormat,

    #[error("neplatne heslo, nebo je soubor poskozeny")]
    InvalidPasswordOrCorrupt,

    #[error("chyba pri odvozovani klice z hesla")]
    KeyDerivation,

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}
