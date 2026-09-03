//! sftp_browser.rs - obsah "Sftp" tabu (viz `app.rs::TabKind::Sftp`):
//! jednoduchy prohlizec vzdalenych souboru napojeny na
//! `termx_ssh::spawn_sftp_session` - navigace slozkami, stazeni souboru
//! (nativni "Uložit jako..." dialog), nahrani souboru (nativni
//! "Otevřít..." dialog) do aktualni slozky. Zamerne BEZ prejmenovani/
//! mazani/vytvareni slozek v teto prvni verzi (uzivatelsky pozadavek byl
//! "procházet soubory a stahovat a nahrávat na něj") - da se doplnit
//! pozdeji, az zakladni pripojeni overi skutecny build.
//!
//! Stejny vzor jako `terminal::TerminalSession`: spojeni se zaklada
//! rovnou v [`SftpBrowser::new`] (vlastni vlakno, viz
//! `termx_ssh::spawn_sftp_session`), GUI kazdy snimek "vycerpa" prichozi
//! udalosti pres [`SftpBrowser::pump`] (`try_recv`, stejny idiom jako
//! `TerminalSession::pump`).

use std::path::PathBuf;

use termx_core::Session;
use termx_ssh::{spawn_sftp_session, SftpCommand, SftpEntry, SftpEvent, SftpHandle};

use crate::i18n::{self, Lang};

/// Stav SFTP relace tohoto tabu - viz `terminal::ConnState` (obdobny
/// ucel), zjednoduseny o "jeste nikdy nepripojeno" rozliseni, ktere tu
/// na rozdil od terminalu neni potreba (zadny automaticky reconnect v
/// teto prvni verzi).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SftpState {
    Connecting,
    AwaitingCredentials,
    Connected,
    Disconnected,
}

/// Posledni vysledek operace (stazeni/nahrani/chyba) - drzi se jako
/// SUROVA data (ne uz hotovy text), aby se preklad do aktualniho jazyka
/// (`i18n::t`) delal az pri vykreslovani (`render`), ne uvnitr `pump`,
/// kde zadny `Lang` k dispozici neni.
enum StatusMsg {
    Error(String),
    Downloaded { remote: String, local: PathBuf },
    Uploaded { local: PathBuf, remote: String },
}

pub struct SftpBrowser {
    handle: SftpHandle,
    state: SftpState,
    current_path: String,
    entries: Vec<SftpEntry>,
    status: Option<StatusMsg>,
    cred_username: String,
    cred_password: String,
}

impl SftpBrowser {
    pub fn new(session: &Session) -> Self {
        Self {
            handle: spawn_sftp_session(session.clone()),
            state: SftpState::Connecting,
            current_path: String::new(),
            entries: Vec::new(),
            status: None,
            cred_username: session_username(session),
            cred_password: String::new(),
        }
    }

    pub fn state(&self) -> SftpState {
        self.state
    }

