//! Hlavni stav a vykreslovani aplikace Term-IX: uvodni "zamcena"
//! obrazovka pro zadani/nastaveni hlavniho hesla trezoru, a po odemceni
//! hlavni okno - horni menu, levy strom serveru/slozek, horni lista
//! tabu a obsah aktivniho tabu.
//!
//! Zamerne vse v jednom souboru (mensi riziko chyb v pravidlech
//! viditelnosti mezi moduly, kdyz zde nejde spustit skutecny
//! `cargo build` - viz poznamka v `lib.rs`).
//!
//! Strukturu tvori dve vrstvy:
//! - [`TermxApp`] - vnejsi typ implementujici `eframe::App`, ktery jen
//!   drzi cestu k trezoru, registr modulu (nez se preda dal) a stav
//!   [`LockState`] (Locked/Unlocked). Zamcena obrazovka nema zadny
//!   pristup k datum trezoru - ten jeste neni odemceny.
//! - [`MainApp`] - puvodni "cela aplikace" (menu, strom, taby) tak, jak
//!   fungovala driv - vznikne az po uspesnem odemceni/vytvoreni trezoru
//!   a od te chvile uz drzi `Vault` primo (ne v `Option`), takze zbytek
//!   teto implementace se odemykanim vubec nemusi zabyvat.

use std::collections::BTreeMap;
use std::path::PathBuf;

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

/// Formular pro "rychle spojeni" - stejne udaje jako [`NewSessionForm`]
/// (krome slozky, ktera zde nedava smysl), ale vysledek se NEUKLADA do
/// trezoru - jen do docasneho `MainApp::ad_hoc_sessions` na dobu behu
/// aplikace. Pouziva se hlavne v hostovskem rezimu (bez zadaneho
/// hlavniho hesla), ale dostupne je odkudkoliv pro jednorazova spojeni,
/// ktera si uzivatel nechce ukladat.
struct QuickConnectForm {
    name: String,
    host: String,
    port: String,
    username: String,
    password: String,
}

