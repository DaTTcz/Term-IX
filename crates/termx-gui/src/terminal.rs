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
//! Architektura kolem (SSH vlakno v `termx-ssh`, kanaly, GUI tab) na
//! techto detailech nezavisi - jde o lokalizovanou opravu jednoho
//! souboru.
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

use termx_core::Session;
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

/// Jedno bezici SSH spojeni napojene na vestaveny terminal - jeden
/// otevreny Connection tab = jedna instance (viz `MainApp::terminal_sessions`
/// v `app.rs`).
pub struct TerminalSession {
    term: Term<EventProxy>,
    parser: Processor,
    handle: SshHandle,
    connected: bool,
    error: Option<String>,
    /// Aktualni velikost mrizky ve znacich - drzena zvlast (mimo
    /// `self.term`), aby `resize_to_fit` mohla levne kazdy snimek
    /// zjistit, jestli se vubec neco zmenilo, bez nutnosti se pokazde
    /// ptat `self.term` (a hlavne bez zbytecneho odesilani
    /// `SshInput::Resize` na server, kdyz se velikost od minuleho
    /// snimku nezmenila).
    cols: usize,
    rows: usize,
    /// Jmeno ulozeneho spojeni (viz `Session::name`) - zobrazuje se v
    /// info proužku pod terminalem (`render_status_bar`), aby bylo na
    /// prvni pohled jasne, ke kteremu serveru statistiky patri (uzitecne
    /// zejmena kdyz ma uzivatel otevrenych vic Connection tabu najednou).
    session_name: String,
    /// Posledni prijate systemove metriky (viz `termx_ssh::SystemStats`) -
    /// `None`, dokud po pripojeni jeste nedorazilo prvni periodicke
    /// obcerstveni (viz `SshEvent::Stats` v `pump`); do te doby se info
    /// proužek proste nezobrazuje (viz `render_status_bar`).
    stats: Option<SystemStats>,
}

impl TerminalSession {
    /// Zalozi nove SSH spojeni (na pozadi, viz `termx_ssh::spawn_ssh_session`)
    /// a pripravi prazdnou terminalovou obrazovku, do ktere se bude
    /// postupne (`pump`) vykreslovat.
    pub fn new(session: &Session) -> Self {
        let size = TermSize { cols: DEFAULT_COLS, rows: DEFAULT_ROWS };
        let term = Term::new(TermConfig::default(), &size, EventProxy);
        let handle = spawn_ssh_session(session.clone(), DEFAULT_COLS as u16, DEFAULT_ROWS as u16);
        Self {
            term,
            parser: Processor::new(),
            handle,
            connected: false,
            error: None,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            session_name: session.name.clone(),
            stats: None,
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
                    self.error = None;
                }
                Ok(SshEvent::Error(e)) => {
                    self.error = Some(e);
                    self.connected = false;
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
    pub fn render(&mut self, ui: &mut egui::Ui) {
        self.pump();
        self.handle_keyboard(ui);

        // Dokud je tab otevreny/aktivni, chceme obrazovku prubezne
        // obcerstvovat i bez interakce uzivatele (aby se novy vystup ze
        // serveru objevil hned, ne az pri dalsim kliknuti/klavese).
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));

        if let Some(err) = &self.error {
            ui.colored_label(egui::Color32::from_rgb(0xe0, 0x6c, 0x6c), format!("Spojení skončilo chybou: {err}"));
            ui.add_space(6.0);
        } else if !self.connected {
            ui.label(egui::RichText::new("Připojuji…").small());
            ui.add_space(6.0);
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
            format!("🔌 {}", self.session_name),
            "Název tohoto uloženého spojení.",
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
                format!("▲ {} Mb/s", fmt_decimal(up, 2)),
                "Aktuální rychlost odesílání dat ze serveru (upload).",
            ));
        }
        if let Some(down) = stats.net_down_mbps {
            items.push((
                format!("▼ {} Mb/s", fmt_decimal(down, 2)),
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

        egui::TopBottomPanel::bottom(egui::Id::new(("term_status_bar", &self.session_name)))
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
