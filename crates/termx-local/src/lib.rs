//! termx-local
//!
//! Mistni (lokalni) terminal - shell spusteny na TOMTO pocitaci v
//! pseudoterminalu (PTY), zobrazeny ve stejnem vestavenem terminalu
//! (`termx-gui/src/terminal.rs`, `alacritty_terminal`) jako SSH/seriova
//! linka. Zadna sit, zadne prihlasovani. Na Linuxu/macOS klasicky PTY
//! (`openpty`), na Windows ConPTY - obojí pres `portable-pty` (WezTerm).
//!
//! Stejny vzor jako `termx-serial`: vlastni vlakna + `std::sync::mpsc`
//! kanaly (`LocalInput` dovnitr, `LocalEvent` ven), GUI se napojuje primo
//! na [`spawn_local_session`]; [`ProtocolModule::run`] je jen "legacy" TUI
//! cesta, v GUI se nevola.
//!
//! `Session::host` = program shellu (napr. `/bin/zsh`, `pwsh.exe`);
//! prazdne = vychozi shell uzivatele (`$SHELL` na Unixu, `%COMSPEC%` na
//! Windows) - viz `session.rs`.

mod session;

use termx_core::{ConnectionContext, ProtocolModule};

pub use session::{default_shell_label, spawn_local_session, LocalEvent, LocalHandle, LocalInput};

#[derive(Default)]
pub struct LocalModule;

impl LocalModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl ProtocolModule for LocalModule {
    fn protocol_key(&self) -> &'static str {
        "local"
    }

    fn display_name(&self) -> &'static str {
        "Místní terminál"
    }

    async fn run(&self, _ctx: ConnectionContext<'_>) -> termx_core::Result<()> {
        Err(termx_core::CoreError::Module(
            "samostatný (mimo GUI) režim místního terminálu zatím není implementován".into(),
        ))
    }
}