    fn pump(&mut self) {
        loop {
            match self.handle.event_rx.try_recv() {
                Ok(SftpEvent::AwaitingCredentials) => {
                    self.state = SftpState::AwaitingCredentials;
                }
                Ok(SftpEvent::AuthFailed(msg)) => {
                    self.state = SftpState::AwaitingCredentials;
                    self.status = Some(StatusMsg::Error(msg));
                }
                Ok(SftpEvent::Connected { home }) => {
                    self.state = SftpState::Connected;
                    self.status = None;
                    self.request_list(home);
                }
                Ok(SftpEvent::Error(msg)) => {
                    self.status = Some(StatusMsg::Error(msg));
                }
                Ok(SftpEvent::Listing { path, entries }) => {
                    self.current_path = path;
                    self.entries = entries;
                }
                Ok(SftpEvent::Downloaded { remote, local }) => {
                    self.status = Some(StatusMsg::Downloaded { remote, local });
                }
                Ok(SftpEvent::Uploaded { local, remote }) => {
                    self.status = Some(StatusMsg::Uploaded { local, remote });
                    // Po nahrani rovnou obnovit vypis aktualni slozky,
                    // at je novy soubor hned videt v seznamu.
                    self.request_list(self.current_path.clone());
                }
                Ok(SftpEvent::Closed) => {
                    if self.state != SftpState::AwaitingCredentials {
                        self.state = SftpState::Disconnected;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }
    }

    fn request_list(&mut self, path: String) {
        let _ = self.handle.cmd_tx.send(SftpCommand::List(path));
    }

    /// Prelozi `self.status` (surova data, viz [`StatusMsg`]) do textu v
    /// aktualnim jazyce - volano az tady, ne uvnitr `pump`.
    fn status_text(&self, tr: &'static i18n::Strings) -> Option<String> {
        match self.status.as_ref()? {
            StatusMsg::Error(e) => Some(e.clone()),
            StatusMsg::Downloaded { remote, local } => {
                Some(format!("{}: {} → {}", tr.sftp_status_downloaded, remote, local.display()))
            }
            StatusMsg::Uploaded { local, remote } => {
                Some(format!("{}: {} → {}", tr.sftp_status_uploaded, local.display(), remote))
            }
        }
    }

    pub fn render(&mut self, ui: &mut egui::Ui, lang: Lang) {
        self.pump();
        let tr = i18n::t(lang);
        let status_text = self.status_text(tr);

        match self.state {
            SftpState::Connecting => {
                ui.label(tr.sftp_connecting);
                return;
            }
            SftpState::Disconnected => {
                ui.label(tr.sftp_disconnected);
                if let Some(status) = &status_text {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(status).small());
                }
                return;
            }
            SftpState::AwaitingCredentials => {
                ui.heading(tr.sftp_login_heading);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(tr.field_username);
                    ui.text_edit_singleline(&mut self.cred_username);
                });
                ui.horizontal(|ui| {
                    ui.label(tr.field_password);
                    ui.add(egui::TextEdit::singleline(&mut self.cred_password).password(true));
                });
                ui.add_space(4.0);
                if let Some(status) = &status_text {
                    ui.colored_label(egui::Color32::from_rgb(0xe0, 0x6c, 0x6c), status);
                    ui.add_space(4.0);
                }
                if ui.button(tr.btn_connect).clicked() {
                    let _ = self.handle.cmd_tx.send(SftpCommand::Credentials {
                        username: self.cred_username.clone(),
                        password: self.cred_password.clone(),
                    });
                    self.status = None;
                }
                return;
            }
            SftpState::Connected => {}
        }

        let mut navigate_to: Option<String> = None;
        let mut download_name: Option<String> = None;

        ui.horizontal(|ui| {
            let parent = parent_path(&self.current_path);
            if ui.add_enabled(parent.is_some(), egui::Button::new(tr.btn_sftp_up)).clicked() {
                navigate_to = parent;
            }
            if ui.button(tr.btn_refresh).clicked() {
                navigate_to = Some(self.current_path.clone());
            }
            if ui.button(tr.btn_sftp_upload).clicked() {
                if let Some(local) = rfd::FileDialog::new().pick_file() {
                    let file_name = local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if !file_name.is_empty() {
                        let remote = join_remote(&self.current_path, &file_name);
                        let _ = self.handle.cmd_tx.send(SftpCommand::Upload { local, remote });
                    }
                }
            }
        });
        ui.add_space(4.0);
        ui.label(egui::RichText::new(&self.current_path).monospace());
        if let Some(status) = &status_text {
            ui.add_space(2.0);
            ui.label(egui::RichText::new(status).small());
        }
        ui.add_space(6.0);
        ui.separator();

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for entry in &self.entries {
                ui.horizontal(|ui| {
                    if entry.is_dir {
                        // Koncova "/" misto ikonky slozky - font pouzity
                        // v teto appce nema kompletni pokryti emoji/
                        // Dingbats bloku (viz podobny problem uz drive
                        // vyreseny jinde v `app.rs` u ikon tabu), takze
                        // se zamerne drzime obycejneho textu.
                        let label = format!("{}/", entry.name);
                        if ui.selectable_label(false, label).double_clicked() {
                            navigate_to = Some(join_remote(&self.current_path, &entry.name));
                        }
                    } else {
                        ui.label(&entry.name);
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(format_size(entry.size)).weak());
                        if ui.small_button(tr.btn_sftp_download).clicked() {
                            download_name = Some(entry.name.clone());
                        }
                    }
                });
            }
            if self.entries.is_empty() {
                ui.label(egui::RichText::new(tr.sftp_empty_folder).weak());
            }
        });

        if let Some(path) = navigate_to {
            self.request_list(path);
        }
        if let Some(name) = download_name {
            if let Some(local) = rfd::FileDialog::new().set_file_name(&name).save_file() {
                let remote = join_remote(&self.current_path, &name);
                let _ = self.handle.cmd_tx.send(SftpCommand::Download { remote, local });
            }
        }
    }
}

/// Pocatecni hodnota pole "Uživatelské jméno" v prihlasovacim formulari -
/// pokud uz `Session` uzivatele obsahuje (bezny pripad ulozeneho
/// serveru), preda se rovnou, at ho uzivatel nemusi psat znovu; jinak
/// prazdne (ad-hoc/rychle spojeni bez ulozenych udaju).
fn session_username(session: &Session) -> String {
    match &session.auth {
        termx_core::AuthMethod::Password { username, .. } if !username.trim().is_empty() => username.clone(),
        _ => String::new(),
    }
}

/// Rodicovska slozka dane ABSOLUTNI cesty (SFTP protokol vzdy pouziva
/// '/' jako oddelovac, bez ohledu na OS cileho serveru) - `None` uz pro
/// koren (`/`), kde "Nahoru" nema kam vest.
fn parent_path(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    match trimmed.rfind('/') {
        Some(0) => Some("/".to_string()),
        Some(idx) => Some(trimmed[..idx].to_string()),
        None => Some("/".to_string()),
    }
}

fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GB {
        format!("{:.1} GB", bytes / GB)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes / MB)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes / KB)
    } else {
        format!("{} B", bytes as u64)
    }
}
