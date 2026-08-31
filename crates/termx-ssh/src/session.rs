//! Interaktivni SSH relace pro vestaveny terminal v `termx-gui`.
//!
//! Na rozdil od [`crate::SshModule::run`] (puvodni "TUI" cesta, ktera
//! primo prebira stdin/stdout bezicicho procesu a bezi v ramci
//! existujiciho tokio runtime) tady zadny sdileny runtime neni k
//! dispozici - `termx-gui` (eframe) bezi ve vlastni blokujici smycce na
//! hlavnim vlakne bez tokio. Kazde SSH spojeni si proto zaklada
//! VLASTNI male vlakno se svym vlastnim jednovlaknovym tokio runtime
//! (`spawn_ssh_session`) a komunikuje s GUI vlaknem pres kanaly - stejny
//! vzor (`std::thread::spawn` + kanal), jaky uz `termx-gui` pouziva pro
//! kontrolu aktualizaci na pozadi.
//!
//! Vstup (klavesnice z GUI -> SSH kanal) jde pres `tokio::sync::mpsc`
//! (`UnboundedSender::send` je bezna, ne-asynchronni metoda, jde tedy
//! volat primo z GUI vlakna bez tokio kontextu). Vystup (data ze SSH ->
//! GUI k vykresleni) jde naopak pres `std::sync::mpsc`, aby ho GUI mohlo
//! kazdy snimek nekonfliktne "vycerpat" pres `try_recv()` bez zavislosti
//! na tokio.
//!
//! POZNAMKA K OVERENI: presne nazvy metod/typu `russh` 0.44 (`Handle`,
//! `Channel`, `ChannelMsg`) vychazi z puvodniho `SshModule::run` vyse v
//! tomto crate (ten uz uzivatel jednou zkompiloval - viz historie
//! projektu), takze tato cast by mela byt spolehliva. Nove/neoverene je
//! jen samotne zapouzdreni do vlakna+kanalu zde.
use std::sync::Arc;

use termx_core::{AuthMethod, Session};

use crate::handler::TofuHandler;

/// Prichozi udalost ze SSH spojeni smerem ke GUI.
pub enum SshEvent {
    /// Syrova data prijata ze vzdaleneho shellu (stdout/stderr spojene
    /// dohromady, jak je posila PTY na druhe strane) - k parsovani ANSI
    /// escape sekvenci na strane GUI (`alacritty_terminal`).
    Data(Vec<u8>),
    /// Spojeni bylo uspesne navazano a autentizovano - az od teto
    /// chvile ma smysl zacit odesilat vstup.
    Connected,
    /// Spojeni skoncilo chybou (sitova chyba, spatne heslo, ...).
    Error(String),
    /// Spojeni bylo cistě ukonceno (server zavrel kanal, apod.) bez chyby.
    Closed,
}

/// Odchozi prikaz od GUI smerem k bezicimu SSH spojeni.
pub enum SshInput {
    /// Bajty k odeslani do vzdaleneho shellu (napsany text, ovladaci
    /// znaky prelozene z klaves - viz `terminal.rs` v `termx-gui`).
    Data(Vec<u8>),
    /// Zmena velikosti terminaloveho okna (pocet sloupcu/radku) - preda
    /// se na PTY serveru (`window_change`), aby napr. `vim`/`htop`
    /// vedely, jak velkou obrazovku maji k dispozici.
    Resize { cols: u32, rows: u32 },
}

/// Uchyt na bezici SSH spojeni - vstupni odesilac a vystupni prijemce.
/// Zahozenim (`drop`) `input_tx` (napr. kdyz uzivatel zavre Connection
/// tab) prijemce na druhe strane vrati `None` a vlakno se samo a cistě
/// ukonci (zadne rucni "kill" vlakna neni potreba).
pub struct SshHandle {
    pub input_tx: tokio::sync::mpsc::UnboundedSender<SshInput>,
    pub output_rx: std::sync::mpsc::Receiver<SshEvent>,
}

