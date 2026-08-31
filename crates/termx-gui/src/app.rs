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
//!
//! POZNAMKA K OVERENI (obnoveni maximalizace okna, `TermxApp::update`/
//! `save`/`wants_maximized` a `lib.rs`): `ctx.input(|i| i.viewport().maximized)`
//! a `egui::ViewportCommand::Maximized(bool)` jsou standardni cast
//! viewport API zavedeneho v `egui`/`eframe` ~0.24+ (pouziva je i
//! oficialni `eframe` priklad vlastniho ramu okna), takze by mely v
//! pouzite verzi 0.29 byt spolehlive - ale pokud presto build selze
//! prave na nich, jde o izolovanou opravu jen techto par mist, zbytek
//! ukladani nastaveni (`eframe::get_value`/`set_value`) na tom
//! nezavisi.
//!
//! Prihlaseni z hostovskeho rezimu bez restartu (`render_guest_login`/
//! `submit_guest_login`) zadne nove API nepridava - pouziva stejne
//! `Vault::unlock`/`Vault::create` volani jako `TermxApp::render_lock_screen`,
//! jen vysledek zapise primo do jiz bezici `MainApp` instance misto
//! zalozeni nove.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use termx_core::{AuthMethod, ModuleRegistry, Protocol, Session};
use termx_update::LatestRelease;
use termx_vault::{Vault, VaultData};
use uuid::Uuid;

use crate::i18n::{self, Lang};
use crate::terminal;
use crate::theme;

/// Uzivatelska nastaveni aplikace (na rozdil od obsahu trezoru) - zatim
/// jen chovani pri ztrate SSH spojeni (viz `MainApp::render_settings`,
/// `terminal::TerminalSession::render`). Uklada se přes běžný eframe
/// perzistentní úložný prostor ("persistence" cargo feature u `eframe`,
/// uz drive vyuzivana pro `persist_window` v `lib.rs`) - obycejny
/// NEsifrovany soubor mimo trezor (zadne citlive udaje se sem
/// neukladaji), takze preziva restart aplikace i pro hostovsky rezim
/// (na rozdil od `vault`). Viz `TermxApp::new`/`TermxApp::save`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// Kdyz `true`, po ztrate SSH spojeni se aplikace sama periodicky
    /// pokousi pripojit znovu (viz `terminal::TerminalSession::maybe_auto_reconnect`).
    /// Kdyz `false` (vychozi), spojeni zustane prerusene a jen se
    /// obarvi prislusny tab (viz `tab_bar`) - obnovit jde pak jen rucne
    /// tlacitkem primo v tabu terminalu.
    #[serde(default)]
    pub auto_reconnect: bool,
    /// Jestli bylo hlavni okno naposledy MAXIMALIZOVANE (ne
    /// minimalizovane - to se zamerne vubec nesleduje ani neuklada, viz
    /// `TermxApp::update`) - obnovuje se pri pristim spusteni (viz
    /// `lib.rs::run_app`), navic k puvodni poloze/velikosti okna, kterou
    /// uz drive sam obnovuje `persist_window` (eframe).
    #[serde(default)]
    pub window_maximized: bool,
    /// Jazyk UI aplikace - viz [`crate::i18n::Lang`] a pozadavek "do
    /// nastavení bych dal možnost dropdown vybrat si jazyk". `#[serde(default)]`
    /// kvuli zpetne kompatibilite s nastavenimi ulozenymi pred zavedenim
    /// teto volby (chybejici pole ve starem souboru = `Lang::default()`,
    /// ne chyba pri nacitani).
    #[serde(default)]
    pub lang: Lang,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self { auto_reconnect: false, window_maximized: false, lang: Lang::default() }
    }
}

/// Cely graficky "wordmark" (znacka + napis "TERM-IX" pod ni v jednom
/// obrazku) pro Home tab (`render_home`) - viz zpetna vazba "na
/// hometabu bych použil term-ix_logo.png je tam i název pěkně v
/// obrázku". Predtim se tu pouzivala jen ctvercova ikona aplikace
/// (`lib.rs`, `assets/icons/hicolor/128x128/apps/term-ix.png`, bez
/// napisu) - ten obrazek zustava vyhrazeny pro ikonu okna/tasklisty,
/// tady uz se nepouziva. Stejny soubor jako `termx-splash` pro uvodni
/// "boot" obrazovku (`crates/termx-splash/src/lib.rs::LOGO_BYTES`).
const LOGO_BYTES: &[u8] = include_bytes!("../../../assets/term-ix_logo.png");

/// Stav kontroly dostupnosti nove verze (viz `MainApp::maybe_start_update_check`
/// a `MainApp::poll_update_check`) - zobrazuje se v Home tabu.
enum UpdateCheck {
    /// Jeste nezacalo (hned po startu aplikace/nacteni Home tabu se
    /// zmeni na `Checking`).
    NotStarted,
    /// Kontrola bezi na pozadi (samostatne vlakno - sitovy pozadavek
    /// na GitHub nesmi zablokovat vykreslovani GUI).
    Checking,
    /// Aktualni verze je nejnovejsi dostupna.
    UpToDate,
    /// Na GitHubu je dostupna novejsi verze.
    Available(LatestRelease),
    /// Kontrolu se nepodarilo provest (napr. bez pripojeni k internetu) -
    /// nikdy nesmi byt fatalni, jen se to takto tise zobrazi v Home tabu.
    Failed(String),
}

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
    /// `false` jen do prvniho vykresleni tohoto dialogu - pak se pole
    /// "Název:" samo fokusne, aby slo rovnou psat bez nutnosti tam
    /// nejdriv kliknout (viz pozadavek "+server , + složka, vyskakovacímu
    /// oknu bychom měli dát fokus na první okénko ať můžeme rovnou
    /// začít psát"). Stejny vzor jako uz drive `LockScreen::focus_requested`/
    /// `GuestLoginForm::focus_requested`.
    focus_requested: bool,
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
            focus_requested: false,
        }
    }
}

/// Formular pro editaci JIZ ULOZENEHO serveru (viz `show_edit_session_dialog`,
/// otevirany polozkou "Upravit..." v kontextovem menu stromu -
/// `render_session_row`) - stejna pole jako [`NewSessionForm`], jen
/// navic `id` puvodni session (aby `submit` vedel, kterou polozku v
/// `vault.data.servers` prepsat) a predvyplnena aktualnimi hodnotami
/// (viz `EditSessionForm::from_session`).
struct EditSessionForm {
    id: Uuid,
    name: String,
    folder: String,
    host: String,
    port: String,
    username: String,
    password: String,
    /// Viz `NewSessionForm::focus_requested` - stejny ucel (fokus na
    /// pole "Název:" hned pri otevreni dialogu).
    focus_requested: bool,
}

impl EditSessionForm {
    fn from_session(session: &Session) -> Self {
        // Aplikace zatim umi zakladat/editovat jen `AuthMethod::Password`
        // (viz `NewSessionForm`/`QuickConnectForm`/`HomeConnectForm`) -
        // u ostatnich variant (PrivateKey/Agent/None, zatim nikde v UI
        // nezalozitelnych) se pole proste predvyplni prazdna, at editaci
        // formulare nic neblokuje.
        let (username, password) = match &session.auth {
            AuthMethod::Password { username, password } => (username.clone(), password.clone()),
            _ => (String::new(), String::new()),
        };
        Self {
            id: session.id,
            name: session.name.clone(),
            folder: session.group.clone().unwrap_or_default(),
            host: session.host.clone(),
            port: session.port.to_string(),
            username,
            password,
            focus_requested: false,
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
    /// Viz `NewSessionForm::focus_requested`.
    focus_requested: bool,
}

impl Default for QuickConnectForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            host: String::new(),
            port: "22".to_string(),
            username: String::new(),
            password: String::new(),
            focus_requested: false,
        }
    }
}

/// Vestaveny formular primo na Home tabu pro rychle pripojeni k novemu
/// serveru (viz `MainApp::render_home`) - misto puvodniho prazdneho
/// mista pod logem, kam se logo+verze presunuly nize (feedback "logo
/// bych a info o verzi bych dal dolů ... do toho prostoru ... bych dal
/// formulář pro připojení k novému serveru"). Na rozdil od
/// [`NewSessionForm`]/[`QuickConnectForm`] (samostatne modalni dialogy
/// z menu Sessions, ktere zustavaji beze zmeny) jde o JEDEN spolecny
/// formular s prepinacem `save` - podle nej se pri "Připojit" bud'
/// (a) session ulozi do trezoru stejne jako `NewSessionForm`, nebo (b)
/// jen jako docasne `MainApp::ad_hoc_sessions` spojeni jako u
/// `QuickConnectForm` - v obou pripadech se navic (na rozdil od
/// `NewSessionForm`) hned otevre Connection tab, protoze smysl tohoto
/// formulare je primo "připojit", ne jen "přidat do seznamu".
struct HomeConnectForm {
    name: String,
    folder: String,
    host: String,
    port: String,
    username: String,
    password: String,
    save: bool,
}

impl Default for HomeConnectForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            folder: String::new(),
            host: String::new(),
            port: "22".to_string(),
            username: String::new(),
            password: String::new(),
            save: true,
        }
    }
}

