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
mod sftp;

use std::sync::Arc;

use async_trait::async_trait;
use termx_core::{AuthMethod, ConnectionContext, CoreError, ProtocolModule};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use handler::TofuHandler;

pub use session::{spawn_ssh_session, SshEvent, SshHandle, SshInput, SystemStats};
pub use sftp::{spawn_sftp_session, SftpCommand, SftpEntry, SftpEvent, SftpHandle};

/// Sestavi `russh::client::Config` se stejnymi vychozimi hodnotami jako
/// `Config::default()`, jen s rozsirenym seznamem preferovanych
/// key-exchange algoritmu - pouziva se pro KAZDE SSH spojeni (viz
/// `SshModule::run` nize i `session::run_session`).
///
/// Duvod: nektera starsi/embedded zarizeni (typicky prave Avaya
/// Communication Manager SAT rozhrani, ale i starsi sitovy hardware v
/// domacim labu) nabizeji jen stare KEX algoritmy (`diffie-hellman-
/// group14-sha1`, pripadne jeste starsi `group1-sha1`), ktere `russh`
/// ve vychozim nastaveni NEMA mezi preferovanymi (z bezpecnostnich
/// duvodu) - bez tohoto rozsireni spojeni skonci chybou "No common key
/// exchange algorithm", i kdyz je server jinak dostupny. Oba algoritmy
/// se pridavaji AZ NA KONEC seznamu (nejnizsi priorita) - modernejsi
/// servery se tim nijak neovlivni, pouziji se jen kdyz server nic
/// modernejsiho spolecne s klientem nema.
pub(crate) fn legacy_friendly_config() -> russh::client::Config {
    let mut config = russh::client::Config::default();
    let mut kex = config.preferred.kex.to_vec();
    kex.push(russh::kex::DH_G14_SHA1);
    kex.push(russh::kex::DH_G1_SHA1);
    config.preferred.kex = kex.into();

    // Stejny duvod jako u KEX vyse, jen pro algoritmus HOST KLICE
    // serveru ("No common key algorithm") - `russh` ve vychozim
    // nastaveni prijima jen `ssh-ed25519`/`ecdsa-sha2-*`/`rsa-sha2-*`,
    // ale starsi zarizeni (Avaya CM SAT apod.) casto nabizeji jen
    // puvodni `ssh-rsa` (RSA podpis se SHA-1). `russh_keys::key::Name`
    // je stejny typ, ktery `russh::client::Config::preferred.key`
    // pouziva (`russh` interne re-exportuje `russh_keys` jako `keys`).
    let mut key = config.preferred.key.to_vec();
    key.push(russh_keys::key::SSH_RSA);
    config.preferred.key = key.into();

    config
}

/// Prihlasi se na jiz navazany SSH transport - nejdriv zkusi standardni
/// "password" auth method, a pokud ji server odmitne, automaticky to
/// zkusi jeste jednou pres "keyboard-interactive" (na kazdou vyzvu
/// odpovi stejnym heslem). Pouziva se pro KAZDY pokus o prihlaseni (viz
/// `SshModule::run` nize i `session::run_session`).
///
/// Duvod: nektera zarizeni (typicky prave Avaya CM SAT, ale i ruzne
/// PAM-based Linux/Unix krabice) "password" auth method vubec
/// nenabizeji/neakceptuji - jen "keyboard-interactive". Z pohledu
/// uzivatele jde o naprosto stejnou vyzvu na heslo (PuTTY/OpenSSH tohle
/// rozliseni resi transparentne sami), ale `russh` tyhle dve auth
/// methods rozlisuje - proto se predtim (kdy se zkousela jen
/// "password") u takovych zarizeni pripojeni porad vracelo "Permission
/// denied", i kdyz uzivatelske jmeno/heslo bylo spravne.
pub(crate) async fn authenticate_password_or_keyboard_interactive(
    handle: &mut russh::client::Handle<TofuHandler>,
    username: &str,
    password: &str,
) -> anyhow::Result<bool> {
    // POZOR NA PORADI: zkousi se NEJDRIV "keyboard-interactive", teprve
    // pak (fallback) klasicka "password" metoda - obracene poradi bylo
    // vyzkouseno jako prvni a s Avaya CM SAT (a pravdepodobne dalsimi
    // PAM-based zarizenimi) zpusobovalo, ze se cele prihlaseni po
    // predchozim odmitnutem pokusu o "password" jen tise zaseklo (zadna
    // odpoved od serveru, zadna chyba - viz zpetna vazba "po zadani
    // hesla to zustane viset"). Presne v tomhle poradi (rovnou
    // "keyboard-interactive") postupuje i PuTTY (viz jeho log), takze
    // se ted chovame stejne - "password" je jen fallback pro servery,
    // ktere "keyboard-interactive" vubec nenabizi.
    let mut response = tokio::time::timeout(
        AUTH_STEP_TIMEOUT,
        handle.authenticate_keyboard_interactive_start(username, None),
    )
    .await
    .map_err(|_| anyhow::anyhow!("timeout pri zahajeni keyboard-interactive autentizace"))??;

    loop {
        match response {
            russh::client::KeyboardInteractiveAuthResponse::Success => return Ok(true),
            // Server "keyboard-interactive" vubec nenabizi (nebo ho
            // rovnou odmitl bez jedine vyzvy) - zkusi se jeste klasicke
            // "password".
            russh::client::KeyboardInteractiveAuthResponse::Failure => break,
            russh::client::KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                // Nejcastejsi pripad je jedina vyzva "Password:" - na
                // vsechny vyzvy se proto (v souladu s tim, jak by na ne
                // odpovedel bezny uzivatel se stejnym heslem pro vsechno)
                // odpovi stejnym heslem, ktere zadal do formulare.
                let answers = vec![password.to_string(); prompts.len()];
                response = tokio::time::timeout(
                    AUTH_STEP_TIMEOUT,
                    handle.authenticate_keyboard_interactive_respond(answers),
                )
                .await
                .map_err(|_| anyhow::anyhow!("timeout pri odpovedi na keyboard-interactive vyzvu"))??;
            }
        }
    }

    let authenticated = tokio::time::timeout(AUTH_STEP_TIMEOUT, handle.authenticate_password(username, password))
        .await
        .map_err(|_| anyhow::anyhow!("timeout pri password autentizaci"))??;
    Ok(authenticated)
}

/// Kolik nejdele se ceka na JEDEN krok autentizace (start/odpoved
/// keyboard-interactive, nebo password) - viz
/// `authenticate_password_or_keyboard_interactive`. Bez tohoto by se
/// pri zarizeni, ktere na nejaky auth pozadavek proste vubec
/// neodpovi, cele prihlaseni ticho tise zaseklo na neurcito misto
/// srozumitelne chyby, kterou lze zobrazit a zkusit znovu.
const AUTH_STEP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

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

        let config = Arc::new(legacy_friendly_config());
        let addr = (session.host.as_str(), session.port);

        let mut handle = russh::client::connect(config, addr, TofuHandler)
            .await
            .map_err(|e| CoreError::Module(format!("SSH pripojeni selhalo: {e}")))?;

        let authenticated = authenticate_password_or_keyboard_interactive(&mut handle, &username, &password)
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
        // Viz `Session::term_type` - typicky nastaveno napr. na "513"
        // pro Avaya CM SAT, kde vychozi `xterm-256color` nefunguje
        // spravne (ocekavane mapovani funkcnich klaves F1-F8/Cancel).
        let term_type = session.term_type.as_deref().unwrap_or("xterm-256color");
        channel
            .request_pty(false, term_type, cols as u32, rows as u32, 0, 0, &[])
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
