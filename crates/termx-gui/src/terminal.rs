//! Vestaveny terminalovy emulator (VT100/ANSI) pro Connection tab -
//! nahrazuje puvodni "nahradni" obrazovku skutecnym pripojenim: bajty
//! prichozi ze SSH kanalu (`termx_ssh::spawn_ssh_session`) se prohanaji
//! pres ANSI parser `alacritty_terminal`u, ktery drzi stav cele
//! obrazovky (mrizka znaku, barvy, kurzor) - tu pak `TerminalSession::render`
//! kazdy snimek vykresli primo do tabu.
//!
//! POZNAMKA K OVERENI (NEJVYSSI RIZIKO V CELEM PROJEKTU): `alacritty_terminal`
//! je interni knihovna terminaloveho emulatoru Alacritty - neni to
//! knihovna primarne udelana pro pouziti mimo Alacritty samotny a jeji
//! API se mezi verzemi pomerne casto meni. Bez pristupu na crates.io v
//! tomto prostredi NEBYLO mozne overit verzi 0.24 skutecnym `cargo
//! build`. Pokud build po stazeni zavislosti selze, nejpravdepodobnejsi
//! mista k oprave (serazeno dle pravdepodobnosti) jsou:
//!   1. `Processor::advance` - tady pouzito bajt-po-bajtu
//!      (`parser.advance(&mut term, byte)`); nektere verze mohou misto
//!      toho chtit cely slice najednou (`parser.advance(&mut term, &bytes)`).
//!   2. Cesta k ANSI typum - zkoušeno `alacritty_terminal::vte::ansi::*`
//!      (Processor/Color/NamedColor); u starsich verzi to muze byt
//!      primo `alacritty_terminal::ansi::*` (bez `vte::`).
//!   3. `Term::new(config, &size, event_proxy)` - presny pocet/poradi
//!      parametru se mezi verzemi drobne lisil.
//!   4. `grid.cursor.point` - pozice kurzoru; pokud `Grid` toto pole
//!      nema, zkusit `term.renderable_content().cursor.point`
//!      (novejsi, primo pro tento ucel urcene API).
//!   5. Jmena poli bunky (`cell.c`, `cell.fg`, `cell.bg`) a variant
//!      `Color`/`NamedColor` - zde pouzito jen tech nejzakladnejsich 16
//!      barev + Indexed/Spec, vse ostatni ma bezpecny fallback (`_ =>`),
//!      takze i kdyby se nejaka varianta jmenovala jinak/pribyla nova,
//!      staci upravit jen `named_color`.
//!   6. (nove, dynamicke prizpusobovani velikosti) `Fonts::glyph_width`/
//!      `Fonts::row_height` pouzite v `TerminalSession::resize_to_fit` -
//!      pokud presne tyto nazvy metod v pouzite verzi `egui` neexistuji,
//!      resenim je zafixovat pocet sloupcu/radku zpet na pevnou hodnotu
//!      (puvodni `DEFAULT_COLS`/`DEFAULT_ROWS` zustavaji jako vychozi
//!      velikost pri vytvoreni spojeni, nez se poprve prepocita).
//!   7. (nove, info proužek se statistikami) `TerminalSession::render_status_bar`
//!      pouziva `egui::TopBottomPanel::bottom(id).show_inside(ui, ...)` a
//!      `egui::Frame::none()` - u verze `egui` 0.29 by to melo sedet,
//!      ale kdyby `Frame::none()` v pouzite verzi nebyla (u novejsich
//!      `egui` byla nahrazena konstantou `Frame::NONE`), staci upravit
//!      jen toto jedno volani; zbytek proužku (skladani `parts` do
//!      textu) na tom nezavisi.
//!   8. (nove, oznaceni textu mysi + automaticke kopirovani) `TerminalSession::render_grid`
//!      nahradilo puvodni `ui.add(Label::new(job).selectable(false))`
//!      rucnim `ui.fonts(|f| f.layout_job(job))` + `ui.painter().galley(...)`,
//!      aby slo presne mapovat pozici mysi na bunku mrizky. Vsechny
//!      pouzite metody (`Fonts::layout_job`, `Painter::galley`,
//!      `Context::copy_text`, `Context::set_cursor_icon`, `Response::drag_stopped`)
//!      byly u verze egui 0.29.1 overeny primo v jeji dokumentaci
//!      (docs.rs) - narozdil od `alacritty_terminal` tady tedy jde o
//!      relativne nizke riziko. Pokud by presto `copy_text`/`drag_stopped`
//!      v pouzite verzi chybely, nejpravdepodobnejsi nahrada za prvni je
//!      starsi `ctx.output_mut(|o| o.copied_text = text)`, za druhe pak
//!      rucni porovnani `dragged()` mezi po sobe jdoucimi snimky.
//!      (PUVODNI navrh vazal kopirovani na Ctrl+C/Cmd+C - zpetna vazba
//!      "používáme CTRL+c/v na kopírování/vložení, budeme to muset
//!      změnit" to zmenila na oznaceni = automaticka kopie /
//!      `handle_selection_input`/, Ctrl+C uz jde VZDY primo na server
//!      /`handle_keyboard`, `ctrl_control_code`/ - stejne UX jako
//!      PuTTY a vetsina "klasickych" terminalu.)
//!   9. (nove, skrolovani historie kolieckem mysi) `Grid::display_offset`,
//!      `Term::scroll_display`, a `alacritty_terminal::grid::Scroll`
//!      (varianty `Delta(isize)`/`Bottom` pouzite v `render_grid`/
//!      `handle_keyboard`/`handle_credentials_keyboard`) - vcetne
//!      predpokladu, ze KLADNA hodnota `Scroll::Delta` posouva pohled
//!      NAHORU do historie (viz `render_grid`). Pokud by `Scroll` v
//!      pouzite verzi nebyl primo v modulu `grid` (ale napr. primo v
//!      `term`), nebo by mel znamenko obracene (kolecko by pak
//!      skrolovalo "naopak"), jde o izolovanou opravu importu/znamenka v
//!      `render_grid` - zbytek (vykresleni podle `display_offset` v
//!      `build_layout_job`, `selected_text`) na spravnem znamenku
//!      nezavisi, jen na tom, ze `display_offset` roste smerem do
//!      historie.
//! Rucni/automaticke znovupripojeni (`reconnect`/`maybe_auto_reconnect`,
//! viz nize) zadne nove nejiste API nepridava - jen znovu vola uz
//! overene `spawn_ssh_session`/`Term::new` se stejnymi parametry jako
//! `TerminalSession::new`.
//! Architektura kolem (SSH vlakno v `termx-ssh`, kanaly, GUI tab) na
//! techto detailech nezavisi - jde o lokalizovanou opravu jednoho
//! souboru.
//! Odlozene zadani prihlasovacich udaju (`ConnState::AwaitingCredentials`)
//! take nepridava zadne nove nejiste API na teto strane - SSH spojeni
//! (`spawn_ssh_session`) se pořád zaklada primo v `new`, stejne jako
//! drive; jedina zmena je NOVA VARIANTA udalosti `SshEvent::AwaitingCredentials`
//! (posilana z `termx-ssh`, viz tamni POZNAMKA K OVERENI) a odpovidajici
//! nova varianta prikazu `SshInput::Credentials` (`submit_credentials` ji
//! jen posle po jiz existujicim `handle.input_tx` - zadne nove spojeni/
//! vlakno). Prihlasovaci "prompt" ("login as: "/"Password: ") se navic
//! misto samostatneho formulare vykresluje primo jako soucast
//! terminaloveho bufferu (`handle_credentials_keyboard`/`local_echo`) -
//! zadna nova zavislost, jen znovupouziti uz existujiciho
//! `self.parser.advance(&mut self.term, byte)`, ktere jinak `pump`
//! pouziva pro skutecna data ze site.
//!
//! ZNAME OMEZENI TETO PRVNI VERZE (vedomy kompromis kvuli rozsahu):
//! - Velikost terminalu se prizpusobuje velikosti Connection tabu
//!   (viz `TerminalSession::resize`, volane kazdy snimek z `render`) -
//!   pocet sloupcu/radku se pocita z dostupne plochy a rozmeru
//!   monospace pisma. `egui::ScrollArea` zustava jako pojistka pro
//!   pripad nepresnosti tohoto vypoctu.
//! - Tucne/kurziva/podtrzeni (`cell.flags`) se zatim nevykresluji, jen
//!   barvy popredi/pozadi.
//! - Zmena barevne palety pres OSC escape sekvence (redefinice
//!   pojmenovanych barev za behu) se nezohlednuje - `Foreground`/
//!   `Background`/neznama pojmenovana barva pouzije barvy tematu
//!   aplikace.
//! - Info proužek pod terminalem (CPU/RAM/sit/disk/uzivatele, viz
//!   `render_status_bar`) se obcerstvuje kazdych 5 sekund na samostatnem
//!   docasnem SSH kanalu (`termx_ssh::fetch_stats_output`) - na tuto
//!   dobu (max. 3s, viz timeout tamtez) se muze interaktivni kanal na
//!   chvili zpozdit. Zamerne zvoleny jednodussi kompromis oproti
//!   spousteni na uplne samostatnem tokio tasku se sdilenym stavem.
//!   Nez po pripojeni dorazi prvni sada statistik, proužek se
//!   nezobrazuje vubec.
//! - Oznaceni textu mysi (`Selection`) funguje jen v ramci AKTUALNE
//!   VYKRESLENE obrazovky (stejny souradnicovy prostor jako `build_layout_job`,
//!   viz `display_offset` nize - kdyz je uzivatel odscrollovany do
//!   historie, tyka se to prave TE zobrazene casti, ne vzdy jen
//!   posledni aktivni obrazovky). Je to LINEARNI vyber po radcich (jako
//!   Windows Terminal/gnome-terminal), ne "blokovy" obdelnikovy vyber
//!   (jako napr. Alt+tazeni v nekterych terminalech).
//! - Skrolovani kolieckem mysi (`render_grid`) posouva `alacritty_terminal`uv
//!   vlastni `display_offset` (`Term::scroll_display`, viz
//!   `Grid::scrollback_history`/`TermConfig::default()`u vychozi velikost
//!   scrollbacku) - `build_layout_job` pak o tento offset posouva, ktere
//!   radky mrizky se skutecne vykresluji. Zadny vlastni "posuvnik" (scrollbar)
//!   zatim neni - jen kolecko mysi nad terminalem; napsani/odeslani
//!   noveho vstupu automaticky vrati zobrazeni zpet na spodek (viz
//!   `handle_keyboard`/`handle_credentials_keyboard`).

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config as TermConfig, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Processor};

use termx_core::{AuthMethod, Protocol, Session};
use termx_serial::{spawn_serial_session, SerialEvent, SerialHandle, SerialInput};
use termx_ssh::{spawn_ssh_session, SshEvent, SshHandle, SshInput, SystemStats};

use crate::i18n::{self, Lang};
use crate::theme;

/// Vychozi velikost terminalu ve znacich, nez se pri prvnim vykresleni
/// prepocita podle skutecne dostupne plochy tabu (viz
/// `TerminalSession::resize_to_fit`).
const DEFAULT_COLS: usize = 100;
const DEFAULT_ROWS: usize = 32;

/// Meze pro dynamicky prepocitavanou velikost - nikdy neuz nez by bylo
/// prakticky pouzitelne (napr. behem zmensovani/otevirani okna, kdy
/// tab jeste na chvili muze mit skoro nulovou velikost), a nikdy vic,
/// nez je rozumne pro vykon vykreslovani/PTY na druhe strane.
const MIN_COLS: usize = 20;
const MIN_ROWS: usize = 5;
const MAX_COLS: usize = 400;
const MAX_ROWS: usize = 150;

/// Nejvyssi pocet radku, o ktere se zobrazeni posune za JEDEN snimek,
/// kdyz behem tazeni vyberu (`handle_selection_input`) drzi mys uzivatel
/// za hornim/dolnim okrajem terminalu (viz `render_grid`) - strop proti
/// "utrzeni" pri velmi vzdalenem drzeni (napr. az u status prouzku pod
/// terminalem), kdy by jinak jeden snimek preskocil desitky/stovky
/// radku najednou.
const EDGE_AUTOSCROLL_MAX_LINES: i32 = 6;

/// Vychozi velikost pisma terminalu (`AppSettings::term_font_size`) pro
/// noveho uzivatele/pri chybejicim poli ve starych ulozenych nastavenich
/// (viz `#[serde(default = "...")]` u toho pole v `app.rs`) - stejna
/// hodnota jako drivejsi napevno dana `FONT_SIZE`, takze existujici
/// uzivatele zadnou zmenu nepoznaji, dokud si sami velikost nezmeni
/// (Nastaveni, nebo View menu +/-).
pub const DEFAULT_FONT_SIZE: f32 = 14.0;
/// Meze pro `menu_view_font_increase`/`_decrease` a Slider v Nastaveni -
/// pod `MIN_FONT_SIZE` uz je text prakticky neciteny, nad `MAX_FONT_SIZE`
/// se do tabu vejde jen par znaku na radek.
pub const MIN_FONT_SIZE: f32 = 8.0;
pub const MAX_FONT_SIZE: f32 = 28.0;

/// Pomocna funkce pro `#[serde(default = "terminal::default_font_size")]`
/// u `AppSettings::term_font_size` v `app.rs` - `serde` potrebuje
/// jmenovanou funkci (nejde tam dat konstantu/literal primo).
pub fn default_font_size() -> f32 {
    DEFAULT_FONT_SIZE
}

/// Velikost pisma info proužku pod terminalem (`render_status_bar`) -
/// zamerne vetsi nez vychozi "male" (`RichText::small()`, ~9-10px),
/// podle uzivatelovy zpetne vazby.
const STATUS_BAR_FONT_SIZE: f32 = 14.0;

/// Nejmensi rozestup mezi dvema po sobe jdoucimi POKUSY o automaticke
/// obnoveni spojeni (viz `TerminalSession::maybe_auto_reconnect`) - aby
/// se pri dlouhodobe nedostupnem serveru nezkousel novy pokus uplne
/// kazdy snimek (desitky za sekundu), ale v rozumnych intervalech.
const AUTO_RECONNECT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// Kolik posledne prijatych znaku (viz `TerminalSession::recent_plain_output`)
/// se drzi pro detekci Avaya SAT promptu "Terminal Type (...)" (viz
/// `TerminalSession::maybe_send_avaya_term_type_reply`) - staci male
/// okno, hledany retezec je kratky a prompt prijde jen jednou, tesne
/// po prihlaseni.
const AVAYA_PROMPT_SCAN_WINDOW: usize = 256;

