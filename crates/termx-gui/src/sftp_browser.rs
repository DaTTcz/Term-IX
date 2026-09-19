//! sftp_browser.rs - obsah "Sftp" tabu (viz `app.rs::TabKind::Sftp`):
//! jednoduchy prohlizec vzdalenych souboru napojeny na
//! `termx_ssh::spawn_sftp_session` - navigace slozkami, stazeni/nahrani
//! jednotlivych souboru i CELYCH slozek rekurzivne (nativni "Uložit
//! jako.../Otevřít.../Vybrat složku..." dialogy). Prubeh hromadneho
//! prenosu slozky se ukazuje jen jako "hotovo/celkem" (bez bajtoveho
//! progressu jednotlivych souboru) a existujici soubory se pri kolizi
//! jmen VZDY prepisuji - obojí vedomé zjednodušení pro první verzi.
//! Umi i prejmenovani, mazani (jen souboru/PRAZDNYCH slozek - SFTP samo
//! neumi rekurzivni smazani neprazdne slozky) a vytvoreni nove prazdne
//! slozky.
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
    DirDownloaded { remote: String, local: PathBuf, count: usize },
    DirUploaded { local: PathBuf, remote: String, count: usize },
    Renamed { from: String, to: String },
    Deleted { path: String },
    Created { path: String },
}

