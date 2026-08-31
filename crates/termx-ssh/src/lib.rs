//! termx-ssh
//!
//! SSH modul - prvni implementace [`termx_core::ProtocolModule`].
//! Zamerne pouziva `russh` (cisty Rust) mesto `ssh2`/libssh2, aby build
//! na Windows i Linuxu nezavisel na pritomnosti OpenSSL/C tooolchainu.
//!
//! POZNAMKA K OVERENI: presna verze API `russh` (nazvy metod na
//! `Handle`/`Channel`, typy zprav) se mezi vydanimi drobne meni. Tento
//! soubor byl psan rucne bez pristupu na crates.io v tomto prostredi
//! (nebylo mozne spustit `cargo check`), takze po prvnim `cargo build`
//! na Vasem pocitaci muze byt potreba doladit par nazvu metod/typu podle
//! verze `russh`, kterou si Cargo stahne. Architektura okolo (trait
//! ProtocolModule, napojeni na TUI) tim dotcena neni.
//!
//! MVP rozsah: autentizace heslem. Prihlaseni privatnim klicem / pres
//! ssh-agent je pripraveno v datovem modelu (`AuthMethod`), ale modul
//! zatim vraci chybu "not implemented" - dalsi krok vyvoje.
//!
//! Overeni identity serveru (known_hosts) NENI v MVP implementovano -
//! modul zatim prijme jakykoliv klic serveru (viz `handler.rs`). Pred
//! pouzitim na produkcnich/verejnych serverech je potreba doplnit.

mod handler;
mod session;

use std::sync::Arc;

use async_trait::async_trait;
use termx_core::{AuthMethod, ConnectionContext, CoreError, ProtocolModule};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use handler::TofuHandler;

pub use session::{spawn_ssh_session, SshEvent, SshHandle, SshInput, SystemStats};

#[derive(Default)]
pub struct SshModule;

impl SshModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ProtocolModule for SshModule {
    fn protocol_key(&self) -> &'static str {
        "ssh"
    }

    fn display_name(&self) -> &'static str {
        "SSH"
    }

    async fn run(&self, ctx: ConnectionContext<'_>) -> termx_core::Result<()> {
        let session = ctx.session;

        let (username, password) = match &session.auth {
            AuthMethod::Password { username, password } => (username.clone(), password.clone()),
            AuthMethod::PrivateKey { .. } => {
                return Err(CoreError::Module(
                    "prihlaseni privatnim klicem zatim neni v SSH modulu implementovano (MVP)".into(),
                ))
            }
            AuthMethod::Agent { .. } => {
                return Err(CoreError::Module(
                    "prihlaseni pres ssh-agent zatim neni v SSH modulu implementovano (MVP)".into(),
                ))
            }
            AuthMethod::None => {
                return Err(CoreError::Module("SSH vyzaduje prihlasovaci udaje".into()))
            }
        };

        let config = Arc::new(russh::client::Config::default());
        let addr = (session.host.as_str(), session.port);

        let mut handle = russh::client::connect(config, addr, TofuHandler)
            .await
            .map_err(|e| CoreError::Module(format!("SSH pripojeni selhalo: {e}")))?;

        let authenticated = handle
            .authenticate_password(&username, &password)
            .await
            .map_err(|e| CoreError::Module(format!("SSH autentizace selhala: {e}")))?;

        if !authenticated {
            return Err(CoreError::Module(
                "SSH autentizace odmitnuta - zkontrolujte uzivatelske jmeno a heslo".into(),
            ));
        }

        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|e| CoreError::Module(format!("nelze otevrit SSH kanal: {e}")))?;

        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        channel
            .request_pty(false, "xterm-256color", cols as u32, rows as u32, 0, 0, &[])
            .await
            .map_err(|e| CoreError::Module(format!("pozadavek na pty selhal: {e}")))?;
        channel
            .request_shell(true)
            .await
            .map_err(|e| CoreError::Module(format!("pozadavek na shell selhal: {e}")))?;

        println!("-- Term-IX: pripojeno k {}@{}:{} (Ctrl+D pro odpojeni) --", username, session.host, session.port);

        crossterm::terminal::enable_raw_mode().ok();
        let result = bridge_io(&mut channel).await;
        crossterm::terminal::disable_raw_mode().ok();

        println!("\r\n-- Term-IX: odpojeno --");

        result.map_err(|e| CoreError::Module(format!("SSH relace skoncila chybou: {e}")))
    }
}

/// Prepojuje standardni vstup/vystup terminalu s SSH kanalem, dokud
/// jedna ze stran spojeni neukonci (Ctrl+D na strane klienta, nebo
/// EOF/Close od serveru).
async fn bridge_io(channel: &mut russh::Channel<russh::client::Msg>) -> anyhow::Result<()> {
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut buf = [0u8; 4096];

    loop {
        tokio::select! {
            n = stdin.read(&mut buf) => {
                let n = n?;
                if n == 0 {
                    break;
                }
                channel.data(&buf[..n]).await?;
            }
            msg = channel.wait() => {
                match msg {
                    Some(russh::ChannelMsg::Data { data }) => {
                        stdout.write_all(&data).await?;
                        stdout.flush().await?;
                    }
                    Some(russh::ChannelMsg::Eof) | Some(russh::ChannelMsg::Close) | None => break,
                    _ => {}
                }
            }
        }
    }

    Ok(())
}
