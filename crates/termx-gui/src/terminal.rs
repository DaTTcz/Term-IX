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
//! Rucni/automaticke znovupripojeni (`reconnect`/`maybe_auto_reconnect`,
//! viz nize) zadne nove nejiste API nepridava - jen znovu vola uz
//! overene `spawn_ssh_session`/`Term::new` se stejnymi parametry jako
//! `TerminalSession::new`.
//! Architektura kolem (SSH vlakno v `termx-ssh`, kanaly, GUI tab) na
//! techto detailech nezavisi - jde o lokalizovanou opravu jednoho
//! souboru.
//! Odlozene zadani prihlasovacich udaju (`ConnState::AwaitingCredentials`,
//! `render_credentials_prompt`) take nepridava zadne nove nejiste API na
//! teto strane - SSH spojeni (`spawn_ssh_session`) se pořád zaklada
//! primo v `new`, stejne jako drive; jedina zmena je NOVA VARIANTA
//! udalosti `SshEvent::AwaitingCredentials` (posilana z `termx-ssh`, viz
//! tamni POZNAMKA K OVERENI) a odpovidajici nova varianta prikazu
//! `SshInput::Credentials` (`submit_credentials` ji jen posle po jiz
//! existujicim `handle.input_tx` - zadne nove spojeni/vlakno).
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

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Processor};

use termx_core::{AuthMethod, Session};
use termx_ssh::{spawn_ssh_session, SshEvent, SshHandle, SshInput, SystemStats};

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

const FONT_SIZE: f32 = 14.0;

/// Velikost pisma info proužku pod terminalem (`render_status_bar`) -
/// zamerne vetsi nez vychozi "male" (`RichText::small()`, ~9-10px),
/// podle uzivatelovy zpetne vazby.
const STATUS_BAR_FONT_SIZE: f32 = 14.0;

/// Nejmensi rozestup mezi dvema po sobe jdoucimi POKUSY o automaticke
/// obnoveni spojeni (viz `TerminalSession::maybe_auto_reconnect`) - aby
/// se pri dlouhodobe nedostupnem serveru nezkousel novy pokus uplne
/// kazdy snimek (desitky za sekundu), ale v rozumnych intervalech.
const AUTO_RECONNECT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

fn terminal_font() -> egui::FontId {
    egui::FontId::monospace(FONT_SIZE)
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
    /// vyplneneho uzivatele, takze se ceka, az ho uzivatel primo v
    /// tomto tabu doplni (viz `TerminalSession::render_credentials_prompt`) -
    /// stejne jako u obycejneho `ssh` klienta, ktery se taky nejdriv
    /// pripoji a teprve pak se zepta na heslo. Typicky nastane po Home
    /// formulari odeslanem jen s hostem/portem, viz zpetna vazba "já
    /// bych raději viděl už přímo komunikaci se serverem a tam až
    /// dával uživatele a heslo".
    AwaitingCredentials,
    /// Prvni navazovani spojeni, nebo prubeh automatickeho/rucniho
    /// pokusu o jeho obnoveni (viz `reconnect`).
    Connecting,
    Connected,
    /// Spojeni skoncilo - at uz chybou (`error` je pak `Some`), nebo
    /// cistě (napr. `exit`/`logout` na druhe strane).
    Disconnected,
}

/// Jedno bezici SSH spojeni napojene na vestaveny terminal - jeden
/// otevreny Connection tab = jedna instance (viz `MainApp::terminal_sessions`
/// v `app.rs`).
pub struct TerminalSession {
    term: Term<EventProxy>,
    parser: Processor,
    /// SSH spojeni se zaklada VZDY hned v `new` (i kdyz `session.auth`
    /// jeste nema uzivatele) - transport se navaze nezavisle na tom,
    /// jestli uz jsou prihlasovaci udaje k dispozici (viz
    /// `ConnState::AwaitingCredentials`/`submit_credentials`), takze
    /// `handle` uz neni potreba drzet jako `Option`.
    handle: SshHandle,
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
    /// [`ConnState::AwaitingCredentials`]/`render_credentials_prompt`.
    /// Po odeslani formulare (`submit_credentials`) uz zustava `false`
    /// natrvalo (i kdyz spojeni pozdeji spadne a `reconnect` ho obnovi -
    /// tehdy uz `session.auth` ma uzivatele/heslo vyplnene, takze se
    /// znovu ptat netreba).
    awaiting_credentials: bool,
    /// Rozepsany uzivatel/heslo v `render_credentials_prompt`, nez se
    /// odesle (viz `submit_credentials`).
    pending_username: String,
    pending_password: String,
    /// Stejny ucel jako `LockScreen::focus_requested` v `app.rs` - pole
    /// uzivatele v `render_credentials_prompt` se ma fokusnout jen
    /// jednou (prvni snimek), ne kazdy snimek znovu (to by bojovalo s
    /// uzivatelovym vlastnim kliknutim treba do pole hesla).
    credentials_focus_requested: bool,
}

