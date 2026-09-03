//! sftp.rs - pozadi bezici SFTP relace pro "Sftp" tab (prohlizec
//! souboru vedle terminaloveho Connection tabu), viz `termx-gui/src/sftp_browser.rs`.
//!
//! Na rozdil od [`crate::session`] (jeden interaktivni shell kanal,
//! streamovany byte-po-bytu do vestaveneho terminalu) je SFTP
//! pozadavek/odpoved - kazda operace (vypsat adresar, stahnout/nahrat
//! soubor...) ma jasny zacatek a konec. Proto misto souvisleho proudu
//! [`crate::SshEvent`] pouziva jednodussi model "prikaz -> presne jedna
//! odpoved": GUI posle [`SftpCommand`] pres `SftpHandle::cmd_tx`, vlakno
//! na pozadi (vlastni tokio runtime, stejny vzor jako `spawn_ssh_session`)
//! ho provede a posle zpet odpovidajici [`SftpEvent`].
//!
//! Vlastni, NEZAVISLE SSH spojeni (samostatny TCP+auth, ne sdileny kanal
//! s uz bezicim terminalem) - stejny pristup, jaky uz `session.rs`
//! pouziva pro `fetch_stats_output` (samostatny kanal na temze spojeni)
//! by tu znamenal zasahovat do jiz overene a citlive hlavni smycky
//! interaktivniho shellu (`run_session`); samostatne spojeni je naproti
//! tomu plne oddelene a nese jen mensi cenu (jedno navic TCP
//! spojeni+prihlaseni na server) - stejny kompromis, jaky bezne delaji i
//! jina GUI (FileZilla/WinSCP/MobaXterm) mezi terminalem a SFTP panelem.
//!
//! POZNAMKA K OVERENI: cely tento soubor (vc. `russh-sftp` 2.4 API -
//! `SftpSession::new`, `read_dir`/`DirEntry`/`Metadata`, `read`/`write`)
//! byl napsan bez moznosti `cargo build` v tomto prostredi, jen podle
//! dokumentace (docs.rs). Napojeni `russh::Channel` -> `SftpSession`
//! (`channel.request_subsystem(true, "sftp")` + `channel.into_stream()`)
//! vychazi z oficialnich prikladu knihovny. Prvni sestaveni na
//! uzivatelove pocitaci muze odhalit drobne nesrovnalosti v nazvech
//! metod/typu - architektura okolo (vlakno+kanaly, `SftpCommand`/`SftpEvent`)
//! tim dotcena neni.

use std::path::PathBuf;
use std::sync::Arc;

use russh_sftp::client::SftpSession;
use termx_core::{AuthMethod, Session};

use crate::handler::TofuHandler;

/// Jedna polozka vypsaneho adresare (soubor nebo podslozka) - zjednodusene
/// (jen to, co prohlizec v `termx-gui` skutecne potrebuje) oproti plnym
/// `russh_sftp::protocol::FileAttributes`.
#[derive(Debug, Clone)]
pub struct SftpEntry {
    pub name: String,
    pub is_dir: bool,
    /// U slozek vzdy `0` (SFTP protokol velikost slozky smysluplne
    /// neudava).
    pub size: u64,
}

/// Prikaz od GUI (`termx-gui::sftp_browser::SftpBrowser`) smerem k
/// bezici SFTP relaci.
pub enum SftpCommand {
    /// Vypsat obsah slozky na dane ABSOLUTNI ceste (pouziva se i pro
    /// pocatecni/domovskou slozku a pro navigaci ".." - `SftpBrowser` si
    /// cestu sam sklada, tady uz prichazi hotova).
    List(String),
    /// Stahnout vzdaleny soubor `remote` (absolutni cesta) do lokalniho
    /// souboru `local` (cesta vybrana pres nativni "Ulozit jako..."
    /// dialog v GUI).
    Download { remote: String, local: PathBuf },
    /// Nahrat lokalni soubor `local` na vzdalenou cestu `remote`
    /// (aktualni slozka + jmeno lokalniho souboru, sestaveno v GUI).
    Upload { local: PathBuf, remote: String },
    /// Dodatecne doplnene prihlasovaci udaje po [`SftpEvent::AwaitingCredentials`] -
    /// stejny vzor jako `SshInput::Credentials` v `session.rs`.
    Credentials { username: String, password: String },
}

