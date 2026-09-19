//! ftp_browser.rs - obsah "Ftp" tabu (viz `app.rs::TabKind::Ftp`):
//! prohlizec vzdalenych souboru pres FTP/FTPS napojeny na
//! `termx_ftp::spawn_ftp_session` - stejny vzhled i ovladani jako u SFTP
//! (`sftp_browser.rs`, viz jeho hlavicka pro popis zjednoduseni: prubeh
//! hromadneho prenosu slozky se ukazuje jen jako "hotovo/celkem" a
//! existujici soubory se pri kolizi jmen VZDY prepisuji). Tento soubor je
//! ZAMERNE samostatna kopie (ne sdilena logika s `sftp_browser.rs`) -
//! stejna konvence jako uz drive v projektu (vlastni vyberova/zavirava
//! logika tabu misto sdileni s `tab_bar`).
//!
//! Na rozdil od SFTP tu neni "prejmenovani" ani "mazani slozky" o nic
//! jinak omezene - obycejne FTP take neumi smazat neprazdnou slozku
//! primo, takze plati stejne omezeni ("jen soubory/PRAZDNE slozky").
//!
//! Stejny vzor jako `sftp_browser::SftpBrowser`/`terminal::TerminalSession`:
//! spojeni se zaklada rovnou v [`FtpBrowser::new`] (vlastni vlakno, viz
//! `termx_ftp::spawn_ftp_session`), GUI kazdy snimek "vycerpa" prichozi
//! udalosti pres [`FtpBrowser::pump`] (`try_recv`).

use std::path::PathBuf;

use termx_core::Session;
use termx_ftp::{spawn_ftp_session, FtpCommand, FtpEntry, FtpEvent, FtpHandle};

use crate::i18n::{self, Lang};

/// Stav FTP relace tohoto tabu - viz `sftp_browser::SftpState` (obdobny
/// ucel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FtpState {
    Connecting,
    AwaitingCredentials,
    Connected,
    Disconnected,
}

/// Posledni vysledek operace (stazeni/nahrani/chyba) - viz
/// `sftp_browser::StatusMsg` (stejny duvod: surova data, preklad az v
/// `render`).
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

pub struct FtpBrowser {
    handle: FtpHandle,
    state: FtpState,
    current_path: String,
    entries: Vec<FtpEntry>,
    status: Option<StatusMsg>,
    /// Prubeh prave beziciho hromadneho prenosu slozky (done, total) -
    /// viz `FtpEvent::DirProgress`. `None`, kdyz zadny neprobiha.
    progress: Option<(usize, usize)>,
    /// `true` mezi odeslanim `FtpCommand::List` a prijetim odpovedi
    /// (`FtpEvent::Listing`/`Error`) - viz `sftp_browser::SftpBrowser::loading`.
    loading: bool,
    mkdir_input: Option<String>,
    rename_input: Option<(String, String)>,
    delete_confirm: Option<(String, bool, String)>,
    cred_username: String,
    cred_password: String,
}

