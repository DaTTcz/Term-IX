//! Hlavni stav a vykreslovani aplikace Term-IX: horni menu, levy strom
//! serveru/slozek, horni lista tabu a obsah aktivniho tabu.
//!
//! Zamerne vse v jednom souboru (mensi riziko chyb v pravidlech
//! viditelnosti mezi moduly, kdyz zde nejde spustit skutecny
//! `cargo build` - viz poznamka v `lib.rs`).

use std::collections::BTreeMap;

use termx_core::{AuthMethod, ModuleRegistry, Protocol, Session};
use termx_vault::{Vault, VaultData};
use uuid::Uuid;

use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
enum TabKind {
    Home,
    Settings,
    Connection(Uuid),
}

struct NewSessionForm {
    name: String,
    folder: String,
    host: String,
    port: String,
    username: String,
    password: String,
}

impl Default for NewSessionForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            folder: String::new(),
            host: String::new(),
            port: "22".to_string(),
            username: String::new(),
            password: String::new(),
        }
    }
}

enum RenameTarget {
    Session(Uuid),
    Folder(String),
}

struct RenameDialog {
    target: RenameTarget,
    value: String,
}

struct MoveDialog {
    session_id: Uuid,
    value: String,
}

enum DeleteTarget {
    Session(Uuid),
    Folder(String),
}

enum TreeAction {
    Open(Uuid),
    RenameSession(Uuid),
    MoveSession(Uuid),
    DeleteSession(Uuid),
    RenameFolder(String),
    DeleteFolder(String),
}

/// Vnitrni pomocna struktura pro vykresleni stromu - stavi se znovu kazdy
/// snimek z aktualnich dat trezoru (levne, poctu polozek jsou radove
/// desitky/stovky), takze zadny stav stromu se nemusi rucne udrzovat v
/// synchronizaci s daty.
#[derive(Default)]
struct FolderNode {
    children: BTreeMap<String, FolderNode>,
    session_ids: Vec<Uuid>,
}

impl FolderNode {
    fn ensure_path(&mut self, path: &str) -> &mut FolderNode {
        let mut node = self;
        for segment in path.split('/') {
            if segment.is_empty() {
                continue;
            }
            node = node.children.entry(segment.to_string()).or_default();
        }
        node
    }
}

fn build_tree(data: &VaultData) -> FolderNode {
    let mut root = FolderNode::default();
    for folder_path in &data.folders {
        root.ensure_path(folder_path);
    }
    for session in &data.servers {
        match &session.group {
            Some(path) if !path.trim().is_empty() => {
                root.ensure_path(path).session_ids.push(session.id);
            }
            _ => root.session_ids.push(session.id),
        }
    }
    root
}

pub struct TermxApp {
    vault: Vault,
    master_password: String,
    #[allow(dead_code)] // navazujici krok: napojeni Connection tabu na skutecny modul
    registry: ModuleRegistry,

    tabs: Vec<TabKind>,
    active_tab: usize,

    new_session_form: Option<NewSessionForm>,
    new_folder_dialog: Option<String>,
    rename_dialog: Option<RenameDialog>,
    move_dialog: Option<MoveDialog>,
    delete_confirm: Option<DeleteTarget>,

    status_message: Option<String>,
}

impl TermxApp {
    pub fn new(vault: Vault, master_password: String, registry: ModuleRegistry) -> Self {
        Self {
            vault,
            master_password,
            registry,
            tabs: vec![TabKind::Home],
            active_tab: 0,
            new_session_form: None,
            new_folder_dialog: None,
            rename_dialog: None,
            move_dialog: None,
            delete_confirm: None,
            status_message: None,
        }
    }

    fn save_vault(&mut self) {
        if let Err(e) = self.vault.save(&self.master_password) {
            self.status_message = Some(format!("Ulozeni trezoru selhalo: {e}"));
        }
    }

    // -- taby ---------------------------------------------------------