/// Zalozi nove SSH spojeni na samostatnem vlakne a vrati uchyt pro
/// komunikaci s nim. Nikdy nepanikari - jakakoliv chyba (spatne heslo,
/// nedostupny server, nepodporovana metoda prihlaseni) se preda jako
/// [`SshEvent::Error`], aby ji GUI mohlo zobrazit primo v Connection
/// tabu.
pub fn spawn_ssh_session(session: Session, initial_cols: u16, initial_rows: u16) -> SshHandle {
    let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel::<SshInput>();
    let (output_tx, output_rx) = std::sync::mpsc::channel::<SshEvent>();

    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                let _ = output_tx.send(SshEvent::Error(format!("nepodařilo se spustit síťové vlákno: {e}")));
                return;
            }
        };

        runtime.block_on(async move {
            let mut input_rx = input_rx;
            if let Err(e) = run_session(&session, initial_cols, initial_rows, &mut input_rx, &output_tx).await {
                let _ = output_tx.send(SshEvent::Error(e.to_string()));
            }
            let _ = output_tx.send(SshEvent::Closed);
        });
    });

    SshHandle { input_tx, output_rx }
}

async fn run_session(
    session: &Session,
    cols: u16,
    rows: u16,
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<SshInput>,
    output_tx: &std::sync::mpsc::Sender<SshEvent>,
) -> anyhow::Result<()> {
    let (username, password) = match &session.auth {
        AuthMethod::Password { username, password } => (username.clone(), password.clone()),
        AuthMethod::PrivateKey { .. } => {
            anyhow::bail!("přihlášení privátním klíčem zatím není v SSH modulu implementováno")
        }
        AuthMethod::Agent { .. } => {
            anyhow::bail!("přihlášení přes ssh-agent zatím není v SSH modulu implementováno")
        }
        AuthMethod::None => anyhow::bail!("SSH vyžaduje přihlašovací údaje"),
    };

    let config = Arc::new(russh::client::Config::default());
    let addr = (session.host.as_str(), session.port);

    let mut handle = russh::client::connect(config, addr, TofuHandler)
        .await
        .map_err(|e| anyhow::anyhow!("SSH připojení selhalo: {e}"))?;

    let authenticated = handle
        .authenticate_password(&username, &password)
        .await
        .map_err(|e| anyhow::anyhow!("SSH autentizace selhala: {e}"))?;

    if !authenticated {
        anyhow::bail!("SSH autentizace odmítnuta - zkontrolujte uživatelské jméno a heslo");
    }

    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| anyhow::anyhow!("nelze otevřít SSH kanál: {e}"))?;

    channel
        .request_pty(false, "xterm-256color", cols as u32, rows as u32, 0, 0, &[])
        .await
        .map_err(|e| anyhow::anyhow!("požadavek na pty selhal: {e}"))?;
    channel
        .request_shell(true)
        .await
        .map_err(|e| anyhow::anyhow!("požadavek na shell selhal: {e}"))?;

    let _ = output_tx.send(SshEvent::Connected);

    loop {
        tokio::select! {
            input = input_rx.recv() => {
                match input {
                    Some(SshInput::Data(bytes)) => {
                        channel.data(&bytes[..]).await.map_err(|e| anyhow::anyhow!("odeslani dat selhalo: {e}"))?;
                    }
                    Some(SshInput::Resize { cols, rows }) => {
                        channel
                            .window_change(cols, rows, 0, 0)
                            .await
                            .map_err(|e| anyhow::anyhow!("zmena velikosti terminalu selhala: {e}"))?;
                    }
                    // GUI strana zahodila `input_tx` (zavreny tab) - cas
                    // se cistě odpojit, nejde o chybu.
                    None => break,
                }
            }
            msg = channel.wait() => {
                match msg {
                    Some(russh::ChannelMsg::Data { data }) => {
                        // Prijemce (GUI vlakno) uz nemusi poslouchat
                        // (napr. tab byl mezitim zavreny) - poslani se
                        // pak proste nezdari, to neni duvod koncit s
                        // chybou.
                        let _ = output_tx.send(SshEvent::Data(data.to_vec()));
                    }
                    Some(russh::ChannelMsg::Eof) | Some(russh::ChannelMsg::Close) | None => break,
                    _ => {}
                }
            }
        }
    }

    Ok(())
}