impl TerminalSession {
    /// Zalozi nove SSH spojeni (na pozadi, viz `termx_ssh::spawn_ssh_session`)
    /// a pripravi prazdnou terminalovou obrazovku, do ktere se bude
    /// postupne (`pump`) vykreslovat.
    pub fn new(session: &Session) -> Self {
        let size = TermSize { cols: DEFAULT_COLS, rows: DEFAULT_ROWS };
        let term = Term::new(TermConfig::default(), &size, EventProxy);
        // SSH spojeni se zaklada VZDY hned - i kdyz `session.auth` jeste
        // nema vyplneneho uzivatele. Transport (TCP + vymena klicu) se
        // navaze nezavisle na tom; kdyz `termx_ssh::run_session` zjisti
        // chybejiciho uzivatele, sam posle `SshEvent::AwaitingCredentials`
        // (viz `pump`) a pocka na dodatecne udaje (`submit_credentials`).
        let handle = spawn_ssh_session(session.clone(), DEFAULT_COLS as u16, DEFAULT_ROWS as u16);

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
            pending_username: String::new(),
            pending_password: String::new(),
            credentials_focus_requested: false,
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

    /// Zpracuje odeslani prihlasovacich udaju z `render_credentials_prompt` -
    /// SSH spojeni uz beselo (transport je navazan, viz `pump`/
    /// `SshEvent::AwaitingCredentials`), takze se jen posle
    /// `SshInput::Credentials` po jiz existujicim kanalu; zadne nove
    /// spojeni se nezaklada. Udaje se navic ulozi primo do
    /// `self.session` (diky tomu pripadny pozdejsi `reconnect` uz zadne
    /// dalsi doplneni nepotrebuje - pouzije stejnou, uz jednou zadanou,
    /// kombinaci uzivatel/heslo).
    fn submit_credentials(&mut self) {
        let username = self.pending_username.trim().to_string();
        if username.is_empty() {
            return;
        }
        let password = std::mem::take(&mut self.pending_password);
        self.session.auth = AuthMethod::Password { username: username.clone(), password: password.clone() };
        self.awaiting_credentials = false;
        let _ = self.handle.input_tx.send(SshInput::Credentials { username, password });
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
        self.handle = spawn_ssh_session(self.session.clone(), self.cols as u16, self.rows as u16);
        self.connected = false;
        self.ever_connected = false;
        self.error = None;
        self.stats = None;
        self.last_reconnect_attempt = Some(std::time::Instant::now());
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
        let _ = self.handle.input_tx.send(SshInput::Resize { cols: cols as u32, rows: rows as u32 });
    }

    /// Spocita, kolik sloupcu/radku monospace pisma se vejde do dane
    /// dostupne plochy, a podle toho (pripadne) zmeni velikost terminalu
    /// - viz `resize`. Volano kazdy snimek z `render`, tesne pred tim,
    /// nez se vlastni obsah terminalu vykresli, aby prepocet pouzival
    /// aktualni dostupnou plochu tohoto snimku (uz po pripadnych
    /// hlaskach o stavu spojeni nad terminalem, ktere taky zabiraji
    /// misto).
    fn resize_to_fit(&mut self, ui: &egui::Ui) {
        let available = ui.available_size();
        let font_id = terminal_font();
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
            match self.handle.output_rx.try_recv() {
                Ok(SshEvent::Data(bytes)) => {
                    for byte in bytes {
                        self.parser.advance(&mut self.term, byte);
                    }
                }
                Ok(SshEvent::Connected) => {
                    self.connected = true;
                    self.ever_connected = true;
                    self.error = None;
                    self.awaiting_credentials = false;
                }
                Ok(SshEvent::AwaitingCredentials) => {
                    self.awaiting_credentials = true;
                }
                Ok(SshEvent::Error(e)) => {
                    self.error = Some(e);
                    self.connected = false;
                    self.awaiting_credentials = false;
                }
                Ok(SshEvent::Closed) => {
                    self.connected = false;
                }
                Ok(SshEvent::Stats(stats)) => {
                    self.stats = Some(stats);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }
    }

    fn send_bytes(&self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        // Prijemce (SSH vlakno) uz nemusi bezet (napr. spojeni mezitim
        // skoncilo chybou) - poslani se pak proste nezdari, nic se
        // nedeje.
        let _ = self.handle.input_tx.send(SshInput::Data(bytes));
    }

    /// Zachyti klavesovy vstup z aktualniho snimku a preposle jej (jako
    /// syrove bajty/ANSI escape sekvence) do SSH spojeni.
    fn handle_keyboard(&self, ui: &egui::Ui) {
        let events = ui.input(|i| i.events.clone());
        for event in events {
            match event {
                // Bezny text (pismena, cislice, mezera, diakritika, ...) -
                // egui uz sam rozlisuje "napsatelny" text od ridicich
                // klaves, takze staci poslat rovnou.
                egui::Event::Text(text) => self.send_bytes(text.into_bytes()),
                egui::Event::Paste(text) => self.send_bytes(text.into_bytes()),
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    if let Some(bytes) = key_to_bytes(key, modifiers) {
                        self.send_bytes(bytes);
                    }
                }
                _ => {}
            }
        }
    }

    /// Vykresli aktualni stav terminalu do daneho `Ui` (cely obsah
    /// Connection tabu) a zpracuje klavesovy vstup z tohoto snimku.
    /// `auto_reconnect` je aktualni hodnota nastaveni "automaticky se
    /// pokoušet obnovit ztracené spojení" (viz `MainApp::settings` v
    /// `app.rs`) - samotny `TerminalSession` si zadne globalni
    /// nastaveni nedrzi, dostava ho pri kazdem vykresleni zvenci.
    pub fn render(&mut self, ui: &mut egui::Ui, auto_reconnect: bool) {
        self.pump();

        if self.state() == ConnState::AwaitingCredentials {
            // Transport uz je navazan (viz `pump`/`SshEvent::AwaitingCredentials`),
            // jen chybi uzivatel/heslo - misto terminalu (a bez
            // `handle_keyboard`/`resize_to_fit`/status proužku, ktere
            // tu jeste nemaji co delat, dokud neni relace autentizovana)
            // se zobrazi prihlasovaci formular. Po jeho odeslani
            // (`submit_credentials`) uz dalsi snimek pokracuje normalne -
            // `state()` bude `Connecting`, nez dorazi `SshEvent::Connected`.
            self.render_credentials_prompt(ui);
            return;
        }

        self.handle_keyboard(ui);

        // Dokud je tab otevreny/aktivni, chceme obrazovku prubezne
        // obcerstvovat i bez interakce uzivatele (aby se novy vystup ze
        // serveru objevil hned, ne az pri dalsim kliknuti/klavese, a aby
        // fungoval i casovac automatickeho obnoveni spojeni nize).
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));