/// Prihlasovaci formular zobrazeny v levem panelu MISTO stromu
/// ulozenych serveru, kdyz je aplikace v hostovskem rezimu (viz
/// `MainApp::render_guest_login`, volane ze `show_tree`) - zpetna
/// vazba "v hostovském režimu bychom mohli mít možnost se přihlásit...
/// okénko pro přihlášení v místě kde se ukazuje sloup uložených
/// serverů." Stejne udaje/logika jako na uvodni zamcene obrazovce
/// (`LockScreen`/`TermxApp::render_lock_screen`) - `confirm` se
/// pouziva jen kdyz trezor jeste vubec neexistuje (nastaveni noveho
/// hlavniho hesla misto prihlaseni k existujicimu).
#[derive(Default)]
struct GuestLoginForm {
    password: String,
    confirm: String,
    error: Option<String>,
    /// Stejny ucel jako `LockScreen::focus_requested` - pole hesla se
    /// ma fokusnout jen jednou (prvni snimek po vstupu do hostovskeho
    /// rezimu, nebo znovu po chybe), ne kazdy snimek.
    focus_requested: bool,
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

/// Rozpracovany stav dialogu "Nová složka" (viz `show_new_folder_dialog`).
/// Puvodne slo jen o holy `Option<String>` - rozsireno o `focus_requested`
/// (viz `NewSessionForm::focus_requested`), protoze i tento jednopolovy
/// dialog se ma pri otevreni sam fokusnout ("+server , + složka,
/// vyskakovacímu oknu bychom měli dát fokus na první okénko"). Tento
/// dialog navic (na rozdil od vicepolovych New/Edit server dialogu, kde
/// by Enter kolidoval s tim, ze uzivatel muze chtit Tabem/Enterem
/// prochazet vice poli) podporuje Enter = vytvorit, Esc = zrusit primo
/// (viz `field_matches`/pozadavek "Složka má jen jedno pole, tam můžeme
/// Enterem uložit esc vyskočit"), stejny vzor jako uz drive
/// `show_close_tab_confirm`.
struct NewFolderDialog {
    value: String,
    focus_requested: bool,
}

impl NewFolderDialog {
    fn new() -> Self {
        Self { value: String::new(), focus_requested: false }
    }
}

enum DeleteTarget {
    Session(Uuid),
    Folder(String),
}

/// Rozpracovany stav potvrzovaciho dialogu pro zavreni tabu s AKTIVNIM
/// (pripojenym) SSH spojenim - viz `tab_bar`/`show_close_tab_confirm`.
/// U tabu bez ziveho spojeni (Home/Nastaveni, nebo Connection tab, ktery
/// uz je odpojeny/se jeste teprve pripojuje) se zavira rovnou bez ptani.
struct CloseTabConfirm {
    idx: usize,
    title: String,
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
    /// Textovy filtr pro strom vyberu (hleda v nazvu serveru/hostu a v
    /// nazvech slozek) - u velkych trezoru (stovky serveru) je bez neho
    /// strom nepouzitelny, viz `render_export_tree`.
    filter: String,
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
            filter: String::new(),
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
    EditSession(Uuid),
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

/// `true`, pokud dany server odpovida (case-insensitive) filtru - podle
/// nazvu nebo hostu. Prazdny filtr odpovida vzdy.
fn session_matches_filter(session: &Session, filter_lower: &str) -> bool {
    filter_lower.is_empty() || session.name.to_lowercase().contains(filter_lower) || session.host.to_lowercase().contains(filter_lower)
}

/// `true`, pokud samotny nazev slozky, nebo cokoliv pod ni (server,
/// podslozka), odpovida filtru - pouziva se k rozhodnuti, jestli se
/// slozka v prohledavanem strome vubec ma zobrazit.
fn node_matches_filter(node: &FolderNode, name: &str, data: &VaultData, filter_lower: &str) -> bool {
    if filter_lower.is_empty() {
        return true;
    }
    if name.to_lowercase().contains(filter_lower) {
        return true;
    }
    if node.session_ids.iter().any(|id| data.servers.iter().find(|s| s.id == *id).is_some_and(|s| session_matches_filter(s, filter_lower))) {
        return true;
    }
    node.children.iter().any(|(cname, child)| node_matches_filter(child, cname, data, filter_lower))
}

/// Celkovy pocet serveru pod danym uzlem a kolik z nich je zrovna
/// vybranych - zobrazuje se jako "(vybráno/celkem)" u kazde slozky, aby
/// bylo u velkych trezoru na prvni pohled videt stav vyberu i bez
/// rozbalovani.
fn count_selection(node: &FolderNode, selected_sessions: &std::collections::HashSet<Uuid>) -> (usize, usize) {
    let mut total = node.session_ids.len();
    let mut selected = node.session_ids.iter().filter(|id| selected_sessions.contains(*id)).count();
    for child in node.children.values() {
        let (t, s) = count_selection(child, selected_sessions);
        total += t;
        selected += s;
    }
    (total, selected)
}

/// Vykresli strom slozek/serveru se zaskrtavatky pro vyber exportu
/// (viz [`ExportDialog`]). Volna funkce misto metody `MainApp` zamerne -
/// pracuje jen s daty ze snapshotu `tree`/`data`, ktery uz je vytvoren
/// pred `.show()`, takze uvnitr UI closure nehrozi zadny konflikt s
/// pujckou `self` (stejny druh problemu, kvuli kteremu byly drive
/// opraveny E0499 chyby u ostatnich dialogu).
///
/// U velkych trezoru (stovky serveru) by plochy seznam se vsemi
/// zaskrtavatky najednou rozbalenymi byl nepouzitelny, proto:
/// - slozky jsou sbalovaci (`CollapsingHeader`/`CollapsingState`),
///   vychozi stav ZAVRENO - dokud uzivatel nehleda, vidi jen nejvyssi
///   uroven a nemusi prochazet stovky radku;
/// - textovy filtr (`filter`) automaticky rozbali jen ty slozky, ve
///   kterych neco odpovida, a schova ostatni;
/// - u kazde slozky je videt pocet vybranych ze vsech pod ni, i kdyz je
///   zrovna zavrena.
fn render_export_tree(
    ui: &mut egui::Ui,
    node: &FolderNode,
    path_prefix: &str,
    data: &VaultData,
    filter_lower: &str,
    selected_sessions: &mut std::collections::HashSet<Uuid>,
    selected_folders: &mut std::collections::HashSet<String>,
) {
    for (name, child) in &node.children {
        if !node_matches_filter(child, name, data, filter_lower) {
            continue;
        }
        let full_path = if path_prefix.is_empty() { name.clone() } else { format!("{path_prefix}/{name}") };
        let (total, selected_count) = count_selection(child, selected_sessions);
        let mut checked = selected_folders.contains(&full_path);
        let mut folder_toggled = false;

        // `default_open` u `load_with_default_open` plati POUZE pri
        // prvnim vytvoreni stavu pro dane `id` - jakmile uz je pro tuto
        // slozku neco ulozene v pameti (napr. z prvniho snimku, kdy byl
        // filtr jeste prazdny a slozka se tedy vytvorila zavrena), dalsi
        // volani se stavem uz jen `default_open` ignoruje. Proto se tu
        // navic explicitne vola `set_open(true)`, kdyz se prave hleda a
        // slozka obsahuje shodu - jinak by hledani vizualne "nic
        // nedelalo", protoze uz jednou zavrena slozka by zustala zavrena
        // i po zadani textu do filtru (presne tenhle bug uzivatel
        // nahlasil).
        let id = ui.make_persistent_id(&full_path);
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
        if !filter_lower.is_empty() {
            state.set_open(true);
        }
        state
            .show_header(ui, |ui| {
                if ui.checkbox(&mut checked, "").changed() {
                    folder_toggled = true;
                }
                ui.label(format!("📁 {name} ({selected_count}/{total})"));
            })
            .body(|ui| {
                render_export_tree(ui, child, &full_path, data, filter_lower, selected_sessions, selected_folders);
            });

        if folder_toggled {
            set_subtree_selected(child, checked, selected_sessions, selected_folders, &full_path);
            if checked {
                selected_folders.insert(full_path.clone());
            } else {
                selected_folders.remove(&full_path);
            }
        }
    }

    for &id in &node.session_ids {
        if let Some(session) = data.servers.iter().find(|s| s.id == id) {
            if !session_matches_filter(session, filter_lower) {
                continue;
            }
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

/// Vystredi vyskakovaci okno (dialog) vuci hlavnimu oknu aplikace, misto
/// vychozi pozice, kterou by jinak `egui::Window` zvolil sam (typicky
/// blizko rohu, a u vice po sobe otevrenych dialogu porad na stejnem
/// miste - jednotlive dialogy by se pak prekryvaly).
///
/// `pivot(CENTER_CENTER)` znamena, ze zadana pozice je STRED okna (ne
/// jeho levy horni roh) - funguje tedy spravne i bez predem zname
/// velikosti okna, ktera se navic u nekterych dialogu behem zobrazeni
/// meni (napr. export po pridani chybove hlasky). `current_pos` (misto
/// `default_pos`) navic drzi okno uprostred KAZDY snimek, ne jen pri
/// prvnim zobrazeni - diky tomu se dialog vzdy objevi vystredeny, i
/// kdyby si ho egui z minula pamatovalo jinde. Dusledek: takto
/// vystredene okno uz jde s myslenim tahat za titulek - u techto
/// kratkych modalnich dialogu (bez titulkoveho pruhu, `collapsible(false)`)
/// to ale neni potreba.
fn centered_dialog<'o>(window: egui::Window<'o>, ctx: &egui::Context) -> egui::Window<'o> {
    window.pivot(egui::Align2::CENTER_CENTER).current_pos(ctx.screen_rect().center())
}

/// Jiz existujici hodnoty (`known` - cesty slozek, nazvy serveru nebo
/// hosty, viz volajici), ktere obsahuji aktualne napsany text - podklad
/// pro napovedu (`render_suggestion_chips`) - viz pozadavek "ve
/// formulářích bychom mohli nabízet doplňování třeba u názvů složek
/// atd. aby nedošlo ke zdvojení" a nasledne "stejnou nápovědu bych dal
/// i pro název a hosta". Presne aktualni hodnota se ze seznamu vyradi
/// (tu uz netreba nabizet - to uz je prave napsano).
fn field_matches(known: &[String], value: &str) -> Vec<String> {
    let query = value.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let query_lower = query.to_lowercase();
    known.iter().filter(|f| f.as_str() != value && f.to_lowercase().contains(&query_lower)).take(6).cloned().collect()
}

/// Vykresli klikaci "chipy" s `matches` (viz `field_matches`) - klik
/// cele pole prepise vybranou existujici hodnotou, takze si uzivatel
/// muze vybrat presne stejny zapis (velikost pismen atd.), misto aby
/// si preklepem/jinym pripadem pismen omylem vytvoril DRUHY, jinak
/// pojmenovany zaznam se stejnym vyznamem (slozku, server...). Volajici
/// jiz sam overil `!matches.is_empty()` (viz pouziti v Gridu, kde je
/// potreba vedet PREDEM, jestli se ma vubec pridavat dalsi radek).
fn render_suggestion_chips(ui: &mut egui::Ui, value: &mut String, matches: &[String]) {
    ui.horizontal_wrapped(|ui| {
        for m in matches {
            if ui.small_button(m.as_str()).clicked() {
                *value = m.clone();
            }
        }
    });
}

/// Vykresli jedno textove pole uvnitr `egui::Grid` (2 sloupce - popisek
/// uz volajici vypsal pred timto volanim) a hned pod nim, v samostatnem
/// radku Gridu, klikaci napovedu z `known` (viz `field_matches`/
/// `render_suggestion_chips`), pokud je vubec co nabidnout. Zamerne BEZ
/// podminky na fokus pole (drivejsi verze napovedu schovavala, jakmile
/// pole ztratilo fokus - coz nastalo uz pri kliknuti na samotnou
/// napovedu, takze zpetna vazba "když na ní kliknu zmizí ale nic se
/// nestane" mohla pusobit, ze se nic nedeje, i kdyz hodnota pole ve
/// skutecnosti prepsana byla) - napoveda ted zmizi jen tehdy, kdyz uz
/// neni co nabidnout (typicky prave DIKY tomu, ze kliknuti hodnotu
/// pole prepsalo na presne tu navrhovanou, ktera uz se sama sobe
/// nenabizi - viz `field_matches`).
///
/// Vraci `Response` samotneho textoveho pole (ne napovedy pod nim) -
/// volajici si na ni muze zavolat `.request_focus()`, pokud potrebuje
/// pole hned pri otevreni dialogu predfokusit (viz pozadavek
/// "vyskakovacímu oknu bychom měli dát fokus na první okénko ať můžeme
/// rovnou začít psát").
fn grid_field_with_suggestions(ui: &mut egui::Ui, value: &mut String, known: &[String]) -> egui::Response {
    let resp = ui.text_edit_singleline(value);
    ui.end_row();
    let matches = field_matches(known, value);
    if !matches.is_empty() {
        ui.label("");
        render_suggestion_chips(ui, value, &matches);
        ui.end_row();
    }
    resp
}

/// Cela aplikace PO uspesnem odemceni/vytvoreni trezoru (nebo po
/// vstupu do hostovskeho rezimu, viz `is_guest`) - totozne s tim, jak
/// vypadal puvodni `TermxApp` pred pridanim zamcene obrazovky.
struct MainApp {
    vault: Vault,
    /// Cesta k souboru trezoru na disku - stejna hodnota, jakou zna
    /// `TermxApp::vault_path`, jen predana sem, aby ji mel k dispozici i
    /// hostovsky rezim (`self.vault.path()` je v nem prazdna, viz
    /// `Vault::in_memory`) pro pripadne pozdejsi prihlaseni primo z
    /// bezicí aplikace - viz `render_guest_login`.
    vault_path: PathBuf,
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

    /// Bezici vestavene terminaly (viz `terminal.rs`) pro otevrene
    /// Connection taby - klic je id session. Zaznam se zaklada lene,
    /// az pri prvnim vykresleni daneho tabu (`render_connection`), a
    /// odstranuje se pri zavreni tabu/smazani serveru (`close_tab`,
    /// `apply_tree_action`) - zahozenim se cistě ukonci i prislusne
    /// pozadi bezici SSH vlakno (viz `termx_ssh::spawn_ssh_session`).
    terminal_sessions: std::collections::HashMap<Uuid, terminal::TerminalSession>,

    new_session_form: Option<NewSessionForm>,
    /// Editace jiz ulozeneho serveru - viz [`EditSessionForm`].
    edit_session_form: Option<EditSessionForm>,
    new_folder_dialog: Option<NewFolderDialog>,
    rename_dialog: Option<RenameDialog>,
    move_dialog: Option<MoveDialog>,
    delete_confirm: Option<DeleteTarget>,
    /// Rozepsany stav zmeny hesla trezoru - na rozdil od ostatnich
    /// "dialogovych" poli NENI `Option` (nema tedy "zavrit"/"otevrit"
    /// stav) - zmena hesla uz neni samostatny modalni dialog otevirany
    /// z menu, ale rovnou soucast Settings tabu (viz
    /// `render_change_password_section` a pozadavek "V nastavení
    /// bychom nemuseli mít podsložky Předvolby a Změnit heslo k
    /// trezoru... vše přímo bez podsložek").
    change_password_form: ChangePasswordDialog,
    quick_connect_form: Option<QuickConnectForm>,
    export_dialog: Option<ExportDialog>,
    import_dialog: Option<ImportDialog>,
    /// Potvrzeni zavreni tabu s aktivnim SSH spojenim - viz
    /// [`CloseTabConfirm`].
    close_tab_confirm: Option<CloseTabConfirm>,
    /// Vestaveny formular na Home tabu, viz [`HomeConnectForm`]. Na
    /// rozdil od ostatnich formularu/dialogu vyse NENI `Option` - je
    /// porad viditelny (soucast Home tabu, ne modalni okno), takze
    /// jednoduse zustava a prubezne se do nej ceka pise, dokud se
    /// nepouzije (`submit_home_connect`, ktera ho zase vrati na
    /// vychozi hodnotu) nebo se z Home tabu neodejde.
    home_connect_form: HomeConnectForm,
    /// Prihlasovaci formular pro hostovsky rezim, viz [`GuestLoginForm`].
    guest_login: GuestLoginForm,

    /// Uzivatelska nastaveni (viz [`AppSettings`]) - nacte se pri startu
    /// v `TermxApp::new` a preda sem, uklada se zpet v `TermxApp::save`.
    settings: AppSettings,

    /// Logo pro Home tab - nacte a nakesuje se jako GPU textura az pri
    /// prvnim vykresleni (`MainApp::logo_texture`), ne uz pri startu
    /// aplikace.
    logo_texture: Option<egui::TextureHandle>,
    /// Stav kontroly dostupnosti nove verze pro Home tab, viz
    /// [`UpdateCheck`].
    update_check: UpdateCheck,
    /// Prijimac vysledku kontroly aktualizace ze samostatneho vlakna
    /// (viz `maybe_start_update_check`) - `None`, kdyz zadna kontrola
    /// zrovna nebezi (jeste nezacala, nebo uz vysledek dorazil).
    update_rx: Option<std::sync::mpsc::Receiver<Result<Option<LatestRelease>, String>>>,

    status_message: Option<String>,
}

impl MainApp {
    fn new(vault: Vault, vault_path: PathBuf, master_password: String, registry: ModuleRegistry, settings: AppSettings) -> Self {
        Self {
            vault,
            vault_path,
            master_password,
            registry,
            is_guest: false,
            ad_hoc_sessions: Vec::new(),
            tabs: vec![TabKind::Home],
            active_tab: 0,
            terminal_sessions: std::collections::HashMap::new(),
            new_session_form: None,
            edit_session_form: None,
            new_folder_dialog: None,
            rename_dialog: None,
            move_dialog: None,
            delete_confirm: None,
            change_password_form: ChangePasswordDialog::default(),
            quick_connect_form: None,
            export_dialog: None,
            import_dialog: None,
            close_tab_confirm: None,
            home_connect_form: HomeConnectForm::default(),
            guest_login: GuestLoginForm::default(),
            settings,
            logo_texture: None,
            update_check: UpdateCheck::NotStarted,
            update_rx: None,
            status_message: None,
        }
    }

    /// "Hostovsky" rezim - uzivatel na uvodni obrazovce nezadal hlavni
    /// heslo. Zadny trezor se necte ani nezapisuje (`Vault::in_memory`),
    /// takze strom je prazdny a pridavani/mazani serveru je v UI
    /// schovane; k pripojeni jde pouzit "rychle spojeni"
    /// (`ad_hoc_sessions`), nebo se lze kdykoliv i dodatecne prihlasit k
    /// existujicimu/vytvorit novy trezor primo v levem panelu (viz
    /// `render_guest_login`) - `vault_path` je proto potreba znat i tady,
    /// i kdyz `self.vault` je do te doby jen `Vault::in_memory`.
    fn new_guest(vault_path: PathBuf, registry: ModuleRegistry, settings: AppSettings) -> Self {
        Self {
            vault: Vault::in_memory(),
            vault_path,
            master_password: String::new(),
            registry,
            is_guest: true,
            ad_hoc_sessions: Vec::new(),
            tabs: vec![TabKind::Home],
            active_tab: 0,
            terminal_sessions: std::collections::HashMap::new(),
            new_session_form: None,
            edit_session_form: None,
            new_folder_dialog: None,
            rename_dialog: None,
            move_dialog: None,
            delete_confirm: None,
            change_password_form: ChangePasswordDialog::default(),
            quick_connect_form: None,
            export_dialog: None,
            import_dialog: None,
            close_tab_confirm: None,
            // `save: false` - v hostovskem rezimu neni trezor kam
            // ukladat (viz `is_guest`), takze vychozi hodnota (jinak
            // `true`, viz `HomeConnectForm::default`) by tu byla
            // matouci; checkbox se navic v `render_home` u hosta vubec
            // nezobrazuje.
            home_connect_form: HomeConnectForm { save: false, ..HomeConnectForm::default() },
            guest_login: GuestLoginForm::default(),
            settings,
            logo_texture: None,
            update_check: UpdateCheck::NotStarted,
            update_rx: None,
            status_message: None,
        }
    }

    fn save_vault(&mut self) {
        if let Err(e) = self.vault.save(&self.master_password) {
            self.status_message = Some(format!("{}: {e}", i18n::t(self.settings.lang).vault_save_failed));
        }
    }

    /// Zavre (zahodi rozepsany stav) vsech ostatnich dialogovych oken -
    /// vola se VZDY pred otevrenim noveho dialogu, aby jich nemohlo byt
    /// otevrenych vic najednou (uzivatel nahlasil, ze "pop-up okna se
    /// vzajemne nezaviraji" a vznika chaos, kdyz jsou otevrena vsechna
    /// najednou).
    fn close_all_dialogs(&mut self) {
        self.new_session_form = None;
        self.edit_session_form = None;
        self.new_folder_dialog = None;
        self.rename_dialog = None;
        self.move_dialog = None;
        self.delete_confirm = None;
        self.quick_connect_form = None;
        self.export_dialog = None;
        self.import_dialog = None;
        self.close_tab_confirm = None;
    }

    /// Najde session podle id - nejdriv mezi ulozenymi servery v
    /// trezoru, pak (pokud tam neni) mezi docasnymi "rychlymi
    /// spojenimi". Diky tomu Connection tab funguje stejne, at uz je
    /// za nim ulozeny nebo jednorazovy server.
    fn find_session(&self, id: Uuid) -> Option<&Session> {
        self.vault.data.servers.iter().find(|s| s.id == id).or_else(|| self.ad_hoc_sessions.iter().find(|s| s.id == id))
    }

    /// Vsechny jiz existujici cesty slozek v trezoru - jak explicitne
    /// ulozene prazdne slozky (`vault.data.folders`), tak cesty pouzite
    /// nejakym serverem (`Session::group`). Podklad pro napovedu v poli
    /// "Složka:"/"Cesta..." ve formularich (viz `render_folder_suggestions`
    /// a pozadavek "ve formulářích bychom mohli nabízet doplňování třeba
    /// u názvů složek atd. aby nedošlo ke zdvojení") - `BTreeSet` dava
    /// bez dalsi prace serazeny seznam bez duplicit.
    fn known_folder_paths(&self) -> Vec<String> {
        let mut set: std::collections::BTreeSet<String> = self.vault.data.folders.iter().cloned().collect();
        for session in &self.vault.data.servers {
            if let Some(group) = &session.group {
                set.insert(group.clone());
            }
        }
        set.into_iter().collect()
    }

    /// Vsechny jiz pouzite nazvy serveru (ulozene i docasne rychla
    /// spojeni) - podklad pro napovedu v poli "Název:" (viz
    /// `grid_field_with_suggestions` a pozadavek "stejnou nápovědu bych
    /// dal i pro název a hosta").
    fn known_session_names(&self) -> Vec<String> {
        let mut set: std::collections::BTreeSet<String> =
            self.vault.data.servers.iter().map(|s| s.name.clone()).collect();
        for session in &self.ad_hoc_sessions {
            set.insert(session.name.clone());
        }
        set.into_iter().collect()
    }

    /// Vsechny jiz pouzite hosty (ulozene i docasne rychla spojeni) -
    /// podklad pro napovedu v poli "Host:" (viz `grid_field_with_suggestions`
    /// a pozadavek "stejnou nápovědu bych dal i pro název a hosta").
    fn known_hosts(&self) -> Vec<String> {
        let mut set: std::collections::BTreeSet<String> =
            self.vault.data.servers.iter().map(|s| s.host.clone()).collect();
        for session in &self.ad_hoc_sessions {
            set.insert(session.host.clone());
        }
        set.into_iter().collect()
    }

    // -- taby ---------------------------------------------------------

    fn tab_title(&self, kind: TabKind) -> String {
        let tr = i18n::t(self.settings.lang);
        match kind {
            TabKind::Home => tr.tab_home.to_string(),
            TabKind::Settings => tr.tab_settings.to_string(),
            TabKind::Connection(id) => self.find_session(id).map(|s| s.name.clone()).unwrap_or_else(|| tr.tab_connection_fallback.to_string()),
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
        // Zavreni tabu ukonci i prislusny bezici terminal (zahozenim
        // `input_tx` uvnitr `TerminalSession` se cistě ukonci pozadi
        // bezici SSH vlakno - viz `termx_ssh::spawn_ssh_session`).
        if let TabKind::Connection(id) = self.tabs[idx] {
            self.terminal_sessions.remove(&id);
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
        // Kdyz uzivatel zavira tab s AKTIVNIM (pripojenym) SSH
        // spojenim, misto rovnou zavrit se nejdriv zepta - viz
        // `show_close_tab_confirm`. Ostatni tab (Home/Nastaveni, nebo
        // Connection tab bez ziveho spojeni) se zavira rovnou, jako
        // driv.
        let mut to_confirm_close: Option<CloseTabConfirm> = None;

        let snapshot: Vec<TabKind> = self.tabs.clone();

        ui.horizontal_wrapped(|ui| {
            for (idx, &kind) in snapshot.iter().enumerate() {
                let selected = idx == self.active_tab;
                let title = self.tab_title(kind);

                // Zbarveni "mrtveho" (odpojeneho) SSH spojeni primo v
                // nazvu tabu, aby to bylo videt i bez jeho otevirani -
                // viz pozadavek "TAB by měl změnit nějakou barvou ať
                // víme že spojení je mrtvé".
                let conn_state = match kind {
                    TabKind::Connection(id) => self.terminal_sessions.get(&id).map(|s| s.state()),
                    _ => None,
                };
                let is_dead = conn_state == Some(terminal::ConnState::Disconnected);

                // Cely tab (puntik + nazev + zavirci "X") je JEDEN
                // spolecny `Frame` se sdilenym pozadim - drive to byly
                // dva samostatne widgety (`selectable_label` a
                // `small_button`), takze "X" mel svuj vlastni oddeleny
                // sedy ctverecek vedle pilulky s nazvem (zpetna vazba
                // "ikonka zavření by mohla být také součástí tabu má
                // šedé pozadí tak by to vypadalo dobře"). Barva pozadi
                // se preberi primo z aktualniho tematu - stejna, jakou
                // by drive pouzil `selectable_label` (vybrany) nebo
                // `small_button` (nevybrany), takze vzhled zustava
                // konzistentni se zbytkem UI.
                let bg = if selected { ui.visuals().selection.bg_fill } else { ui.visuals().widgets.inactive.bg_fill };

                egui::Frame::none().fill(bg).rounding(egui::Rounding::same(4.0)).inner_margin(egui::Margin::symmetric(8.0, 4.0)).show(
                    ui,
                    |ui| {
                        ui.horizontal(|ui| {
                            // O trochu vetsi mezera nez vychozi - jak si
                            // uzivatel vsimnul, s puvodni bylo cislo/puntik
                            // az moc u textu (zpetna vazba "ještě trochu
                            // posunout text").
                            ui.spacing_mut().item_spacing.x = 6.0;

                            if is_dead {
                                // Vykresleny puntik (ne textovy znak - viz
                                // predchozi commity a zpetna vazba k nim),
                                // ted jako radna soucast stejneho `Frame`
                                // jako zbytek tabu, takze uz k nemu
                                // vizualne jasne patri.
                                let (rect, _) =
                                    ui.allocate_exact_size(egui::vec2(8.0, ui.spacing().interact_size.y), egui::Sense::hover());
                                ui.painter().circle_filled(rect.center(), 4.0, theme::DANGER);
                            }

                            if ui.add(egui::Label::new(title.as_str()).sense(egui::Sense::click())).clicked() {
                                to_select = Some(idx);
                            }

                            if !matches!(kind, TabKind::Home) {
                                // Obycejne ASCII "X" (ne Unicode "✕"/Dingbats
                                // "×") - to druhe se v pouzitem pismu
                                // vykreslovalo jako ctverecek (chybejici
                                // znak). V klidu (bez najeti mysi) zadne
                                // vlastni pozadi nema, aby splyvalo s
                                // `Frame` vyse - ale na rozdil od drivejsiho
                                // `.frame(false)` (ktere VYPINALO uplne
                                // celou vizualizaci vc. najeti mysi) se ted
                                // jen ve `scope` docasne "vynuluje" barva
                                // KLIDOVEHO pozadi tlacitka, takze vychozi
                                // podbarveni pri najeti mysi (uz ted
                                // pouzivane jinde v aplikaci, viz
                                // `theme::apply`) zustava funkcni - zpetna
                                // vazba "při přejetí myší by se mohla
                                // ikonka zavření podbarvit".
                                let close_clicked = ui
                                    .scope(|ui| {
                                        let visuals = ui.visuals_mut();
                                        visuals.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                                        visuals.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
                                        ui.add(egui::Button::new(egui::RichText::new("X").small()))
                                    })
                                    .inner
                                    .clicked();
                                if close_clicked {
                                    if conn_state == Some(terminal::ConnState::Connected) {
                                        to_confirm_close = Some(CloseTabConfirm { idx, title: title.clone() });
                                    } else {
                                        to_close = Some(idx);
                                    }
                                }
                            }
                        });
                    },
                );
            }
        });

        if let Some(idx) = to_select {
            self.active_tab = idx;
        }
        if let Some(confirm) = to_confirm_close {
            self.close_tab_confirm = Some(confirm);
        } else if let Some(idx) = to_close {
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
            // Home/Nastaveni si drzi puvodni odsazeni od kraju plochy
            // (vypada to prirozeneji pro text/tlacitka) - zavedeno rucne
            // tady, protoze `CentralPanel` uz zadne vlastni nema (viz
            // `MainApp::update`).
            TabKind::Home => {
                egui::Frame::none().inner_margin(egui::Margin::symmetric(8.0, 8.0)).show(ui, |ui| {
                    self.render_home(ui);
                });
            }
            TabKind::Settings => {
                egui::Frame::none().inner_margin(egui::Margin::symmetric(8.0, 8.0)).show(ui, |ui| {
                    self.render_settings(ui);
                });
            }
            // SSH terminal naopak zadne dodatecne odsazeni nema - jde az
            // ke kraji obsahove plochy (viz pozadavek "okno terminálu
            // bychom mohli dotáhnout až ke kraji").
            TabKind::Connection(id) => self.render_connection(ui, id),
        }
    }

    /// Nacte logo aplikace jako GPU texturu (jen jednou, pak uz se
    /// vraci nakesovany `TextureHandle` - je levne ho klonovat, jde jen
    /// o referenci). Pouziva se v Home tabu (`render_home`).
    fn logo_texture(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        if self.logo_texture.is_none() {
            if let Ok(img) = image::load_from_memory(LOGO_BYTES) {
                let img = img.to_rgba8();
                let (w, h) = img.dimensions();
                let color_image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
                self.logo_texture = Some(ctx.load_texture("term-ix-logo", color_image, egui::TextureOptions::default()));
            }
        }
        self.logo_texture.clone()
    }

    /// Spusti kontrolu dostupnosti nove verze na samostatnem vlakne -
    /// nejvyse jednou za beh aplikace (viz `UpdateCheck::NotStarted`).
    /// Sitovy pozadavek nesmi bezet primo v render smycce, jinak by
    /// pri pomalem/vypadlem pripojeni zamrzlo cele GUI.
    fn maybe_start_update_check(&mut self) {
        if !matches!(self.update_check, UpdateCheck::NotStarted) {
            return;
        }
        self.update_check = UpdateCheck::Checking;
        let (tx, rx) = std::sync::mpsc::channel();
        let current_version = env!("CARGO_PKG_VERSION").to_string();
        std::thread::spawn(move || {
            let result = termx_update::check_latest_version(&current_version).map_err(|e| e.to_string());
            // Prijemce uz nemusi existovat (napr. aplikace se mezitim
            // zavrela) - poslani se pak proste nezdari, nic se nedeje.
            let _ = tx.send(result);
        });
        self.update_rx = Some(rx);
    }

    /// Vyzvedne vysledek kontroly aktualizace, pokud uz z pozadi
    /// dorazil (nikdy neceka - `try_recv`).
    fn poll_update_check(&mut self) {
        let Some(rx) = &self.update_rx else { return };
        match rx.try_recv() {
            Ok(Ok(Some(latest))) => {
                self.update_check = UpdateCheck::Available(latest);
                self.update_rx = None;
            }
            Ok(Ok(None)) => {
                self.update_check = UpdateCheck::UpToDate;
                self.update_rx = None;
            }
            Ok(Err(e)) => {
                self.update_check = UpdateCheck::Failed(e);
                self.update_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // Jeste nic - zkusi se znovu pristi snimek.
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.update_check = UpdateCheck::Failed("kontrola aktualizace neocekavane skoncila".to_string());
                self.update_rx = None;
            }
        }
    }

    /// Home tab - nahore vestaveny formular pro pripojeni k novemu
    /// serveru ([`HomeConnectForm`]), dole (kam se drive puvodne
    /// vykreslovalo hned na zacatku) logo/nazev/verze/stav aktualizace -
    /// viz feedback "logo bych a info o verzi bych dal dolů a do toho
    /// prostoru kde je ted logo a info o verzi bych dal formulář pro
    /// připojení k novému serveru".
    fn render_home(&mut self, ui: &mut egui::Ui) {
        let logo = self.logo_texture(ui.ctx());
        let mut submit = false;
        let tr = i18n::t(self.settings.lang);

        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.heading(tr.home_heading);
            ui.add_space(10.0);

            // `egui::Grid` bohuzel neni "center-aware" - i uvnitr
            // `vertical_centered` (viz vyse) zacina vzdy uplne vlevo v
            // ramci dostupne sirky sveho rodice (drzi se `ui.cursor()`,
            // ne stredoveho zarovnani jako normalni jednotlive widgety),
            // proto na obrazovce vysel formular pribity k levemu okraji
            // ("formulář dáme na střed"). Oprava: Grid se vlozi do
            // vlastniho `ui` s pevnou sirkou (`HOME_FORM_WIDTH`), pred
            // ktere se rucne vlozi polovina zbyvajiciho prostoru
            // (`ui.add_space`) - tim se cely blok manualne vystredi bez
            // ohledu na to, jak siroke zrovna Home tab je.
            const HOME_FORM_WIDTH: f32 = 340.0;
            let known_folders = self.known_folder_paths();
            let known_names = self.known_session_names();
            let known_hosts = self.known_hosts();
            ui.horizontal(|ui| {
                let avail = ui.available_width();
                if avail > HOME_FORM_WIDTH {
                    ui.add_space((avail - HOME_FORM_WIDTH) / 2.0);
                }
                ui.allocate_ui(egui::vec2(HOME_FORM_WIDTH, 0.0), |ui| {
                    egui::Grid::new("home_connect_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                        ui.label(tr.field_name);
                        grid_field_with_suggestions(ui, &mut self.home_connect_form.name, &known_names);

                        // Slozka dava smysl jen kdyz se bude i ukladat
                        // (viz `save` nize) - u hosta (zadny trezor) i u
                        // docasneho rychleho spojeni (`save == false`)
                        // se radek vubec nezobrazi.
                        if !self.is_guest && self.home_connect_form.save {
                            ui.label(tr.field_folder);
                            grid_field_with_suggestions(ui, &mut self.home_connect_form.folder, &known_folders);
                        }

                        ui.label(tr.field_host);
                        grid_field_with_suggestions(ui, &mut self.home_connect_form.host, &known_hosts);

                        ui.label(tr.field_port);
                        ui.text_edit_singleline(&mut self.home_connect_form.port);
                        ui.end_row();

                        ui.label(tr.field_username);
                        ui.text_edit_singleline(&mut self.home_connect_form.username);
                        ui.end_row();

                        ui.label(tr.field_password);
                        ui.add(egui::TextEdit::singleline(&mut self.home_connect_form.password).password(true));
                        ui.end_row();
                    });
                });
            });

            ui.add_space(6.0);
            if self.is_guest {
                // V hostovskem rezimu neni kam ukladat - zadny checkbox,
                // rovnou jen vysvetlujici poznamka (`save` uz je natvrdo
                // `false` z `MainApp::new_guest`).
                ui.label(egui::RichText::new(tr.home_guest_note).small());
            } else {
                ui.checkbox(&mut self.home_connect_form.save, tr.home_save_checkbox);
                ui.label(egui::RichText::new(tr.home_save_hint).small());
            }

            ui.add_space(10.0);
            if ui.add_enabled(!self.home_connect_form.host.trim().is_empty(), egui::Button::new(tr.btn_connect)).clicked() {
                submit = true;
            }

            ui.add_space(20.0);
            ui.separator();
            ui.add_space(16.0);

            if let Some(logo) = logo {
                // Zpetna vazba "logo můžeme ukázat větší na Domácím TABu" a
                // pak "na hometabu bych použil term-ix_logo.png je tam i
                // název pěkně v obrázku" - `LOGO_BYTES` uz je cely
                // "wordmark" (znacka + napis "TERM-IX" v jednom obrazku),
                // takze samostatny `ui.heading("Term-IX")` pod nim by uz
                // byl zbytecne duplicitni a byl odstranen.
                ui.add(egui::Image::new(&logo).max_size(egui::vec2(220.0, 220.0)));
                ui.add_space(8.0);
            }
            ui.label(format!("{} {}", tr.version_label, env!("CARGO_PKG_VERSION")));
            ui.label(egui::RichText::new("DaTTcz").small());
            ui.add_space(10.0);

            match &self.update_check {
                UpdateCheck::NotStarted | UpdateCheck::Checking => {
                    ui.label(egui::RichText::new(tr.checking_update).small());
                }
                UpdateCheck::UpToDate => {
                    ui.colored_label(theme::ACCENT, tr.up_to_date);
                }
                UpdateCheck::Available(latest) => {
                    ui.colored_label(egui::Color32::from_rgb(0xe6, 0xc2, 0x5a), i18n::update_available(self.settings.lang, &latest.version));
                    if ui.button(tr.btn_open_release_page).clicked() {
                        ui.ctx().open_url(egui::OpenUrl {
                            url: latest.url.clone(),
                            new_tab: true,
                        });
                    }
                }
                UpdateCheck::Failed(e) => {
                    ui.label(egui::RichText::new(i18n::update_check_failed(self.settings.lang, e)).small());
                }
            }
        });

        if submit {
            self.submit_home_connect();
        }
    }

    /// Zpracovani tlacitka "Připojit" v [`HomeConnectForm`] - podle
    /// `form.save` bud' ulozi novou session do trezoru (stejne jako
    /// `show_new_session_dialog`), nebo ji zalozi jen jako docasnou
    /// (`ad_hoc_sessions`, stejne jako `show_quick_connect_dialog`), a v
    /// obou pripadech rovnou otevre jeji Connection tab. Hodnoty z
    /// formulare se napred zkopiruji do lokalnich promennych (ne
    /// pujcka `&self.home_connect_form` drzena pres cele telo metody),
    /// aby slo hned nato normalne pujcit `self` mutable (`self.vault`,
    /// `self.ad_hoc_sessions`, `self.open_session_tab`) - stejny duvod,
    /// proc `render_connection` drive kopiruje (`.cloned()`) session
    /// misto drzeni reference.
    fn submit_home_connect(&mut self) {
        let host = self.home_connect_form.host.trim().to_string();
        if host.is_empty() {
            return;
        }
        let port: u16 = self.home_connect_form.port.trim().parse().unwrap_or(22);
        let name = if self.home_connect_form.name.trim().is_empty() { host.clone() } else { self.home_connect_form.name.clone() };
        let username = self.home_connect_form.username.clone();
        let password = self.home_connect_form.password.clone();
        let folder = self.home_connect_form.folder.trim().to_string();
        let save = self.home_connect_form.save && !self.is_guest;

        let mut session = Session::new(name, Protocol::Ssh, host, port, AuthMethod::Password { username, password });
        let id = session.id;

        if save {
            if !folder.is_empty() {
                session.group = Some(folder);
            }
            self.vault.data.servers.push(session);
            self.save_vault();
        } else {
            self.ad_hoc_sessions.push(session);
        }

        self.open_session_tab(id);
        // Ceka minule pouzita hodnota `save` (pokud neni host) se
        // zamerne zachova do dalsiho formulare - kdo si uklada servery,
        // si je typicky bude ukladat i priste.
        let keep_save = self.home_connect_form.save;
        self.home_connect_form = HomeConnectForm { save: keep_save, ..HomeConnectForm::default() };
    }

    fn render_settings(&mut self, ui: &mut egui::Ui) {
        let tr = i18n::t(self.settings.lang);
        ui.heading(tr.settings_heading);
        ui.separator();
        if self.is_guest {
            ui.label(tr.settings_guest_note);
        } else {
            ui.label(tr.settings_vault_location);
            ui.code(self.vault.path().display().to_string());
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button(tr.btn_export_vault).clicked() {
                    let dialog = ExportDialog::new(&self.vault.data);
                    self.close_all_dialogs();
                    self.export_dialog = Some(dialog);
                }
                if ui.button(tr.btn_import_vault).clicked() {
                    self.close_all_dialogs();
                    self.import_dialog = Some(ImportDialog::default());
                }
            });

            ui.add_space(18.0);
            ui.separator();
            ui.add_space(12.0);
            self.render_change_password_section(ui);
        }