/// Udalost ze SFTP relace smerem ke GUI.
pub enum SftpEvent {
    /// Transport navazan, ale `Session::auth` nema vyplneneho uzivatele -
    /// `SftpBrowser` v tomto stavu zobrazi jednoduchy formular
    /// (uzivatel/heslo), stejne jako `ConnState::AwaitingCredentials` u
    /// terminalu (jen ne primo v terminalovem bufferu, ale jako bezne
    /// textove pole - SFTP prohlizec zadny "terminal" nema).
    AwaitingCredentials,
    /// Predchozi pokus o prihlaseni byl odmitnut - relace pokracuje,
    /// GUI muze nabidnout dalsi pokus (stejne jako `SshEvent::AuthFailed`).
    AuthFailed(String),
    /// Uspesne pripojeno a autentizovano - `home` je pocatecni
    /// (domovska) slozka vzdaleneho uctu, ktere se `SftpBrowser` prvne
    /// zeptal pres `canonicalize(".")`.
    Connected { home: String },
    /// Chyba jednotlive operace (vypsani/stazeni/nahrani/...) - NEUKONCUJE
    /// relaci, jen se zobrazi jako chybova hlaska (`SftpBrowser::status`).
    Error(String),
    /// Odpoved na [`SftpCommand::List`].
    Listing { path: String, entries: Vec<SftpEntry> },
    Downloaded { remote: String, local: PathBuf },
    Uploaded { local: PathBuf, remote: String },
    /// Spojeni skoncilo (chybou i cistě) - `SftpBrowser` uz na tomto
    /// handle dal nic neposila.
    Closed,
}

/// Uchyt na bezici SFTP relaci - vstupni odesilac a vystupni prijemce,
/// stejny vzor jako `SshHandle`.
pub struct SftpHandle {
    pub cmd_tx: tokio::sync::mpsc::UnboundedSender<SftpCommand>,
    pub event_rx: std::sync::mpsc::Receiver<SftpEvent>,
}

/// Zalozi novou SFTP relaci na samostatnem vlakne (vlastni jednovlaknovy
/// tokio runtime, stejne jako `spawn_ssh_session`) a vrati uchyt pro
/// komunikaci s ni. Nikdy nepanikari - kazda chyba se preda jako
/// [`SftpEvent::Error`]/`Closed`.
pub fn spawn_sftp_session(session: Session) -> SftpHandle {
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<SftpCommand>();
    let (event_tx, event_rx) = std::sync::mpsc::channel::<SftpEvent>();

    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                let _ = event_tx.send(SftpEvent::Error(format!("nepodařilo se spustit síťové vlákno: {e}")));
                let _ = event_tx.send(SftpEvent::Closed);
                return;
            }
        };

        runtime.block_on(async move {
            let mut cmd_rx = cmd_rx;
            if let Err(e) = run_sftp_session(&session, &mut cmd_rx, &event_tx).await {
                let _ = event_tx.send(SftpEvent::Error(e.to_string()));
            }
            let _ = event_tx.send(SftpEvent::Closed);
        });
    });

    SftpHandle { cmd_tx, event_rx }
}