impl Default for QuickConnectForm {
    fn default() -> Self {
        Self {
            name: String::new(),
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

#[derive(Default)]
struct ChangePasswordDialog {
    old: String,
    new1: String,
    new2: String,
    error: Option<String>,
}

/// Vyber toho, co se ma exportovat - ktere slozky (jejich cesty) a ktere
/// jednotlive servery (jejich id). Vychozi stav (viz [`ExportDialog::new`])
/// ma vybrane uplne vsechno, takze kdo si vyberem nechce zabyvat, dostane
/// puvodni chovani (export celeho trezoru) beze zmeny.
struct ExportDialog {
    path: String,
    password: String,
    confirm: String,
    error: Option<String>,
    selected_sessions: std::collections::HashSet<Uuid>,
    selected_folders: std::collections::HashSet<String>,
}

impl ExportDialog {
    fn new(data: &VaultData) -> Self {
        Self {
            path: String::new(),
            password: String::new(),
            confirm: String::new(),
            error: None,
            selected_sessions: data.servers.iter().map(|s| s.id).collect(),
            selected_folders: data.folders.iter().cloned().collect(),
        }
    }
}

#[derive(Default)]
struct ImportDialog {
    path: String,
    password: String,
    /// `false` (vychozi) = sloucit s aktualnim obsahem trezoru (pridat
    /// importovane servery/slozky k tem stavajicim). `true` = aktualni
    /// obsah trezoru cely nahradit importem.
    replace: bool,
    error: Option<String>,
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

/// Vykresli strom slozek/serveru se zaskrtavatky pro vyber exportu
/// (viz [`ExportDialog`]). Volna funkce misto metody `MainApp` zamerne -
/// pracuje jen s daty ze snapshotu `tree`/`data`, ktery uz je vytvoren
/// pred `.show()`, takze uvnitr UI closure nehrozi zadny konflikt s
/// pujckou `self` (stejny druh problemu, kvuli kteremu byly drive
/// opraveny E0499 chyby u ostatnich dialogu).
fn render_export_tree(
    ui: &mut egui::Ui,
    node: &FolderNode,
    path_prefix: &str,
    data: &VaultData,
    selected_sessions: &mut std::collections::HashSet<Uuid>,
    selected_folders: &mut std::collections::HashSet<String>,
) {
    for (name, child) in &node.children {
        let full_path = if path_prefix.is_empty() { name.clone() } else { format!("{path_prefix}/{name}") };
        let mut checked = selected_folders.contains(&full_path);
        let changed = ui.checkbox(&mut checked, format!("📁 {name}")).changed();
        if changed {
            set_subtree_selected(child, checked, selected_sessions, selected_folders, &full_path);
            if checked {
                selected_folders.insert(full_path.clone());
            } else {
                selected_folders.remove(&full_path);
            }
        }
        ui.indent(full_path.clone(), |ui| {
            render_export_tree(ui, child, &full_path, data, selected_sessions, selected_folders);
        });
    }

    for &id in &node.session_ids {
        if let Some(session) = data.servers.iter().find(|s| s.id == id) {
            let mut checked = selected_sessions.contains(&id);
            if ui.checkbox(&mut checked, &session.name).changed() {
                if checked {
                    selected_sessions.insert(id);
                } else {
                    selected_sessions.remove(&id);
                }
            }
        }
    }
}

/// Zaskrtne/odskrtne vsechny servery a podslozky pod danou slozkou
/// najednou - pouziva se, kdyz uzivatel v exportnim dialogu (od)zaskrtne
/// primo celou slozku.
fn set_subtree_selected(
    node: &FolderNode,
    selected: bool,
    selected_sessions: &mut std::collections::HashSet<Uuid>,
    selected_folders: &mut std::collections::HashSet<String>,
    path_prefix: &str,
) {
    for &id in &node.session_ids {
        if selected {
            selected_sessions.insert(id);
        } else {
            selected_sessions.remove(&id);
        }
    }
    for (name, child) in &node.children {
        let full_path = format!("{path_prefix}/{name}");
        if selected {
            selected_folders.insert(full_path.clone());
        } else {
            selected_folders.remove(&full_path);
        }
        set_subtree_selected(child, selected, selected_sessions, selected_folders, &full_path);
    }
}

/// Vsechna id serveru/cesty slozek v celem stromu - pro tlacitka
/// "Vybrat vše" / "Nic nevybírat" v exportnim dialogu.
fn all_selection(data: &VaultData) -> (std::collections::HashSet<Uuid>, std::collections::HashSet<String>) {
    (data.servers.iter().map(|s| s.id).collect(), data.folders.iter().cloned().collect())
}

/// Cela aplikace PO uspesnem odemceni/vytvoreni trezoru (nebo po
/// vstupu do hostovskeho rezimu, viz `is_guest`) - totozne s tim, jak
/// vypadal puvodni `TermxApp` pred pridanim zamcene obrazovky.
struct MainApp {
    vault: Vault,
    master_password: String,
    #[allow(dead_code)] // navazujici krok: napojeni Connection tabu na skutecny modul
    registry: ModuleRegistry,

    /// `true`, kdyz uzivatel na uvodni obrazovce zvolil "Pokračovat bez
    /// hesla" - `vault` je pak jen prazdny [`Vault::in_memory`] (nikdy
    /// se nezapisuje na disk) a UI pro spravu ulozenych serveru
    /// (pridavani/mazani/presouvani, zmena hesla, export) se schova -
    /// viz kontroly `if !self.is_guest` u prislusnych tlacitek/polozek
    /// menu. Otevrit jde jen "rychle spojeni" (`ad_hoc_sessions`).
    is_guest: bool,
    /// Docasna spojeni zalozena pres "Nové rychlé spojení..." - nikdy
    /// se neuklada do `vault`, zije jen po dobu behu aplikace. Dostupne
    /// v obou rezimech, ale hlavni ucel je hostovsky rezim.
    ad_hoc_sessions: Vec<Session>,

    tabs: Vec<TabKind>,
    active_tab: usize,

    new_session_form: Option<NewSessionForm>,
    new_folder_dialog: Option<String>,
    rename_dialog: Option<RenameDialog>,
    move_dialog: Option<MoveDialog>,
    delete_confirm: Option<DeleteTarget>,
    change_password_dialog: Option<ChangePasswordDialog>,
    quick_connect_form: Option<QuickConnectForm>,
    export_dialog: Option<ExportDialog>,
    import_dialog: Option<ImportDialog>,

    status_message: Option<String>,
}

impl MainApp {
    fn new(vault: Vault, master_password: String, registry: ModuleRegistry) -> Self {
        Self {
            vault,
            master_password,
            registry,
            is_guest: false,
            ad_hoc_sessions: Vec::new(),
            tabs: vec![TabKind::Home],
            active_tab: 0,
            new_session_form: None,
            new_folder_dialog: None,
            rename_dialog: None,
            move_dialog: None,
            delete_confirm: None,
            change_password_dialog: None,
            quick_connect_form: None,
            export_dialog: None,
            import_dialog: None,
            status_message: None,
        }
    }

    /// "Hostovsky" rezim - uzivatel na uvodni obrazovce nezadal hlavni
    /// heslo. Zadny trezor se necte ani nezapisuje (`Vault::in_memory`),
    /// takze strom je prazdny a pridavani/mazani serveru je v UI
    /// schovane; jedina dostupna cesta k pripojeni je "rychle spojeni"
    /// (`quick_connect_form`/`ad_hoc_sessions`), ktere si uzivatel musi
    /// zadat pri kazdem spusteni znovu.
    fn new_guest(registry: ModuleRegistry) -> Self {
        Self {
            vault: Vault::in_memory(),
            master_password: String::new(),
            registry,
            is_guest: true,
            ad_hoc_sessions: Vec::new(),
            tabs: vec![TabKind::Home],
            active_tab: 0,
            new_session_form: None,
            new_folder_dialog: None,
            rename_dialog: None,
            move_dialog: None,
            delete_confirm: None,
            change_password_dialog: None,
            quick_connect_form: None,
            export_dialog: None,
            import_dialog: None,
            status_message: None,
        }
    }

    fn save_vault(&mut self) {
        if let Err(e) = self.vault.save(&self.master_password) {
            self.status_message = Some(format!("Ulozeni trezoru selhalo: {e}"));
        }
    }

    /// Zavre (zahodi rozepsany stav) vsech ostatnich dialogovych oken -
    /// vola se VZDY pred otevrenim noveho dialogu, aby jich nemohlo byt
    /// otevrenych vic najednou (uzivatel nahlasil, ze "pop-up okna se
    /// vzajemne nezaviraji" a vznika chaos, kdyz jsou otevrena vsechna
    /// najednou).
    fn close_all_dialogs(&mut self) {
        self.new_session_form = None;
        self.new_folder_dialog = None;
        self.rename_dialog = None;
        self.move_dialog = None;
        self.delete_confirm = None;
        self.change_password_dialog = None;
        self.quick_connect_form = None;
        self.export_dialog = None;
        self.import_dialog = None;
    }

    /// Najde session podle id - nejdriv mezi ulozenymi servery v
    /// trezoru, pak (pokud tam neni) mezi docasnymi "rychlymi
    /// spojenimi". Diky tomu Connection tab funguje stejne, at uz je
    /// za nim ulozeny nebo jednorazovy server.
    fn find_session(&self, id: Uuid) -> Option<&Session> {
        self.vault.data.servers.iter().find(|s| s.id == id).or_else(|| self.ad_hoc_sessions.iter().find(|s| s.id == id))
    }

    // -- taby ---------------------------------------------------------

    fn tab_title(&self, kind: TabKind) -> String {
        match kind {
            TabKind::Home => "Domů".to_string(),
            TabKind::Settings => "Nastavení".to_string(),
            TabKind::Connection(id) => self.find_session(id).map(|s| s.name.clone()).unwrap_or_else(|| "Spojení".to_string()),
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
            if self.is_guest {
                ui.label("Hostovský režim: uložené servery nejsou dostupné.");
                ui.label("Otevřete rychlé spojení přes menu Sessions → Nové rychlé spojení...");
            } else {
                ui.label("Vyberte server vlevo, nebo přidejte nový přes menu Sessions.");
            }
        });
    }

    fn render_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Nastavení");
        ui.separator();
        if self.is_guest {
            ui.label(
                "Jste přihlášeni v hostovském režimu (bez hlavního hesla) - žádný trezor \
                 se nečte ani nezapisuje. Pro přístup k uloženým serverům aplikaci restartujte \
                 a zadejte hlavní heslo.",
            );
        } else {
            ui.label("Umístění trezoru:");
            ui.code(self.vault.path().display().to_string());
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Exportovat trezor...").clicked() {
                    let dialog = ExportDialog::new(&self.vault.data);
                    self.close_all_dialogs();
                    self.export_dialog = Some(dialog);
                }
                if ui.button("Importovat trezor...").clicked() {
                    self.close_all_dialogs();
                    self.import_dialog = Some(ImportDialog::default());
                }
            });
        }
        ui.add_space(12.0);
        ui.label(
            "Vzhled: v této verzi je k dispozici jen 'terminálové' tmavé téma. \
             Modernější téma přibude jako další volba zde.",
        );
    }

    fn render_connection(&mut self, ui: &mut egui::Ui, id: Uuid) {
        let Some(session) = self.find_session(id) else {
            ui.label("Tento server už neexistuje (byl smazán nebo šlo o dočasné rychlé spojení, které skončilo se zavřením tabu).");
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
                    if !self.is_guest {
                        if ui.button("Nový server...").clicked() {
                            self.close_all_dialogs();
                            self.new_session_form = Some(NewSessionForm::default());
                            ui.close_menu();
                        }
                        if ui.button("Nová složka...").clicked() {
                            self.close_all_dialogs();
                            self.new_folder_dialog = Some(String::new());
                            ui.close_menu();
                        }
                        ui.separator();
                    }
                    if ui.button("Nové rychlé spojení...").clicked() {
                        self.close_all_dialogs();
                        self.quick_connect_form = Some(QuickConnectForm::default());
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
                    if !self.is_guest && ui.button("Změnit heslo trezoru...").clicked() {
                        self.close_all_dialogs();
                        self.change_password_dialog = Some(ChangePasswordDialog::default());
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

    /// Zmena hlavniho hesla trezoru z bezicí aplikace (misto pres cmd
    /// pri startu). Trezor uz je odemceny (drzime `Vault` primo), takze
    /// zmena hesla znamena jen znovu zasifrovat aktualni obsah novym
    /// heslem (`Vault::save`) a od te chvile drzet v pameti uz jen to
    /// nove - stare heslo se overuje porovnanim s tim, ktere uz mame
    /// od odemceni v pameti (zadny dalsi pristup na disk neni potreba).
    fn show_change_password_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.change_password_dialog.take() else { return };
        let mut open = true;
        let mut confirmed = false;
        let mut cancel = false;

        egui::Window::new("Změnit heslo trezoru")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::Grid::new("change_password_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("Současné heslo:");
                    ui.add(egui::TextEdit::singleline(&mut dialog.old).password(true));
                    ui.end_row();

                    ui.label("Nové heslo:");
                    ui.add(egui::TextEdit::singleline(&mut dialog.new1).password(true));
                    ui.end_row();

                    ui.label("Zopakujte nové heslo:");
                    ui.add(egui::TextEdit::singleline(&mut dialog.new2).password(true));
                    ui.end_row();
                });

                if let Some(err) = &dialog.error {
                    ui.add_space(6.0);
                    ui.colored_label(egui::Color32::from_rgb(0xe0, 0x6c, 0x6c), err);
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Změnit").clicked() {
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
            if dialog.old != self.master_password {
                dialog.error = Some("Současné heslo je nesprávné.".to_string());
                self.change_password_dialog = Some(dialog);
                return;
            }
            if dialog.new1.is_empty() {
                dialog.error = Some("Nové heslo nesmí být prázdné.".to_string());
                self.change_password_dialog = Some(dialog);
                return;
            }
            if dialog.new1 != dialog.new2 {
                dialog.error = Some("Zadaná nová hesla se neshodují.".to_string());
                self.change_password_dialog = Some(dialog);
                return;
            }
            match self.vault.save(&dialog.new1) {
                Ok(()) => {
                    self.master_password = dialog.new1.clone();
                    self.status_message = Some("Heslo trezoru bylo změněno.".to_string());
                }
                Err(e) => {
                    dialog.error = Some(format!("Uložení trezoru s novým heslem selhalo: {e}"));
                    self.change_password_dialog = Some(dialog);
                }
            }
        } else if open {
            self.change_password_dialog = Some(dialog);
        }
    }

    /// "Rychlé spojení" - vytvoří dočasnou session, která se NEUKLÁDÁ do
    /// trezoru (jen do `ad_hoc_sessions` na dobu běhu aplikace), a rovnou
    /// otevře její Connection tab. Hlavní cesta k připojení v hostovském
    /// režimu, ale dostupné kdykoliv (i po přihlášení) pro spojení, které
    /// si uživatel nechce ukládat.
    fn show_quick_connect_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut form) = self.quick_connect_form.take() else { return };
        let mut open = true;
        let mut submit = false;
        let mut cancel = false;

        egui::Window::new("Nové rychlé spojení")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::Grid::new("quick_connect_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("Název:");
                    ui.text_edit_singleline(&mut form.name);
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

                ui.add_space(6.0);
                ui.label(egui::RichText::new("Toto spojení se nikam neukládá - platí jen do zavření tabu/aplikace.").small());

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Připojit").clicked() {
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
            let session = Session::new(
                name,
                Protocol::Ssh,
                form.host.clone(),
                port,
                AuthMethod::Password {
                    username: form.username.clone(),
                    password: form.password.clone(),
                },
            );
            let id = session.id;
            self.ad_hoc_sessions.push(session);
            self.open_session_tab(id);
        } else if open {
            self.quick_connect_form = Some(form);
        }
    }

    /// Vyexportuje vybranou cast trezoru (jednotlive servery a/nebo cele
    /// slozky, viz strom se zaskrtavatky v `ExportDialog`) do
    /// samostatneho sifrovaneho souboru (`Vault::export_data`) - klidne
    /// i s jinym heslem, nez ma hlavni trezor (napr. pro predani jen
    /// nekolika serveru kolegovi).
    fn show_export_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.export_dialog.take() else { return };
        let mut open = true;
        let mut confirmed = false;
        let mut cancel = false;
        let mut browse = false;
        let mut select_all = false;
        let mut select_none = false;

        // Snapshot stromu/dat pro vykresleni vyberu - stejny vzor jako
        // `show_tree` (`build_tree` je levne zavolat kazdy snimek), jen
        // se zde navic k otevirani slozek pridavaji zaskrtavatka.
        let tree = build_tree(&self.vault.data);

        egui::Window::new("Exportovat trezor")
            .collapsible(false)
            .resizable(true)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Co exportovat:");
                ui.horizontal(|ui| {
                    if ui.button("Vybrat vše").clicked() {
                        select_all = true;
                    }
                    if ui.button("Nic nevybírat").clicked() {
                        select_none = true;
                    }
                });
                ui.group(|ui| {
                    egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                        if tree.children.is_empty() && tree.session_ids.is_empty() {
                            ui.label(egui::RichText::new("Trezor je prázdný - není co exportovat.").small());
                        } else {
                            render_export_tree(
                                ui,
                                &tree,
                                "",
                                &self.vault.data,
                                &mut dialog.selected_sessions,
                                &mut dialog.selected_folders,
                            );
                        }
                    });
                });
                ui.add_space(10.0);

                ui.label("Cílový soubor:");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut dialog.path);
                    if ui.button("Procházet...").clicked() {
                        browse = true;
                    }
                });
                ui.add_space(6.0);
                egui::Grid::new("export_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("Heslo exportu:");
                    ui.add(egui::TextEdit::singleline(&mut dialog.password).password(true));
                    ui.end_row();

                    ui.label("Zopakujte heslo:");
                    ui.add(egui::TextEdit::singleline(&mut dialog.confirm).password(true));
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(
                        "Heslo exportu může být jiné než hlavní heslo trezoru - hodí se \
                         např. při předání serverů kolegovi.",
                    )
                    .small(),
                );

                if let Some(err) = &dialog.error {
                    ui.add_space(6.0);
                    ui.colored_label(egui::Color32::from_rgb(0xe0, 0x6c, 0x6c), err);
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Exportovat").clicked() {
                        confirmed = true;
                    }
                    if ui.button("Zrušit").clicked() {
                        cancel = true;
                    }
                });
            });

        // "Procházet..." se resi az po .show() (viz stejny vzor u
        // ostatnich dialogu) - nativni dialog souboru (`rfd`) je
        // blokujici volani, takze se hodi az po vykresleni tohoto snimku.
        if browse {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Exportovat trezor jako...")
                .set_file_name("term-ix-export.termx")
                .add_filter("Term-IX trezor", &["termx"])
                .save_file()
            {
                dialog.path = path.display().to_string();
            }
        }

        if select_all {
            let (sessions, folders) = all_selection(&self.vault.data);
            dialog.selected_sessions = sessions;
            dialog.selected_folders = folders;
        }
        if select_none {
            dialog.selected_sessions.clear();
            dialog.selected_folders.clear();
        }

        if cancel {
            open = false;
        }

        if confirmed {
            let path = dialog.path.trim().to_string();
            if path.is_empty() {
                dialog.error = Some("Zadejte cílový soubor.".to_string());
                self.export_dialog = Some(dialog);
                return;
            }
            if dialog.selected_sessions.is_empty() && dialog.selected_folders.is_empty() {
                dialog.error = Some("Vyberte alespoň jeden server nebo složku k exportu.".to_string());
                self.export_dialog = Some(dialog);
                return;
            }
            if dialog.password.is_empty() {
                dialog.error = Some("Heslo exportu nesmí být prázdné.".to_string());
                self.export_dialog = Some(dialog);
                return;
            }
            if dialog.password != dialog.confirm {
                dialog.error = Some("Zadaná hesla se neshodují.".to_string());
                self.export_dialog = Some(dialog);
                return;
            }
            // Vyexportuje se jen vybrana podmnozina - vybrane servery a
            // vybrane (explicitne existujici) prazdne slozky. Slozky,
            // ktere obsahuji jen vybrane servery, se do exportu dostanou
            // uz automaticky pres `Session::group` techto serveru, i
            // kdyby samotna cesta slozky nebyla v `selected_folders`.
            let filtered = VaultData {
                servers: self
                    .vault
                    .data
                    .servers
                    .iter()
                    .filter(|s| dialog.selected_sessions.contains(&s.id))
                    .cloned()
                    .collect(),
                folders: self
                    .vault
                    .data
                    .folders
                    .iter()
                    .filter(|f| dialog.selected_folders.contains(*f))
                    .cloned()
                    .collect(),
            };
            match Vault::export_data(&filtered, &path, &dialog.password) {
                Ok(()) => {
                    self.status_message = Some(format!(
                        "Trezor exportován do {path} ({} serverů, {} složek)",
                        filtered.servers.len(),
                        filtered.folders.len()
                    ));
                }
                Err(e) => {
                    dialog.error = Some(format!("Export selhal: {e}"));
                    self.export_dialog = Some(dialog);
                }
            }
        } else if open {
            self.export_dialog = Some(dialog);
        }
    }

    /// Nacte samostatny exportovany soubor (`Vault::import`) a jeho
    /// obsah bud sloucí s aktualnim trezorem, nebo (kdyz uzivatel
    /// zaskrtne "Nahradit") jim aktualni obsah cely nahradi.
    fn show_import_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.import_dialog.take() else { return };
        let mut open = true;
        let mut confirmed = false;
        let mut cancel = false;
        let mut browse = false;

        egui::Window::new("Importovat trezor")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("Zdrojový soubor:");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut dialog.path);
                    if ui.button("Procházet...").clicked() {
                        browse = true;
                    }
                });
                ui.add_space(6.0);
                ui.label("Heslo souboru:");
                ui.add(egui::TextEdit::singleline(&mut dialog.password).password(true));
                ui.add_space(8.0);
                ui.checkbox(&mut dialog.replace, "Nahradit aktuální trezor (místo sloučení)");
                ui.label(
                    egui::RichText::new(
                        "Sloučení přidá importované servery a složky k těm stávajícím. \
                         Nahrazení aktuální trezor kompletně přepíše obsahem importu.",
                    )
                    .small(),
                );

                if let Some(err) = &dialog.error {
                    ui.add_space(6.0);
                    ui.colored_label(egui::Color32::from_rgb(0xe0, 0x6c, 0x6c), err);
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Importovat").clicked() {
                        confirmed = true;
                    }
                    if ui.button("Zrušit").clicked() {
                        cancel = true;
                    }
                });
            });

        if browse {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Importovat trezor...")
                .add_filter("Term-IX trezor", &["termx"])
                .pick_file()
            {
                dialog.path = path.display().to_string();
            }
        }

        if cancel {
            open = false;
        }

        if confirmed {
            let path = dialog.path.trim().to_string();
            if path.is_empty() {
                dialog.error = Some("Zadejte zdrojový soubor.".to_string());
                self.import_dialog = Some(dialog);
                return;
            }
            match Vault::import(&path, &dialog.password) {
                Ok(imported) => {
                    if dialog.replace {
                        self.vault.data = imported;
                    } else {
                        for folder in imported.folders {
                            if !self.vault.data.folders.contains(&folder) {
                                self.vault.data.folders.push(folder);
                            }
                        }
                        self.vault.data.servers.extend(imported.servers);
                    }
                    self.save_vault();
                    self.status_message = Some("Trezor byl úspěšně importován.".to_string());
                }
                Err(e) => {
                    dialog.error = Some(format!("Import selhal: {e}"));
                    self.import_dialog = Some(dialog);
                }
            }
        } else if open {
            self.import_dialog = Some(dialog);
        }
    }

    // -- strom serveru -----------------------------------------------

    fn show_tree(&mut self, ui: &mut egui::Ui) {
        if self.is_guest {
            ui.label(
                egui::RichText::new("Hostovský režim - uložené servery nejsou vidět. Přihlaste se (restart aplikace) pro přístup k trezoru.")
                    .small(),
            );
            ui.separator();
            return;
        }

        ui.horizontal(|ui| {
            if ui.button("+ Server").clicked() {
                self.close_all_dialogs();
                self.new_session_form = Some(NewSessionForm::default());
            }
            if ui.button("+ Složka").clicked() {
                self.close_all_dialogs();
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
                    let value = session.name.clone();
                    self.close_all_dialogs();
                    self.rename_dialog = Some(RenameDialog { target: RenameTarget::Session(id), value });
                }
            }
            TreeAction::MoveSession(id) => {
                if let Some(session) = self.vault.data.servers.iter().find(|s| s.id == id) {
                    let value = session.group.clone().unwrap_or_default();
                    self.close_all_dialogs();
                    self.move_dialog = Some(MoveDialog { session_id: id, value });
                }
            }
            TreeAction::DeleteSession(id) => {
                self.close_all_dialogs();
                self.delete_confirm = Some(DeleteTarget::Session(id));
            }
            TreeAction::RenameFolder(path) => {
                let current_name = path.rsplit('/').next().unwrap_or(&path).to_string();
                self.close_all_dialogs();
                self.rename_dialog = Some(RenameDialog {
                    target: RenameTarget::Folder(path),
                    value: current_name,
                });
            }
            TreeAction::DeleteFolder(path) => {
                self.close_all_dialogs();
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

impl MainApp {
    /// Vykresli cely hlavni snimek aplikace. Neni to `eframe::App::update`
    /// primo - o dispatch mezi zamcenou obrazovkou a touto (odemcenou)
    /// aplikaci se stara vnejsi [`TermxApp`] nize.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.top_menu(ctx);

        self.show_new_session_dialog(ctx);
        self.show_new_folder_dialog(ctx);
        self.show_rename_dialog(ctx);
        self.show_move_dialog(ctx);
        self.show_delete_confirm(ctx);
        self.show_change_password_dialog(ctx);
        self.show_quick_connect_dialog(ctx);
        self.show_export_dialog(ctx);
        self.show_import_dialog(ctx);

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

// ---------------------------------------------------------------------
// Zamcena obrazovka (zadani/nastaveni hlavniho hesla trezoru) a vnejsi
// typ implementujici `eframe::App`.
// ---------------------------------------------------------------------

/// Stav zamcene obrazovky - jen to, co uzivatel prave pise, a pripadna
/// chybova hlaska z posledniho pokusu.
#[derive(Default)]
struct LockScreen {
    password: String,
    confirm: String,
    error: Option<String>,
}

enum LockState {
    Locked(LockScreen),
    Unlocked(MainApp),
}

/// Verejny vstupni bod GUI (viz `lib.rs`). Misto aby trezor odemykalo uz
/// `main.rs` pres cmd konzoli (`rpassword`), okno aplikace se otevre
/// rovnou a hlavni heslo se zadava zde - v jednoduche uvodni obrazovce
/// uvnitr okna samotneho, bez zadneho konzoloveho okna navic.
pub struct TermxApp {
    vault_path: PathBuf,
    /// Registr modulu ceka zde, dokud se trezor neodemkne/nevytvori -
    /// pak se `take()`-ne a preda do [`MainApp`]. `Option`, aby ho bylo
    /// mozne z `self` "vyjmout" bez klonovani (`ModuleRegistry` ho
    /// nema).
    registry: Option<ModuleRegistry>,
    state: LockState,
}

impl TermxApp {
    pub fn new(vault_path: PathBuf, registry: ModuleRegistry) -> Self {
        Self {
            vault_path,
            registry: Some(registry),
            state: LockState::Locked(LockScreen::default()),
        }
    }

    /// Vykresli uvodni obrazovku pro zadani (existujici trezor) nebo
    /// nastaveni (novy trezor) hlavniho hesla.
    ///
    /// Pozor na borrow checker: `egui::Window::open(&mut open)` si drzi
    /// mutable pujcku `open` po celou dobu `.show(...)` (stejny problem,
    /// jaky uz byl opraven u dialogovych oken v `MainApp` - viz tamni
    /// poznamky), takze i zde se pracuje jen s LOKALNIMI kopiemi
    /// (`password`, `confirm`, `error`) uvnitr UI closure, ne primo s
    /// `self` - do `self.state` se vysledek zapise az po navratu z
    /// `.show()`, kdy uz zadna pujcka aktivni neni.
    fn render_lock_screen(&mut self, ctx: &egui::Context) {
        let vault_exists = self.vault_path.exists();

        let LockState::Locked(screen) = &mut self.state else { return };
        let mut password = std::mem::take(&mut screen.password);
        let mut confirm = std::mem::take(&mut screen.confirm);
        let error = screen.error.clone();

        let mut submit = false;
        let mut skip = false;

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() / 2.0 - 160.0).max(24.0));
                ui.heading("Term-IX");
                ui.add_space(8.0);
                ui.label(if vault_exists {
                    "Zadejte hlavní heslo trezoru:"
                } else {
                    "Trezor ještě neexistuje – nastavte hlavní heslo:"
                });
                ui.add_space(10.0);

                let pw_resp = ui.add(
                    egui::TextEdit::singleline(&mut password)
                        .password(true)
                        .hint_text("Hlavní heslo")
                        .desired_width(260.0),
                );
                let enter_pressed = pw_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                if !vault_exists {
                    ui.add_space(4.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut confirm)
                            .password(true)
                            .hint_text("Zopakujte heslo")
                            .desired_width(260.0),
                    );
                }

                ui.add_space(10.0);
                let clicked = ui.button(if vault_exists { "Odemknout" } else { "Vytvořit trezor" }).clicked();
                if clicked || enter_pressed {
                    submit = true;
                }

                if let Some(err) = &error {
                    ui.add_space(8.0);
                    ui.colored_label(egui::Color32::from_rgb(0xe0, 0x6c, 0x6c), err);
                }

                if !vault_exists {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new("Pozor: při zapomenutí tohoto hesla se k uloženým údajům už nikdo nedostane.")
                            .small(),
                    );
                }

                ui.add_space(18.0);
                ui.separator();
                ui.add_space(6.0);
                if ui.button("Pokračovat bez hesla").clicked() {
                    skip = true;
                }
                ui.label(
                    egui::RichText::new(
                        "Hostovský režim: uložené servery nejsou vidět a nová se neukládají - \
                         jen rychlé spojení bez uložení.",
                    )
                    .small(),
                );
            });
        });

        // Pujcka `screen` z minula skoncila (posledni pouziti bylo pred
        // vstupem do .show()) - muzeme si ji vzit znovu.
        let LockState::Locked(screen) = &mut self.state else { return };
        screen.password = password.clone();
        screen.confirm = confirm.clone();

        if skip {
            let registry = self.registry.take().expect("registry byl jiz spotrebovan");
            self.state = LockState::Unlocked(MainApp::new_guest(registry));
            return;
        }

        if !submit {
            return;
        }

        let outcome = if vault_exists {
            Vault::unlock(&self.vault_path, &password).map_err(|e| format!("Nepodařilo se odemknout trezor: {e}"))
        } else if password.is_empty() {
            Err("Hlavní heslo nesmí být prázdné.".to_string())
        } else if password != confirm {
            Err("Zadaná hesla se neshodují.".to_string())
        } else {
            Vault::create(&self.vault_path, &password).map_err(|e| format!("Nepodařilo se vytvořit trezor: {e}"))
        };

        match outcome {
            Ok(vault) => {
                let registry = self.registry.take().expect("registry byl jiz spotrebovan");
                self.state = LockState::Unlocked(MainApp::new(vault, password, registry));
            }
            Err(e) => {
                let LockState::Locked(screen) = &mut self.state else { return };
                screen.error = Some(e);
            }
        }
    }
}

impl eframe::App for TermxApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let locked = matches!(self.state, LockState::Locked(_));
        if locked {
            self.render_lock_screen(ctx);
        } else if let LockState::Unlocked(app) = &mut self.state {
            app.update(ctx, frame);
        }
    }
}