        ui.add_space(18.0);
        ui.separator();
        ui.add_space(12.0);
        ui.heading(tr.settings_language_heading);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(tr.settings_language_label);
            // Jazyk se cte/uklada primo v `self.settings.lang` -
            // zmenene UI se hned od pristiho snimku vykresli v novem
            // jazyce (`tr` vyse se pocita znovu kazdy snimek), zadny
            // restart aplikace neni potreba. Zaroven je to bezna
            // `AppSettings`, takze volba preziva restart (`TermxApp::save`).
            egui::ComboBox::from_id_salt("settings_lang_combo")
                .selected_text(self.settings.lang.native_name())
                .show_ui(ui, |ui| {
                    for lang in Lang::ALL {
                        ui.selectable_value(&mut self.settings.lang, lang, lang.native_name());
                    }
                });
        });

        // Vzhled/tema - zpetna vazba "to bych nedával pod heslo trezoru.
        // ale v dalším oddíle asi pod jazyk" - puvodne to byl jen radek
        // textu hned pod sekci zmeny hesla trezoru, ted vlastni oddil
        // hned pod Jazykem (podobne jako ostatni sekce nize).
        ui.add_space(18.0);
        ui.separator();
        ui.add_space(12.0);
        ui.heading(tr.settings_appearance_heading);
        ui.add_space(4.0);
        ui.label(tr.settings_theme_note);

        ui.add_space(18.0);
        ui.separator();
        ui.add_space(12.0);
        ui.heading(tr.settings_ssh_loss_heading);
        ui.add_space(4.0);
        ui.checkbox(&mut self.settings.auto_reconnect, tr.settings_auto_reconnect_checkbox);
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tr.settings_auto_reconnect_note).small());
    }

    fn render_connection(&mut self, ui: &mut egui::Ui, id: Uuid) {
        // `.cloned()` zamerne - `find_session` pujcuje `self` jako celek,
        // takze kdybychom si `session` drzeli jen jako referenci, nesel
        // by hned pod tim pouzit `self.terminal_sessions` (mutable
        // pujcka `self`) - stejny druh problemu, jaky uz byl v tomto
        // souboru drive opraven u dialogovych oken. `Session` je levne
        // klonovatelna (`derive(Clone)` v termx-core).
        let Some(session) = self.find_session(id).cloned() else {
            // Tento tab nema (na rozdil od SSH terminalu) duvod jit az
            // ke kraji - drobne odsazeni jen pro tuto textovou hlasku.
            egui::Frame::none().inner_margin(egui::Margin::symmetric(8.0, 8.0)).show(ui, |ui| {
                ui.label(i18n::t(self.settings.lang).connection_gone);
            });
            return;
        };

        if session.protocol != Protocol::Ssh {
            egui::Frame::none().inner_margin(egui::Margin::symmetric(8.0, 8.0)).show(ui, |ui| {
                ui.heading(&session.name);
                ui.add_space(8.0);
                ui.label(i18n::protocol_not_supported(self.settings.lang, session.protocol));
            });
            return;
        }

        self.terminal_sessions.entry(id).or_insert_with(|| terminal::TerminalSession::new(&session));

        if let Some(term_session) = self.terminal_sessions.get_mut(&id) {
            term_session.render(ui, self.settings.auto_reconnect, self.settings.lang);
        }
    }

    // -- horni menu -----------------------------------------------------

    fn top_menu(&mut self, ctx: &egui::Context) {
        let tr = i18n::t(self.settings.lang);
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button(tr.menu_terminal, |ui| {
                    if ui.button(tr.menu_terminal_exit).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        ui.close_menu();
                    }
                });
                ui.menu_button(tr.menu_sessions, |ui| {
                    if !self.is_guest {
                        if ui.button(tr.menu_sessions_new_server).clicked() {
                            self.close_all_dialogs();
                            self.new_session_form = Some(NewSessionForm::default());
                            ui.close_menu();
                        }
                        if ui.button(tr.menu_sessions_new_folder).clicked() {
                            self.close_all_dialogs();
                            self.new_folder_dialog = Some(NewFolderDialog::new());
                            ui.close_menu();
                        }
                        ui.separator();
                    }
                    if ui.button(tr.menu_sessions_new_quick_connect).clicked() {
                        self.close_all_dialogs();
                        self.quick_connect_form = Some(QuickConnectForm::default());
                        ui.close_menu();
                    }
                });
                ui.menu_button(tr.menu_view, |_ui| {});
                ui.menu_button(tr.menu_tools, |_ui| {});
                // Zpetna vazba "V nastavení bychom nemuseli mít podsložky
                // Předvolby a Změnit heslo k trezoru... zatím tam toho
                // nemáme tolik takže bych dal vše přímo bez podsložek" -
                // "Nastavení" uz proto neni rozklikavaci menu se dvema
                // polozkami, ale rovnou tlacitko, ktere primo otevre
                // Settings tab (`open_settings_tab`); zmena hesla trezoru
                // uz neni samostatny dialog otevirany odtud, ale soucast
                // primo Settings tabu (`render_settings`/
                // `render_change_password_section`).
                if ui.button(tr.menu_settings).clicked() {
                    self.open_settings_tab();
                }
                ui.menu_button(tr.menu_help, |ui| {
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
        let tr = i18n::t(self.settings.lang);
        let known_folders = self.known_folder_paths();
        let known_names = self.known_session_names();
        let known_hosts = self.known_hosts();

        centered_dialog(egui::Window::new(tr.dialog_new_server_title), ctx)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::Grid::new("new_session_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label(tr.field_name);
                    let name_resp = grid_field_with_suggestions(ui, &mut form.name, &known_names);
                    if !form.focus_requested {
                        name_resp.request_focus();
                        form.focus_requested = true;
                    }

                    ui.label(tr.field_folder);
                    grid_field_with_suggestions(ui, &mut form.folder, &known_folders);

                    ui.label(tr.field_host);
                    grid_field_with_suggestions(ui, &mut form.host, &known_hosts);

                    ui.label(tr.field_port);
                    ui.text_edit_singleline(&mut form.port);
                    ui.end_row();

                    ui.label(tr.field_username);
                    ui.text_edit_singleline(&mut form.username);
                    ui.end_row();

                    ui.label(tr.field_password);
                    ui.add(egui::TextEdit::singleline(&mut form.password).password(true));
                    ui.end_row();
                });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(tr.btn_add).clicked() {
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

    /// Editace jiz ulozeneho serveru - otevirano polozkou "Upravit..."
    /// v kontextovem menu stromu (`render_session_row`/`TreeAction::EditSession`).
    /// Stejny formular jako `show_new_session_dialog`, jen predvyplneny
    /// (viz `EditSessionForm::from_session`) a pri odeslani PREPISUJE
    /// existujici polozku v `vault.data.servers` (podle `form.id`) misto
    /// pridani nove.
    fn show_edit_session_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut form) = self.edit_session_form.take() else { return };
        let mut open = true;
        let mut submit = false;
        let mut cancel = false;
        let tr = i18n::t(self.settings.lang);
        let known_folders = self.known_folder_paths();
        let known_names = self.known_session_names();
        let known_hosts = self.known_hosts();

        centered_dialog(egui::Window::new(tr.dialog_edit_server_title), ctx)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::Grid::new("edit_session_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label(tr.field_name);
                    let name_resp = grid_field_with_suggestions(ui, &mut form.name, &known_names);
                    if !form.focus_requested {
                        name_resp.request_focus();
                        form.focus_requested = true;
                    }

                    ui.label(tr.field_folder);
                    grid_field_with_suggestions(ui, &mut form.folder, &known_folders);

                    ui.label(tr.field_host);
                    grid_field_with_suggestions(ui, &mut form.host, &known_hosts);

                    ui.label(tr.field_port);
                    ui.text_edit_singleline(&mut form.port);
                    ui.end_row();

                    ui.label(tr.field_username);
                    ui.text_edit_singleline(&mut form.username);
                    ui.end_row();

                    ui.label(tr.field_password);
                    ui.add(egui::TextEdit::singleline(&mut form.password).password(true));
                    ui.end_row();
                });

                ui.add_space(8.0);
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
            // Server mezitim mohl byt smazan (napr. z jineho otevreneho
            // dialogu) - v tom pripade proste neni co ulozit. `found`
            // drzi vysledek jen do konce pujcky `session` (`iter_mut`),
            // aby `self.save_vault()` pod tim slo zavolat bez konfliktu
            // pujcek (`session` by jinak drzela mutable pujcku `self.vault`
            // po celou dobu tohoto bloku).
            let found = if let Some(session) = self.vault.data.servers.iter_mut().find(|s| s.id == form.id) {
                let port: u16 = form.port.trim().parse().unwrap_or(22);
                session.name = if form.name.trim().is_empty() { form.host.clone() } else { form.name.clone() };
                session.host = form.host.clone();
                session.port = port;
                session.auth = AuthMethod::Password { username: form.username.clone(), password: form.password.clone() };
                let folder = form.folder.trim();
                session.group = if folder.is_empty() { None } else { Some(folder.to_string()) };
                true
            } else {
                false
            };
            if found {
                self.save_vault();
            }
        } else if open {
            self.edit_session_form = Some(form);
        }
    }

    fn show_new_folder_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.new_folder_dialog.take() else { return };
        let mut open = true;
        let mut confirmed = false;
        // Esc = zrusit - cteno primo z `ctx`, protoze v tomto dialogu
        // neni jine textove pole ani tlacitko, ktere by na Esc melo
        // vlastni vyznam (stejny vzor jako `show_close_tab_confirm`).
        let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let tr = i18n::t(self.settings.lang);
        let known_folders = self.known_folder_paths();

        centered_dialog(egui::Window::new(tr.dialog_new_folder_title), ctx)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(tr.new_folder_path_hint);
                let value_resp = ui.text_edit_singleline(&mut dialog.value);
                // Viz `NewSessionForm::focus_requested` - fokus na jedine
                // pole tohoto dialogu hned pri otevreni.
                if !dialog.focus_requested {
                    value_resp.request_focus();
                }
                dialog.focus_requested = true;
                // Enter = vytvorit - jelikoz je v dialogu jen jedno pole,
                // Enter v nem jednoznacne znamena "hotovo" (viz pozadavek
                // "Složka má jen jedno pole, tam můžeme Enterem uložit").
                if value_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    confirmed = true;
                }
                let matches = field_matches(&known_folders, &dialog.value);
                if !matches.is_empty() {
                    render_suggestion_chips(ui, &mut dialog.value, &matches);
                }
                ui.horizontal(|ui| {
                    if ui.button(tr.btn_create).clicked() {
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
            let trimmed = dialog.value.trim().to_string();
            if !trimmed.is_empty() && !self.vault.data.folders.contains(&trimmed) {
                self.vault.data.folders.push(trimmed);
                self.save_vault();
            }
        } else if open {
            self.new_folder_dialog = Some(dialog);
        }
    }

    fn show_rename_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.rename_dialog.take() else { return };
        let mut open = true;
        let mut confirmed = false;
        let mut cancel = false;
        let tr = i18n::t(self.settings.lang);

        centered_dialog(egui::Window::new(tr.dialog_rename_title), ctx)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.text_edit_singleline(&mut dialog.value);
                ui.horizontal(|ui| {
                    if ui.button(tr.btn_save).clicked() {
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
        let tr = i18n::t(self.settings.lang);
        let known_folders = self.known_folder_paths();

        centered_dialog(egui::Window::new(tr.dialog_move_title), ctx)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(tr.move_folder_path_hint);
                ui.text_edit_singleline(&mut dialog.value);
                let matches = field_matches(&known_folders, &dialog.value);
                if !matches.is_empty() {
                    render_suggestion_chips(ui, &mut dialog.value, &matches);
                }
                ui.horizontal(|ui| {
                    if ui.button(tr.btn_move).clicked() {
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
        let tr = i18n::t(self.settings.lang);

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
                i18n::confirm_delete_server(self.settings.lang, &name)
            }
            DeleteTarget::Folder(path) => i18n::confirm_delete_folder(self.settings.lang, path),
        };

        centered_dialog(egui::Window::new(tr.dialog_delete_title), ctx).collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
            ui.label(&message);
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
            match &target {
                DeleteTarget::Session(id) => {
                    self.vault.data.servers.retain(|s| s.id != *id);
                    self.tabs.retain(|t| !matches!(t, TabKind::Connection(sid) if sid == id));
                    self.terminal_sessions.remove(id);
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

    /// Potvrzeni pred zavrenim tabu s AKTIVNIM (pripojenym) SSH
    /// spojenim - viz `tab_bar` (kde se `close_tab_confirm` nastavuje) a
    /// pozadavek "když ho dám vypnout tak mě poprosí o potvrzení zda ho
    /// chci opravdu vypnout". Stejny vzor jako `show_delete_confirm`
    /// vyse.
    fn show_close_tab_confirm(&mut self, ctx: &egui::Context) {
        let Some(confirm) = self.close_tab_confirm.take() else { return };
        let mut open = true;
        // Enter = potvrdit zavreni, Esc = zrusit (zavrit dialog beze
        // zmeny) - viz pozadavek "enter potvrdí zavření, esc zmizí
        // dialog". Cteno primo z `ctx` (ne z konkretniho widgetu uvnitr
        // okna), protoze v tomto dialogu neni zadne textove pole, ktere
        // by melo fokus drzet - okno je jedine otevrene modalni okno v
        // danou chvili, takze globalni stisk klavesy tady jednoznacne
        // patri jemu.
        let mut confirmed = ctx.input(|i| i.key_pressed(egui::Key::Enter));
        let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let tr = i18n::t(self.settings.lang);

        centered_dialog(egui::Window::new(tr.dialog_close_connection_title), ctx).collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
            ui.label(i18n::confirm_close_connection(self.settings.lang, &confirm.title));
            ui.horizontal(|ui| {
                if ui.button(tr.btn_close).clicked() {
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
            self.close_tab(confirm.idx);
        } else if open {
            self.close_tab_confirm = Some(confirm);
        }
    }

    /// Zmena hlavniho hesla trezoru z bezicí aplikace (misto pres cmd
    /// pri startu). Trezor uz je odemceny (drzime `Vault` primo), takze
    /// zmena hesla znamena jen znovu zasifrovat aktualni obsah novym
    /// heslem (`Vault::save`) a od te chvile drzet v pameti uz jen to
    /// nove - stare heslo se overuje porovnanim s tim, ktere uz mame
    /// od odemceni v pameti (zadny dalsi pristup na disk neni potreba).
    /// Zmena hlavniho hesla trezoru - soucast primo Settings tabu
    /// (`render_settings`), ne uz samostatny modalni dialog otevirany z
    /// menu - viz pozadavek "V nastavení bychom nemuseli mít podsložky
    /// Předvolby a Změnit heslo k trezoru... zatím tam toho nemáme
    /// tolik takže bych dal vše přímo bez podsložek". Trezor uz je
    /// odemceny (drzime `Vault` primo), takze zmena hesla znamena jen
    /// znovu zasifrovat aktualni obsah novym heslem (`Vault::save`) a
    /// od te chvile drzet v pameti uz jen to nove - stare heslo se
    /// overuje porovnanim s tim, ktere uz mame od odemceni v pameti
    /// (zadny dalsi pristup na disk neni potreba).
    fn render_change_password_section(&mut self, ui: &mut egui::Ui) {
        let tr = i18n::t(self.settings.lang);
        let mut confirmed = false;

        ui.heading(tr.dialog_change_password_title);
        ui.add_space(4.0);
        egui::Grid::new("change_password_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label(tr.current_password_label);
            ui.add(egui::TextEdit::singleline(&mut self.change_password_form.old).password(true));
            ui.end_row();

            ui.label(tr.new_password_label);
            ui.add(egui::TextEdit::singleline(&mut self.change_password_form.new1).password(true));
            ui.end_row();

            ui.label(tr.repeat_new_password_label);
            ui.add(egui::TextEdit::singleline(&mut self.change_password_form.new2).password(true));
            ui.end_row();
        });

        if let Some(err) = &self.change_password_form.error {
            ui.add_space(6.0);
            ui.colored_label(egui::Color32::from_rgb(0xe0, 0x6c, 0x6c), err);
        }

        ui.add_space(8.0);
        if ui.button(tr.btn_change).clicked() {
            confirmed = true;
        }

        if !confirmed {
            return;
        }

        if self.change_password_form.old != self.master_password {
            self.change_password_form.error = Some(tr.current_password_wrong.to_string());
            return;
        }
        if self.change_password_form.new1.is_empty() {
            self.change_password_form.error = Some(tr.new_password_empty.to_string());
            return;
        }
        if self.change_password_form.new1 != self.change_password_form.new2 {
            self.change_password_form.error = Some(tr.new_passwords_mismatch.to_string());
            return;
        }
        match self.vault.save(&self.change_password_form.new1) {
            Ok(()) => {
                self.master_password = self.change_password_form.new1.clone();
                self.status_message = Some(tr.vault_password_changed.to_string());
                self.change_password_form = ChangePasswordDialog::default();
            }
            Err(e) => {
                self.change_password_form.error = Some(format!("{}: {e}", tr.vault_save_failed));
            }
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
        let tr = i18n::t(self.settings.lang);
        let known_names = self.known_session_names();
        let known_hosts = self.known_hosts();

        centered_dialog(egui::Window::new(tr.dialog_quick_connect_title), ctx)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::Grid::new("quick_connect_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label(tr.field_name);
                    let name_resp = grid_field_with_suggestions(ui, &mut form.name, &known_names);
                    if !form.focus_requested {
                        name_resp.request_focus();
                        form.focus_requested = true;
                    }

                    ui.label(tr.field_host);
                    grid_field_with_suggestions(ui, &mut form.host, &known_hosts);

                    ui.label(tr.field_port);
                    ui.text_edit_singleline(&mut form.port);
                    ui.end_row();

                    ui.label(tr.field_username);
                    ui.text_edit_singleline(&mut form.username);
                    ui.end_row();

                    ui.label(tr.field_password);
                    ui.add(egui::TextEdit::singleline(&mut form.password).password(true));
                    ui.end_row();
                });

                ui.add_space(6.0);
                ui.label(egui::RichText::new(tr.quick_connect_note).small());

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(tr.btn_connect).clicked() {
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
        let tr = i18n::t(self.settings.lang);

        // Snapshot stromu/dat pro vykresleni vyberu - stejny vzor jako
        // `show_tree` (`build_tree` je levne zavolat kazdy snimek), jen
        // se zde navic k otevirani slozek pridavaji zaskrtavatka.
        let tree = build_tree(&self.vault.data);

        centered_dialog(egui::Window::new(tr.dialog_export_title), ctx)
            .collapsible(false)
            .resizable(true)
            .default_width(440.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(tr.export_what_label);
                ui.horizontal_wrapped(|ui| {
                    ui.label("🔍");
                    ui.add(egui::TextEdit::singleline(&mut dialog.filter).desired_width(160.0))
                        .on_hover_text(tr.export_search_hover);
                    if ui.button(tr.btn_select_all).clicked() {
                        select_all = true;
                    }
                    if ui.button(tr.btn_select_none).clicked() {
                        select_none = true;
                    }
                });
                ui.add_space(4.0);
                ui.group(|ui| {
                    egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                        if tree.children.is_empty() && tree.session_ids.is_empty() {
                            ui.label(egui::RichText::new(tr.export_empty_vault).small());
                        } else {
                            let filter_lower = dialog.filter.trim().to_lowercase();
                            render_export_tree(
                                ui,
                                &tree,
                                "",
                                &self.vault.data,
                                &filter_lower,
                                &mut dialog.selected_sessions,
                                &mut dialog.selected_folders,
                            );
                        }
                    });
                });
                ui.add_space(10.0);

                ui.label(tr.export_target_file_label);
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut dialog.path);
                    if ui.button(tr.btn_browse).clicked() {
                        browse = true;
                    }
                });
                ui.add_space(6.0);
                egui::Grid::new("export_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label(tr.export_password_label);
                    ui.add(egui::TextEdit::singleline(&mut dialog.password).password(true));
                    ui.end_row();

                    ui.label(tr.repeat_password_label);
                    ui.add(egui::TextEdit::singleline(&mut dialog.confirm).password(true));
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.label(egui::RichText::new(tr.export_password_note).small());

                if let Some(err) = &dialog.error {
                    ui.add_space(6.0);
                    ui.colored_label(egui::Color32::from_rgb(0xe0, 0x6c, 0x6c), err);
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(tr.btn_export).clicked() {
                        confirmed = true;
                    }
                    if ui.button(tr.btn_cancel).clicked() {
                        cancel = true;
                    }
                });
            });

        // "Procházet..." se resi az po .show() (viz stejny vzor u
        // ostatnich dialogu) - nativni dialog souboru (`rfd`) je
        // blokujici volani, takze se hodi az po vykresleni tohoto snimku.
        if browse {
            if let Some(path) = rfd::FileDialog::new()
                .set_title(tr.export_save_dialog_title)
                .set_file_name("term-ix-export.termx")
                .add_filter(tr.vault_file_filter_name, &["termx"])
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
                dialog.error = Some(tr.export_target_missing.to_string());
                self.export_dialog = Some(dialog);
                return;
            }
            if dialog.selected_sessions.is_empty() && dialog.selected_folders.is_empty() {
                dialog.error = Some(tr.export_selection_empty.to_string());
                self.export_dialog = Some(dialog);
                return;
            }
            if dialog.password.is_empty() {
                dialog.error = Some(tr.export_password_empty.to_string());
                self.export_dialog = Some(dialog);
                return;
            }
            if dialog.password != dialog.confirm {
                dialog.error = Some(tr.passwords_mismatch.to_string());
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
                    self.status_message =
                        Some(i18n::export_saved(self.settings.lang, &path, filtered.servers.len(), filtered.folders.len()));
                }
                Err(e) => {
                    dialog.error = Some(i18n::export_failed(self.settings.lang, e));
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
        let tr = i18n::t(self.settings.lang);

        centered_dialog(egui::Window::new(tr.dialog_import_title), ctx)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(tr.import_source_file_label);
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut dialog.path);
                    if ui.button(tr.btn_browse).clicked() {
                        browse = true;
                    }
                });
                ui.add_space(6.0);
                ui.label(tr.import_password_label);
                ui.add(egui::TextEdit::singleline(&mut dialog.password).password(true));
                ui.add_space(8.0);
                ui.checkbox(&mut dialog.replace, tr.import_replace_checkbox);
                ui.label(egui::RichText::new(tr.import_merge_note).small());

                if let Some(err) = &dialog.error {
                    ui.add_space(6.0);
                    ui.colored_label(egui::Color32::from_rgb(0xe0, 0x6c, 0x6c), err);
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(tr.btn_import).clicked() {
                        confirmed = true;
                    }
                    if ui.button(tr.btn_cancel).clicked() {
                        cancel = true;
                    }
                });
            });

        if browse {
            if let Some(path) = rfd::FileDialog::new()
                .set_title(tr.import_open_dialog_title)
                .add_filter(tr.vault_file_filter_name, &["termx"])
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
                dialog.error = Some(tr.import_source_missing.to_string());
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
                    self.status_message = Some(tr.import_success.to_string());
                }
                Err(e) => {
                    dialog.error = Some(i18n::import_failed(self.settings.lang, e));
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
            self.render_guest_login(ui);
            return;
        }

        let tr = i18n::t(self.settings.lang);
        ui.horizontal(|ui| {
            if ui.button(tr.btn_new_server).clicked() {
                self.close_all_dialogs();
                self.new_session_form = Some(NewSessionForm::default());
            }
            if ui.button(tr.btn_new_folder).clicked() {
                self.close_all_dialogs();
                self.new_folder_dialog = Some(NewFolderDialog::new());
            }
        });
        ui.separator();

        let tree = build_tree(&self.vault.data);
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.render_folder_contents(ui, &tree, "");
        });
    }

    /// Prihlasovaci formular zobrazeny v levem panelu misto stromu
    /// serveru, dokud je aplikace v hostovskem rezimu - viz
    /// [`GuestLoginForm`]. Na rozdil od puvodni hlasky uz neni potreba
    /// aplikaci restartovat, viz `submit_guest_login`.
    fn render_guest_login(&mut self, ui: &mut egui::Ui) {
        let vault_exists = self.vault_path.exists();
        let tr = i18n::t(self.settings.lang);

        ui.label(egui::RichText::new(tr.guest_mode_heading).strong());
        ui.label(egui::RichText::new(tr.guest_mode_hint).small());
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        ui.label(if vault_exists { tr.guest_login_prompt } else { tr.guest_create_prompt });
        ui.add_space(4.0);

        let pw_resp = ui.add(
            egui::TextEdit::singleline(&mut self.guest_login.password)
                .password(true)
                .hint_text(tr.main_password_hint)
                .desired_width(f32::INFINITY),
        );
        // Stejny vzor jako `LockScreen::focus_requested` - fokus se
        // nabidne jen jednou (prvni snimek po vstupu do hostovskeho
        // rezimu, nebo znovu po chybe nize), ne kazdy snimek.
        if !self.guest_login.focus_requested {
            pw_resp.request_focus();
        }
        let enter_in_password = pw_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

        let mut enter_in_confirm = false;
        if !vault_exists {
            ui.add_space(4.0);
            let confirm_resp = ui.add(
                egui::TextEdit::singleline(&mut self.guest_login.confirm)
                    .password(true)
                    .hint_text(tr.repeat_password_hint)
                    .desired_width(f32::INFINITY),
            );
            enter_in_confirm = confirm_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        }

        self.guest_login.focus_requested = true;

        ui.add_space(6.0);
        let clicked = ui.button(if vault_exists { tr.btn_login } else { tr.btn_create_vault }).clicked();

        if let Some(err) = self.guest_login.error.clone() {
            ui.add_space(6.0);
            ui.colored_label(theme::DANGER, err);
        }

        if !vault_exists {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(tr.vault_password_warning).small());
        }

        if clicked || enter_in_password || enter_in_confirm {
            self.submit_guest_login(vault_exists);
        }
    }

    /// Zpracovani formulare z `render_guest_login` - stejna logika jako
    /// `TermxApp::render_lock_screen` (`Vault::unlock`/`Vault::create`),
    /// jen se pri uspechu misto zalozeni nove `MainApp` instance rovnou
    /// prepise `self.vault`/`self.master_password`/`self.is_guest` -
    /// diky tomu zustanou zachovany uz otevrene taby a docasna
    /// (`ad_hoc_sessions`) spojeni, ktere uzivatel v hostovskem rezimu
    /// pripadne uz mel rozdelane.
    fn submit_guest_login(&mut self, vault_exists: bool) {
        let password = std::mem::take(&mut self.guest_login.password);
        let confirm = std::mem::take(&mut self.guest_login.confirm);

        let tr = i18n::t(self.settings.lang);
        let outcome = if vault_exists {
            Vault::unlock(&self.vault_path, &password).map_err(|e| format!("{}: {e}", tr.vault_unlock_failed))
        } else if password.is_empty() {
            Err(tr.main_password_empty.to_string())
        } else if password != confirm {
            Err(tr.passwords_mismatch.to_string())
        } else {
            Vault::create(&self.vault_path, &password).map_err(|e| format!("{}: {e}", tr.vault_create_failed))
        };

        match outcome {
            Ok(vault) => {
                self.vault = vault;
                self.master_password = password;
                self.is_guest = false;
                self.guest_login = GuestLoginForm::default();
            }
            Err(e) => {
                self.guest_login.error = Some(e);
                self.guest_login.focus_requested = false;
            }
        }
    }

    fn render_folder_contents(&mut self, ui: &mut egui::Ui, node: &FolderNode, path_prefix: &str) {
        let mut actions: Vec<TreeAction> = Vec::new();
        let tr = i18n::t(self.settings.lang);

        for (name, child) in &node.children {
            let full_path = if path_prefix.is_empty() { name.clone() } else { format!("{path_prefix}/{name}") };
            let is_empty = child.children.is_empty() && child.session_ids.is_empty();

            let header = egui::CollapsingHeader::new(format!("📁 {name}")).id_salt(&full_path).default_open(false).show(ui, |ui| {
                self.render_folder_contents(ui, child, &full_path);
            });

            header.header_response.context_menu(|ui| {
                if ui.button(tr.btn_rename_folder).clicked() {
                    actions.push(TreeAction::RenameFolder(full_path.clone()));
                    ui.close_menu();
                }
                if is_empty && ui.button(tr.btn_delete_empty_folder).clicked() {
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
        let tr = i18n::t(self.settings.lang);

        let response = ui.selectable_label(false, label);
        if response.double_clicked() {
            actions.push(TreeAction::Open(id));
        }
        response.context_menu(|ui| {
            if ui.button(tr.btn_open).clicked() {
                actions.push(TreeAction::Open(id));
                ui.close_menu();
            }
            if ui.button(tr.btn_edit).clicked() {
                actions.push(TreeAction::EditSession(id));
                ui.close_menu();
            }
            if ui.button(tr.btn_rename).clicked() {
                actions.push(TreeAction::RenameSession(id));
                ui.close_menu();
            }
            if ui.button(tr.btn_move_to_folder).clicked() {
                actions.push(TreeAction::MoveSession(id));
                ui.close_menu();
            }
            if ui.button(tr.btn_delete).clicked() {
                actions.push(TreeAction::DeleteSession(id));
                ui.close_menu();
            }
        });
    }

    fn apply_tree_action(&mut self, action: TreeAction) {
        match action {
            TreeAction::Open(id) => self.open_session_tab(id),
            TreeAction::EditSession(id) => {
                if let Some(session) = self.vault.data.servers.iter().find(|s| s.id == id) {
                    let form = EditSessionForm::from_session(session);
                    self.close_all_dialogs();
                    self.edit_session_form = Some(form);
                }
            }
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
        self.maybe_start_update_check();
        self.poll_update_check();

        self.top_menu(ctx);

        self.show_new_session_dialog(ctx);
        self.show_edit_session_dialog(ctx);
        self.show_new_folder_dialog(ctx);
        self.show_rename_dialog(ctx);
        self.show_move_dialog(ctx);
        self.show_delete_confirm(ctx);
        self.show_quick_connect_dialog(ctx);
        self.show_export_dialog(ctx);
        self.show_import_dialog(ctx);
        self.show_close_tab_confirm(ctx);

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

        // Bez vychoziho odsazeni (`inner_margin(Margin::same(0.0))`) -
        // panel listy (Home/Nastaveni/spojeni) a oddelovac pod ni si
        // sve odsazeni pridavaji zvlast (viz nize), ale obsah SSH
        // terminalu (`render_connection`) uz ne, aby oknem terminalu
        // slo dotahnout az ke kraji plochy (viz `active_tab_content`).
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::BG_PANEL).inner_margin(egui::Margin::same(0.0)))
            .show(ctx, |ui| {
                egui::Frame::none().inner_margin(egui::Margin::symmetric(8.0, 6.0)).show(ui, |ui| {
                    self.tab_bar(ui);
                });
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
    /// `false` jen do prvniho vykresleni (nebo po chybe - viz nize) -
    /// pak se pole hlavniho hesla samo fokusne, aby uzivatel po
    /// najeti aplikace do teto obrazovky mohl heslo rovnou psat bez
    /// nutnosti tam nejdriv kliknout.
    focus_requested: bool,
}

enum LockState {
    Locked(LockScreen),
    Unlocked(MainApp),
}

/// Klic pod kterym se [`AppSettings`] uklada do bezneho eframe
/// perzistentniho ulozneho prostoru (viz `TermxApp::new`/`TermxApp::save`) -
/// samostatny od `eframe::APP_KEY` (ten by ocekaval serializaci cele
/// `TermxApp`, coz nechceme - `Vault`/hesla v pameti tam nepatri).
const SETTINGS_STORAGE_KEY: &str = "term-ix-settings";

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
    /// Nastaveni nactene pri startu (viz `new`) - drzi se tu jako
    /// zaloha pro `save`, kdyby eframe ulozilo stav jeste pred prvnim
    /// odemcenim (kdy [`MainApp`], ktery jinak drzi "zivou" kopii,
    /// jeste neexistuje).
    initial_settings: AppSettings,
    /// Jestli je hlavni okno PRAVE TED maximalizovane - prubezne
    /// aktualizovano kazdy snimek v `update` (viz tam) a pouzito v
    /// `save`. Drzeno oddelene od `initial_settings`/`MainApp::settings`,
    /// protoze se musi sledovat NEZAVISLE na tom, jestli je trezor
    /// zamceny/odemceny (okno jde maximalizovat i na zamcene
    /// obrazovce).
    window_maximized: bool,
}

impl TermxApp {
    /// `storage` je `cc.storage` z uzavření `eframe::run_native` v
    /// `lib.rs` - odtud se (pokud existuje) hned pri startu nactou drive
    /// ulozena nastaveni (viz [`AppSettings`]); kdyz zadna jeste
    /// neexistuji (prvni spusteni), pouzije se `AppSettings::default()`.
    pub fn new(vault_path: PathBuf, registry: ModuleRegistry, storage: Option<&dyn eframe::Storage>) -> Self {
        let initial_settings: AppSettings = storage.and_then(|s| eframe::get_value(s, SETTINGS_STORAGE_KEY)).unwrap_or_default();
        let window_maximized = initial_settings.window_maximized;
        Self {
            vault_path,
            registry: Some(registry),
            state: LockState::Locked(LockScreen::default()),
            initial_settings,
            window_maximized,
        }
    }

    /// `true`, kdyz uz pri startu ulozene nastaveni rika, ze bylo okno
    /// naposledy maximalizovane - `lib.rs::run_app` podle toho hned po
    /// vytvoreni okna posle `egui::ViewportCommand::Maximized(true)`
    /// (samotne `persist_window`/eframe obnovuje jen polohu a velikost,
    /// ne maximalizaci).
    pub fn wants_maximized(&self) -> bool {
        self.initial_settings.window_maximized
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
        let focus_requested = screen.focus_requested;

        let mut submit = false;
        let mut skip = false;
        // Zamcena obrazovka jeste nema pristup k `Vault` (ten se
        // odemyka az prave tady), ale `AppSettings` (vc. zvoleneho
        // jazyka) se nacita uz pri startu (`TermxApp::new`, pred
        // odemcenim) - viz `self.initial_settings`, takze i tato
        // obrazovka uz muze respektovat drive zvoleny jazyk.
        let tr = i18n::t(self.initial_settings.lang);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() / 2.0 - 160.0).max(24.0));
                ui.heading("Term-IX");
                ui.add_space(8.0);
                ui.label(if vault_exists { tr.lock_unlock_prompt } else { tr.lock_create_prompt });
                ui.add_space(10.0);

                let pw_resp = ui.add(
                    egui::TextEdit::singleline(&mut password)
                        .password(true)
                        .hint_text(tr.main_password_hint)
                        .desired_width(260.0),
                );
                // Jen jednou (pri prvnim vykresleni teto obrazovky, nebo
                // znovu po chybe - viz `focus_requested = false` nize) -
                // jinak by fokus kazdy snimek "krad" i kdyz uzivatel
                // zrovna kliknul jinam (napr. do pole pro zopakovani
                // hesla u noveho trezoru).
                if !focus_requested {
                    pw_resp.request_focus();
                }
                let enter_pressed = pw_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                if !vault_exists {
                    ui.add_space(4.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut confirm)
                            .password(true)
                            .hint_text(tr.repeat_password_hint)
                            .desired_width(260.0),
                    );
                }

                ui.add_space(10.0);
                let clicked = ui.button(if vault_exists { tr.btn_unlock } else { tr.btn_create_vault }).clicked();
                if clicked || enter_pressed {
                    submit = true;
                }

                if let Some(err) = &error {
                    ui.add_space(8.0);
                    ui.colored_label(egui::Color32::from_rgb(0xe0, 0x6c, 0x6c), err);
                }

                if !vault_exists {
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new(tr.vault_password_warning).small());
                }

                ui.add_space(18.0);
                ui.separator();
                ui.add_space(6.0);
                if ui.button(tr.btn_continue_without_password).clicked() {
                    skip = true;
                }
                ui.label(egui::RichText::new(tr.lock_guest_note).small());
            });
        });

        // Pujcka `screen` z minula skoncila (posledni pouziti bylo pred
        // vstupem do .show()) - muzeme si ji vzit znovu.
        let LockState::Locked(screen) = &mut self.state else { return };
        screen.password = password.clone();
        screen.confirm = confirm.clone();
        screen.focus_requested = true;

        if skip {
            let registry = self.registry.take().expect("registry byl jiz spotrebovan");
            self.state = LockState::Unlocked(MainApp::new_guest(self.vault_path.clone(), registry, self.initial_settings.clone()));
            return;
        }

        if !submit {
            return;
        }

        let outcome = if vault_exists {
            Vault::unlock(&self.vault_path, &password).map_err(|e| format!("{}: {e}", tr.vault_unlock_failed))
        } else if password.is_empty() {
            Err(tr.main_password_empty.to_string())
        } else if password != confirm {
            Err(tr.passwords_mismatch.to_string())
        } else {
            Vault::create(&self.vault_path, &password).map_err(|e| format!("{}: {e}", tr.vault_create_failed))
        };

        match outcome {
            Ok(vault) => {
                let registry = self.registry.take().expect("registry byl jiz spotrebovan");
                self.state =
                    LockState::Unlocked(MainApp::new(vault, self.vault_path.clone(), password, registry, self.initial_settings.clone()));
            }
            Err(e) => {
                let LockState::Locked(screen) = &mut self.state else { return };
                screen.error = Some(e);
                // Po neuspesnem pokusu se pole hesla znovu samo
                // fokusne (viz `pw_resp.request_focus()` vyse), aby
                // uzivatel mohl rovnou zkusit heslo napsat znovu.
                screen.focus_requested = false;
            }
        }
    }
}

