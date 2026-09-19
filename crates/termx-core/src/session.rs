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

/// Pocet datovych bitu seriove linky - viz `Session::serial_data_bits`.
/// Zrcadli `serialport::DataBits` (`termx-serial` prevadi 1:1), ale
/// `termx-core` samo o sobe na `serialport` nezavisi - viz stejny duvod
/// jako u `SerialParity`/`SerialStopBits`/`SerialFlowControl` nize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SerialDataBits {
    Five,
    Six,
    Seven,
    Eight,
}

/// Parita seriove linky - viz `Session::serial_parity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SerialParity {
    None,
    Odd,
    Even,
}

/// Pocet stop bitu seriove linky - viz `Session::serial_stop_bits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SerialStopBits {
    One,
    Two,
}

/// Rizeni toku seriove linky - viz `Session::serial_flow_control`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SerialFlowControl {
    None,
    /// Software (XON/XOFF).
    Software,
    /// Hardware (RTS/CTS).
    Hardware,
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
    /// Terminal type poslany serveru v SSH `pty-req` (napr. jako `$TERM`
    /// na druhe strane) - napr. pro Avaya Communication Manager SAT
    /// rozhrani, ktere ocekava "513" (AT&T Terminal 513 emulation)
    /// misto beznych `xterm-256color`/`vt100`. `None`/prazdne =
    /// zustava puvodni chovani (`xterm-256color`, viz `termx-ssh`).
    /// `#[serde(default)]`, aby stare ulozene servery v trezoru (bez
    /// tohoto pole) sly nadale nacist.
    #[serde(default)]
    pub term_type: Option<String>,
    /// Rychlost seriove linky (baud rate, napr. 9600/19200/115200) -
    /// relevantni jen pro `Protocol::Serial` (viz `termx-serial`); u
    /// ostatnich protokolu se proste nepouziva. `None` = vychozi
    /// `termx_serial::DEFAULT_BAUD_RATE` (9600 - nejcastejsi vychozi
    /// hodnota u konzolovych/sitovych zarizeni jako Cisco/Avaya).
    ///
    /// Port samotny (napr. `/dev/ttyUSB0` nebo `COM3`) NEMA vlastni pole -
    /// pro `Protocol::Serial` se ulozi primo do jiz existujiciho `host`
    /// (viz doc-komentar u `ProtocolModule` - modul si specificke udaje
    /// muze cist rovnou ze `Session`), `port`/`auth` zustavaji u
    /// serioveho spojeni nevyuzite (vychozi `0`/`AuthMethod::None`).
    ///
    /// `#[serde(default)]`, aby stare ulozene servery v trezoru (bez
    /// tohoto pole - vsechny existujici jsou `Protocol::Ssh`, kde na tom
    /// stejne nezalezi) sly nadale nacist.
    #[serde(default)]
    pub serial_baud_rate: Option<u32>,
    /// Pocet datovych bitu - `None` = vychozi `Eight` (nejbeznejsi).
    /// Uzivatelsky pozadavek "chci komplet nastavení portu" - na rozdil od
    /// puvodniho zjednoduseneho navrhu (jen baud rate) jsou ted
    /// nastavitelne vsechny 4 parametry seriove linky.
    #[serde(default)]
    pub serial_data_bits: Option<SerialDataBits>,
    /// Parita - `None` = vychozi `SerialParity::None` (bez parity).
    #[serde(default)]
    pub serial_parity: Option<SerialParity>,
    /// Pocet stop bitu - `None` = vychozi `One`.
    #[serde(default)]
    pub serial_stop_bits: Option<SerialStopBits>,
    /// Rizeni toku - `None` = vychozi `SerialFlowControl::None` (zadne).
    #[serde(default)]
    pub serial_flow_control: Option<SerialFlowControl>,
    /// Kdyz `true`, FTP spojeni (`Protocol::Ftp`) se hned po navazani TCP
    /// spojeni povysi na FTPS explicitnim "AUTH TLS" prikazem (viz
    /// `termx-ftp::ftp::connect`) - `false` = obycejne nesifrovane FTP.
    /// U ostatnich protokolu se nepouziva. `#[serde(default)]`, aby stare
    /// ulozene servery v trezoru (bez tohoto pole) sly nadale nacist.
    #[serde(default)]
    pub ftp_use_tls: bool,
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
            term_type: None,
            serial_baud_rate: None,
            serial_data_bits: None,
            serial_parity: None,
            serial_stop_bits: None,
            serial_flow_control: None,
            ftp_use_tls: false,
        }
    }
}