pub struct SftpBrowser {
    handle: SftpHandle,
    state: SftpState,
    current_path: String,
    entries: Vec<SftpEntry>,
    status: Option<StatusMsg>,
    /// Prubeh prave beziciho hromadneho prenosu slozky (done, total) -
    /// viz `SftpEvent::DirProgress`. `None`, kdyz zadny neprobiha.
    progress: Option<(usize, usize)>,
    /// `true` mezi odeslanim `SftpCommand::List` a prijetim odpovedi
    /// (`SftpEvent::Listing`/`Error`) - zobrazuje se jako "Načítám…" u
    /// cesty, at je jasne, ze se na klik/navigaci neco deje (predtim
    /// pri pomalejsim spojeni chvili vypadalo, ze se nestalo nic).
    loading: bool,
    /// "Nová složka..." dialog - `Some(rozepsany_nazev)` kdyz je
    /// otevreny, `None` kdyz zavreny. Viz `render` (blok na konci).
    mkdir_input: Option<String>,
    /// "Přejmenovat" dialog - `Some((puvodni_nazev, rozepsany_novy_nazev))`.
    rename_input: Option<(String, String)>,
    /// Potvrzeni mazani - `Some((absolutni_cesta, je_slozka, zobrazovany_nazev))`.
    delete_confirm: Option<(String, bool, String)>,
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
            progress: None,
            loading: false,
            mkdir_input: None,
            rename_input: None,
            delete_confirm: None,
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
                    self.loading = false;
                }
                Ok(SftpEvent::Listing { path, entries }) => {
                    self.current_path = path;
                    self.entries = entries;
                    self.loading = false;
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
                Ok(SftpEvent::DirProgress { done, total }) => {
                    self.progress = Some((done, total));
                }
                Ok(SftpEvent::DirDownloaded { remote, local, count }) => {
                    self.progress = None;
                    self.status = Some(StatusMsg::DirDownloaded { remote, local, count });
                }
                Ok(SftpEvent::DirUploaded { local, remote, count }) => {
                    self.progress = None;
                    self.status = Some(StatusMsg::DirUploaded { local, remote, count });
                    // Stejne jako po jednotlivem nahrani - rovnou
                    // obnovit vypis, at je nova slozka hned videt.
                    self.request_list(self.current_path.clone());
                }
                Ok(SftpEvent::Renamed { from, to }) => {
                    self.status = Some(StatusMsg::Renamed { from, to });
                    self.request_list(self.current_path.clone());
                }
                Ok(SftpEvent::Deleted { path }) => {
                    self.status = Some(StatusMsg::Deleted { path });
                    self.request_list(self.current_path.clone());
                }
                Ok(SftpEvent::Created { path }) => {
                    self.status = Some(StatusMsg::Created { path });
                    self.request_list(self.current_path.clone());
                }
                Ok(SftpEvent::Closed) => {
                    if self.state != SftpState::AwaitingCredentials {
                        self.state = SftpState::Disconnected;
                    }
                    self.loading = false;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }
    }

    fn request_list(&mut self, path: String) {
        let _ = self.handle.cmd_tx.send(SftpCommand::List(path));
        self.loading = true;
    }

    /// Prelozi `self.status` (surova data, viz [`StatusMsg`]) do textu v
    /// aktualnim jazyce - volano az tady, ne uvnitr `pump`.
    fn status_text(&self, lang: Lang) -> Option<String> {
        let tr = i18n::t(lang);
        match self.status.as_ref()? {
            StatusMsg::Error(e) => Some(e.clone()),
            StatusMsg::Downloaded { remote, local } => {
                Some(format!("{}: {} → {}", tr.sftp_status_downloaded, remote, local.display()))
            }
            StatusMsg::Uploaded { local, remote } => {
                Some(format!("{}: {} → {}", tr.sftp_status_uploaded, local.display(), remote))
            }
            StatusMsg::DirDownloaded { remote, local, count } => {
                Some(i18n::sftp_dir_downloaded(lang, *count, remote, &local.display().to_string()))
            }
            StatusMsg::DirUploaded { local, remote, count } => {
                Some(i18n::sftp_dir_uploaded(lang, *count, &local.display().to_string(), remote))
            }
            StatusMsg::Renamed { from, to } => Some(format!("{}: {} → {}", tr.sftp_status_renamed, from, to)),
            StatusMsg::Deleted { path } => Some(format!("{}: {}", tr.sftp_status_deleted, path)),
            StatusMsg::Created { path } => Some(format!("{}: {}", tr.sftp_status_created, path)),
        }
    }

    pub fn render(&mut self, ui: &mut egui::Ui, lang: Lang) {
        self.pump();
        let tr = i18n::t(lang);
        let status_text = self.status_text(lang);

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
        let mut download_dir_name: Option<String> = None;
        let mut rename_click: Option<String> = None;
        let mut delete_click: Option<(String, bool)> = None;

        ui.horizontal(|ui| {
            // Stejny druh rucne vykreslenych ikonek jako u akci v seznamu
            // souboru nize (`icon_button`/`SftpIcon`) - jen se tu (na
            // rozdil od tam) PRIDAVAJI PRED puvodni text, misto aby ho
            // nahradily, protoze v horni liste je dost mista a popisky
            // zustavaji uzitecne (zpetna vazba "tady bych přidal do
            // textových tlačítek taky ikonky").
            let parent = parent_path(&self.current_path);
            if icon_text_button(ui, ToolbarIcon::Up, tr.btn_sftp_up, parent.is_some()).clicked() {
                navigate_to = parent;
            }
            if icon_text_button(ui, ToolbarIcon::Refresh, tr.btn_refresh, true).clicked() {
                navigate_to = Some(self.current_path.clone());
            }
            if icon_text_button(ui, ToolbarIcon::UploadFile, tr.btn_sftp_upload, true).clicked() {
                if let Some(local) = rfd::FileDialog::new().pick_file() {
                    let file_name = local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if !file_name.is_empty() {
                        let remote = join_remote(&self.current_path, &file_name);
                        let _ = self.handle.cmd_tx.send(SftpCommand::Upload { local, remote });
                    }
                }
            }
            if icon_text_button(ui, ToolbarIcon::UploadFolder, tr.btn_sftp_upload_folder, true).clicked() {
                if let Some(local) = rfd::FileDialog::new().pick_folder() {
                    let folder_name = local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if !folder_name.is_empty() {
                        let remote = join_remote(&self.current_path, &folder_name);
                        let _ = self.handle.cmd_tx.send(SftpCommand::UploadDir { local, remote });
                    }
                }
            }
            if icon_text_button(ui, ToolbarIcon::NewFolder, tr.btn_sftp_mkdir, true).clicked() {
                self.mkdir_input = Some(String::new());
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&self.current_path).monospace());
            if self.loading {
                ui.add_space(6.0);
                ui.spinner();
                ui.label(egui::RichText::new(tr.sftp_loading).weak());
            }
        });
        if let Some((done, total)) = self.progress {
            ui.add_space(4.0);
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(egui::RichText::new(format!("{}: {done}/{total}", tr.sftp_transferring)).strong());
                });
            });
        } else if let Some(status) = &status_text {
            // Vyrazny ohraniceny "banner" (misto puvodniho drobneho
            // sedeho textu, ktery byl snadno prehlednutelny) - zelena
            // pro uspesne dokonceni prenosu, cervena pro chybu (stejny
            // odstin, jaky uz appka pouziva jinde pro chyby).
            ui.add_space(4.0);
            let is_error = matches!(self.status, Some(StatusMsg::Error(_)));
            let color = if is_error {
                egui::Color32::from_rgb(0xe0, 0x6c, 0x6c)
            } else {
                egui::Color32::from_rgb(0x6c, 0xba, 0x7e)
            };
            ui.group(|ui| {
                ui.colored_label(color, egui::RichText::new(status).strong());
            });
        }
        ui.add_space(6.0);
        ui.separator();

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            // `Grid` misto puvodniho `ui.horizontal` na radek - kazdy
            // sloupec (nazev/velikost/akce) se tak automaticky zarovna
            // podle nejsirsi bunky v nem napric VSEMI radky (zpetna
            // vazba "SFTP bychom mohli zarovnat do sloupců"), misto aby
            // pozice tlacitek "plavala" podle delky nazvu souboru.
            egui::Grid::new("sftp_entries_grid")
                .num_columns(5)
                .spacing([10.0, 3.0])
                .striped(true)
                .show(ui, |ui| {
                    for entry in &self.entries {
                        if entry.is_dir {
                            // Koncova "/" misto ikonky slozky - font
                            // pouzity v teto appce nema kompletni pokryti
                            // emoji/Dingbats bloku (viz podobny problem
                            // uz drive vyreseny jinde v `app.rs` u ikon
                            // tabu i nize u `icon_button`), takze se
                            // zamerne drzime obycejneho textu.
                            let label = format!("{}/", entry.name);
                            if ui.selectable_label(false, label).double_clicked() {
                                navigate_to = Some(join_remote(&self.current_path, &entry.name));
                            }
                            // Slozky nemaji velikost - prazdna bunka, at
                            // sloupec s akcemi zustane zarovnany se
                            // souborovymi radky nize.
                            ui.label("");
                            // Stejna akce "stahnout" jako u souboru nize -
                            // stahne CELOU slozku rekurzivne (viz
                            // `SftpCommand::DownloadDir`), bez nutnosti do
                            // ni napred navigovat.
                            if icon_button(ui, SftpIcon::Download, tr.btn_sftp_download).clicked() {
                                download_dir_name = Some(entry.name.clone());
                            }
                            if icon_button(ui, SftpIcon::Rename, tr.btn_rename).clicked() {
                                rename_click = Some(entry.name.clone());
                            }
                            if icon_button(ui, SftpIcon::Delete, tr.btn_delete).clicked() {
                                delete_click = Some((entry.name.clone(), true));
                            }
                        } else {
                            ui.label(&entry.name);
                            ui.label(egui::RichText::new(format_size(entry.size)).weak());
                            if icon_button(ui, SftpIcon::Download, tr.btn_sftp_download).clicked() {
                                download_name = Some(entry.name.clone());
                            }
                            if icon_button(ui, SftpIcon::Rename, tr.btn_rename).clicked() {
                                rename_click = Some(entry.name.clone());
                            }
                            if icon_button(ui, SftpIcon::Delete, tr.btn_delete).clicked() {
                                delete_click = Some((entry.name.clone(), false));
                            }
                        }
                        ui.end_row();
                    }
                });
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
        if let Some(name) = download_dir_name {
            // Uzivatel vybira RODICOVSKOU slozku - stahovana slozka se
            // v ni vytvori jako podslozka se svym puvodnim jmenem
            // (stejna konvence jako WinSCP/FileZilla), ne primo obsah
            // vybrane slozky bez obalu.
            if let Some(parent) = rfd::FileDialog::new().pick_folder() {
                let remote = join_remote(&self.current_path, &name);
                let local = parent.join(&name);
                let _ = self.handle.cmd_tx.send(SftpCommand::DownloadDir { remote, local });
            }
        }
        if let Some(name) = rename_click {
            self.rename_input = Some((name.clone(), name));
        }
        if let Some((name, is_dir)) = delete_click {
            let path = join_remote(&self.current_path, &name);
            self.delete_confirm = Some((path, is_dir, name));
        }

        self.render_mkdir_dialog(ui, tr);
        self.render_rename_dialog(ui, tr);
        self.render_delete_dialog(ui, lang, tr);
    }

    /// "Nová složka..." modal - stejny vzor "take/show/pripadne vratit
    /// zpet", jaky uz `app.rs` pouziva pro sve dialogy (`show_new_session_dialog`
    /// apod.) - nejde jednoduse pujcit `&mut self.mkdir_input` primo do
    /// uzavreni `egui::Window::show`, protoze uvnitr `if submit {...}`
    /// nize je potreba `self.mkdir_input` znovu zapsat/vynulovat, coz
    /// by kolidovalo s jeste zivou pujckou.
    fn render_mkdir_dialog(&mut self, ui: &mut egui::Ui, tr: &i18n::Strings) {
        let Some(mut name) = self.mkdir_input.take() else { return };
        let mut open = true;
        let mut submit = false;
        let mut cancel = false;

        egui::Window::new(tr.dialog_sftp_mkdir_title)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .current_pos(ui.ctx().screen_rect().center())
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.label(tr.field_name);
                ui.text_edit_singleline(&mut name);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(tr.btn_sftp_create).clicked() {
                        submit = true;
                    }
                    if ui.button(tr.btn_cancel).clicked() {
                        cancel = true;
                    }
                });
            });

        if cancel {
            open = false;
        }

        if submit {
            let trimmed = name.trim().to_string();
            if !trimmed.is_empty() {
                let path = join_remote(&self.current_path, &trimmed);
                let _ = self.handle.cmd_tx.send(SftpCommand::Mkdir { path });
            }
        } else if open {
            self.mkdir_input = Some(name);
        }
    }

    /// "Přejmenovat" modal - viz `render_mkdir_dialog` pro vysvetleni
    /// take/show/pripadne-vratit vzoru.
    fn render_rename_dialog(&mut self, ui: &mut egui::Ui, tr: &i18n::Strings) {
        let Some((old_name, mut new_name)) = self.rename_input.take() else { return };
        let mut open = true;
        let mut submit = false;
        let mut cancel = false;

        egui::Window::new(tr.dialog_rename_title)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .current_pos(ui.ctx().screen_rect().center())
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.label(tr.field_name);
                ui.text_edit_singleline(&mut new_name);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(tr.btn_save).clicked() {
                        submit = true;
                    }
                    if ui.button(tr.btn_cancel).clicked() {
                        cancel = true;
                    }
                });
            });

        if cancel {
            open = false;
        }

        if submit {
            let trimmed = new_name.trim().to_string();
            if !trimmed.is_empty() && trimmed != old_name {
                let from = join_remote(&self.current_path, &old_name);
                let to = join_remote(&self.current_path, &trimmed);
                let _ = self.handle.cmd_tx.send(SftpCommand::Rename { from, to });
            }
        } else if open {
            self.rename_input = Some((old_name, new_name));
        }
    }

    /// Potvrzeni smazani - viz `render_mkdir_dialog` pro vysvetleni
    /// take/show/pripadne-vratit vzoru.
    fn render_delete_dialog(&mut self, ui: &mut egui::Ui, lang: Lang, tr: &i18n::Strings) {
        let Some((path, is_dir, name)) = self.delete_confirm.take() else { return };
        let mut open = true;
        let mut confirmed = false;
        let mut cancel = false;

        egui::Window::new(tr.dialog_delete_title)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .current_pos(ui.ctx().screen_rect().center())
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.label(i18n::confirm_delete_sftp_entry(lang, &name));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(tr.btn_delete).clicked() {
                        confirmed = true;
                    }
                    if ui.button(tr.btn_cancel).clicked() {
                        cancel = true;
                    }
                });
            });

        if cancel {
            open = false;
        }

        if confirmed {
            let _ = self.handle.cmd_tx.send(SftpCommand::Delete { path, is_dir });
        } else if open {
            self.delete_confirm = Some((path, is_dir, name));
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

/// Druh rucne vykreslene ikonky v akcnich tlacitkach seznamu souboru -
/// viz `icon_button`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SftpIcon {
    Download,
    Rename,
    Delete,
}