/// Typy terminalu, ktere Avaya CM SAT na svem prihlasovacim promptu
/// "Terminal Type (513, 715, 4410, 4425, VT220, NTT, W2KTT, SUNT):
/// [513]" skutecne prijima - viz `TerminalSession::maybe_send_avaya_term_type_reply`.
/// Porovnava se case-insensitive (`Session::term_type` muze byt
/// napsany/vybrany v libovolne velikosti pismen).
const AVAYA_TERMINAL_TYPES: &[&str] = &["513", "715", "4410", "4425", "vt220", "ntt", "w2ktt", "sunt"];

fn terminal_font(font_size: f32) -> egui::FontId {
    egui::FontId::monospace(font_size)
}

/// Sirka JEDNOHO znaku aktualne pouziteho terminaloveho pisma (v
/// bodech UI, ne pixelech obrazovky) - pouziva `app.rs` k odsazeni
/// terminalu od leveho okraje (viz `MainApp::render_connection`),
/// aby velikost odsazeni sedela s aktualni velikosti terminaloveho
/// pisma (`Nastaveni`), misto pevneho cisla v pixelech.
///
/// Zpetna vazba "já bych dal klidně 20a nebo možná lépe písmenka
/// odsadil na šířku jednoho písmena od okraje místo posouvat
/// posouvátko bočního menu" - drivejsi pokusy s pevnymi hodnotami
/// (5px, pak 14px) zpetnou vazbu na presne mnozstvi nutneho odsazeni
/// nikdy nedostaly (viz predchozi zpravy), a pevne cislo navic
/// nesedi se zvetsenym/zmensenym pismem terminalu - sirka jednoho
/// znaku aktualniho pisma je tak rozumnejsi, sama-se-skalujici
/// jednotka odsazeni.
///
/// Stejny vypocet jako `char_w` v `render_grid` nize
/// (`Fonts::glyph_width` na znaku 'M', reprezentativni sirka
/// monospace pisma), jen vytazeny do vlastni funkce, aby ho mohl
/// pouzit i kod mimo tento soubor (`app.rs` nema pristup k
/// privatnimu `terminal_font` primo).
pub(crate) fn char_width(ui: &egui::Ui, font_size: f32) -> f32 {
    let font_id = terminal_font(font_size);
    ui.fonts(|f| f.glyph_width(&font_id, 'M'))
}

/// Vlastni rozmery mrizky pro `Term::new` - `alacritty_terminal` sam o
/// sobe nezna zadnou konkretni velikost, ocekava typ implementujici
/// `Dimensions`.
#[derive(Clone, Copy)]
struct TermSize {
    cols: usize,
    rows: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// `alacritty_terminal` posila udalosti (zmena titulku, zvonek, pozadavek
/// na zmenu velikosti od aplikace bezici v terminalu, ...) pres tento
/// trait - v teto prvni verzi vsechny tise zahazujeme, zadnou z nich
/// zatim nepotrebujeme resit.
#[derive(Clone)]
struct EventProxy;

impl EventListener for EventProxy {
    fn send_event(&self, _event: Event) {}
}

/// Stav SSH spojeni tohoto terminalu (viz `TerminalSession::state`) -
/// pouziva se jak pro vykresleni obsahu tabu (`render`), tak z
/// `app.rs` (`tab_bar`) pro obarveni "mrtveho" tabu a pro rozhodnuti,
/// jestli je pri jeho zavirani potreba potvrzeni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    /// SSH TRANSPORT uz je navazan (TCP + vymena klicu, viz
    /// `termx_ssh::run_session`), ale `Session::auth` nemela
    /// vyplneneho uzivatele, takze se ceka, az ho uzivatel doplni -
    /// primo napsanim do samotneho terminalu (viz
    /// `TerminalSession::handle_credentials_keyboard`), stejne jako u
    /// obycejneho `ssh` klienta ("login as: "/"Password: " prompt jeste
    /// pred autentizaci). Zadny samostatny formular/dialog uz se
    /// nezobrazuje - "mezikus s okénky pro přihlášení" byl podle
    /// zpetne vazby matouci ("s mezikusem kdy mám okénka pro přihlášení
    /// je matoucí").
    AwaitingCredentials,
    /// Prvni navazovani spojeni, nebo prubeh automatickeho/rucniho
    /// pokusu o jeho obnoveni (viz `reconnect`).
    Connecting,
    Connected,
    /// Spojeni skoncilo - at uz chybou (`error` je pak `Some`), nebo
    /// cistě (napr. `exit`/`logout` na druhe strane).
    Disconnected,
}

/// Ktery udaj se prave zadava v prihlasovacim "promptu" primo v
/// terminalu behem `ConnState::AwaitingCredentials` - viz
/// `TerminalSession::handle_credentials_keyboard`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialsStage {
    Username,
    Password,
}

/// Jedno bezici spojeni (SSH nebo seriove/COM, viz [`ConnHandle`]) napojene
/// na vestaveny terminal - jeden otevreny Connection tab = jedna instance
/// (viz `MainApp::terminal_sessions` v `app.rs`).
pub struct TerminalSession {
    term: Term<EventProxy>,
    parser: Processor,
    /// Spojeni se zaklada VZDY hned v `new` (i kdyz `session.auth` jeste
    /// (u SSH) nema uzivatele) - transport se navaze nezavisle na tom,
    /// jestli uz jsou prihlasovaci udaje k dispozici (viz
    /// `ConnState::AwaitingCredentials`/`submit_credentials`), takze
    /// `handle` uz neni potreba drzet jako `Option`.
    handle: ConnHandle,
    connected: bool,
    /// `true` od okamziku, kdy poprve dorazilo `SshEvent::Connected` -
    /// odlisuje "jeste vubec nikdy nepripojeno" (stav `Connecting`) od
    /// "bylo pripojeno, ale spojeni spadlo" (stav `Disconnected`), viz
    /// `state`. Resetuje se zpet na `false` pri kazdem `reconnect`.
    ever_connected: bool,
    error: Option<String>,
    /// Aktualni velikost mrizky ve znacich - drzena zvlast (mimo
    /// `self.term`), aby `resize_to_fit` mohla levne kazdy snimek
    /// zjistit, jestli se vubec neco zmenilo, bez nutnosti se pokazde
    /// ptat `self.term` (a hlavne bez zbytecneho odesilani
    /// `SshInput::Resize` na server, kdyz se velikost od minuleho
    /// snimku nezmenila).
    cols: usize,
    rows: usize,
    /// Puvodni ulozena/docasna session, ze ktere toto spojeni vzniklo -
    /// drzena cela (ne jen id/host), aby ji `reconnect` mohl znovu
    /// pouzit pro `spawn_ssh_session` bez nutnosti ji odjinud znovu
    /// hledat (Connection tab uz zadnou referenci na `MainApp` nema).
    session: Session,
    /// Kdy naposledy probehl pokus o (automaticke) obnoveni spojeni -
    /// viz `maybe_auto_reconnect`/`AUTO_RECONNECT_INTERVAL`. `None`,
    /// dokud jeste zadny pokus o obnoveni neproběhl.
    last_reconnect_attempt: Option<std::time::Instant>,
    /// Posledni prijate systemove metriky (viz `termx_ssh::SystemStats`) -
    /// `None`, dokud po pripojeni jeste nedorazilo prvni periodicke
    /// obcerstveni (viz `SshEvent::Stats` v `pump`); do te doby se info
    /// proužek proste nezobrazuje (viz `render_status_bar`).
    stats: Option<SystemStats>,
    /// `true` od prijeti `SshEvent::AwaitingCredentials` (transport
    /// navazan, ale chybi uzivatel/heslo) - viz
    /// [`ConnState::AwaitingCredentials`]/`handle_credentials_keyboard`.
    /// Po odeslani udaju (`submit_credentials`) uz zustava `false`
    /// natrvalo (i kdyz spojeni pozdeji spadne a `reconnect` ho obnovi -
    /// tehdy uz `session.auth` ma uzivatele/heslo vyplnene, takze se
    /// znovu ptat netreba).
    awaiting_credentials: bool,
    /// Ktery z dvou kroku prihlasovaciho "promptu" prave probiha - viz
    /// [`CredentialsStage`]/`handle_credentials_keyboard`.
    credentials_stage: CredentialsStage,
    /// Rozepsany uzivatel/heslo, jak je uzivatel postupne napsal primo
    /// do terminalu (viz `handle_credentials_keyboard`), nez se po
    /// stisknuti Enter na konci hesla odesle (`submit_credentials`).
    pending_username: String,
    pending_password: String,
    /// Aktivni oznaceni textu tazenim mysi (`None` kdyz nic neni
    /// oznaceno) - viz `handle_selection_input`/`selected_text`. Kliknuti
    /// bez tazeni oznaceni zrusi (stejne jako v kazdem beznem terminalu).
    selection: Option<Selection>,
    /// Posledni prijaty (nezpracovany) vystup ze serveru, jen jako
    /// prosty text (bez ANSI kodu, `String::from_utf8_lossy` na kazdy
    /// prichozi kus dat) - pouzito VYHRADNE k detekci Avaya SAT promptu
    /// "Terminal Type (...)" (viz `maybe_send_avaya_term_type_reply`).
    /// Drzi jen ohranicene mnozstvi znaku (viz `AVAYA_PROMPT_SCAN_WINDOW`)
    /// pro pripad, ze by hledany text prisel rozdeleny na dva prichozi
    /// kusy dat.
    recent_plain_output: String,
    /// `true` od okamziku, kdy uz byla (jednou za tuto relaci spojeni)
    /// automaticky odeslana odpoved na Avaya SAT prompt "Terminal Type
    /// (...)" - viz `maybe_send_avaya_term_type_reply`. Resetuje se na
    /// `false` pri kazdem `reconnect` (novy prompt = nova prilezitost
    /// odpovedet).
    avaya_term_type_replied: bool,
}

/// Oznaceny usek textu v terminalu. `row` v `anchor`/`current` je ABSOLUTNI
/// radek mrizky (stejny prostor jako `Line` u `alacritty_terminal` -
/// `0` = horni radek AKTIVNI (spodni) obrazovky, ZAPORNE hodnoty sahaji do
/// scrollbacku), NE jen aktualne vykresleny vizualni radek (`0..grid.screen_lines()`,
/// viz `build_layout_job`) - tazeni vyberu tak muze pokracovat i pres
/// skrolovani (kolieckem mysi, nebo automaticky u okraje behem tazeni,
/// viz `render_grid`), aniz by se uz oznacene radky "rozjely" od toho, co
/// uzivatel doopravdy oznacil. Prevod mezi vizualnim radkem a timhle
/// absolutnim prostorem (`vizualni_radek - display_offset`) resi
/// `point_to_cell` pri vzniku a `build_layout_job`/`selected_text` pri
/// pouziti - viz zpetna vazba "jak bychom označili víc textu tedy i ten
/// který je momentálně už nahoře mimo viditelné okno".
///
/// `anchor` je bunka, kde tazeni zacalo, `current` kam mys aktualne (nebo
/// naposledy behem tazeni) ukazovala - poradi mezi nimi NENI zarucene
/// (uzivatel muze tahnout jak doprava-dolu, tak doleva-nahoru), proto
/// `normalized` pro skutecne pouziti (zvyrazneni, extrakce textu).
#[derive(Debug, Clone, Copy)]
struct Selection {
    anchor: (i32, usize),
    current: (i32, usize),
}

impl Selection {
    /// Vrati (start, konec) serazene tak, ze `start <= konec` v poradi
    /// (radek, sloupec) - tuple uz ma spravne lexikograficke porovnani
    /// vestavene, presne odpovidajici poradi znaku v terminalu.
    fn normalized(&self) -> ((i32, usize), (i32, usize)) {
        if self.anchor <= self.current {
            (self.anchor, self.current)
        } else {
            (self.current, self.anchor)
        }
    }

    /// Je dana bunka (absolutni radek mrizky, sloupec - viz komentar u
    /// struktury) soucasti oznaceni? Linearni vyber po radcich (ne
    /// "blokovy" obdelnikovy) - stejne chovani jako vetsina beznych
    /// terminalu (Windows Terminal, gnome-terminal, ...): prvni oznaceny
    /// radek od pocatecniho sloupce do konce, prostredni radky cele,
    /// posledni radek od zacatku do koncoveho sloupce.
    fn contains(&self, row: i32, col: usize) -> bool {
        let (start, end) = self.normalized();
        if row < start.0 || row > end.0 {
            return false;
        }
        if start.0 == end.0 {
            col >= start.1 && col <= end.1
        } else if row == start.0 {
            col >= start.1
        } else if row == end.0 {
            col <= end.1
        } else {
            true
        }
    }
}

/// Trida znaku pro urceni hranic "slova" pri dvojkliku - viz
/// `TerminalSession::word_bounds_at`/`handle_selection_input`. Bezny vzor
/// beznych terminalu (xterm, gnome-terminal, ...): alfanumericke znaky
/// (+ podtrzitko) tvori "slovo", souvisly usek bilych znaku dalsi tridu a
/// souvisly usek ostatnich znaku (interpunkce/symboly, napr. "://", "-",
/// "@") jeste dalsi - dvojklik pak oznaci cely souvisly usek STEJNE tridy
/// kolem klikleho znaku (klik primo na mezeru tak oznaci celou souvislou
/// mezeru, klik na oddelovac jako "/" oznaci sousedici oddelovace).
#[derive(PartialEq, Eq)]
enum CharClass {
    Word,
    Space,
    Punct,
}