impl FtpBrowser {
    pub fn new(session: &Session) -> Self {
        Self {
            handle: spawn_ftp_session(session.clone()),
            state: FtpState::Connecting,
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

    pub fn state(&self) -> FtpState {
        self.state
    }

    fn pump(&mut self) {
        loop {
            match self.handle.event_rx.try_recv() {
                Ok(FtpEvent::AwaitingCredentials) => {
                    self.state = FtpState::AwaitingCredentials;
                }
                Ok(FtpEvent::AuthFailed(msg)) => {
                    self.state = FtpState::AwaitingCredentials;
                    self.status = Some(StatusMsg::Error(msg));
                }
                Ok(FtpEvent::Connected { home }) => {
                    self.state = FtpState::Connected;
                    self.status = None;
                    self.request_list(home);
                }
                Ok(FtpEvent::Error(msg)) => {
                    self.status = Some(StatusMsg::Error(msg));
                    self.loading = false;
                }
                Ok(FtpEvent::Listing { path, entries }) => {
                    self.current_path = path;
                    self.entries = entries;
                    self.loading = false;
                }
                Ok(FtpEvent::Downloaded { remote, local }) => {
                    self.status = Some(StatusMsg::Downloaded { remote, local });
                }
                Ok(FtpEvent::Uploaded { local, remote }) => {
                    self.status = Some(StatusMsg::Uploaded { local, remote });
                    // Po nahrani rovnou obnovit vypis aktualni slozky, at
                    // je novy soubor hned videt v seznamu.
                    self.request_list(self.current_path.clone());
                }
                Ok(FtpEvent::DirProgress { done, total }) => {
                    self.progress = Some((done, total));
                }
                Ok(FtpEvent::DirDownloaded { remote, local, count }) => {
                    self.progress = None;
                    self.status = Some(StatusMsg::DirDownloaded { remote, local, count });
                }
                Ok(FtpEvent::DirUploaded { local, remote, count }) => {
                    self.progress = None;
                    self.status = Some(StatusMsg::DirUploaded { local, remote, count });
                    self.request_list(self.current_path.clone());
                }
                Ok(FtpEvent::Renamed { from, to }) => {
                    self.status = Some(StatusMsg::Renamed { from, to });
                    self.request_list(self.current_path.clone());
                }
                Ok(FtpEvent::Deleted { path }) => {
                    self.status = Some(StatusMsg::Deleted { path });
                    self.request_list(self.current_path.clone());
                }
                Ok(FtpEvent::Created { path }) => {
                    self.status = Some(StatusMsg::Created { path });
                    self.request_list(self.current_path.clone());
                }
                Ok(FtpEvent::Closed) => {
                    if self.state != FtpState::AwaitingCredentials {
                        self.state = FtpState::Disconnected;
                    }
                    self.loading = false;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }
    }

    fn request_list(&mut self, path: String) {
        let _ = self.handle.cmd_tx.send(FtpCommand::List(path));
        self.loading = true;
    }

    /// Prelozi `self.status` do textu v aktualnim jazyce - viz
    /// `sftp_browser::SftpBrowser::status_text`. Pouziva zamerne stejne
    /// (obsahove obecne, protokolem nepopsane) preklady jako SFTP
    /// prohlizec - viz `i18n::sftp_dir_downloaded`/`sftp_dir_uploaded` a
    /// `tr.sftp_status_*` (zadne "SFTP" slovo v samotnem textu).
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
            FtpState::Connecting => {
                ui.label(tr.sftp_connecting);
                return;
            }
            FtpState::Disconnected => {
                ui.label(tr.ftp_disconnected);
                if let Some(status) = &status_text {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(status).small());
                }
                return;
            }
            FtpState::AwaitingCredentials => {
                ui.heading(tr.ftp_login_heading);
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
                    let _ = self.handle.cmd_tx.send(FtpCommand::Credentials {
                        username: self.cred_username.clone(),
                        password: self.cred_password.clone(),
                    });
                    self.status = None;
                }
                return;
            }
            FtpState::Connected => {}
        }

        let mut navigate_to: Option<String> = None;
        let mut download_name: Option<String> = None;
        let mut download_dir_name: Option<String> = None;
        let mut rename_click: Option<String> = None;
        let mut delete_click: Option<(String, bool)> = None;

        ui.horizontal(|ui| {
            // Stejny druh rucne vykreslenych ikonek jako u SFTP prohlizece
            // (`sftp_browser::icon_text_button`) - "stejný vzhled a
            // ovládání jako u SFTP".
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
                        let _ = self.handle.cmd_tx.send(FtpCommand::Upload { local, remote });
                    }
                }
            }
            if icon_text_button(ui, ToolbarIcon::UploadFolder, tr.btn_sftp_upload_folder, true).clicked() {
                if let Some(local) = rfd::FileDialog::new().pick_folder() {
                    let folder_name = local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if !folder_name.is_empty() {
                        let remote = join_remote(&self.current_path, &folder_name);
                        let _ = self.handle.cmd_tx.send(FtpCommand::UploadDir { local, remote });
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
            egui::Grid::new("ftp_entries_grid")
                .num_columns(5)
                .spacing([10.0, 3.0])
                .striped(true)
                .show(ui, |ui| {
                    for entry in &self.entries {
                        if entry.is_dir {
                            let label = format!("{}/", entry.name);
                            if ui.selectable_label(false, label).double_clicked() {
                                navigate_to = Some(join_remote(&self.current_path, &entry.name));
                            }
                            ui.label("");
                            if icon_button(ui, FtpIcon::Download, tr.btn_sftp_download).clicked() {
                                download_dir_name = Some(entry.name.clone());
                            }
                            if icon_button(ui, FtpIcon::Rename, tr.btn_rename).clicked() {
                                rename_click = Some(entry.name.clone());
                            }
                            if icon_button(ui, FtpIcon::Delete, tr.btn_delete).clicked() {
                                delete_click = Some((entry.name.clone(), true));
                            }
                        } else {
                            ui.label(&entry.name);
                            ui.label(egui::RichText::new(format_size(entry.size)).weak());
                            if icon_button(ui, FtpIcon::Download, tr.btn_sftp_download).clicked() {
                                download_name = Some(entry.name.clone());
                            }
                            if icon_button(ui, FtpIcon::Rename, tr.btn_rename).clicked() {
                                rename_click = Some(entry.name.clone());
                            }
                            if icon_button(ui, FtpIcon::Delete, tr.btn_delete).clicked() {
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
                let _ = self.handle.cmd_tx.send(FtpCommand::Download { remote, local });
            }
        }
        if let Some(name) = download_dir_name {
            if let Some(parent) = rfd::FileDialog::new().pick_folder() {
                let remote = join_remote(&self.current_path, &name);
                let local = parent.join(&name);
                let _ = self.handle.cmd_tx.send(FtpCommand::DownloadDir { remote, local });
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

    /// "Nová složka..." modal - viz `sftp_browser::SftpBrowser::render_mkdir_dialog`
    /// pro vysvetleni vzoru "take/show/pripadne vratit zpet".
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
                let _ = self.handle.cmd_tx.send(FtpCommand::Mkdir { path });
            }
        } else if open {
            self.mkdir_input = Some(name);
        }
    }

    /// "Přejmenovat" modal - viz vysvetleni u `render_mkdir_dialog`.
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
                let _ = self.handle.cmd_tx.send(FtpCommand::Rename { from, to });
            }
        } else if open {
            self.rename_input = Some((old_name, new_name));
        }
    }

    /// Potvrzeni smazani - viz vysvetleni u `render_mkdir_dialog`.
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
            let _ = self.handle.cmd_tx.send(FtpCommand::Delete { path, is_dir });
        } else if open {
            self.delete_confirm = Some((path, is_dir, name));
        }
    }
}

