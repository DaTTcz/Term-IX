//! termx-ftp
//!
//! FTP/FTPS protokolovy modul. Na rozdil od `termx-ssh` (async, kvuli
//! `russh`) pouziva stejny pristup jako `termx-serial`: `suppaftp` je
//! synchronni (blokujici) knihovna, takze zadny tokio runtime tu neni
//! potreba - skutecne spojeni bezi ve vlastnim OS vlakne a komunikuje s
//! GUI pres obycejne `std::sync::mpsc` kanaly (`FtpCommand`/`FtpEvent`,
//! viz `ftp.rs`).
//!
//! FTPS (explicitni AUTH TLS pres `rustls-ring` - cisty Rust, bez
//! zavislosti na OpenSSL) je soucasti uz od prvni verze tohoto modulu
//! (viz `Session::ftp_use_tls` v `termx-core` a `ftp::connect`), ne
//! pozdejsi doplnek.

mod ftp;

use termx_core::{ConnectionContext, ProtocolModule};

pub use ftp::{spawn_ftp_session, FtpCommand, FtpEntry, FtpEvent, FtpHandle};

#[derive(Default)]
pub struct FtpModule;

impl FtpModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl ProtocolModule for FtpModule {
    fn protocol_key(&self) -> &'static str {
        "ftp"
    }

    fn display_name(&self) -> &'static str {
        "FTP/FTPS"
    }

    /// Stejne jako u `SerialModule`/`SshModule::run` - tenhle obecny
    /// (mimo-GUI) beh se aktualne nepouziva, GUI (`termx-gui`) se napojuje
    /// primo na [`spawn_ftp_session`] (viz `ftp_browser.rs`).
    async fn run(&self, _ctx: ConnectionContext<'_>) -> termx_core::Result<()> {
        Err(termx_core::CoreError::Module(
            "samostatný (mimo GUI) režim FTP modulu zatím není implementován".into(),
        ))
    }
}
