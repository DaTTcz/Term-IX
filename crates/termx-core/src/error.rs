use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("neznamy protokol: {0}")]
    UnknownProtocol(String),

    #[error("chyba konfigurace: {0}")]
    Config(String),

    #[error("chyba modulu protokolu: {0}")]
    Module(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
