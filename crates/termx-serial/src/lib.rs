//! termx-serial
//!
//! Seriovy/COM port modul - druha implementace [`termx_core::ProtocolModule`]
//! (po `termx-ssh`), pridana na zaklade pozadavku "Připojení přes COM/
//! seriový port jako další 'plugin'" - stejny vzor jako SSH modul: novy
//! crate implementujici `ProtocolModule`, zaregistrovany v
//! `ModuleRegistry` (`src/main.rs`), zadne zmeny v existujicich modulech
//! nebyly potreba.
//!
//! Skutecna GUI integrace (`termx-gui/src/terminal.rs`) jde - stejne jako
//! u SSH - MIMO [`ProtocolModule::run`] primo pres [`spawn_serial_session`]
//! (vlastni vlakno + kanaly, viz `session.rs`); `run` zde zustava (stejne
//! jako u puvodniho `SshModule::run`) jen jako "legacy" cesta pro pripadny
//! budouci samostatny TUI rezim bez GUI - v soucasnem GUI-only binary
//! (`src/main.rs`) se nikdy nevola.

mod session;

use termx_core::{ConnectionContext, ProtocolModule};

pub use session::{available_ports, spawn_serial_session, SerialEvent, SerialHandle, SerialInput, DEFAULT_BAUD_RATE};

#[derive(Default)]
pub struct SerialModule;

impl SerialModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl ProtocolModule for SerialModule {
    fn protocol_key(&self) -> &'static str {
        "serial"
    }

    fn display_name(&self) -> &'static str {
        "Serial/COM"
    }

    /// Viz doc-komentar modulu vyse - tato "legacy" TUI cesta neni v
    /// soucasnem GUI-only Term-IX pouzivana (skutecne spojeni resi
    /// primo `termx-gui/src/terminal.rs` pres `spawn_serial_session`).
    /// Na rozdil od `SshModule::run` (ktery si tuto cestu jeste
    /// pamatuje z doby pred GUI pivotem) tu zadna funkcni implementace
    /// nikdy nebyla - rovnou vraci srozumitelnou chybu, stejny vzor,
    /// jaky uz `SshModule::run` pouziva pro zatim nepodporovane
    /// kombinace (`AuthMethod::Agent`/`PrivateKey`).
    async fn run(&self, _ctx: ConnectionContext<'_>) -> termx_core::Result<()> {
        Err(termx_core::CoreError::Module(
            "samostatný (mimo GUI) režim sériového modulu zatím není implementován".into(),
        ))
    }
}