async fn run_sftp_session(
    session: &Session,
    cmd_rx: &mut tokio::sync::mpsc::UnboundedReceiver<SftpCommand>,
    event_tx: &std::sync::mpsc::Sender<SftpEvent>,
) -> anyhow::Result<()> {
    let early_auth = match &session.auth {
        AuthMethod::Password { username, password } if !username.trim().is_empty() => {
            Some((username.clone(), password.clone()))
        }
        AuthMethod::PrivateKey { .. } => {
            anyhow::bail!("přihlášení privátním klíčem zatím není v SFTP prohlížeči implementováno")
        }
        AuthMethod::Agent { .. } => {
            anyhow::bail!("přihlášení přes ssh-agent zatím není v SFTP prohlížeči implementováno")
        }
        AuthMethod::Password { .. } | AuthMethod::None => None,
    };

    let config = Arc::new(russh::client::Config::default());
    let addr = (session.host.as_str(), session.port);

    let mut handle = russh::client::connect(config, addr, TofuHandler)
        .await
        .map_err(|e| anyhow::anyhow!("SSH připojení selhalo: {e}"))?;

    // Stejna smycka opakovanych pokusu jako `session.rs::run_session` -
    // viz tam pro podrobne zduvodneni (zpetna vazba "když nemáme login
    // úspěšný tak už se k zadání nedostaneme").
    let mut creds = early_auth;
    if creds.is_none() {
        let _ = event_tx.send(SftpEvent::AwaitingCredentials);
    }
    loop {
        let (username, password) = match creds.take() {
            Some(c) => c,
            None => loop {
                match cmd_rx.recv().await {
                    Some(SftpCommand::Credentials { username, password }) => break (username, password),
                    Some(_) => continue,
                    None => return Ok(()),
                }
            },
        };

        let authenticated = handle
            .authenticate_password(&username, &password)
            .await
            .map_err(|e| anyhow::anyhow!("SSH autentizace selhala: {e}"))?;

        if authenticated {
            break;
        }

        let _ = event_tx.send(SftpEvent::AuthFailed(
            "SSH autentizace odmítnuta - zkontrolujte uživatelské jméno a heslo".to_string(),
        ));
    }

    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| anyhow::anyhow!("nelze otevřít SSH kanál: {e}"))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| anyhow::anyhow!("server nepodporuje SFTP subsystém: {e}"))?;

    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| anyhow::anyhow!("inicializace SFTP relace selhala: {e}"))?;

    let home = sftp.canonicalize(".").await.unwrap_or_else(|_| "/".to_string());
    let _ = event_tx.send(SftpEvent::Connected { home });

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            SftpCommand::List(path) => match sftp.read_dir(path.as_str()).await {
                Ok(read_dir) => {
                    let mut entries: Vec<SftpEntry> = read_dir
                        .map(|entry| {
                            let meta = entry.metadata();
                            SftpEntry {
                                name: entry.file_name(),
                                is_dir: meta.is_dir(),
                                size: meta.size.unwrap_or(0),
                            }
                        })
                        .collect();
                    // Slozky prvni, pak abecedne (bez ohledu na
                    // velikost pismen) - stejne razeni, jake uzivatel
                    // cekaji z bezneho souboroveho manazeru.
                    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
                    let _ = event_tx.send(SftpEvent::Listing { path, entries });
                }
                Err(e) => {
                    let _ = event_tx.send(SftpEvent::Error(format!("nelze vypsat „{path}“: {e}")));
                }
            },
            SftpCommand::Download { remote, local } => match sftp.read(remote.as_str()).await {
                Ok(data) => match std::fs::write(&local, &data) {
                    Ok(()) => {
                        let _ = event_tx.send(SftpEvent::Downloaded { remote, local });
                    }
                    Err(e) => {
                        let _ = event_tx.send(SftpEvent::Error(format!("zápis „{}“ selhal: {e}", local.display())));
                    }
                },
                Err(e) => {
                    let _ = event_tx.send(SftpEvent::Error(format!("stažení „{remote}“ selhalo: {e}")));
                }
            },
            SftpCommand::Upload { local, remote } => match std::fs::read(&local) {
                Ok(data) => match sftp.write(remote.as_str(), &data).await {
                    Ok(()) => {
                        let _ = event_tx.send(SftpEvent::Uploaded { local, remote });
                    }
                    Err(e) => {
                        let _ = event_tx.send(SftpEvent::Error(format!("nahrání „{remote}“ selhalo: {e}")));
                    }
                },
                Err(e) => {
                    let _ = event_tx.send(SftpEvent::Error(format!("čtení „{}“ selhalo: {e}", local.display())));
                }
            },
            // Uz autentizovano - dalsi zprava tohoto typu (nemela by
            // nastat) se proste ignoruje, stejne jako v `session.rs`.
            SftpCommand::Credentials { .. } => {}
        }
    }

    Ok(())
}