fn char_class(c: char) -> CharClass {
    if c == '\0' || c.is_whitespace() {
        CharClass::Space
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

/// Bezici spojeni napojene na `TerminalSession::term` - SSH
/// (`termx_ssh::SshHandle`) nebo seriovy/COM port (`termx_serial::SerialHandle`),
/// podle `Session::protocol` (viz `TerminalSession::new`/`reconnect`).
/// Zpetna vazba "Připojení přes COM/sériový port jako další 'plugin'" -
/// `termx-gui` puvodne (kdyz existoval jen SSH) drzelo primo `SshHandle`
/// bez jakekoliv abstrakce nad nim; tento enum je nejmensi zmena, ktera
/// oba protokoly umozni bez zdvojeni cele `TerminalSession` nebo generickych
/// parametru - vsechna mista, ktera s `handle` pracuji (`pump`/`send_bytes`/
/// `resize`/`submit_credentials`), se na nej jen doptaji patternem podle
/// varianty.
enum ConnHandle {
    Ssh(SshHandle),
    Serial(SerialHandle),
}

/// Sjednocena udalost pro `pump` bez ohledu na to, jestli `self.handle`
/// prave je [`ConnHandle::Ssh`] nebo [`ConnHandle::Serial`] - `termx_serial`
/// nezna prihlasovaci prompt ani statistiky (na holé seriové lince
/// nedavaji smysl, viz `termx_serial::SerialEvent`), takze varianty
/// `AwaitingCredentials`/`AuthFailed`/`Stats` u seriove linky proste
/// nikdy nevzniknou (viz `PumpEvent::from_serial`) - zbytek `pump` na
/// tom ale nemusi nijak zavizet, protoze uz jen pracuje s timto
/// spolecnym typem, ne primo s `SshEvent`/`SerialEvent`.
enum PumpEvent {
    Data(Vec<u8>),
    Connected,
    AwaitingCredentials,
    AuthFailed(String),
    Error(String),
    Closed,
    Stats(SystemStats),
}

impl PumpEvent {
    fn from_ssh(event: SshEvent) -> Self {
        match event {
            SshEvent::Data(bytes) => PumpEvent::Data(bytes),
            SshEvent::Connected => PumpEvent::Connected,
            SshEvent::AwaitingCredentials => PumpEvent::AwaitingCredentials,
            SshEvent::AuthFailed(msg) => PumpEvent::AuthFailed(msg),
            SshEvent::Error(msg) => PumpEvent::Error(msg),
            SshEvent::Closed => PumpEvent::Closed,
            SshEvent::Stats(stats) => PumpEvent::Stats(stats),
        }
    }

    fn from_serial(event: SerialEvent) -> Self {
        match event {
            SerialEvent::Data(bytes) => PumpEvent::Data(bytes),
            SerialEvent::Connected => PumpEvent::Connected,
            SerialEvent::Error(msg) => PumpEvent::Error(msg),
            SerialEvent::Closed => PumpEvent::Closed,
        }
    }
}

impl TerminalSession {
    /// Zalozi nove spojeni (na pozadi, viz `termx_ssh::spawn_ssh_session`/
    /// `termx_serial::spawn_serial_session` - podle `Session::protocol`)
    /// a pripravi prazdnou terminalovou obrazovku, do ktere se bude
    /// postupne (`pump`) vykreslovat.
    pub fn new(session: &Session) -> Self {
        let size = TermSize { cols: DEFAULT_COLS, rows: DEFAULT_ROWS };
        let term = Term::new(TermConfig::default(), &size, EventProxy);
        // Spojeni se zaklada VZDY hned - i kdyz `session.auth` jeste
        // nema (u SSH) vyplneneho uzivatele. Transport (TCP + vymena
        // klicu) se navaze nezavisle na tom; kdyz `termx_ssh::run_session`
        // zjisti chybejiciho uzivatele, sam posle `SshEvent::AwaitingCredentials`
        // (viz `pump`) a pocka na dodatecne udaje (`submit_credentials`).
        // Seriove spojeni (`Protocol::Serial`) zadny takovy prihlasovaci
        // krok nema - `SerialEvent::Connected` prijde (skoro) hned po
        // otevreni portu.
        let handle = match session.protocol {
            Protocol::Serial => ConnHandle::Serial(spawn_serial_session(session.clone())),
            _ => ConnHandle::Ssh(spawn_ssh_session(session.clone(), DEFAULT_COLS as u16, DEFAULT_ROWS as u16)),
        };

        Self {
            term,
            parser: Processor::new(),
            handle,
            connected: false,
            ever_connected: false,
            error: None,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            session: session.clone(),
            last_reconnect_attempt: None,
            stats: None,
            awaiting_credentials: false,
            credentials_stage: CredentialsStage::Username,
            pending_username: String::new(),
            pending_password: String::new(),
            selection: None,
            recent_plain_output: String::new(),
            avaya_term_type_replied: false,
        }
    }

    /// Odvozeny stav spojeni (viz [`ConnState`]) z `awaiting_credentials`/
    /// `connected`/`ever_connected`/`error`.
    pub fn state(&self) -> ConnState {
        if self.awaiting_credentials {
            ConnState::AwaitingCredentials
        } else if self.connected {
            ConnState::Connected
        } else if self.ever_connected || self.error.is_some() {
            ConnState::Disconnected
        } else {
            ConnState::Connecting
        }
    }

    /// Zpracuje odeslani prihlasovacich udaju, jak je uzivatel napsal
    /// primo do terminalu (viz `handle_credentials_keyboard`, volano po
    /// Enteru na konci zadavani hesla) - SSH spojeni uz bezelo
    /// (transport je navazan, viz `pump`/`SshEvent::AwaitingCredentials`),
    /// takze se jen posle `SshInput::Credentials` po jiz existujicim
    /// kanalu; zadne nove spojeni se nezaklada. Udaje se navic ulozi
    /// primo do `self.session` (diky tomu pripadny pozdejsi `reconnect`
    /// uz zadne dalsi doplneni nepotrebuje - pouzije stejnou, uz jednou
    /// zadanou, kombinaci uzivatel/heslo).
    fn submit_credentials(&mut self) {
        let username = self.pending_username.trim().to_string();
        if username.is_empty() {
            return;
        }
        let password = std::mem::take(&mut self.pending_password);
        self.session.auth = AuthMethod::Password { username: username.clone(), password: password.clone() };
        self.awaiting_credentials = false;
        // `awaiting_credentials` je `true` jen po `SshEvent::AwaitingCredentials`
        // (viz `pump`), ktere seriove spojeni nikdy neposila (`termx_serial`
        // zadny prihlasovaci prompt nema) - vetev `ConnHandle::Serial` by
        // sem tedy v praxi nikdy nemela dojit, ale `match` misto
        // `if let ConnHandle::Ssh(..)` je stejne bezpecnejsi nez tise nic
        // neudelat, kdyby se to nekdy zmenilo.
        match &self.handle {
            ConnHandle::Ssh(handle) => {
                let _ = handle.input_tx.send(SshInput::Credentials { username, password });
            }
            ConnHandle::Serial(_) => {}
        }
    }

    /// Zahodi aktualni SSH spojeni (pokud jeste bezi - zahozenim
    /// `self.handle` se stejne jako pri zavreni tabu, viz
    /// `MainApp::close_tab`, cistě ukonci i prislusne pozadi bezici SSH
    /// vlakno) a zalozi nove, se stejnymi udaji a stejnou velikosti
    /// mrizky. Pouzito jak pro rucni tlacitko "Připojit znovu" v
    /// odpojenem tabu (`render`), tak pro automaticke obnoveni
    /// (`maybe_auto_reconnect`). Obsah obrazovky (`self.term`) se
    /// resetuje - predchozi vystup (napr. "logout" z minule relace) by
    /// po znovupripojeni uz nedaval smysl.
    fn reconnect(&mut self) {
        let size = TermSize { cols: self.cols, rows: self.rows };
        self.term = Term::new(TermConfig::default(), &size, EventProxy);
        self.parser = Processor::new();
        self.handle = match self.session.protocol {
            Protocol::Serial => ConnHandle::Serial(spawn_serial_session(self.session.clone())),
            _ => ConnHandle::Ssh(spawn_ssh_session(self.session.clone(), self.cols as u16, self.rows as u16)),
        };
        self.connected = false;
        self.ever_connected = false;
        self.error = None;
        self.stats = None;
        self.last_reconnect_attempt = Some(std::time::Instant::now());
        // Oznaceni odkazovalo na obsah PREDCHOZI obrazovky (ktera se prave
        // zahodila spolu s `self.term`) - po znovupripojeni uz nedava
        // smysl.
        self.selection = None;
        // Nove spojeni = novy Avaya SAT "Terminal Type (...)" prompt
        // (pokud vubec prijde) - viz `maybe_send_avaya_term_type_reply`.
        self.recent_plain_output.clear();
        self.avaya_term_type_replied = false;
    }

    /// Kdyz je automaticke obnoveni v Nastaveni zapnute (`enabled`),
    /// zkusi (nejvyse jednou za [`AUTO_RECONNECT_INTERVAL`]) spojeni
    /// samo obnovit. Volano jen kdyz uz je stav [`ConnState::Disconnected`]
    /// (viz `render`) - dokud prvni pripojovaci pokus jeste bezi
    /// (`ConnState::Connecting`), zadny dalsi netreba spoustet.
    fn maybe_auto_reconnect(&mut self, enabled: bool) {
        if !enabled {
            return;
        }
        let ready = match self.last_reconnect_attempt {
            None => true,
            Some(last) => last.elapsed() >= AUTO_RECONNECT_INTERVAL,
        };
        if ready {
            self.reconnect();
        }
    }

    /// Zmeni velikost mrizky terminalu (pocet sloupcu/radku) a da vedet
    /// i druhe strane spojeni (`SshInput::Resize` - `window_change` na
    /// SSH kanalu), aby napr. `vim`/`htop` vedely, jak velkou obrazovku
    /// maji k dispozici. No-op, kdyz se velikost od minule nezmenila.
    fn resize(&mut self, cols: usize, rows: usize) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.term.resize(TermSize { cols, rows });
        // Seriova linka nema pojem PTY velikosti (na rozdil od SSH tu
        // neni druha strana, ktera by o zmene velikosti terminalu
        // potrebovala vedet - viz `termx_serial::SerialInput`, ktere
        // zadnou `Resize` variantu vubec nema), takze se posila jen u
        // `ConnHandle::Ssh`.
        if let ConnHandle::Ssh(handle) = &self.handle {
            let _ = handle.input_tx.send(SshInput::Resize { cols: cols as u32, rows: rows as u32 });
        }
    }

    /// Spocita, kolik sloupcu/radku monospace pisma se vejde do dane
    /// dostupne plochy, a podle toho (pripadne) zmeni velikost terminalu
    /// - viz `resize`. Volano kazdy snimek z `render`, tesne pred tim,
    /// nez se vlastni obsah terminalu vykresli, aby prepocet pouzival
    /// aktualni dostupnou plochu tohoto snimku (uz po pripadnych
    /// hlaskach o stavu spojeni nad terminalem, ktere taky zabiraji
    /// misto).
    fn resize_to_fit(&mut self, ui: &egui::Ui, font_size: f32) {
        let available = ui.available_size();
        let font_id = terminal_font(font_size);
        let (char_w, row_h) = ui.fonts(|f| (f.glyph_width(&font_id, 'M'), f.row_height(&font_id)));

        if char_w <= 0.0 || row_h <= 0.0 {
            // Pismo se jeste nepodarilo zmerit (napr. uplne prvni
            // snimek) - radeji nic nemenit, nez pocitat s nesmyslnymi
            // rozmery.
            return;
        }

        let cols = ((available.x / char_w).floor() as usize).clamp(MIN_COLS, MAX_COLS);
        let rows = ((available.y / row_h).floor() as usize).clamp(MIN_ROWS, MAX_ROWS);
        self.resize(cols, rows);
    }

    /// Vycerpa vsechny cekajici udalosti ze SSH vlakna (nikdy neceka) a
    /// prijata data prozene pres ANSI parser, cimz se aktualizuje stav
    /// obrazovky (`self.term`).
    fn pump(&mut self) {
        loop {
            // Vycerpava se z toho kanalu, ktery zrovna odpovida bezicimu
            // `self.handle` (viz [`ConnHandle`]) - vysledek se hned
            // prevede na sjednocene [`PumpEvent`] (`PumpEvent::from_ssh`/
            // `from_serial`), takze zbytek teto smycky nize uz je pro oba
            // protokoly spolecny a nemusi se vubec vetvit.
            let event = match &self.handle {
                ConnHandle::Ssh(handle) => match handle.output_rx.try_recv() {
                    Ok(e) => Some(PumpEvent::from_ssh(e)),
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
                },
                ConnHandle::Serial(handle) => match handle.output_rx.try_recv() {
                    Ok(e) => Some(PumpEvent::from_serial(e)),
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
                },
            };
            let Some(event) = event else { break };

            match event {
                PumpEvent::Data(bytes) => {
                    for &byte in &bytes {
                        self.parser.advance(&mut self.term, byte);
                    }
                    // Avaya CM SAT prompt "Terminal Type (...)" - viz
                    // `maybe_send_avaya_term_type_reply`. Kontroluje se
                    // AZ TADY, po zpracovani parserem, ale citano z
                    // puvodnich syrovych bajtu (ne z `self.term` mrizky) -
                    // je to jednodussi a spolehlivejsi nez hledat text
                    // ve viditelne obrazovce terminalu. Funguje stejne
                    // i pres seriovou konzoli (napr. primy pristup k
                    // Avaya CM pres RS-232), ne jen SSH.
                    self.maybe_send_avaya_term_type_reply(&bytes);
                }
                PumpEvent::Connected => {
                    self.connected = true;
                    self.ever_connected = true;
                    self.error = None;
                    self.awaiting_credentials = false;
                }
                PumpEvent::AwaitingCredentials => {
                    self.begin_credentials_prompt();
                }
                PumpEvent::AuthFailed(msg) => {
                    // Na rozdil od `PumpEvent::Error` tohle NEukoncuje
                    // spojeni (`termx_ssh::run_session` uz zase samo
                    // ceka na dalsi pokus, viz tamni POZNAMKA K OVERENI) -
                    // jen se vypise hlaska (podobna, jakou by na tomto
                    // miste ukazal skutecny `ssh` klient, doplnena o
                    // konkretni duvod od serveru, pokud nejaky poslal) a
                    // znovu nabidne prompt, misto aby tab spadl do
                    // `ConnState::Disconnected` bez moznosti to rovnou
                    // zkusit znovu - viz zpetna vazba "když nemáme login
                    // úspěšný tak už se k zadání nedostaneme". (Seriove
                    // spojeni tuto udalost nikdy neposila, viz
                    // `PumpEvent::from_serial`.)
                    self.local_echo(format!("Permission denied, please try again. ({msg})\r\n").as_bytes());
                    self.begin_credentials_prompt();
                }
                PumpEvent::Error(e) => {
                    self.error = Some(e);
                    self.connected = false;
                    self.awaiting_credentials = false;
                }
                PumpEvent::Closed => {
                    self.connected = false;
                }
                PumpEvent::Stats(stats) => {
                    self.stats = Some(stats);
                }
            }
        }
    }

    fn send_bytes(&self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        // Prijemce (SSH/seriove vlakno) uz nemusi bezet (napr. spojeni
        // mezitim skoncilo chybou) - poslani se pak proste nezdari, nic
        // se nedeje.
        match &self.handle {
            ConnHandle::Ssh(handle) => {
                let _ = handle.input_tx.send(SshInput::Data(bytes));
            }
            ConnHandle::Serial(handle) => {
                let _ = handle.input_tx.send(SerialInput::Data(bytes));
            }
        }
    }

    /// Jakmile se ve vystupu ze serveru poprve objevi Avaya SAT prompt
    /// "Terminal Type (513, 715, 4410, 4425, VT220, NTT, W2KTT, SUNT):
    /// [513]" (prijde hned po prihlaseni, pred "Command:" promptem - viz
    /// zivy PuTTY test) a uzivatel ma u tohoto spojeni nastaveny nektery
    /// z Avaya-rozpoznanych typu terminalu (`AVAYA_TERMINAL_TYPES` - viz
    /// `Session::term_type`), automaticky se odesle presne tato hodnota
    /// + Enter, stejne jako by ji uzivatel rucne napsal (pozadavek
    /// "dokážeme automaticky i poslat avaya serveru vybraný typ abychom
    /// ho nemuseli psát ručně"). Pro vsechny ostatni (ne-Avaya) typy
    /// terminalu se nic nedeje, presne jako drive.
    ///
    /// Odpoved se posle NEJVYSE JEDNOU za spojeni (`avaya_term_type_replied`,
    /// resetovano v `reconnect`) - kdyby se text "Terminal Type ("
    /// nekdy objevil znovu (napr. shodou okolnosti v jinem vystupu),
    /// dalsi automaticke odesilani uz nehrozi.
    fn maybe_send_avaya_term_type_reply(&mut self, new_bytes: &[u8]) {
        if self.avaya_term_type_replied {
            return;
        }
        let Some(term_type) = self.session.term_type.as_deref() else {
            return;
        };
        if !AVAYA_TERMINAL_TYPES.iter().any(|t| t.eq_ignore_ascii_case(term_type)) {
            return;
        }

        self.recent_plain_output.push_str(&String::from_utf8_lossy(new_bytes));
        if self.recent_plain_output.len() > AVAYA_PROMPT_SCAN_WINDOW {
            let over = self.recent_plain_output.len() - AVAYA_PROMPT_SCAN_WINDOW;
            // Orez az na nejblizsi platnou hranici UTF-8 znaku (`over`
            // samotne muze padnout doprostred vicebajtoveho znaku) -
            // `String::from_utf8_lossy` sice vraci platny UTF-8 retezec
            // jako celek, ale libovolny bajtovy index uprostred neho uz
            // platny byt nemusi.
            let cut = (over..=self.recent_plain_output.len())
                .find(|&i| self.recent_plain_output.is_char_boundary(i))
                .unwrap_or(self.recent_plain_output.len());
            self.recent_plain_output.drain(..cut);
        }

        if self.recent_plain_output.contains("Terminal Type (") {
            self.avaya_term_type_replied = true;
            let mut reply = term_type.as_bytes().to_vec();
            reply.push(b'\r');
            self.send_bytes(reply);
        }
    }

    /// Stabilni `Id` pro klavesovy fokus terminalu - odvozene primo z
    /// `self.session.id` (`Uuid`, nezavisly na pozici v strome `Ui`), aby
    /// bylo VZDY stejne bez ohledu na to, odkud se pocita
    /// (`handle_keyboard`/`handle_credentials_keyboard` vs `render_grid`,
    /// ktere jsou volane z ruznych/vnorenych `Ui` - napr. `render_grid`
    /// bezi uvnitr `egui::ScrollArea`, takze `ui.id()` tam NENI stejne
    /// jako `ui.id()` v `handle_keyboard`). Skutecne pozadani o fokus
    /// (`Response::request_focus`) se deje jen v `render_grid`, kde tohle
    /// `id` patri SKUTECNE interaktivni `Response` (viz tamni komentar).
    fn focus_id(&self) -> egui::Id {
        egui::Id::new(("term_kbd_focus", self.session.id))
    }

    /// Rucne "doopise" dane bajty primo do terminaloveho bufferu
    /// (stejnou cestou, jakou normalne prochazeji data prijata ze
    /// serveru, viz `pump`) - pouzito pro lokalni prihlasovaci prompt
    /// (`handle_credentials_keyboard`), ktery ZE SITE nikam nechodi,
    /// takze bez tohoto rucniho zapisu by v terminalu vubec nebyl
    /// videt.
    fn local_echo(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.parser.advance(&mut self.term, b);
        }
    }

    /// Zacne (nebo znovu zacne po odmitnutem hesle, viz
    /// SshEvent::AuthFailed v pump) lokalni prihlasovaci prompt primo
    /// v bufferu terminalu - sdileno mezi prvnim pokusem
    /// (SshEvent::AwaitingCredentials) a kazdym dalsim opakovanim po
    /// spatnem hesle, aby se oba pripady nelisily v nicem jinem nez v
    /// tom, jestli pred timto volanim pump jeste vypise hlasku o
    /// odmitnuti. Rozepsane udaje z pripadneho predchoziho pokusu se
    /// zahodi - uzivatel zadava jmeno i heslo od znovu.
    fn begin_credentials_prompt(&mut self) {
        self.awaiting_credentials = true;
        self.credentials_stage = CredentialsStage::Username;
        self.pending_username.clear();
        self.pending_password.clear();
        self.local_echo(b"login as: ");
    }

    /// Zachyti klavesovy vstup z aktualniho snimku a preposle jej (jako
    /// syrove bajty/ANSI escape sekvence) do SSH spojeni.
    ///
    /// Ctrl+C (i Cmd+C na Macu) VZDY jde primo na server jako obycejny
    /// ridici bajt 0x03 (SIGINT) - presne jako kazde jine Ctrl+pismeno,
    /// viz `key_to_bytes`/`ctrl_control_code`. Zpetna vazba "používáme
    /// CTRL+c/v na kopírování/vložení... CTRL+c je třeba poslat
    /// serveru" - kopirovani oznaceneho textu uz NENI navazane na
    /// zadnou klavesu, deje se automaticky primo pri dokonceni tazeni
    /// mysi (viz `handle_selection_input`), presne jak se chova PuTTY
    /// a vetsina "klasickych" terminalu. Ctrl+V zustava beze zmeny -
    /// vkladani ze schranky resi `egui::Event::Paste` nize.
    ///
    /// Kdyz ma prave fokus nejaky JINY widget (napr. textove pole v
    /// otevrenem dialogu "Nový server"/Nastaveni, nebo vyhledavani ve
    /// stromu serveru), klavesnice terminalu vubec nepatri - jinak by se
    /// psani do takoveho pole soucasne posilalo i na SSH kanal (zpetna
    /// vazba: "když píšu v popup okně tak zároven píšu i v terminálu").
    /// Terminal sam jinak zadny skutecny fokusovatelny widget (`TextEdit`
    /// apod.) nema - vzdy jen rucne vykresluje (`render_grid`) - proto se
    /// pro sebe explicitne "zamyka" klavesovy fokus nize (`request_focus`
    /// + `set_focus_lock_filter` pod vlastnim `focus_id`): bez toho by (protoze
    /// zadny jiny widget fokus nedrzi) holy TAB odbocil na vestavenou
    /// fokus-navigaci egui (skok na nejaky fokusovatelny prvek v UI),
    /// misto aby se poslal na server jako bajt 0x09 - zpetna vazba
    /// "šipky pro posun fungují, ale TAB pro doplňování příkazů ne" (a
    /// pak jeste "TAB mi pořád neposílá do terminálu ale běhá po menu" -
    /// PRVNI pokus o opravu pouzival vlastni `Id` nenavazany na zadny
    /// skutecny vykresleny widget, coz egui proste ignorovalo). Skutecne
    /// pozadani/zamknuti fokusu se ted deje AZ v `render_grid` - jedine
    /// misto, kde existuje skutecna interaktivni `Response` terminalu
    /// (`self.focus_id()` je jeji `id`) - tahle funkce uz jen kontroluje,
    /// jestli fokus prave nedrzi NEJAKY JINY widget.
    /// `ctx.memory(|m| m.focused())` je tak `Some(nejake_jine_id)` prave
    /// a jen tehdy, kdyz fokus drzi skutecne JINY widget; jakmile ho
    /// ztrati (dialog se zavre, pole ztrati fokus), egui to samo rozpozna
    /// na dalsim snimku a klavesnice se terminalu vrati bez dalsiho
    /// zasahu.
    fn handle_keyboard(&mut self, ui: &egui::Ui) {
        let focus_id = self.focus_id();
        if ui.ctx().memory(|m| m.focused().is_some_and(|f| f != focus_id)) {
            return;
        }

        let events = ui.input(|i| i.events.clone());
        // Jestli se v tomto snimku poslalo na server aspon neco - kdyz
        // ano, na konci se zobrazeni vrati zpet na spodek (viz nize),
        // stejne jako to dela vetsina beznych terminalu, kdyz uzivatel
        // zacne psat, zatimco je odscrollovany do historie (viz zpetna
        // vazba "posun v konverzaci, když se okno posune tak nemám čím
        // skrolovat v TABu", ktera si zaroven vyzadala samotne skrolovani
        // v `render_grid`).
        let mut sent_input = false;

        for event in events {
            match event {
                // Bezny text (pismena, cislice, mezera, diakritika, ...) -
                // egui uz sam rozlisuje "napsatelny" text od ridicich
                // klaves, takze staci poslat rovnou.
                egui::Event::Text(text) => {
                    self.send_bytes(text.into_bytes());
                    sent_input = true;
                }
                // Vlozeni ze schranky (Ctrl+V/Cmd+V) - viz i vyjimka pro
                // `Key::V` v `key_to_bytes` nize, ktera brani tomu, aby se
                // NAVIC poslal i syrovy ridici bajt 0x16 za tento vlozeny
                // text.
                egui::Event::Paste(text) => {
                    self.send_bytes(text.into_bytes());
                    sent_input = true;
                }
                // Ctrl+C/Cmd+C uz NENI vyjimka - jde vzdy primo na server
                // jako normalni ridici bajt (0x03), viz `key_to_bytes`/
                // `ctrl_control_code`. Kopirovani oznaceneho textu resi
                // misto toho automaticky `handle_selection_input`.
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    // `APP_CURSOR` = server pozadal (DECCKM escape
                    // sekvenci) o "aplikacni" rezim sipek (`ESC O <pismeno>`
                    // misto normalniho `ESC [ <pismeno>`) - presne tohle
                    // `alacritty_terminal` uz sam parsuje/sleduje z
                    // prichozich dat. Bez tohoto rozliseni (drive se
                    // posilal vzdy jen "normalni" rezim) Avaya CM SAT
                    // sipky doleva/doprava interpretoval jinak, nez
                    // ocekavano ("skáčou po formuláři místo aby se
                    // posouvaly v textu").
                    let app_cursor_keys = self.term.mode().contains(TermMode::APP_CURSOR);
                    if let Some(bytes) = key_to_bytes(key, modifiers, self.session.term_type.as_deref(), app_cursor_keys) {
                        self.send_bytes(bytes);
                        sent_input = true;
                    }
                }
                // DULEZITE: backend (egui-winit) zachytava Ctrl+C/Cmd+C
                // (a na Windows i Ctrl+Insert) uz PRED tim, nez by se
                // dostal jako normalni `Event::Key{key: C, ..}` sem -
                // misto toho posle rovnou tenhle `Event::Copy` (viz
                // `is_copy_command`/`on_keyboard_input` v egui-winit;
                // Key varianta se v tomhle pripade vubec NEPOSLE). Bez
                // tehle vetve tak Ctrl+C skoncil tise v `_ => {}` a na
                // server se nikdy neposlal ridici bajt 0x03 - zpetna
                // vazba "nefunguje poslání CTRL+c". Kopirovani
                // oznaceneho textu uz mezitim resi samo
                // `handle_selection_input` (auto-copy pri pusteni
                // tazeni mysi), takze tady je spravne vzdy poslat 0x03
                // na server, presne jako u kazdeho jineho Ctrl+pismene.
                egui::Event::Copy => {
                    self.send_bytes(vec![0x03]);
                    sent_input = true;
                }
                _ => {}
            }
        }

        if sent_input {
            self.term.scroll_display(Scroll::Bottom);
        }
    }

    /// Zachyti klavesovy vstup behem `ConnState::AwaitingCredentials` -
    /// na rozdil od `handle_keyboard` (ktera bajty posila primo na SSH
    /// kanal) tady znaky NIKAM PO SITI nejdou - jen se (u uzivatele)
    /// rucne "doopisou" primo do terminaloveho bufferu (`local_echo`),
    /// takze prihlasovaci prompt vypada jako soucast normalniho
    /// terminalu, presne jak se chova skutecny `ssh` klient. Heslo se
    /// (take stejne jako u `ssh`) vubec neechuje - zadny znak, zadna
    /// hvezdicka; i Backspace pri psani hesla zustava tiche (jen zmensi
    /// `pending_password`, na obrazovce se nic nezmeni).
    ///
    /// (Oznaceni/kopirovani textu mysi behem prihlasovaciho promptu
    /// resi uz `handle_selection_input` samo o sobe stejne jako jinde -
    /// ve hre tu je jen "login as: "/"Password: ", zadny skutecny vystup
    /// ze serveru, takze zvlastni zpracovani tady netreba.)
    ///
    /// Stejna pojistka proti "utoku" klavesnice do soucasne otevreneho
    /// dialogu jako u `handle_keyboard` - viz tamni komentar (vcetne
    /// stejneho `focus_id` - TAB tu sice nema smysl zamykat, ale
    /// sdileny identifikator drzi chovani konzistentni pri prepnuti
    /// mezi timto stavem a `handle_keyboard` v ramci jedne relace).
    fn handle_credentials_keyboard(&mut self, ui: &egui::Ui) {
        let focus_id = self.focus_id();
        if ui.ctx().memory(|m| m.focused().is_some_and(|f| f != focus_id)) {
            return;
        }

        let events = ui.input(|i| i.events.clone());
        // Stejny duvod jako `sent_input` v `handle_keyboard` - viz tam.
        let mut sent_input = false;
        for event in events {
            match event {
                egui::Event::Text(text) | egui::Event::Paste(text) => {
                    for ch in text.chars() {
                        // Ridici znaky (Enter/Backspace/...) uz reseji
                        // samostatne vetve `Event::Key` nize - `Text`
                        // by je normalne poslat nemelo, ale pro jistotu
                        // se tu jeste jednou vyfiltruji.
                        if !ch.is_control() {
                            self.credentials_push_char(ch);
                            sent_input = true;
                        }
                    }
                }
                egui::Event::Key { key: egui::Key::Backspace, pressed: true, .. } => {
                    self.credentials_backspace();
                    sent_input = true;
                }
                egui::Event::Key { key: egui::Key::Enter, pressed: true, .. } => {
                    self.credentials_submit_line();
                    sent_input = true;
                }
                _ => {}
            }
        }

        if sent_input {
            self.term.scroll_display(Scroll::Bottom);
        }
    }

    fn credentials_push_char(&mut self, ch: char) {
        match self.credentials_stage {
            CredentialsStage::Username => {
                self.pending_username.push(ch);
                let mut buf = [0u8; 4];
                self.local_echo(ch.encode_utf8(&mut buf).as_bytes());
            }
            CredentialsStage::Password => {
                self.pending_password.push(ch);
            }
        }
    }

    fn credentials_backspace(&mut self) {
        match self.credentials_stage {
            CredentialsStage::Username => {
                if self.pending_username.pop().is_some() {
                    // Posun kurzoru zpet, prepsani mezerou, posun zpet
                    // znovu - standardni zpusob, jak se "smaze" znak na
                    // terminalu, ktery sam o sobe zadny backspace
                    // nezna.
                    self.local_echo(b"\x08 \x08");
                }
            }
            CredentialsStage::Password => {
                self.pending_password.pop();
            }
        }
    }

    /// Slozi text aktualne oznaceneho useku (`self.selection`) primo z
    /// mrizky terminalu - `None` kdyz nic neni oznaceno NEBO by vysledny
    /// text byl prazdny (napr. oznaceni jen prazdnych bunek). Krajni
    /// mezery na konci kazdeho radku se orizavaji (bezne vycpavkove
    /// mezery az do sirky mrizky by jinak zbytecne zaneradily zkopirovany
    /// text) - vnitrni mezery zustavaji zachovane.
    fn selected_text(&self) -> Option<String> {
        let selection = self.selection?;
        let grid = self.term.grid();
        // `row` v `selection` uz je ABSOLUTNI radek mrizky (viz komentar u
        // `Selection`/`point_to_cell`) - zadny dalsi prevod pres
        // `display_offset` tu tedy netreba, na rozdil od drivejsi verze,
        // ktera si (nespravne pro pripad skrolovani BEHEM tazeni) drzela
        // jen vizualni radky.
        let (start, end) = selection.normalized();
        let mut result = String::new();
        for row in start.0..=end.0 {
            if row > start.0 {
                result.push('\n');
            }
            let mut line = String::new();
            for col in 0..grid.columns() {
                if selection.contains(row, col) {
                    let point = Point::new(Line(row), Column(col));
                    let ch = grid[point].c;
                    line.push(if ch == '\0' { ' ' } else { ch });
                }
            }
            result.push_str(line.trim_end());
        }
        if result.trim().is_empty() {
            None
        } else {
            Some(result)
        }
    }

    /// Prevede pozici mysi (v souradnicich cele obrazovky) na bunku
    /// mrizky (radek, sloupec) - pouzito pro `handle_selection_input`.
    /// `rect` je oblast, ve ktere se terminalovy text vykresluje (viz
    /// `render_grid`).
    ///
    /// POZOR: zamerne se to pocita ze SKUTECNE polohy znaku ve
    /// vykreslenem `galley` (`Galley::cursor_from_pos`), NE odhadem
    /// "sirka znaku (`char_w` z `glyph_width('M')`) krat poradove cislo
    /// sloupce" - ten byl u pouziteho pisma/DPI mirne nepresny (skutecny
    /// rozestup znaku se od `glyph_width('M')` drobet lisil) a chyba se s
    /// kazdym dalsim sloupcem SCITALA, takze cim dal doprava se kliklo,
    /// tim vetsi byl vysledny posun oznaceni oproti mistu kliknuti -
    /// presne podle zpetne vazby "kliknu těsně za 2 a táhnu myš pro
    /// označení ale označí se mi až x znaků od kliknutí". `cursor_from_pos`
    /// se pta primo `galley`, kde skutecne (pixel presne) ktery znak lezi,
    /// takze zadna takova kumulujici se chyba vzniknout nemuze. Kazdy
    /// radek v `galley` ma vzdy presne `cols` znaku (viz `build_layout_job` -
    /// prazdne bunky se vyplnuji mezerou), takze vraceny sloupec by mel
    /// vzdy uz sam o sobe byt v platnem rozsahu - `min` nize je jen
    /// pojistka.
    ///
    /// `display_offset` (viz `Grid::display_offset`/`Selection` komentar) -
    /// vraceny radek uz je prevedeny do ABSOLUTNIHO prostoru mrizky
    /// (`vizualni_radek - display_offset`), ne jen vizualni 0-based radek
    /// aktualne vykreslene obrazovky, aby vysledna `Selection` zustala
    /// spravna i pri pozdejsim skrolovani (viz `render_grid`).
    fn point_to_cell(pos: egui::Pos2, rect: egui::Rect, galley: &egui::Galley, cols: usize, rows: usize, display_offset: i32) -> (i32, usize) {
        // Mys se behem tazeni muze dostat i MIMO `rect` (napr. tazenim za
        // horni/dolni/pravy okraj terminalu) - `interact_pointer_pos`
        // (viz `handle_selection_input`) vraci SKUTECNOU polohu kurzoru,
        // nijak neorezanou na `rect`. Bez tohoto rucniho oriznuti tak
        // `local` nize mohlo vyjit zaporne (tazeni nad/vlevo od `rect`)
        // nebo vetsi, nez `galley` skutecne ma (tazeni pod/vpravo), coz
        // `cursor_from_pos` na krajich spolehlive nezvladalo - presne
        // zpetna vazba "nepřesně mi chytá začátek označení" (klik/zacatek
        // tazeni tesne u okraje) a "když jedu přes okraj... nedovolí
        // označit posunout a nebo označí nesmysl" (tazeni MIMO rect behem
        // pokracovani vyberu). Priskrtenim pozice dovnitr `rect` PRED
        // dotazem na `galley` se misto toho vyber jednoduse rozsiri az k
        // posledni/prvni viditelne bunce - presne UX beznych terminalu
        // (v kombinaci s automatickym skrolovanim u okraje v `render_grid`
        // navic jde takto dosahnout i na obsah, ktery je PRAVE TED mimo
        // viditelnou cast, viz komentar u `Selection`).
        let clamped = egui::pos2(pos.x.clamp(rect.min.x, rect.max.x), pos.y.clamp(rect.min.y, rect.max.y));
        let local = clamped - rect.min;
        let cursor = galley.cursor_from_pos(local);
        let visual_row = cursor.rcursor.row.min(rows.saturating_sub(1));
        let col = cursor.rcursor.column.min(cols.saturating_sub(1));
        (visual_row as i32 - display_offset, col)
    }

    /// Najde hranice "slova" (viz [`CharClass`]/`char_class`) na
    /// (ABSOLUTNIM, viz `Selection`) radku `row` mrizky kolem sloupce
    /// `col` a vrati (pocatecni, koncovy) sloupec oznaceni (oba vcetne) -
    /// pouzito dvojklikem (viz `handle_selection_input`). Kdyz je mrizka
    /// prazdna (`cols() == 0`), vrati proste `(col, col)` - v praxi by
    /// nemelo nastat, jen pojistka.
    fn word_bounds_at(&self, row: i32, col: usize) -> (usize, usize) {
        let grid = self.term.grid();
        let cols = grid.columns();
        if cols == 0 {
            return (col, col);
        }
        let col = col.min(cols - 1);
        let char_at = |c: usize| -> char {
            let ch = grid[Point::new(Line(row), Column(c))].c;
            if ch == '\0' { ' ' } else { ch }
        };
        let target = char_class(char_at(col));
        let mut start = col;
        while start > 0 && char_class(char_at(start - 1)) == target {
            start -= 1;
        }
        let mut end = col;
        while end + 1 < cols && char_class(char_at(end + 1)) == target {
            end += 1;
        }
        (start, end)
    }

    /// Zpracuje tazeni/klik mysi nad textem terminalu (viz `render_grid`)
    /// a podle toho aktualizuje `self.selection`. Zacatek tazeni zalozi
    /// nove oznaceni, pokracujici tazeni posouva jeho konec, a obycejny
    /// klik (bez tazeni) predchozi oznaceni zrusi - presne jak se chova
    /// kazdy bezny terminal.
    ///
    /// Jakmile tazeni SKONCI (`drag_stopped`), oznaceny text se rovnou
    /// automaticky zkopiruje do schranky - zadna dalsi klavesa/kliknuti
    /// netreba (presne UX jako PuTTY a vetsina "klasickych" terminalu).
    /// Zpetna vazba "používáme CTRL+c/v na kopírování/vložení...
    /// kopírování by mohlo být automatické když něco označíme" - Ctrl+C
    /// uz tak nema dvoji vyznam a vzdy jde primo na server, viz
    /// `handle_keyboard`.
    fn handle_selection_input(&mut self, response: &egui::Response, rect: egui::Rect, galley: &egui::Galley, cols: usize, rows: usize, display_offset: i32) {
        // Trojklik (oznaceni cele radky) a dvojklik (oznaceni "slova" pod
        // kurzorem, viz `word_bounds_at`) MUSI se resit PRED obycejnym
        // tazenim/klikem nize - treti/druhy klik v rychlem sledu totiz
        // vedle `triple_clicked()`/`double_clicked()` vyvola egui i
        // beznou sadu udalosti (`clicked()`, pripadne i `drag_started()`)
        // pro TENTO stejny snimek, takze bez tohoto poradi (od
        // nejspecifictejsiho) by trojklik/dvojklik skoncil jen jako
        // dvojklik/obycejny klik (nebo by rovnou zrusil prave vytvorene
        // oznaceni pres vetev `clicked()` nize). Stejne jako tazeni mysi
        // (`drag_stopped` nize) se i tady oznaceny text rovnou automaticky
        // zkopiruje - zadna dalsi klavesa netreba (presne UX beznych
        // terminalu, viz i komentar u funkce).
        if response.triple_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let (row, _) = Self::point_to_cell(pos, rect, galley, cols, rows, display_offset);
                self.selection = Some(Selection { anchor: (row, 0), current: (row, cols.saturating_sub(1)) });
                if let Some(text) = self.selected_text() {
                    if !text.is_empty() {
                        response.ctx.copy_text(text);
                    }
                }
            }
        } else if response.double_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let (row, col) = Self::point_to_cell(pos, rect, galley, cols, rows, display_offset);
                let (start_col, end_col) = self.word_bounds_at(row, col);
                self.selection = Some(Selection { anchor: (row, start_col), current: (row, end_col) });
                if let Some(text) = self.selected_text() {
                    if !text.is_empty() {
                        response.ctx.copy_text(text);
                    }
                }
            }
        } else if response.drag_started() {
            // POZOR: zamerne NE `response.interact_pointer_pos()` tady -
            // `drag_started()` je `true` az na snimku, kdy tazeni prekroci
            // vnitrni prah egui (nekolik pixelu pohybu od stisknuti
            // tlacitka), a `interact_pointer_pos()` v tu chvili vraci
            // AKTUALNI (uz posunutou) polohu kurzoru, ne misto puvodniho
            // stisknuti. Kotva oznaceni tak byla o par pixelu (typicky i o
            // cely znak) posunuta oproti mistu, kam uzivatel skutecne
            // kliknul - presne zpetna vazba "nepřesně mi chytá začátek
            // označení", ktera prezila i drivejsi opravu orezavani na
            // `rect` (ta resila jen tazeni MIMO okraj, ne tohle).
            // `PointerState::press_origin()` vraci presne misto, kde bylo
            // tlacitko puvodne stisknuto, bez ohledu na tento prah - proto
            // se pouziva pro kotvu misto `interact_pointer_pos()`.
            // `interact_pointer_pos()` zustava jako zalozni varianta pro
            // nepravdepodobny pripad, ze by `press_origin()` bylo `None`.
            let pos = response
                .ctx
                .input(|i| i.pointer.press_origin())
                .or_else(|| response.interact_pointer_pos());
            if let Some(pos) = pos {
                let cell = Self::point_to_cell(pos, rect, galley, cols, rows, display_offset);
                self.selection = Some(Selection { anchor: cell, current: cell });
            }
        } else if response.dragged() {
            if let (Some(pos), Some(sel)) = (response.interact_pointer_pos(), self.selection.as_mut()) {
                sel.current = Self::point_to_cell(pos, rect, galley, cols, rows, display_offset);
            }
        } else if response.drag_stopped() {
            if let Some(text) = self.selected_text() {
                if !text.is_empty() {
                    response.ctx.copy_text(text);
                }
            }
        } else if response.clicked() {
            self.selection = None;
        }
    }

    /// Vykresli mrizku terminalu (obsah `self.term`) do vlastni presne
    /// vypocitane oblasti a zpracuje nad ni tazeni/klik mysi pro oznaceni
    /// textu (`handle_selection_input`) - narozdil od puvodniho
    /// `ui.add(Label::new(job).selectable(false))` jde o rucni vykresleni
    /// (`ui.painter().galley`), aby bylo mozne presne mapovat pozici mysi
    /// na konkretni bunku mrizky (radek/sloupec) a nad textem zaroven
    /// registrovat vlastni interaktivni oblast (`Sense::click_and_drag`).
    fn render_grid(&mut self, ui: &mut egui::Ui, font_size: f32, focused: bool) {
        let font_id = terminal_font(font_size);
        let (char_w, row_h) = ui.fonts(|f| (f.glyph_width(&font_id, 'M'), f.row_height(&font_id)));
        let (cols, rows) = { let grid = self.term.grid(); (grid.columns(), grid.screen_lines()) };

        // `char_w`/`row_h` (odhad ze sirky znaku 'M') se pouzivaji JEN pro
        // celkovou velikost oblasti (tady) - presne mapovani pozice mysi
        // na konkretni znak uz nize resi `handle_selection_input` primo
        // pres skutecnou geometrii `galley` (viz `point_to_cell`), takze
        // pripadna drobna nepresnost tohoto odhadu uz oznaceni textu
        // neovlivni.
        let size = egui::vec2(char_w * cols as f32, row_h * rows as f32);
        // DULEZITE: `ui.interact` s VLASTNIM explicitnim `id` (misto
        // `ui.allocate_exact_size`, ktere by vygenerovalo sve vlastni
        // automaticke id) - jen tak `self.focus_id()` (pouzite i v
        // `handle_keyboard`/`handle_credentials_keyboard`) skutecne
        // odpovida id teto konkretni interaktivni `Response`, a egui
        // pozadavek na fokus/zamknuti Tabu nize bere vazne. (Puvodni
        // pokus pouzival `ui.id().with(...)` vytvorene v UPLNE JINE
        // casti stromu Ui - takove id nepatrilo zadnemu skutecnemu
        // widgetu, takze ho egui pri navigaci Tabem proste ignorovalo.)
        let focus_id = self.focus_id();
        let (_, rect) = ui.allocate_space(size);
        // `Sense::click_and_drag()` ma vnitrne `focusable: false` (jen
        // klik/tazeni, ne klavesovy fokus) - protoze na tuto plochu nize
        // AKTIVNE voláme `request_focus()`, musime `focusable` rucne
        // nastavit na `true`, jinak zustava widget v nekonzistentnim
        // stavu ("ma fokus, ale sam sebe fokusovatelny neoznacuje"), coz
        // pravdepodobne zpusobovalo, ze nektere klavesy (napr. Ctrl+C)
        // uz nedosly az do `handle_keyboard` - zpetna vazba "nefunguje
        // poslání CTRL+c a pravé tlačítko myši" (pravé tlačítko bylo
        // mezitim docasne odstraneno kvuli jinemu problemu, viz predchozi
        // commit - tohle se tyka jen Ctrl+C).
        let sense = egui::Sense { focusable: true, ..egui::Sense::click_and_drag() };
        let response = ui.interact(rect, focus_id, sense);

        if focused {
            // Nechceme krast fokus jinemu, skutecne fokusovanemu widgetu
            // (napr. otevrenemu dialogu) - viz `handle_keyboard`.
            let other_focused = ui.ctx().memory(|m| m.focused().is_some_and(|f| f != focus_id));
            if !other_focused {
                response.request_focus();
                // `set_focus_lock_filter` rika egui, ktere klavesy s
                // timto fokusem NEMAJI spoustet jeho vestavene chovani
                // (fokus-navigace pro Tab/sipky, zruseni fokusu pro
                // Escape), ale maji se misto toho poslat jako normalni
                // `Event::Key` do `handle_keyboard` - presne to, co
                // terminal potrebuje pro TAB (doplnovani prikazu), sipky
                // (historie) i Escape (napr. `vim`).
                ui.memory_mut(|m| {
                    m.set_focus_lock_filter(
                        focus_id,
                        egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true },
                    );
                });
            }
        } else if response.has_focus() {
            // Tento panel prave neni aktivni (napr. Ctrl+Tab v rozdelenem
            // zobrazeni prepnul na druhy panel) - uvolni drzeny fokus, aby
            // TAB v prave aktivnim panelu fungoval bez omezeni.
            response.surrender_focus();
        }

        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }

        // Skrolovani historie kolieckem mysi (zpetna vazba "posun v
        // konverzaci, když se okno posune tak nemám čím skrolovat v
        // TABu") - jen kdyz je mys prave NAD timto konkretnim terminalem
        // (`response.hovered()`), aby v rozdelenem zobrazeni (dva
        // `TerminalSession` vedle sebe) kolecko ovlivnilo vzdy jen ten
        // panel, nad kterym mys skutecne je - na rozdil od klavesnice
        // (viz `focused` u `render`) tu totiz zadne takove rozliseni
        // netreba resit navic, poloha mysi uz to rika sama.
        //
        // `raw_scroll_delta.y` je v pixelech za tento snimek - preveden
        // na pocet radku pres uz zmereny `row_h` (vyska radku aktualniho
        // pisma). Kdyz je vysledek po zaokrouhleni `0` (typicky u
        // trackpadu s hodne jemnymi/pomalymi kroky), posune se aspon o
        // jeden radek smerem odpovidajicim znamenku puvodni hodnoty, aby
        // sebemensi pohyb kolecka/trackpadu vzdy neco udelal.
        //
        // `!response.dragged()` navic zamerne vylucuje skrolovani behem
        // PRAVE PROBIHAJICIHO tazeni vyberu textu (`handle_selection_input`
        // nize) - `selected_text`/`build_layout_job` pocitaji s
        // `display_offset` platnym AZ PO dokonceni tazeni, takze kdyby se
        // behem tazeni zmenil (napr. kolieckem na trackpadu, ktery umi
        // poslat scroll udalost i pri drzenem tlacitku), vysledny
        // zkopirovany text by mohl odpovidat uplne jinym radkum, nez
        // uzivatel skutecne oznacil.
        if response.hovered() && !response.dragged() {
            let scroll_y = ui.input(|i| i.raw_scroll_delta.y);
            if scroll_y != 0.0 {
                // `Scroll::Delta` bere `i32` (overeno skutecnym `cargo
                // build` - alacritty_terminal 0.24.2, `grid/mod.rs`), ne
                // `isize`, jak puvodne (neovereno) predpokladal predchozi
                // POZNAMKA K OVERENI zde.
                let mut lines = (scroll_y / row_h).round() as i32;
                if lines == 0 {
                    lines = if scroll_y > 0.0 { 1 } else { -1 };
                }
                // POZNAMKA K OVERENI: znamenko `Scroll::Delta` (kladna
                // hodnota by mela podle dokumentace `alacritty_terminal`
                // posouvat pohled NAHORU do historie, zaporna zpet dolu)
                // porad nebylo mozne overit skutecnym behem, jen typ
                // parametru vyse - pokud by kolecko skrolovalo obracene,
                // staci tu prohodit `1`/`-1` (a znamenko `scroll_y`).
                self.term.scroll_display(Scroll::Delta(lines));
            }
        }

        // Zpetna vazba "CTRL+v bychom mohli dát i kliknutí pravou myší" -
        // klik pravym tlacitkem myslu VZDY vlozi aktualni obsah schranky
        // (presne UX jako PuTTY a vetsina "klasickych" terminalu), bez
        // ohledu na `focused` - jde o klik NA KONKRETNI panel, ne o
        // globalni klavesovou udalost snimku (na rozdil od Ctrl+V), takze
        // se prirozene tyka jen toho panelu, na ktery uzivatel skutecne
        // klikl. Cteni schranky (na rozdil od `Context::copy_text`, ktere
        // ma egui vestavene) resi primo `arboard` - `eframe` uz ho sam
        // pouziva interne pro vlastni Ctrl+V, jde tedy o vyuziti uz
        // existujici zavislosti (viz `Cargo.toml`), ne o novou.
        //
        // (Predchozi pokus o tuhle funkci v0.3.4 rozbil release build,
        // protoze zmena v `Cargo.lock` - novy primy zaznam zavislosti na
        // `arboard` - byla dopsana rucne bez skutecneho spusteni cargo.
        // Tentokrat se `Cargo.lock` NEUPRAVUJE rucne vubec - musi se
        // pred tagovanim noveho vydani spustit skutecny `cargo
        // build`/`cargo check`, aby si lockfile spravne doplnil sam.)
        if response.secondary_clicked() {
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                if let Ok(text) = clipboard.get_text() {
                    if !text.is_empty() {
                        self.send_bytes(text.into_bytes());
                    }
                }
            }
        }

        // Automaticke skrolovani historie behem tazeni vyberu, kdyz mys
        // drzi ZA hornim/dolnim okrajem terminalu (zpetna vazba "a jak
        // bychom označili víc textu tedy i ten který je momentálně už
        // nahoře mimo viditelné okno?") - bez tohohle by uzivatel nemel
        // jak tazenim vubec DOSAHNOUT na radky, ktere jsou prave
        // odscrolovane mimo viditelnou oblast (`rect`), protoze
        // `interact_pointer_pos()` v `handle_selection_input` nize umi
        // mapovat jen to, co uz galley vykresluje.
        //
        // Vzdalenost drzeni mysi nad/pod `rect` (v pixelech, `.max(0.0)`
        // kdyz je mys uvnitr) se preposte pres `row_h` na pocet radku a
        // pouzije primo jako velikost kroku `Scroll::Delta` pro tento
        // snimek - cim dal uzivatel mysi drzi, tim rychleji se historie
        // posouva, ale `EDGE_AUTOSCROLL_MAX_LINES` tuhle rychlost shora
        // omezuje (viz komentar u konstanty), aby drzeni daleko od okraje
        // (napr. az u stavoveho prouzku pod terminalem) neproskocilo
        // desitky/stovky radku za jediny snimek. `+1`, aby uz samotne
        // opusteni `rect` (vzdalenost tesne nad 0) hned zpusobilo posun
        // aspon o jeden radek, misto aby cekalo, az vzdalenost naroste na
        // cely radek.
        //
        // Znamenko: drzeni NAD hornim okrajem odhaluje STARSI historii
        // (kladne `Scroll::Delta`, stejne jako kolecko mysi nahoru vyse),
        // drzeni POD dolnim okrajem posouva zpet k aktualnimu obsahu
        // (zaporne). Bezi jen kdyz `response.dragged()` je `true`, takze
        // mimo aktivni tazeni vyberu se historie takhle sama neposouva.
        if response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                let over_top = (rect.min.y - pos.y).max(0.0);
                let over_bottom = (pos.y - rect.max.y).max(0.0);
                if over_top > 0.0 {
                    let lines = 1 + (over_top / row_h).floor() as i32;
                    self.term.scroll_display(Scroll::Delta(lines.min(EDGE_AUTOSCROLL_MAX_LINES)));
                    ui.ctx().request_repaint();
                } else if over_bottom > 0.0 {
                    let lines = 1 + (over_bottom / row_h).floor() as i32;
                    self.term.scroll_display(Scroll::Delta(-lines.min(EDGE_AUTOSCROLL_MAX_LINES)));
                    ui.ctx().request_repaint();
                }
            }
        }

        // Galley se sestavi PRED zpracovanim tazeni mysi (viz nize), aby
        // `handle_selection_input` mohlo pro tento snimek pouzit jeho
        // skutecnou geometrii - barvy zvyrazneni oznaceni v nem sice jeste
        // odpovidaji stavu `self.selection` PRED timto snimkem (o jeden
        // snimek pozadu, cca 33 ms - viz `request_repaint_after` v
        // `render`), ale to je pro polohu/rozmery jednotlivych znaku
        // bezvyznamne (barva na rozlozeni znaku nema vliv), takze presnost
        // mapovani mysi na znak tim nijak netrpi.
        let job = self.build_layout_job(font_size);
        let galley = ui.fonts(|f| f.layout_job(job));

        // `display_offset` se cte AZ TED, po pripadnem autoscrollu vyse -
        // `handle_selection_input`/`point_to_cell` nize s nim preklada
        // viditelnou (vizualni) pozici mysi na absolutni radek mrizky
        // (viz `Selection`), takze musi odpovidat prave tomuto snimku.
        let display_offset = self.term.grid().display_offset() as i32;
        self.handle_selection_input(&response, rect, &galley, cols, rows, display_offset);

        ui.painter().galley(rect.min, galley, theme::TEXT);
    }

    fn credentials_submit_line(&mut self) {
        match self.credentials_stage {
            CredentialsStage::Username => {
                if self.pending_username.trim().is_empty() {
                    return;
                }
                self.local_echo(b"\r\n");
                self.local_echo(b"Password: ");
                self.credentials_stage = CredentialsStage::Password;
            }
            CredentialsStage::Password => {
                self.local_echo(b"\r\n");
                self.submit_credentials();
            }
        }
    }

    /// Vykresli aktualni stav terminalu do daneho `Ui` (cely obsah
    /// Connection tabu) a zpracuje klavesovy vstup z tohoto snimku.
    /// `auto_reconnect` je aktualni hodnota nastaveni "automaticky se
    /// pokoušet obnovit ztracené spojení" (viz `MainApp::settings` v
    /// `app.rs`) - samotny `TerminalSession` si zadne globalni
    /// nastaveni nedrzi, dostava ho pri kazdem vykresleni zvenci. Stejne
    /// tak `font_size` (`AppSettings::term_font_size`, ovladane z
    /// Nastaveni i z View menu +/-, viz `app.rs`).
    ///
    /// `focused` rika, jestli tento terminal ma prave prijimat klavesovy
    /// vstup - v normalnim (nerozdelenem) zobrazeni je vzdy `true`, ale v
    /// rozdelenem zobrazeni (viz `app::MainApp::render_split_view`) je
    /// videt DVA `TerminalSession` soucasne, pricemz klavesove udalosti
    /// snimku (`ui.input(|i| i.events...)`) jsou GLOBALNI pro cely snimek,
    /// ne vazane na konkretni panel - bez tohoto priznaku by tak jeden
    /// stisk klavesy odeslal stejny bajt do OBOU SSH spojeni najednou.
    /// Vsechno ostatni (obcerstvovani obsahu, stavovy pruh, zmena
    /// velikosti) zustava bez ohledu na `focused` - i nefokusovany panel
    /// ma zustat "zivy" (aby v nem bylo videt novy vystup ze serveru).
    pub fn render(&mut self, ui: &mut egui::Ui, auto_reconnect: bool, lang: Lang, font_size: f32, focused: bool) {
        self.pump();
        let tr = i18n::t(lang);

        // Behem prihlasovaciho promptu (`ConnState::AwaitingCredentials`)
        // se klavesnice zpracovava JINAK - znaky nejdou na SSH kanal
        // (jeste neni autentizovany), ale rucne se "doopisou" primo do
        // terminaloveho bufferu, viz `handle_credentials_keyboard`. Ve
        // vsech ostatnich stavech beze zmeny jako drive. Cele se to navic
        // zpracuje jen kdyz je tento panel `focused` (viz komentar u
        // signatury vyse).
        if focused {
            if self.state() == ConnState::AwaitingCredentials {
                self.handle_credentials_keyboard(ui);
            } else {
                self.handle_keyboard(ui);
            }
        }
        // (Uvolneni drzeneho fokusu, kdyz `focused == false`, resi
        // `render_grid` nize - jedine misto se skutecnou interaktivni
        // `Response` terminalu.)

        // Dokud je tab otevreny/aktivni, chceme obrazovku prubezne
        // obcerstvovat i bez interakce uzivatele (aby se novy vystup ze
        // serveru objevil hned, ne az pri dalsim kliknuti/klavese, a aby
        // fungoval i casovac automatickeho obnoveni spojeni nize).
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));

        match self.state() {
            // Zadna hlaska nad terminalem - prihlasovaci prompt ("login
            // as: "/"Password: ") uz je videt primo V terminalu (viz
            // `pump`/`handle_credentials_keyboard`), zadny dalsi banner
            // navic netreba.
            ConnState::AwaitingCredentials => {}
            ConnState::Disconnected => {
                // Kdyz je automaticke obnoveni zapnute, zkusi se samo -
                // tlacitko "Připojit znovu" nize zustava funkcni i tak
                // (okamzity rucni pokus, bez cekani na dalsi casovy
                // interval).
                self.maybe_auto_reconnect(auto_reconnect);

                ui.horizontal(|ui| {
                    let message = match &self.error {
                        Some(err) => i18n::connection_failed(lang, err),
                        None => tr.connection_ended.to_string(),
                    };
                    ui.colored_label(theme::DANGER, message);
                    if ui.button(tr.btn_reconnect).clicked() {
                        self.reconnect();
                    }
                    if auto_reconnect {
                        ui.label(egui::RichText::new(tr.auto_reconnect_hint).small().weak());
                    }
                });
                ui.add_space(6.0);
            }
            ConnState::Connecting => {
                ui.label(egui::RichText::new(tr.connecting_label).small());
                ui.add_space(6.0);
            }
            ConnState::Connected => {}
        }

        // Info proužek se systemovymi metrikami se pripne dolu JESTE
        // PRED `resize_to_fit`, aby si vzal svuj kousek plochy jako
        // prvni a prepocet velikosti terminalu uz pocital jen s tim, co
        // zbyde nad nim (presne jak to vypada na predloze z MobaXtermu -
        // proužek pod oknem terminalu, ne pres nej).
        self.render_status_bar(ui, lang);

        // Az TED (po pripadnych hlaskach vyse, ktere uz zabraly kus
        // plochy tohoto snimku) - viz `resize_to_fit`.
        self.resize_to_fit(ui, font_size);

        egui::ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            self.render_grid(ui, font_size, focused);
        });
    }

    /// Vykresli MobaXterm-podobny info proužek se systemovymi metrikami
    /// (`self.stats`) pripnuty ke spodnimu okraji tabu
    /// (`egui::TopBottomPanel::bottom(...).show_inside`, ne `.show` - ten
    /// by se vztahoval na cele okno aplikace, ne jen na tento tab).
    /// Dokud jeste nedorazilo prvni periodicke obcerstveni statistik
    /// (`self.stats == None`, viz `pump`), proužek se vubec nezobrazuje -
    /// jednodussi a citelnejsi nez zobrazovat radek plny "-" hodnot.
    fn render_status_bar(&self, ui: &mut egui::Ui, lang: Lang) {
        let Some(stats) = &self.stats else { return };
        let tr = i18n::t(lang);

        // (zobrazeny text, popisek do bubliny při najetí myší) - viz
        // pozadavek "najetím myši nad info proužek bychom mohli v
        // bublině říct co dané znamená".
        let mut items: Vec<(String, &'static str)> =
            vec![(format!("🔌 {}", format_host_display(&self.session, stats)), tr.status_host_tooltip)];

        if let Some(cpu) = stats.cpu_percent {
            items.push((format!("⚙ {}%", fmt_decimal(cpu as f64, 0)), tr.status_cpu_tooltip));
        }
        if let (Some(used), Some(total)) = (stats.mem_used_gb, stats.mem_total_gb) {
            items.push((format!("📊 {} / {} GB", fmt_decimal(used, 2), fmt_decimal(total, 2)), tr.status_mem_tooltip));
        }
        if let Some(up) = stats.net_up_mbps {
            items.push((
                // `🔼`/`🔽` (ne geometricke `▲`/`▼`) zamerne - `▲`/`▼`
                // patri do bloku "Geometric Shapes", ktery v pisme
                // pouzivanem pro tyto ikonky (stejne jako `🔌`/`⚙`/`📊`/
                // `🖥`/`👤`/`💾`) chybel, takze se obe vykreslovaly jako
                // stejny "chybejici znak" ctverecek - matoucí presne
                // podle zpetne vazby uzivatele.
                format!("🔼 {} Mb/s", fmt_decimal(up, 2)),
                tr.status_net_up_tooltip,
            ));
        }
        if let Some(down) = stats.net_down_mbps {
            items.push((format!("🔽 {} Mb/s", fmt_decimal(down, 2)), tr.status_net_down_tooltip));
        }
        if let Some(days) = stats.uptime_days {
            items.push((format!("🖥 {}", i18n::uptime_days(lang, days)), tr.status_uptime_tooltip));
        }
        items.push((
            if stats.user_sessions > 1 {
                format!("👤 {} (x{})", stats.username, stats.user_sessions)
            } else {
                format!("👤 {}", stats.username)
            },
            tr.status_user_tooltip,
        ));
        if let Some(disk) = stats.disk_percent {
            items.push((format!("💾 /: {disk}%"), tr.status_disk_tooltip));
        }

        // Pozadi bereme dynamicky z aktualniho tematu (`ui.visuals().panel_fill`),
        // ne napevno `theme::BG_PANEL` - stejna oprava jako u hlavniho
        // `CentralPanel` v `app.rs` (viz tam zpetna vazba "světlé téma
        // je nečitelné" a nasledne "ještě info panel vespod se
        // nepřepl barevně" - tenhle stavovy prouzek byl presne ten
        // zbyvajici hardcoded kousek).
        let panel_fill = ui.visuals().panel_fill;
        egui::TopBottomPanel::bottom(egui::Id::new(("term_status_bar", self.session.id)))
            .frame(
                egui::Frame::none()
                    .fill(panel_fill)
                    .inner_margin(egui::Margin::symmetric(10.0, 6.0)),
            )
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    // Kazda polozka je VLASTNI label (ne jeden spojeny
                    // retezec jako driv), aby na ni slo napojit vlastni
                    // bublinu (`on_hover_text`) - viz `items` vyse.
                    for (idx, (text, tooltip)) in items.iter().enumerate() {
                        if idx > 0 {
                            ui.label(egui::RichText::new("|").size(STATUS_BAR_FONT_SIZE).weak());
                        }
                        ui.label(egui::RichText::new(text).size(STATUS_BAR_FONT_SIZE)).on_hover_text(*tooltip);
                    }
                });
            });
    }

    /// Sestavi obsah cele obrazovky terminalu jako jeden `LayoutJob` -
    /// po radcich, uvnitr radku po "behach" znaku se stejnou
    /// barvou popredi/pozadi (misto jednoho segmentu na kazdy jednotlivy
    /// znak, coz by bylo zbytecne pomale).
    ///
    /// `display_offset` (viz `Grid::display_offset`, meneny kolieckem mysi
    /// v `render_grid`) rika, o kolik radku je uzivatel prave odscrollovany
    /// do historie - `0` = normalni pohled na aktivni (spodni) obrazovku.
    /// Vizualni radek `line` (0 = horni radek vykreslene mrizky) pak
    /// neodpovida primo mrizkovemu `Line(line)`, ale `Line(line -
    /// display_offset)` - zaporne hodnoty `Line` u `alacritty_terminal`
    /// sahaji do scrollbacku (viz `Grid`/`Storage`), presne to same
    /// pouziva i vlastni renderer Alacritty pro vykresleni odscrollovaneho
    /// zobrazeni.
    ///
    /// POZNAMKA K OVERENI: tohle indexovani zapornych `Line` hodnot pro
    /// pristup do scrollbacku nebylo mozne overit skutecnym `cargo build`/
    /// `cargo run` (zadny pristup na crates.io v tomto prostredi) - je to
    /// zdokumentovane chovani `Grid::index`/`Storage::index` u
    /// `alacritty_terminal`, ale kdyby presto build/beh na teto casti
    /// selhal, jde o izolovanou funkci (skrolovani historie), zbytek
    /// (normalni zobrazeni na spodku, `display_offset == 0`) na tom
    /// nezavisi.
    fn build_layout_job(&self, font_size: f32) -> egui::text::LayoutJob {
        let mut job = egui::text::LayoutJob::default();
        let font_id = terminal_font(font_size);
        let grid = self.term.grid();
        let cursor_point = grid.cursor.point;
        let display_offset = grid.display_offset() as i32;

        for line in 0..grid.screen_lines() {
            if line > 0 {
                job.append("\n", 0.0, egui::TextFormat::default());
            }

            let mut run = String::new();
            // Trojice (fg, bg, podtrzeni) - kazda odlisna kombinace
            // zacina novy "run" (davku pro jedno `job.append`), stejne
            // jako drivejsi pouha dvojice barev.
            let mut run_style: Option<(egui::Color32, egui::Color32, bool)> = None;

            for col in 0..grid.columns() {
                let point = Point::new(Line(line as i32 - display_offset), Column(col));
                let cell = &grid[point];

                let (mut fg, mut bg) = cell_colors(cell.fg, cell.bg);
                // Reverzni video (SGR "inverse"/`Flags::INVERSE`) - bezny
                // zpusob, jak terminalove aplikace (Avaya SAT formulare
                // vcetne) vizualne odlisi editovatelne pole od statickeho
                // popisku ("Dokážeme obarvit formulář v místech kde má
                // možnost psát do formuláře?") - donedavna se tento
                // priznak z `alacritty_terminal` vubec necetl, takze se
                // vzdalenou aplikaci odeslane zvyrazneni ztratilo. Prohozeni
                // fg/bg PRED pripadnym prohozenim kvuli kurzoru nize, aby
                // se oba efekty (kurzor NA jiz inverzni bunce) spravne
                // slozily/vyrusily - presne jako v beznem terminalu.
                if cell.flags.contains(Flags::INVERSE) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if point == cursor_point {
                    std::mem::swap(&mut fg, &mut bg);
                }
                // Zvyrazneni "editovatelnych" poli formulare - diagnosticky
                // log (`TERMX_DEBUG_RAW_LOG`) z realneho Avaya SAT formulare
                // ukazal, ze server tato pole neoznacuje ani reverznim
                // videem, ani barvou, jen SGR podtrzenim (`ESC[0;4m` pred
                // hodnotou, `ESC[0m` za ni) - tenke 1px podtrzeni (viz
                // `text_format` nize) je proto na tmavem pozadi terminalu
                // sotva videt ("krom bílých pruhů"). Aby byl pozadavek
                // "obarvit formulář v místech kde má možnost psát" skutecne
                // splnen, prolne se navic i pozadi bunky smerem k barve
                // tematu - jde o cistou vykreslovaci volbu na strane
                // klienta (server o ni nevi a nic se mu neodesila), takze
                // je bezpecne pridat i bez dalsiho potvrzeni protokolu.
                if cell.flags.intersects(Flags::ALL_UNDERLINES) {
                    bg = blend_toward(bg, theme::ACCENT, 0.22);
                }
                // Oznaceny text (viz `Selection`/`handle_selection_input`) -
                // pozadi bunky se prolne s barvou zvyrazneni tematu misto
                // proste prehozeni fg/bg (jako u kurzoru vyse), aby slo na
                // prvni pohled rozeznat od kurzoru samotneho.
                if self.selection.is_some_and(|sel| sel.contains(line as i32 - display_offset, col)) {
                    bg = blend_toward(bg, theme::ACCENT, 0.45);
                }
                // "Tucny" text (SGR bold) se u monospace pisma nevykresluje
                // jinym rezem (zadne tucne varianty nejsou nactene), misto
                // toho (stejne jako spousta beznych terminalu) proste
                // zesvetli popredi - dalsi zpusob, jak muze server zvyraznit
                // dulezite/editovatelne misto.
                if cell.flags.intersects(Flags::BOLD | Flags::DIM_BOLD) {
                    fg = blend_toward(fg, egui::Color32::WHITE, 0.35);
                }
                // Kazda varianta podtrzeni (obycejne/dvojite/vlnovka/
                // teckovane/carkovane) se vykresli stejne, jako proste
                // podtrzeni - staci na to, aby bylo videt, ze server dane
                // pole nejak zvyraznil, presne rozliseni stylu podtrzeni
                // neni pro ucel "kde se da psat" dulezite.
                let underline = cell.flags.intersects(Flags::ALL_UNDERLINES);
                let style = (fg, bg, underline);
                let ch = if cell.c == '\0' { ' ' } else { cell.c };

                match run_style {
                    Some(existing) if existing == style => run.push(ch),
                    _ => {
                        if let Some((rfg, rbg, runderline)) = run_style.take() {
                            job.append(&run, 0.0, text_format(&font_id, rfg, rbg, runderline));
                        }
                        run.clear();
                        run.push(ch);
                        run_style = Some(style);
                    }
                }
            }

            if let Some((rfg, rbg, runderline)) = run_style.take() {
                job.append(&run, 0.0, text_format(&font_id, rfg, rbg, runderline));
            }
        }

        job
    }
}