/// Pocatecni hodnota pole "Uživatelské jméno" v prihlasovacim formulari -
/// viz `sftp_browser::session_username`.
fn session_username(session: &Session) -> String {
    match &session.auth {
        termx_core::AuthMethod::Password { username, .. } if !username.trim().is_empty() => username.clone(),
        _ => String::new(),
    }
}

/// Rodicovska slozka dane ABSOLUTNI cesty (FTP protokol - stejne jako
/// SFTP - vzdy pouziva '/' jako oddelovac, bez ohledu na OS ciloveho
/// serveru) - `None` uz pro koren (`/`), kde "Nahoru" nema kam vest.
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

/// Druh rucne vykreslene ikonky v akcnich tlacitkach seznamu souboru - viz
/// `sftp_browser::SftpIcon`/`icon_button` (stejny duvod rucniho kresleni:
/// font pouzity v teto appce nema kompletni pokryti Unicode Dingbats/
/// Miscellaneous Symbols bloku).
#[derive(Clone, Copy, PartialEq, Eq)]
enum FtpIcon {
    Download,
    Rename,
    Delete,
}

fn icon_button(ui: &mut egui::Ui, icon: FtpIcon, tooltip: &str) -> egui::Response {
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
            FtpIcon::Download => {
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
            FtpIcon::Rename => {
                let a = c + egui::vec2(-r * 1.1, r * 1.1);
                let b = c + egui::vec2(r * 0.8, -r * 1.1);
                ui.painter().line_segment([a, b], egui::Stroke::new(2.2_f32, color));
                let dir = (b - a).normalized();
                let side = egui::vec2(-dir.y, dir.x) * (r * 0.4);
                let tip = b + dir * (r * 0.55);
                ui.painter().add(egui::Shape::convex_polygon(vec![b - side, b + side, tip], color, egui::Stroke::NONE));
            }
            FtpIcon::Delete => {
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

/// Druh rucne vykreslene ikonky pro tlacitka HORNI LISTY FTP prohlizece -
/// viz `sftp_browser::ToolbarIcon`/`icon_text_button`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolbarIcon {
    Up,
    Refresh,
    UploadFile,
    UploadFolder,
    NewFolder,
}

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

fn draw_toolbar_icon(ui: &egui::Ui, icon: ToolbarIcon, rect: egui::Rect, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.4_f32, color);
    let c = rect.center();
    let r = rect.width() * 0.5;

    match icon {
        ToolbarIcon::Up => {
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
            let plus_c = c + egui::vec2(0.0, r * 0.25);
            let s = r * 0.35;
            let small = egui::Stroke::new(1.3_f32, color);
            ui.painter().line_segment([plus_c + egui::vec2(-s, 0.0), plus_c + egui::vec2(s, 0.0)], small);
            ui.painter().line_segment([plus_c + egui::vec2(0.0, -s), plus_c + egui::vec2(0.0, s)], small);
        }
    }
}

/// Silueta slozky - viz `sftp_browser::draw_folder`.
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
