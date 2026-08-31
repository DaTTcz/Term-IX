use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Protokol pouzity pro dane spojeni. Pridani noveho protokolu
/// = novy varianta zde + novy crate `termx-<protokol>` implementujici
/// [`crate::ProtocolModule`]. Existujici moduly se nemusi menit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Ssh,
    Serial,
    Ftp,
    Sftp,
    Telnet,
    Rdp,
}

impl Protocol {
    /// Interni identifikator - pouziva se napr. pro vyhledani modulu v registru.
    pub fn key(&self) -> &'static str {
        match self {
            Protocol::Ssh => "ssh",
            Protocol::Serial => "serial",
            Protocol::Ftp => "ftp",
            Protocol::Sftp => "sftp",
            Protocol::Telnet => "telnet",
            Protocol::Rdp => "rdp",
        }
    }
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.key().to_uppercase())
    }
}

/// Zpusob autentizace k cilovemu serveru. Tajne udaje (heslo, pasfrase klice)
/// nikdy nejsou soucasti `Session` samotne v cistem tvaru na disku - `Session`
/// je metadata ulozena/serializovana uvnitr sifrovaneho trezoru (termx-vault),
/// takze v ramci procesu je Session + AuthMethod drzena jen v pameti.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthMethod {
    Password { username: String, password: String },
    PrivateKey {
        username: String,
        key_path: String,
        passphrase: Option<String>,
    },
    Agent { username: String },
    None,
}

/// Jeden ulozeny "server" / cilove spojeni, jak jej vidi uzivatel v seznamu.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub name: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub auth: AuthMethod,
    /// Cesta ke slozce v strome serveru, napr. `"Prace/PBX"` pro vnorenou
    /// slozku PBX uvnitr Prace. `None` = korenova uroven (bez slozky).
    /// Segmenty se oddeluji lomitkem; UI (`termx-gui`) z techto cest
    /// pri vykreslovani stromu sestavuje vnorenou strukturu.
    pub group: Option<String>,
    pub notes: Option<String>,
}

impl Session {
    pub fn new(name: impl Into<String>, protocol: Protocol, host: impl Into<String>, port: u16, auth: AuthMethod) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            protocol,
            host: host.into(),
            port,
            auth,
            group: None,
            notes: None,
        }
    }
}