    fn tab_title(&self, kind: TabKind) -> String {
        match kind {
            TabKind::Home => "Domů".to_string(),
            TabKind::Settings => "Nastavení".to_string(),
            TabKind::Connection(id) => self
                .vault
                .data
                .servers
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "Spojení".to_string()),
        }
    }

    fn open_session_tab(&mut self, id: Uuid) {
        if let Some(idx) = self.tabs.iter().position(|t| matches!(t, TabKind::Connection(sid) if *sid == id)) {
            self.active_tab = idx;
            return;
        }
        self.tabs.push(TabKind::Connection(id));
        self.active_tab = self.tabs.len() - 1;
    }

    fn open_settings_tab(&mut self) {
        if let Some(idx) = self.tabs.iter().position(|t| matches!(t, TabKind::Settings)) {
            self.active_tab = idx;
            return;
        }
        self.tabs.push(TabKind::Settings);
        self.active_tab = self.tabs.len() - 1;
    }

    fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() || matches!(self.tabs[idx], TabKind::Home) {
            return;
        }
        self.tabs.remove(idx);
        if self.tabs.is_empty() {
            self.tabs.push(TabKind::Home);
            self.active_tab = 0;
            return;
        }
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        } else if self.active_tab > idx {
            self.active_tab -= 1;
        }
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let mut to_select = None;
        let mut to_close = None;

        let snapshot: Vec<TabKind> = self.tabs.clone();

        ui.horizontal_wrapped(|ui| {
            for (idx, &kind) in snapshot.iter().enumerate() {
                let selected = idx == self.active_tab;
                let title = self.tab_title(kind);
                ui.horizontal(|ui| {
                    if ui.selectable_label(selected, title).clicked() {
                        to_select = Some(idx);
                    }
                    if !matches!(kind, TabKind::Home) && ui.small_button("✕").clicked() {
                        to_close = Some(idx);
                    }
                });
            }
        });

        if let Some(idx) = to_select {
            self.active_tab = idx;
        }
        if let Some(idx) = to_close {
            self.close_tab(idx);
        }
    }

    fn active_tab_content(&mut self, ui: &mut egui::Ui) {
        if self.tabs.is_empty() {
            self.tabs.push(TabKind::Home);
            self.active_tab = 0;
        }
        let idx = self.active_tab.min(self.tabs.len() - 1);
        match self.tabs[idx] {
            TabKind::Home => self.render_home(ui),
            TabKind::Settings => self.render_settings(ui),
            TabKind::Connection(id) => self.render_connection(ui, id),
        }
    }

    fn render_home(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(48.0);
            ui.heading("Term-IX");
            ui.label(format!("verze {}", env!("CARGO_PKG_VERSION")));
            ui.add_space(20.0);
            ui.label("Vyberte server vlevo, nebo přidejte nový přes menu Sessions.");
        });
    }

    fn render_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Nastavení");
        ui.separator();
        ui.label("Umístění trezoru:");
        ui.code(self.vault.path().display().to_string());
        ui.add_space(12.0);
        ui.label(
            "Vzhled: v této verzi je k dispozici jen 'terminálové' tmavé téma. \
             Modernější téma přibude jako další volba zde.",
        );
        ui.add_space(12.0);
        if ui.button("Exportovat trezor...").clicked() {
            self.status_message =
                Some("Export/import trezoru z GUI je připraven v termx-vault, dialog v GUI zatím chybí (další krok).".to_string());
        }
    }

    fn render_connection(&mut self, ui: &mut egui::Ui, id: Uuid) {
        let Some(session) = self.vault.data.servers.iter().find(|s| s.id == id) else {
            ui.label("Tento server už neexistuje (byl smazán).");
            return;
        };

        ui.heading(&session.name);
        ui.label(format!("{} — {}:{}", session.protocol, session.host, session.port));
        ui.add_space(16.0);
        ui.label("Vestavěný terminál zatím není připojený — toto je zatím jen informační záložka.");
        ui.label("Skutečné spojení (bez nativního okna OS, přímo v tomto tabu) je navazující krok.");
    }

    // -- horni menu -----------------------------------------------------

    fn top_menu(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("Terminal", |ui| {
                    if ui.button("Ukončit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        ui.close_menu();
                    }
                });
                ui.menu_button("Sessions", |ui| {
                    if ui.button("Nový server...").clicked() {
                        self.new_session_form = Some(NewSessionForm::default());
                        ui.close_menu();
                    }
                    if ui.button("Nová složka...").clicked() {
                        self.new_folder_dialog = Some(String::new());
                        ui.close_menu();
                    }
                });
                ui.menu_button("View", |_ui| {});
                ui.menu_button("Tools", |_ui| {});
                ui.menu_button("Settings", |ui| {
                    if ui.button("Předvolby...").clicked() {
                        self.open_settings_tab();
                        ui.close_menu();
                    }
                });
                ui.menu_button("Help", |ui| {
                    ui.label(format!("Term-IX v{}", env!("CARGO_PKG_VERSION")));
                });
            });
        });
    }

    // -- dialogy ---------------------------------------------------------

    fn show_new_session_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut form) = self.new_session_form.take() else { return };
        let mut open = true;
        let mut submit = false;
        let mut cancel = false;

        egui::Window::new("Nový server")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::Grid::new("new_session_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("Název:");
                    ui.text_edit_singleline(&mut form.name);
                    ui.end_row();

                    ui.label("Složka:");
                    ui.text_edit_singleline(&mut form.folder);
                    ui.end_row();

                    ui.label("Host:");
                    ui.text_edit_singleline(&mut form.host);
                    ui.end_row();

                    ui.label("Port:");
                    ui.text_edit_singleline(&mut form.port);
                    ui.end_row();

                    ui.label("Uživatel:");
                    ui.text_edit_singleline(&mut form.username);
                    ui.end_row();

                    ui.label("Heslo:");
                    ui.add(egui::TextEdit::singleline(&mut form.password).password(true));
                    ui.end_row();
                });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Přidat").clicked() {
                        submit = true;
                    }
                    if ui.button("Zrušit").clicked() {
                        cancel = true;
                    }
                });
            });

        if cancel {
            open = false;
        }

        if submit {
            let port: u16 = form.port.trim().parse().unwrap_or(22);
            let name = if form.name.trim().is_empty() { form.host.clone() } else { form.name.clone() };
            let mut session = Session::new(
                name,
                Protocol::Ssh,
                form.host.clone(),
                port,
                AuthMethod::Password {
                    username: form.username.clone(),
                    password: form.password.clone(),
                },
            );
            let folder = form.folder.trim();
            if !folder.is_empty() {
                session.group = Some(folder.to_string());
            }
            self.vault.data.servers.push(session);
            self.save_vault();
        } else if open {
            self.new_session_form = Some(form);
        }
    }

    fn show_new_folder_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut value) = self.new_folder_dialog.take() else { return };
        let mut open = true;
        let mut confirmed = false;
        let mut cancel = false;

        egui::Window::new("Nová složka")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Cesta nové složky (např. Práce/Nová):");
                ui.text_edit_singleline(&mut value);
                ui.horizontal(|ui| {
                    if ui.button("Vytvořit").clicked() {
                        confirmed = true;
                    }
                    if ui.button("Zrušit").clicked() {
                        cancel = true;
                    }
                });
            });

        if cancel {
            open = false;
        }

        if confirmed {
            let trimmed = value.trim().to_string();
            if !trimmed.is_empty() && !self.vault.data.folders.contains(&trimmed) {
                self.vault.data.folders.push(trimmed);
                self.save_vault();
            }
        } else if open {
            self.new_folder_dialog = Some(value);
        }
    }

    fn show_rename_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.rename_dialog.take() else { return };
        let mut open = true;
        let mut confirmed = false;
        let mut cancel = false;

        egui::Window::new("Přejmenovat")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.text_edit_singleline(&mut dialog.value);
                ui.horizontal(|ui| {
                    if ui.button("Uložit").clicked() {
                        confirmed = true;
                    }
                    if ui.button("Zrušit").clicked() {
                        cancel = true;
                    }
                });
            });

        if cancel {
            open = false;
        }

        if confirmed {
            let new_value = dialog.value.trim().to_string();
            if !new_value.is_empty() {
                match &dialog.target {
                    RenameTarget::Session(id) => {
                        if let Some(session) = self.vault.data.servers.iter_mut().find(|s| s.id == *id) {
                            session.name = new_value;
                        }
                    }
                    RenameTarget::Folder(old_path) => {
                        self.rename_folder(old_path, &new_value);
                    }
                }
                self.save_vault();
            }
        } else if open {
            self.rename_dialog = Some(dialog);
        }
    }

    fn show_move_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.move_dialog.take() else { return };
        let mut open = true;
        let mut confirmed = false;
        let mut cancel = false;

        egui::Window::new("Přesunout do složky")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Cesta složky (např. Práce/PBX), prázdné = kořenová úroveň:");
                ui.text_edit_singleline(&mut dialog.value);
                ui.horizontal(|ui| {
                    if ui.button("Přesunout").clicked() {
                        confirmed = true;
                    }
                    if ui.button("Zrušit").clicked() {
                        cancel = true;
                    }
                });
            });

        if cancel {
            open = false;
        }

        if confirmed {
            if let Some(session) = self.vault.data.servers.iter_mut().find(|s| s.id == dialog.session_id) {
                let trimmed = dialog.value.trim();
                session.group = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };
            }
            self.save_vault();
        } else if open {
            self.move_dialog = Some(dialog);
        }
    }

    fn show_delete_confirm(&mut self, ctx: &egui::Context) {
        let Some(target) = self.delete_confirm.take() else { return };
        let mut open = true;
        let mut confirmed = false;
        let mut cancel = false;

        let message = match &target {
            DeleteTarget::Session(id) => {
                let name = self
                    .vault
                    .data
                    .servers
                    .iter()
                    .find(|s| s.id == *id)
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                format!("Opravdu smazat server „{name}“?")
            }
            DeleteTarget::Folder(path) => format!("Opravdu smazat prázdnou složku „{path}“?"),
        };

        egui::Window::new("Smazat").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
            ui.label(&message);
            ui.horizontal(|ui| {
                if ui.button("Smazat").clicked() {
                    confirmed = true;
                }
                if ui.button("Zrušit").clicked() {
                    cancel = true;
                }
            });
        });

        if cancel {
            open = false;
        }

        if confirmed {
            match &target {
                DeleteTarget::Session(id) => {
                    self.vault.data.servers.retain(|s| s.id != *id);
                    self.tabs.retain(|t| !matches!(t, TabKind::Connection(sid) if sid == id));
                    if self.active_tab >= self.tabs.len() {
                        self.active_tab = self.tabs.len().saturating_sub(1);
                    }
                }
                DeleteTarget::Folder(path) => self.delete_folder(path),
            }
            self.save_vault();
        } else if open {
            self.delete_confirm = Some(target);
        }
    }

    // -- strom serveru -----------------------------------------------

    fn show_tree(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("+ Server").clicked() {
                self.new_session_form = Some(NewSessionForm::default());
            }
            if ui.button("+ Složka").clicked() {
                self.new_folder_dialog = Some(String::new());
            }
        });
        ui.separator();

        let tree = build_tree(&self.vault.data);
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.render_folder_contents(ui, &tree, "");
        });
    }

    fn render_folder_contents(&mut self, ui: &mut egui::Ui, node: &FolderNode, path_prefix: &str) {
        let mut actions: Vec<TreeAction> = Vec::new();

        for (name, child) in &node.children {
            let full_path = if path_prefix.is_empty() { name.clone() } else { format!("{path_prefix}/{name}") };
            let is_empty = child.children.is_empty() && child.session_ids.is_empty();

            let header = egui::CollapsingHeader::new(format!("📁 {name}")).id_salt(&full_path).default_open(false).show(ui, |ui| {
                self.render_folder_contents(ui, child, &full_path);
            });

            header.header_response.context_menu(|ui| {
                if ui.button("Přejmenovat složku...").clicked() {
                    actions.push(TreeAction::RenameFolder(full_path.clone()));
                    ui.close_menu();
                }
                if is_empty && ui.button("Smazat prázdnou složku").clicked() {
                    actions.push(TreeAction::DeleteFolder(full_path.clone()));
                    ui.close_menu();
                }
            });
        }

        for &id in &node.session_ids {
            self.render_session_row(ui, id, &mut actions);
        }

        for action in actions {
            self.apply_tree_action(action);
        }
    }

    fn render_session_row(&mut self, ui: &mut egui::Ui, id: Uuid, actions: &mut Vec<TreeAction>) {
        let Some(session) = self.vault.data.servers.iter().find(|s| s.id == id) else { return };
        let label = format!("{}  [{}] {}:{}", session.name, session.protocol, session.host, session.port);

        let response = ui.selectable_label(false, label);
        if response.double_clicked() {
            actions.push(TreeAction::Open(id));
        }
        response.context_menu(|ui| {
            if ui.button("Otevřít").clicked() {
                actions.push(TreeAction::Open(id));
                ui.close_menu();
            }
            if ui.button("Přejmenovat...").clicked() {
                actions.push(TreeAction::RenameSession(id));
                ui.close_menu();
            }
            if ui.button("Přesunout do složky...").clicked() {
                actions.push(TreeAction::MoveSession(id));
                ui.close_menu();
            }
            if ui.button("Smazat").clicked() {
                actions.push(TreeAction::DeleteSession(id));
                ui.close_menu();
            }
        });
    }

    fn apply_tree_action(&mut self, action: TreeAction) {
        match action {
            TreeAction::Open(id) => self.open_session_tab(id),
            TreeAction::RenameSession(id) => {
                if let Some(session) = self.vault.data.servers.iter().find(|s| s.id == id) {
                    self.rename_dialog = Some(RenameDialog {
                        target: RenameTarget::Session(id),
                        value: session.name.clone(),
                    });
                }
            }
            TreeAction::MoveSession(id) => {
                if let Some(session) = self.vault.data.servers.iter().find(|s| s.id == id) {
                    self.move_dialog = Some(MoveDialog {
                        session_id: id,
                        value: session.group.clone().unwrap_or_default(),
                    });
                }
            }
            TreeAction::DeleteSession(id) => {
                self.delete_confirm = Some(DeleteTarget::Session(id));
            }
            TreeAction::RenameFolder(path) => {
                let current_name = path.rsplit('/').next().unwrap_or(&path).to_string();
                self.rename_dialog = Some(RenameDialog {
                    target: RenameTarget::Folder(path),
                    value: current_name,
                });
            }
            TreeAction::DeleteFolder(path) => {
                self.delete_confirm = Some(DeleteTarget::Folder(path));
            }
        }
    }

    fn rename_folder(&mut self, old_path: &str, new_name: &str) {
        let parent = old_path.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
        let new_path = if parent.is_empty() { new_name.to_string() } else { format!("{parent}/{new_name}") };
        let old_prefix = format!("{old_path}/");
        let new_prefix = format!("{new_path}/");

        for session in self.vault.data.servers.iter_mut() {
            if let Some(group) = &session.group {
                if group == old_path {
                    session.group = Some(new_path.clone());
                } else if let Some(rest) = group.strip_prefix(&old_prefix) {
                    session.group = Some(format!("{new_prefix}{rest}"));
                }
            }
        }

        for folder in self.vault.data.folders.iter_mut() {
            if folder == old_path {
                *folder = new_path.clone();
            } else if let Some(rest) = folder.strip_prefix(&old_prefix) {
                *folder = format!("{new_prefix}{rest}");
            }
        }
    }

    fn delete_folder(&mut self, path: &str) {
        self.vault.data.folders.retain(|f| f != path);
    }
}

impl eframe::App for TermxApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.top_menu(ctx);

        self.show_new_session_dialog(ctx);
        self.show_new_folder_dialog(ctx);
        self.show_rename_dialog(ctx);
        self.show_move_dialog(ctx);
        self.show_delete_confirm(ctx);

        egui::SidePanel::left("session_tree")
            .resizable(true)
            .default_width(240.0)
            .width_range(160.0..=420.0)
            .show(ctx, |ui| {
                self.show_tree(ui);
            });

        if let Some(msg) = self.status_message.clone() {
            egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
                ui.colored_label(theme::ACCENT, msg);
            });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            self.tab_bar(ui);
            ui.separator();
            self.active_tab_content(ui);
        });
    }
}