/// Male ctvercove tlacitko s rucne vykreslenou ikonkou - nahrazuje
/// puvodni textove popisky ("Stáhnout"/"Přejmenovat..."/"Smazat"), ktere
/// v radku zabiraly hodne mista (zpetna vazba "funkcím dát ikonky").
/// Ikonka se KRESLI PRIMO pres `ui.painter()` (sipka/tuzka/kos jako
/// primitiva - usecky a trojuhelnik), misto spolehani na Unicode symbol
/// jako "⬇"/"✎"/"🗑" - font pouzity v teto appce nema kompletni pokryti
/// Unicode bloku jako Dingbats/Miscellaneous Symbols a takovy znak by se
/// vykreslil jako prazdny "tofu" ctverecek. Stejny druh problemu uz drive
/// vyreseny jinde v `app.rs` (viz komentare u "X" tlacitka zavirani tabu
/// a ctverecku stavu tabu v `render_open_tabs_list`/`tab_bar`) - misto
/// spolehani na chybejici glyf se tvar vykresli rovnou jako `egui::Shape`.
/// Nazev akce zustava citelny v tooltipu po najeti mysi (`tooltip`).
fn icon_button(ui: &mut egui::Ui, icon: SftpIcon, tooltip: &str) -> egui::Response {
    let size = ui.spacing().interact_size.y;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, false);
        if response.hovered() {
            ui.painter().rect(rect.expand(visuals.expansion), visuals.rounding, visuals.weak_bg_fill, visuals.bg_stroke);
        }
        let color = visuals.text_color();
        let stroke = egui::Stroke::new(1.4_f32, color);
        let c = rect.center();
        let r = rect.width() * 0.26;

        match icon {
            SftpIcon::Download => {
                // Svisly drik sipky + trojuhelnikovy hrot dolu, nad
                // vodorovnou "podlozkou" (klasicky piktogram stazeni).
                let top = c + egui::vec2(0.0, -r * 1.3);
                let tip = c + egui::vec2(0.0, r * 0.5);
                ui.painter().line_segment([top, tip], stroke);
                let head = r * 0.75;
                ui.painter().add(egui::Shape::convex_polygon(
                    vec![
                        tip + egui::vec2(0.0, head * 0.55),
                        tip + egui::vec2(-head, -head * 0.55),
                        tip + egui::vec2(head, -head * 0.55),
                    ],
                    color,
                    egui::Stroke::NONE,
                ));
                let tray_y = c.y + r * 1.3;
                ui.painter().line_segment(
                    [egui::pos2(c.x - r * 1.2, tray_y), egui::pos2(c.x + r * 1.2, tray_y)],
                    stroke,
                );
            }
            SftpIcon::Rename => {
                // "Tuzka": silny diagonalni drik + maly trojuhelnikovy
                // hrot na jednom konci.
                let a = c + egui::vec2(-r * 1.1, r * 1.1);
                let b = c + egui::vec2(r * 0.8, -r * 1.1);
                ui.painter().line_segment([a, b], egui::Stroke::new(2.2_f32, color));
                let dir = (b - a).normalized();
                let side = egui::vec2(-dir.y, dir.x) * (r * 0.4);
                let tip = b + dir * (r * 0.55);
                ui.painter().add(egui::Shape::convex_polygon(vec![b - side, b + side, tip], color, egui::Stroke::NONE));
            }
            SftpIcon::Delete => {
                // "Kos": obdelnikove telo + vodorovne "vicko" + 3 svisle
                // carky uvnitr.
                let body = egui::Rect::from_center_size(c + egui::vec2(0.0, r * 0.25), egui::vec2(r * 1.7, r * 1.9));
                ui.painter().rect_stroke(body, egui::Rounding::same(1.0), stroke);
                ui.painter().line_segment(
                    [egui::pos2(body.min.x - r * 0.3, body.min.y), egui::pos2(body.max.x + r * 0.3, body.min.y)],
                    stroke,
                );
                for dx in [-r * 0.5, 0.0, r * 0.5] {
                    ui.painter().line_segment(
                        [egui::pos2(c.x + dx, body.min.y + r * 0.45), egui::pos2(c.x + dx, body.max.y - r * 0.3)],
                        egui::Stroke::new(1.1_f32, color),
                    );
                }
            }
        }
    }

    response.on_hover_text(tooltip)
}