        match self.state() {
            // Nedosazitelne v praxi - `AwaitingCredentials` konci
            // brzkym `return` uplne nahore v teto metode, takze se sem
            // tok nikdy nedostane. Prazdna vetev misto `unreachable!()`
            // zamerne - kdyby se tento predpoklad nekdy prestal drzet,
            // je lepsi tab jen chvíli nic nevykreslit, nez shodit celou
            // aplikaci panikou.
            ConnState::AwaitingCredentials => {}
            ConnState::Disconnected => {
                // Kdyz je automaticke obnoveni zapnute, zkusi se samo -
                // tlacitko "Připojit znovu" nize zustava funkcni i tak
                // (okamzity rucni pokus, bez cekani na dalsi casovy
                // interval).
                self.maybe_auto_reconnect(auto_reconnect);

                ui.horizontal(|ui| {
                    let message = match &self.error {
                        Some(err) => format!("Spojení skončilo chybou: {err}"),
                        None => "Spojení bylo ukončeno.".to_string(),
                    };
                    ui.colored_label(theme::DANGER, message);
                    if ui.button("🔄 Připojit znovu").clicked() {
                        self.reconnect();
                    }
                    if auto_reconnect {
                        ui.label(egui::RichText::new("(automaticky se zkouší obnovit)").small().weak());
                    }
                });
                ui.add_space(6.0);
            }
            ConnState::Connecting => {
                ui.label(egui::RichText::new("Připojuji…").small());
                ui.add_space(6.0);
            }
            ConnState::Connected => {}
        }

        // Info proužek se systemovymi metrikami se pripne dolu JESTE
        // PRED `resize_to_fit`, aby si vzal svuj kousek plochy jako
        // prvni a prepocet velikosti terminalu uz pocital jen s tim, co
        // zbyde nad nim (presne jak to vypada na predloze z MobaXtermu -
        // proužek pod oknem terminalu, ne pres nej).
        self.render_status_bar(ui);