/// Adresa (hostname/IP), pripadne i port (kdyz neni vychozich 22),
/// fyzickeho serveru, ke kteremu je dana session pripojena - na rozdil
/// od `Session::name` (libovolny popisek zvoleny uzivatelem pri ulozeni
/// spojeni) jde o to, co se skutecne pouziva k pripojeni. Pouzito v
/// info proužku pod terminalem (`TerminalSession::render_status_bar`).

fn format_host_label(session: &Session) -> String {
    if session.port == 22 {
        session.host.clone()
    } else {
        format!("{}:{}", session.host, session.port)
    }
}

/// Text pro polozku "🔌" v info prouzku - na rozdil od holeho
/// `format_host_label` (adresa/port, jak je uzivatel ZADAL pri vytvareni
/// spojeni) pripoji i skutecny nazev vzdaleneho stroje (`SystemStats::hostname`,
/// vystup prikazu `hostname` na druhe strane), pokud uz dorazil a lisi se
/// od zadane adresy - zpetna vazba "v info panelu ukazujeme jen IP adresu
/// ačkoliv vidíme skutečný název serveru hned v prvním řádku terminálu".
/// Kdyz se hostname jeste nestihl zjistit (`None`, prvnich par sekund po
/// pripojeni - viz `SystemStats`), nebo je stejny jako zadana adresa
/// (uzivatel uz zadal rovnou hostname), zobrazi se jen puvodni adresa
/// beze zmeny.
fn format_host_display(session: &Session, stats: &SystemStats) -> String {
    let label = format_host_label(session);
    match &stats.hostname {
        Some(hostname) if !hostname.eq_ignore_ascii_case(&session.host) => format!("{hostname} ({label})"),
        _ => label,
    }
}