impl eframe::App for TermxApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Prubezne sledovani, jestli je okno PRAVE TED maximalizovane -
        // `ctx.input(|i| i.viewport().maximized)` vraci `None`, dokud to
        // backend jeste nezjistil (typicky jen prvnich par snimku),
        // proto se hodnota jen aktualizuje kdyz uz je znama, jinak
        // zustava posledni znama (viz `save`). Zamerne NEsledujeme
        // minimalizaci - uzivatel vyslovne chtel, aby se okno pri
        // pristim spusteni nikdy neobjevilo minimalizovane.
        if let Some(maximized) = ctx.input(|i| i.viewport().maximized) {
            self.window_maximized = maximized;
        }

        let locked = matches!(self.state, LockState::Locked(_));
        if locked {
            self.render_lock_screen(ctx);
        } else if let LockState::Unlocked(app) = &mut self.state {
            app.update(ctx, frame);
        }
    }

    /// Eframe tuto metodu vola periodicky na pozadi a pri zavirani okna
    /// (stejny mechanismus jako `persist_window` v `lib.rs` uz drive
    /// vyuziva pro polohu/velikost okna) - ulozi [`AppSettings`], aby je
    /// `new` mohlo pri pristim spusteni zase nacist. Pokud uz je trezor
    /// odemceny, uklada se "ziva" kopie primo z [`MainApp`] (tam uzivatel
    /// hodnoty meni - viz `render_settings`); jinak (jeste na zamcene
    /// obrazovce) se uklada `initial_settings` beze zmeny. Maximalizace
    /// okna (`self.window_maximized`, viz `update`) se do ukladane
    /// hodnoty vzdy domicha zvlast, protoze se sleduje nezavisle na
    /// stavu zamku.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let mut settings = match &self.state {
            LockState::Unlocked(app) => app.settings.clone(),
            LockState::Locked(_) => self.initial_settings.clone(),
        };
        settings.window_maximized = self.window_maximized;
        eframe::set_value(storage, SETTINGS_STORAGE_KEY, &settings);
    }
}