/// Druh rucne vykreslene ikonky pro tlacitka HORNI LISTY SFTP prohlizece
/// (Nahoru/Obnovit/Nahrát soubor.../Nahrát složku.../Nová složka...) -
/// na rozdil od `SftpIcon` (ktera text NAHRAZUJE) se tady ikonka jen
/// PRIDA PRED puvodni text (zpetna vazba "tady bych přidal do textových
/// tlačítek taky ikonky") - v horni liste je dost mista a popisky
/// zustavaji uzitecne.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolbarIcon {
    Up,
    Refresh,
    UploadFile,
    UploadFolder,
    NewFolder,
}

/// Tlacitko s rucne vykreslenou ikonkou PRED textem - viz `ToolbarIcon`.
/// Stejny duvod rucniho kresleni pres `ui.painter()` jako u `icon_button`
/// (font pouzity v teto appce nema kompletni pokryti Unicode bloku jako
/// Dingbats/Miscellaneous Symbols, takze by se Unicode symbol vykreslil
/// jako prazdny "tofu" ctverecek). `enabled = false` (jen u "Nahoru" v
/// korenove slozce) tlacitko jen ztlumi a nereaguje na klik - obdoba
/// puvodniho `ui.add_enabled`.
fn icon_text_button(ui: &mut egui::Ui, icon: ToolbarIcon, text: &str, enabled: bool) -> egui::Response {
    let icon_size = ui.text_style_height(&egui::TextStyle::Button);
    let spacing = 6.0;
    let padding = ui.spacing().button_padding;
    let text_color = if enabled { ui.visuals().text_color() } else { ui.visuals().text_color().gamma_multiply(0.5) };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), egui::TextStyle::Button.resolve(ui.style()), text_color);

    let size = egui::vec2(
        padding.x * 2.0 + icon_size + spacing + galley.size().x,
        ui.spacing().interact_size.y.max(icon_size + padding.y * 2.0),
    );
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);

    if ui.is_rect_visible(rect) {
        if enabled {
            let visuals = ui.style().interact_selectable(&response, false);
            if response.hovered() {
                ui.painter().rect(rect.expand(visuals.expansion), visuals.rounding, visuals.weak_bg_fill, visuals.bg_stroke);
            }
        }
        let icon_rect = egui::Rect::from_min_size(
            egui::pos2(rect.min.x + padding.x, rect.center().y - icon_size * 0.5),
            egui::vec2(icon_size, icon_size),
        );
        draw_toolbar_icon(ui, icon, icon_rect, text_color);

        let text_pos = egui::pos2(icon_rect.max.x + spacing, rect.center().y - galley.size().y * 0.5);
        ui.painter().with_clip_rect(rect).galley(text_pos, galley, text_color);
    }

    response
}