/// Naformatuje cislo s danym poctem desetinnych mist a ceskou desetinou
/// carkou (misto anglicke tecky) - pro info proužek pod terminalem (viz
/// `TerminalSession::render_status_bar`), stejne jako zbytek aplikace
/// pouziva ceskou lokalizaci.
fn fmt_decimal(value: f64, decimals: usize) -> String {
    format!("{:.*}", decimals, value).replace('.', ",")
}

/// Cesky sklonovany pocet dni pro dobu behu serveru (1 den, 2-4 dny, 0
/// nebo 5 a vice dní) - pro info proužek pod terminalem.
fn text_format(font_id: &egui::FontId, fg: egui::Color32, bg: egui::Color32, underline: bool) -> egui::TextFormat {
    egui::TextFormat {
        font_id: font_id.clone(),
        color: fg,
        background: bg,
        // SGR podtrzeni (`Flags::ALL_UNDERLINES` v `build_layout_job`) -
        // stejnou barvou jako text, tenky (1px) tah.
        underline: if underline { egui::Stroke::new(1.0_f32, fg) } else { egui::Stroke::NONE },
        ..Default::default()
    }
}

/// Prevede barvu jedne bunky (popredi/pozadi) z `alacritty_terminal` na
/// `egui::Color32`, s fallbackem na barvy tematu aplikace.
fn cell_colors(fg: AnsiColor, bg: AnsiColor) -> (egui::Color32, egui::Color32) {
    (ansi_color(fg, theme::TEXT), ansi_color(bg, theme::BG_DARK))
}