        // Az TED (po pripadnych hlaskach vyse, ktere uz zabraly kus
        // plochy tohoto snimku) - viz `resize_to_fit`.
        self.resize_to_fit(ui);

        let job = self.build_layout_job();
        egui::ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            ui.add(egui::Label::new(job).selectable(false));
        });
    }

    /// Inline prihlasovaci formular zobrazeny MISTO terminalu, dokud je
    /// stav `ConnState::AwaitingCredentials` (viz `render`) - SSH
    /// transport uz je v tuto chvili navazan (viz `pump`/
    /// `SshEvent::AwaitingCredentials`), jen chybi uzivatel/heslo,
    /// protoze session (typicky z Home formulare, viz
    /// `app.rs::submit_home_connect`) mela vyplneneho jen hosta/port
    /// ("chci jen dočasné spojení, nebo si nejsem jistý heslem" - viz
    /// zpetna vazba "já bych raději viděl už přímo komunikaci se
    /// serverem a tam až dával uživatele a heslo"). Po odeslani
    /// formulare (`submit_credentials`) se udaje jen posilaji po JIZ
    /// existujicim spojeni - lze tak klidne zkouset i vice pokusu primo
    /// tady, bez zakladani noveho spojeni od znova.
    fn render_credentials_prompt(&mut self, ui: &mut egui::Ui) {
        let mut submit = false;

        ui.vertical_centered(|ui| {
            ui.add_space(32.0);
            ui.label(format!("Spojeno s {} - zadejte přihlašovací údaje:", format_host_label(&self.session)));
            ui.add_space(10.0);

            egui::Grid::new(("term_credentials_grid", self.session.id)).num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("Uživatel:");
                let resp = ui.text_edit_singleline(&mut self.pending_username);
                // Stejny vzor jako `LockScreen::focus_requested` v
                // `app.rs` - fokus se nabidne jen pri prvnim vykresleni
                // tohoto formulare, ne kazdy snimek znovu (jinak by to
                // bojovalo s uzivatelovym vlastnim kliknutim treba do
                // pole hesla).
                if !self.credentials_focus_requested {
                    resp.request_focus();
                }
                ui.end_row();

                ui.label("Heslo:");
                ui.add(egui::TextEdit::singleline(&mut self.pending_password).password(true));
                ui.end_row();
            });

            ui.add_space(10.0);
            if ui.add_enabled(!self.pending_username.trim().is_empty(), egui::Button::new("Připojit")).clicked() {
                submit = true;
            }
        });

        self.credentials_focus_requested = true;

        let enter_pressed = ui.ctx().input(|i| i.key_pressed(egui::Key::Enter));
        if (submit || enter_pressed) && !self.pending_username.trim().is_empty() {
            self.submit_credentials();
        }
    }

    /// Vykresli MobaXterm-podobny info proužek se systemovymi metrikami
    /// (`self.stats`) pripnuty ke spodnimu okraji tabu
    /// (`egui::TopBottomPanel::bottom(...).show_inside`, ne `.show` - ten
    /// by se vztahoval na cele okno aplikace, ne jen na tento tab).
    /// Dokud jeste nedorazilo prvni periodicke obcerstveni statistik
    /// (`self.stats == None`, viz `pump`), proužek se vubec nezobrazuje -
    /// jednodussi a citelnejsi nez zobrazovat radek plny "-" hodnot.
    fn render_status_bar(&self, ui: &mut egui::Ui) {
        let Some(stats) = &self.stats else { return };

        // (zobrazeny text, popisek do bubliny při najetí myší) - viz
        // pozadavek "najetím myši nad info proužek bychom mohli v
        // bublině říct co dané znamená". Popisky jsou zamerne staticke
        // texty (`&'static str`), zadna dalsi lokalizace/formatovani u
        // nich neni potreba.
        let mut items: Vec<(String, &'static str)> = vec![(
            format!("🔌 {}", format_host_label(&self.session)),
            "Adresa (hostname/IP) fyzického serveru, ke kterému je toto spojení připojeno.",
        )];

        if let Some(cpu) = stats.cpu_percent {
            items.push((
                format!("⚙ {}%", fmt_decimal(cpu as f64, 0)),
                "Vytížení CPU serveru (odhad z 1minutového průměru zátěže vydělený počtem jader).",
            ));
        }
        if let (Some(used), Some(total)) = (stats.mem_used_gb, stats.mem_total_gb) {
            items.push((
                format!("📊 {} / {} GB", fmt_decimal(used, 2), fmt_decimal(total, 2)),
                "Využitá a celková operační paměť (RAM) serveru.",
            ));
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
                "Aktuální rychlost odesílání dat ze serveru (upload).",
            ));
        }
        if let Some(down) = stats.net_down_mbps {
            items.push((
                format!("🔽 {} Mb/s", fmt_decimal(down, 2)),
                "Aktuální rychlost přijímání dat na serveru (download).",
            ));
        }
        if let Some(days) = stats.uptime_days {
            items.push((format!("🖥 {}", czech_days(days)), "Jak dlouho server běží od posledního restartu."));
        }
        items.push((
            if stats.user_sessions > 1 {
                format!("👤 {} (x{})", stats.username, stats.user_sessions)
            } else {
                format!("👤 {}", stats.username)
            },
            "Přihlášený uživatel a počet jeho aktivních přihlášených relací na serveru.",
        ));
        if let Some(disk) = stats.disk_percent {
            items.push((format!("💾 /: {disk}%"), "Zaplnění kořenového disku (/) na serveru."));
        }

        egui::TopBottomPanel::bottom(egui::Id::new(("term_status_bar", self.session.id)))
            .frame(
                egui::Frame::none()
                    .fill(theme::BG_PANEL)
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
    fn build_layout_job(&self) -> egui::text::LayoutJob {
        let mut job = egui::text::LayoutJob::default();
        let font_id = terminal_font();
        let grid = self.term.grid();
        let cursor_point = grid.cursor.point;

        for line in 0..grid.screen_lines() {
            if line > 0 {
                job.append("\n", 0.0, egui::TextFormat::default());
            }

            let mut run = String::new();
            let mut run_colors: Option<(egui::Color32, egui::Color32)> = None;

            for col in 0..grid.columns() {
                let point = Point::new(Line(line as i32), Column(col));
                let cell = &grid[point];

                let (mut fg, mut bg) = cell_colors(cell.fg, cell.bg);
                if point == cursor_point {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let colors = (fg, bg);
                let ch = if cell.c == '\0' { ' ' } else { cell.c };

                match run_colors {
                    Some(existing) if existing == colors => run.push(ch),
                    _ => {
                        if let Some((rfg, rbg)) = run_colors.take() {
                            job.append(&run, 0.0, text_format(&font_id, rfg, rbg));
                        }
                        run.clear();
                        run.push(ch);
                        run_colors = Some(colors);
                    }
                }
            }

            if let Some((rfg, rbg)) = run_colors.take() {
                job.append(&run, 0.0, text_format(&font_id, rfg, rbg));
            }
        }

        job
    }
}

/// Adresa (hostname/IP), pripadne i port (kdyz neni vychozich 22),
/// fyzickeho serveru, ke kteremu je dana session pripojena - na rozdil
/// od `Session::name` (libovolny popisek zvoleny uzivatelem pri ulozeni
/// spojeni) jde o to, co se skutecne pouziva k pripojeni. Pouzito v
/// info proužku pod terminalem (`TerminalSession::render_status_bar`)
/// i v prihlasovacim formulari (`render_credentials_prompt`).

fn format_host_label(session: &Session) -> String {
    if session.port == 22 {
        session.host.clone()
    } else {
        format!("{}:{}", session.host, session.port)
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
fn czech_days(days: u64) -> String {
    let word = match days {
        1 => "den",
        2..=4 => "dny",
        _ => "dní",
    };
    format!("{days} {word}")
}

fn text_format(font_id: &egui::FontId, fg: egui::Color32, bg: egui::Color32) -> egui::TextFormat {
    egui::TextFormat {
        font_id: font_id.clone(),
        color: fg,
        background: bg,
        ..Default::default()
    }
}

/// Prevede barvu jedne bunky (popredi/pozadi) z `alacritty_terminal` na
/// `egui::Color32`, s fallbackem na barvy tematu aplikace.
fn cell_colors(fg: AnsiColor, bg: AnsiColor) -> (egui::Color32, egui::Color32) {
    (ansi_color(fg, theme::TEXT), ansi_color(bg, theme::BG_DARK))
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
fn key_to_bytes(key: egui::Key, modifiers: egui::Modifiers) -> Option<Vec<u8>> {
    use egui::Key;

    if modifiers.ctrl && !modifiers.alt {
        if let Some(code) = ctrl_control_code(key) {
            return Some(vec![code]);
        }
    }

    let bytes: &[u8] = match key {
        Key::Enter => b"\r",
        Key::Backspace => b"\x7f",
        Key::Tab => b"\t",
        Key::Escape => b"\x1b",
        Key::ArrowUp => b"\x1b[A",
        Key::ArrowDown => b"\x1b[B",
        Key::ArrowRight => b"\x1b[C",
        Key::ArrowLeft => b"\x1b[D",
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