/// Vykresli jednu ikonku (primitiva - usecky, trojuhelnikovy hrot,
/// obdelniky) do ctvercove `rect` barvou `color` - viz `icon_text_button`.
fn draw_toolbar_icon(ui: &egui::Ui, icon: ToolbarIcon, rect: egui::Rect, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.4_f32, color);
    let c = rect.center();
    let r = rect.width() * 0.5;

    match icon {
        ToolbarIcon::Up => {
            // Jednoducha sipka nahoru (bez "podlozky") - "o úroveň výš".
            let top = c + egui::vec2(0.0, -r * 0.8);
            let bottom = c + egui::vec2(0.0, r * 0.8);
            ui.painter().line_segment([bottom, top], stroke);
            let head = r * 0.55;
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    top + egui::vec2(0.0, -head * 0.4),
                    top + egui::vec2(-head, head * 0.7),
                    top + egui::vec2(head, head * 0.7),
                ],
                color,
                egui::Stroke::NONE,
            ));
        }
        ToolbarIcon::Refresh => {
            // Kruhova sipka: oblouk (polylinie po bodech na kruznici) +
            // trojuhelnikovy hrot na jednom konci.
            let radius = r * 0.7;
            let start_angle = -std::f32::consts::FRAC_PI_2 * 0.3;
            let end_angle = start_angle + std::f32::consts::PI * 1.6;
            let steps = 20;
            let points: Vec<egui::Pos2> = (0..=steps)
                .map(|i| {
                    let t = start_angle + (end_angle - start_angle) * (i as f32 / steps as f32);
                    c + egui::vec2(t.cos(), t.sin()) * radius
                })
                .collect();
            ui.painter().add(egui::Shape::line(points.clone(), stroke));
            if points.len() >= 2 {
                let last = points[points.len() - 1];
                let prev = points[points.len() - 2];
                let dir = (last - prev).normalized();
                let side = egui::vec2(-dir.y, dir.x) * (r * 0.28);
                let tip = last + dir * (r * 0.4);
                ui.painter().add(egui::Shape::convex_polygon(vec![last - side, last + side, tip], color, egui::Stroke::NONE));
            }
        }
        ToolbarIcon::UploadFile => {
            // Zrcadlena varianta `SftpIcon::Download` (sipka nahoru
            // vystupujici z "podlozky" misto sipky dolu do ni).
            let bottom_tip = c + egui::vec2(0.0, r * 0.55);
            let top_tip = c + egui::vec2(0.0, -r * 0.75);
            ui.painter().line_segment([bottom_tip, top_tip], stroke);
            let head = r * 0.55;
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    top_tip + egui::vec2(0.0, -head * 0.5),
                    top_tip + egui::vec2(-head, head * 0.55),
                    top_tip + egui::vec2(head, head * 0.55),
                ],
                color,
                egui::Stroke::NONE,
            ));
            let tray_y = c.y + r * 0.85;
            ui.painter().line_segment([egui::pos2(c.x - r * 0.75, tray_y), egui::pos2(c.x + r * 0.75, tray_y)], stroke);
        }
        ToolbarIcon::UploadFolder => {
            draw_folder(ui, rect, stroke);
            // Maly "stríška" hrot uvnitr slozky - "nahrát DO slozky".
            let base = c + egui::vec2(0.0, r * 0.15);
            let w = r * 0.35;
            let h = r * 0.35;
            let small = egui::Stroke::new(1.3_f32, color);
            ui.painter().line_segment([base, base + egui::vec2(-w, h)], small);
            ui.painter().line_segment([base, base + egui::vec2(w, h)], small);
            ui.painter().line_segment([base, base + egui::vec2(0.0, h * 1.6)], small);
        }
        ToolbarIcon::NewFolder => {
            draw_folder(ui, rect, stroke);
            // Maly "+" uvnitr slozky - "nová".
            let plus_c = c + egui::vec2(0.0, r * 0.25);
            let s = r * 0.35;
            let small = egui::Stroke::new(1.3_f32, color);
            ui.painter().line_segment([plus_c + egui::vec2(-s, 0.0), plus_c + egui::vec2(s, 0.0)], small);
            ui.painter().line_segment([plus_c + egui::vec2(0.0, -s), plus_c + egui::vec2(0.0, s)], small);
        }
    }
}

/// Silueta slozky (telo + maly "štítek" v levem horním rohu) - sdileno
/// mezi `ToolbarIcon::UploadFolder` a `ToolbarIcon::NewFolder`.
fn draw_folder(ui: &egui::Ui, rect: egui::Rect, stroke: egui::Stroke) {
    let c = rect.center();
    let r = rect.width() * 0.5;
    let body = egui::Rect::from_min_size(egui::pos2(c.x - r * 0.85, c.y - r * 0.15), egui::vec2(r * 1.7, r * 1.0));
    let tab = egui::Rect::from_min_size(egui::pos2(body.min.x, body.min.y - r * 0.35), egui::vec2(r * 0.9, r * 0.35));
    ui.painter().rect_stroke(body, egui::Rounding::same(1.0), stroke);
    ui.painter().line_segment([tab.left_top(), tab.right_top()], stroke);
    ui.painter().line_segment([tab.left_top(), tab.left_bottom()], stroke);
    ui.painter().line_segment([tab.right_top(), tab.right_bottom()], stroke);
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