/// Prolne barvu `from` smerem k `to` o podil `t` (0.0 = beze zmeny, 1.0 =
/// cela `to`) - pouzito pro zvyrazneni pozadi oznaceneho textu
/// (`build_layout_job`). Pocita se rucne (misto spolehnuti na alfa
/// pruhlednost pri vykreslovani), aby vysledek zustal plne krycí bez
/// ohledu na to, co (pripadne nic) je pod terminalem vykresleno.
fn blend_toward(from: egui::Color32, to: egui::Color32, t: f32) -> egui::Color32 {
    let lerp = |a: u8, b: u8| ((a as f32) * (1.0 - t) + (b as f32) * t).round() as u8;
    egui::Color32::from_rgb(lerp(from.r(), to.r()), lerp(from.g(), to.g()), lerp(from.b(), to.b()))
}

fn ansi_color(color: AnsiColor, default: egui::Color32) -> egui::Color32 {
    match color {
        AnsiColor::Named(named) => named_color(named).unwrap_or(default),
        AnsiColor::Spec(rgb) => egui::Color32::from_rgb(rgb.r, rgb.g, rgb.b),
        AnsiColor::Indexed(idx) => indexed_color(idx),
    }
}

/// Standardni 16-barevna paleta (xterm-podobne odstiny). `None` pro
/// "Foreground"/"Background"/cokoliv neocekavaneho - pouzije se pak
/// barva tematu aplikace (viz `cell_colors`), takze i kdyby v pouzite
/// verzi `alacritty_terminal` pribyla/chybela nejaka varianta, zbytek
/// stale zkompiluje diky `_ =>` na konci.
fn named_color(named: NamedColor) -> Option<egui::Color32> {
    use egui::Color32;
    match named {
        NamedColor::Black => Some(Color32::from_rgb(0x00, 0x00, 0x00)),
        NamedColor::Red => Some(Color32::from_rgb(0xcc, 0x33, 0x33)),
        NamedColor::Green => Some(Color32::from_rgb(0x4e, 0x9a, 0x06)),
        NamedColor::Yellow => Some(Color32::from_rgb(0xc4, 0xa0, 0x00)),
        NamedColor::Blue => Some(Color32::from_rgb(0x34, 0x65, 0xa4)),
        NamedColor::Magenta => Some(Color32::from_rgb(0x75, 0x50, 0x7b)),
        NamedColor::Cyan => Some(Color32::from_rgb(0x06, 0x98, 0x9a)),
        NamedColor::White => Some(Color32::from_rgb(0xd3, 0xd7, 0xcf)),
        NamedColor::BrightBlack => Some(Color32::from_rgb(0x55, 0x57, 0x53)),
        NamedColor::BrightRed => Some(Color32::from_rgb(0xef, 0x29, 0x29)),
        NamedColor::BrightGreen => Some(Color32::from_rgb(0x8a, 0xe2, 0x34)),
        NamedColor::BrightYellow => Some(Color32::from_rgb(0xfc, 0xe9, 0x4f)),
        NamedColor::BrightBlue => Some(Color32::from_rgb(0x72, 0x9f, 0xcf)),
        NamedColor::BrightMagenta => Some(Color32::from_rgb(0xad, 0x7f, 0xa8)),
        NamedColor::BrightCyan => Some(Color32::from_rgb(0x34, 0xe2, 0xe2)),
        NamedColor::BrightWhite => Some(Color32::from_rgb(0xee, 0xee, 0xec)),
        _ => None,
    }
}

/// Standardni xterm 256-barevna paleta: 0-15 zakladni barvy, 16-231
/// 6x6x6 barevna kostka, 232-255 stupnice sedi.
fn indexed_color(idx: u8) -> egui::Color32 {
    const BASIC: [(u8, u8, u8); 16] = [
        (0x00, 0x00, 0x00),
        (0xcc, 0x33, 0x33),
        (0x4e, 0x9a, 0x06),
        (0xc4, 0xa0, 0x00),
        (0x34, 0x65, 0xa4),
        (0x75, 0x50, 0x7b),
        (0x06, 0x98, 0x9a),
        (0xd3, 0xd7, 0xcf),
        (0x55, 0x57, 0x53),
        (0xef, 0x29, 0x29),
        (0x8a, 0xe2, 0x34),
        (0xfc, 0xe9, 0x4f),
        (0x72, 0x9f, 0xcf),
        (0xad, 0x7f, 0xa8),
        (0x34, 0xe2, 0xe2),
        (0xee, 0xee, 0xec),
    ];

    if (idx as usize) < 16 {
        let (r, g, b) = BASIC[idx as usize];
        return egui::Color32::from_rgb(r, g, b);
    }
    if idx >= 232 {
        let level = (8u16 + (idx - 232) as u16 * 10).min(255) as u8;
        return egui::Color32::from_rgb(level, level, level);
    }
    let cube = idx - 16;
    let r = cube / 36;
    let g = (cube / 6) % 6;
    let b = cube % 6;
    let conv = |c: u8| if c == 0 { 0 } else { 55 + c * 40 };
    egui::Color32::from_rgb(conv(r), conv(g), conv(b))
}

/// Prevede stisknutou klavesu (mimo bezny text, viz `Event::Text` v
/// `handle_keyboard`) na bajty/ANSI escape sekvenci, kterou ocekava
/// vzdaleny shell - standardni VT100/xterm konvence (sipky, Ctrl+pismeno
/// jako ridici znak, ...).
fn key_to_bytes(key: egui::Key, modifiers: egui::Modifiers, term_type: Option<&str>, app_cursor_keys: bool) -> Option<Vec<u8>> {
    use egui::Key;

    // Ctrl+Tab je VYHRAZENA globalni zkratka pro prepnuti fokusu mezi
    // panely rozdeleneho zobrazeni (viz `MainApp::update`/pozadavek
    // "mohli bychom dát do Zobrazení možnost zobrazit dva TABy vedle
    // sebe... přepínalo by se mezi nimi tabulátorem") - bez tohoto
    // vyjimeckeho pripadu by nize `Key::Tab => b"\t"` (kombinaci Ctrl
    // nijak nerozlisuje) poslal i obycejny Tab do prave fokusovaneho
    // terminalu soucasne s prepnutim panelu, coz by vypadalo jako
    // nechtene doplneni v shellu.
    if modifiers.ctrl && !modifiers.alt && key == Key::Tab {
        return None;
    }

    // Ctrl+V (Cmd+V na Macu) uz zpracovava `handle_keyboard` samostatne
    // pres vysokourovnovou `egui::Event::Paste` (skutecny obsah schranky
    // odeslany jako normalni text) - kdyby se tu NEVYLOUCILO, poslal by
    // se NAVIC jeste syrovy ridici bajt 0x16 (viz `ctrl_control_code`)
    // pred/za vlozeny text, coz by vypadalo jako nechteny extra znak.
    if modifiers.ctrl && !modifiers.alt && key == Key::V {
        return None;
    }

    if modifiers.ctrl && !modifiers.alt {
        if let Some(code) = ctrl_control_code(key) {
            return Some(vec![code]);
        }
    }

    // Funkcni klavesy (F1-F8) SAT rozhrani Avaya Communication Manageru
    // - viz `avaya_function_key_bytes` - beznemu terminalu (`term_type`
    // nenastaveny na "w2ktt"/"ntt") se vubec nedotknou, F-klavesy pro
    // nej zustavaji beze zmeny (nic se neposila, stejne jako predtim).
    if let Some(bytes) = avaya_function_key_bytes(key, term_type) {
        return Some(bytes);
    }

    // PageUp/PageDown/Delete: Avaya CM SAT (typy "w2ktt"/"ntt") tyto
    // klavesy chape jinak, nez bezny xterm predpoklada - standardni
    // "CSI n ~" kody (`ESC[5~`, `ESC[6~`, `ESC[3~`) tu NEJSOU
    // rozpoznany jako prikaz, misto toho server jen zobrazi osamocenou
    // "~" jako obycejny znak (zpetna vazba "del píše tyldu, pg up/dn
    // taky píš tyldu"). PageUp/PageDown se proto presmeruji na uz
    // OVERENE funkcni "Prev Page"/"Next Page" kody (stejne jako
    // F8/F7 - viz `avaya_function_key_bytes`); Delete se pro tyto typy
    // zatim proste NEPOSILA vubec - zadny potvrzeny kod k dispozici,
    // lepsi nic neposlat nez zase poslat spatny bajt do rozpracovaneho
    // formulare.
    if matches!(term_type, Some("w2ktt") | Some("ntt")) {
        match key {
            Key::PageDown => return avaya_function_key_bytes(Key::F7, term_type),
            Key::PageUp => return avaya_function_key_bytes(Key::F8, term_type),
            Key::Delete => return None,
            _ => {}
        }
    }

    // Nektera zarizeni s Avaya-specifickym typem terminalu (viz
    // `avaya_function_key_bytes`) ocekavaji na Backspace klasicky ASCII
    // BS (0x08), ne DEL (0x7f), ktery pouziva bezny xterm - "w2ktt"
    // ("Windows 2000 Telnet Terminal") vychazi z puvodniho Windows
    // Telnet klienta, ktery historicky posilal prave 0x08.
    // NEOVERENO NA ZIVO (na rozdil od F-klaves vyse) - je to nejlepsi
    // dostupny odhad, ne potvrzeny fakt.
    let backspace: &[u8] = match term_type {
        Some("w2ktt") | Some("ntt") => b"\x08",
        _ => b"\x7f",
    };

    // Sipky - viz `app_cursor_keys` v `handle_keyboard` (DECCKM: server
    // muze pozadat o "aplikacni" rezim, kdy se misto `ESC [ <pismeno>`
    // ocekava `ESC O <pismeno>`).
    //
    // POZOR: byl tu vyzkousen i experiment s "holym" VT52-stylem
    // (`ESC A/B/C/D` pro "w2ktt", vzdy SS3 pro "ntt") podle vzoru
    // F-klaves teto emulace - David ho ale naziv otestoval a sipky
    // pak nefungovaly VUBEC (predtim aspon "brouzdaly" mezi poli
    // formulare). Vraceno proto zpet na puvodni/bezpecne DECCKM-based
    // chovani - "brouzdani" mezi poli je sice nepohodlne, ale funkcni
    // a nic nerozbije v rozpracovanem formulari.
    let (arrow_up, arrow_down, arrow_right, arrow_left): (&[u8], &[u8], &[u8], &[u8]) = if app_cursor_keys {
        (b"\x1bOA", b"\x1bOB", b"\x1bOC", b"\x1bOD")
    } else {
        (b"\x1b[A", b"\x1b[B", b"\x1b[C", b"\x1b[D")
    };

    let bytes: &[u8] = match key {
        Key::Enter => b"\r",
        Key::Backspace => backspace,
        Key::Tab => b"\t",
        Key::Escape => b"\x1b",
        Key::ArrowUp => arrow_up,
        Key::ArrowDown => arrow_down,
        Key::ArrowRight => arrow_right,
        Key::ArrowLeft => arrow_left,
        Key::Home => b"\x1b[H",
        Key::End => b"\x1b[F",
        Key::Delete => b"\x1b[3~",
        Key::PageUp => b"\x1b[5~",
        Key::PageDown => b"\x1b[6~",
        // Bezna pismena/cislice/symboly bez Ctrl uz prichazeji jako
        // `egui::Event::Text` - tady by jejich znovu-odeslani znamenalo
        // kazdy znak poslat dvakrat.
        _ => return None,
    };
    Some(bytes.to_vec())
}

/// Ctrl+pismeno -> ridici znak (Ctrl+A = 0x01, Ctrl+C = 0x03, Ctrl+D =
/// 0x04, ...), jak ocekava kazdy bezny shell/terminal.
/// Funkcni klavesy (F1-F8) pro SAT rozhrani Avaya Communication
/// Manageru - jake bajty se maji poslat zavisi na zvolenem typu
/// terminalu (`Session::term_type`/`pty-req`), protoze kazdy typ
/// pouziva jine kody.
///
/// David (uzivatel, testovano na realne Avaya CM SAT): "F3 je Enter,
/// F5 je nápověda (doplnění příkazu), F1 je cancel" - presne odpovida
/// oficialni Avaya dokumentaci ("Terminal Emulation Function Keys for
/// Communication Manager") pro emulace "ntt" a "w2ktt" (F2/F4/F6 jsou
/// v obou tabulkach nevyuzite/prazdne).
///
/// Pro "513" (AT&T Terminal 513, puvodne hardwarovy terminal) se
/// NEPODARILO dohledat oficialni/spolehlivou tabulku escape sekvenci -
/// proto se tu NEHADA (poslani spatne sekvence do rozpracovaneho
/// formulare SAT - napr. "change station" - by mohlo omylem provest
/// nechtenou zmenu konfigurace PBX). Pro funkcni klavesy je tak potreba
/// mit u serveru nastaveny "Typ terminálu" na "w2ktt" (doporuceno -
/// nejjednodussi kody) nebo "ntt"; se "513" (nebo bez nastaveneho typu)
/// tato funkce nevraci nic a F-klavesy se (stejne jako drive) nikam
/// neposilaji.
fn avaya_function_key_bytes(key: egui::Key, term_type: Option<&str>) -> Option<Vec<u8>> {
    use egui::Key;

    match term_type? {
        // Nejjednodussi kody (jeden ESC + jeden znak) - viz Avaya
        // "Terminal Emulation Function Keys for Communication Manager",
        // tabulka pro "w2ktt".
        "w2ktt" => match key {
            Key::F1 => Some(b"\x1bx".to_vec()), // Cancel
            Key::F3 => Some(b"\x1be".to_vec()), // Execute/Enter
            Key::F5 => Some(b"\x1bh".to_vec()), // Help
            Key::F7 => Some(b"\x1bn".to_vec()), // Next Page
            Key::F8 => Some(b"\x1bp".to_vec()), // Previous Page
            _ => None,
        },
        // SS3-stylove kody (`ESC O <pismeno>`, stejny vzor jako
        // vt220 PF1-PF4 rozsireny az na F8) - viz tabulka pro "ntt".
        "ntt" => match key {
            Key::F1 => Some(b"\x1bOP".to_vec()),  // Cancel
            Key::F3 => Some(b"\x1bOR".to_vec()),  // Execute/Enter
            Key::F5 => Some(b"\x1bOT".to_vec()),  // Help
            Key::F6 => Some(b"\x1bOU".to_vec()),  // Go to Page "N"
            Key::F7 => Some(b"\x1bOV".to_vec()),  // Next Page
            Key::F8 => Some(b"\x1bOW".to_vec()),  // Previous Page
            _ => None,
        },
        _ => None,
    }
}

fn ctrl_control_code(key: egui::Key) -> Option<u8> {
    use egui::Key;
    let letter: u8 = match key {
        Key::A => b'a',
        Key::B => b'b',
        Key::C => b'c',
        Key::D => b'd',
        Key::E => b'e',
        Key::F => b'f',
        Key::G => b'g',
        Key::H => b'h',
        Key::I => b'i',
        Key::J => b'j',
        Key::K => b'k',
        Key::L => b'l',
        Key::M => b'm',
        Key::N => b'n',
        Key::O => b'o',
        Key::P => b'p',
        Key::Q => b'q',
        Key::R => b'r',
        Key::S => b's',
        Key::T => b't',
        Key::U => b'u',
        Key::V => b'v',
        Key::W => b'w',
        Key::X => b'x',
        Key::Y => b'y',
        Key::Z => b'z',
        _ => return None,
    };
    Some(letter & 0x1f)
}
