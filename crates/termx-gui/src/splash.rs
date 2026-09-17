//! Uvodni "splash" obrazovka (logo + verze + autor), vykreslena PRIMO
//! uvnitr hlavniho okna aplikace - misto puvodniho samostatneho okna
//! (byvaly crate `termx-splash`, postaveny na `minifb`/`fontdue`/`x11-dl`,
//! ktery uz neni soucasti workspace, viz koren. `Cargo.toml`).
//!
//! DUVOD PRESUNU: samostatne okno (jina X11 relace/toolkit nez pozdejsi
//! `eframe` okno) zpusobovalo, ze hlavni okno po zavreni splashe casto
//! nedostalo OS-level klavesovy fokus (WM ho nemusel povazovat za "to
//! puvodni" okno spustene z terminalu, viz i zpetna vazba "musim do okna
//! nejdriv kliknout, pak jde psat heslo") - `LockScreen::focus_attempts`
//! v `app.rs` uz to resilo aspon opakovanymi pokusy, ale kdyz je splash i
//! zbytek aplikace odjakziva JEDNO a totez okno, cely tenhle problem
//! odpada uplne sam - zadne "druhe" okno, o jehoz fokus by WM musel znovu
//! rozhodovat.
//!
//! Vizualne stejny princip jako puvodni `termx-splash`: logo, pod nim se
//! "pismenko po pismenku" (s kurzorem, ktery bud aktivne "pise", nebo v
//! pauzach blika) vypise verze a autor - font DejaVu Sans Mono Bold
//! (`assets/fonts/`), stejne casovaci konstanty. Vykresleni ale jde primo
//! pres `egui`uv vlastni text/image painter (viz `render`) misto rucniho
//! rasterizovani do pixel bufferu - `fontdue`/`minifb`/`x11-dl` uz proto
//! nejsou potreba vubec.
//!
//! POZNAMKA K OVERENI: `egui::Image::new((texture_id, size))` (viz
//! `render` nize) je bezny zpusob, jak v egui/eframe ~0.24-0.29 vykreslit
//! uz drive nahranou texturu ve zvolene velikosti - v tomto prostredi
//! nebylo mozne overit skutecnym `cargo build` (zadny pristup na
//! crates.io). Pokud presto build selze prave na teto radce, jde o
//! izolovanou opravu jen zpusobu vykresleni loga, zbytek (casovani,
//! textova animace, prechod na `LockScreen`) na tom nezavisi.

use std::time::{Duration, Instant};

use egui::{Color32, FontId};

const LOGO_BYTES: &[u8] = include_bytes!("../../../assets/term-ix_logo.png");
const FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSansMono-Bold.ttf");

/// Jmeno fontu, pod kterym je DejaVu Sans Mono Bold vlozeny do
/// `egui::FontDefinitions` (viz `install_font`) - pouzity jen pro text na
/// teto obrazovce, zbytek aplikace ma dal vychozi fonty beze zmeny.
const FONT_NAME: &str = "term-ix-splash-mono";

/// Kolik milisekund trva "napsani" jednoho znaku - stejna hodnota jako
/// mel puvodni `termx-splash`.
const TYPE_INTERVAL_MS: u128 = 32;
/// Pauza mezi prvnim (verze) a druhym (autor) radkem.
const LINE_PAUSE_MS: u128 = 100;
/// Jak dlouho jeste zustane splash zobrazeny po dopsani textu, nez sam
/// zmizi.
const IDLE_HOLD_MS: u128 = 2000;
/// Jak casto blika kurzor, kdyz zrovna nic nepise.
const CURSOR_BLINK_MS: u128 = 500;
/// Bezpecnostni strop celkove doby zobrazeni.
const MAX_DURATION: Duration = Duration::from_millis(5000);

const COLOR_TEXT: Color32 = Color32::from_rgb(0xE8, 0xEC, 0xEF);
const COLOR_CURSOR: Color32 = Color32::from_rgb(0x7F, 0xE0, 0xDC);

/// Velikost OKNA po dobu splash animace - zamerne male (na rozdil od
/// normalni pracovni velikosti aplikace, `crate::MAIN_WINDOW_SIZE`),
/// stejne jako mival puvodni samostatny splash (byvaly crate
/// `termx-splash`, ktery byl velky priblizne jako samotne logo). Pouziva
/// se v `lib.rs::run_app` jako pocatecni velikost okna, kdyz `show_splash
/// == true` - po dobehnuti animace se okno zvetsi zpet na
/// `MAIN_WINDOW_SIZE` (viz `app::resize_to_main_window`).
pub const WINDOW_SIZE: [f32; 2] = [420.0, 460.0];

/// Stav bezici splash animace - viz `SplashState::new`/`render`.
pub struct SplashState {
    start: Instant,
    /// `None`, dokud se poprve nezavola `render` (textura potrebuje
    /// `egui::Context`, ktery `new` nema k dispozici) - pak uz zustava
    /// `Some` po celou dobu zobrazeni splashe.
    logo_texture: Option<egui::TextureHandle>,
}

impl SplashState {
    pub fn new() -> Self {
        Self { start: Instant::now(), logo_texture: None }
    }
}

/// Vlozi DejaVu Sans Mono Bold do `egui::FontDefinitions` pod `FONT_NAME`
/// jako DALSI (ne nahrazujici) rodinu fontu - volat JEDNOU pri startu,
/// driv nez se cokoliv vykresli (viz `lib.rs::run_app`). `egui::FontDefinitions::default()`
/// uz sam o sobe obsahuje vestavene fonty (`default_fonts` feature u
/// `eframe`, viz Cargo.toml) pouzivane vsude jinde v aplikaci - ty timto
/// zustavaji nedotcene, jen pribyva jedna dalsi pojmenovana rodina navic.
pub fn install_font(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(FONT_NAME.to_owned(), egui::FontData::from_static(FONT_BYTES));
    fonts.families.entry(egui::FontFamily::Name(FONT_NAME.into())).or_default().insert(0, FONT_NAME.to_owned());
    ctx.set_fonts(fonts);
}

fn load_logo_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    // Stejna zasada jako u puvodniho `termx-splash` - kosmeticky bonus
    // nikdy nesmi zabranit startu. Kdyby se vestavene logo (nikdy by se
    // nemelo stat, je to bajty zabalene primo do binarky) z nejakeho
    // duvodu nepodarilo dekodovat, splash proste logo jen nezobrazi -
    // textova cast (verze/autor) beze zmeny.
    let img = image::load_from_memory(LOGO_BYTES).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture("term-ix-splash-logo", color_image, egui::TextureOptions::LINEAR))
}

/// Vykresli jeden snimek splash obrazovky a vrati `true`, kdyz uz ma byt
/// nahrazena zamcenou obrazovkou (viz volajici `TermxApp::update`) -
/// bud proto, ze vyprsel cas (dopsani + `IDLE_HOLD_MS`, nebo bezpecnostni
/// `MAX_DURATION`), nebo uzivatel splash sam preskocil (Esc/Enter/mezernik/klik).
pub fn render(ctx: &egui::Context, state: &mut SplashState, version: &str, author: &str) -> bool {
    if state.logo_texture.is_none() {
        state.logo_texture = load_logo_texture(ctx);
    }

    let elapsed = state.start.elapsed();
    let e_ms = elapsed.as_millis();

    let line1_text = format!("Term-IX v{version}");
    let line2_text = author.to_string();
    let chars1 = line1_text.chars().count() as u128;
    let chars2 = line2_text.chars().count() as u128;

    let t_line1_end = chars1 * TYPE_INTERVAL_MS;
    let t_line2_start = t_line1_end + LINE_PAUSE_MS;
    let t_line2_end = t_line2_start + chars2 * TYPE_INTERVAL_MS;
    let close_after_ms = t_line2_end + IDLE_HOLD_MS;

    let dismiss_requested = ctx.input(|i| {
        i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space) || i.pointer.any_click()
    });

    let reveal1 = ((e_ms / TYPE_INTERVAL_MS) as usize).min(chars1 as usize);
    let reveal2 = if e_ms < t_line2_start { 0 } else { (((e_ms - t_line2_start) / TYPE_INTERVAL_MS) as usize).min(chars2 as usize) };

    let blink_on = (e_ms / CURSOR_BLINK_MS) % 2 == 0;
    // Kurzor: dokud se aktivne pise, je vzdy viditelny (jako pisici se
    // kurzor v terminalu); v pauze mezi radky a po dopsani obou radku
    // blika - stejna logika jako puvodni `termx-splash`.
    let (cursor_on_line1, cursor_on_line2) = if reveal1 < chars1 as usize {
        (true, false)
    } else if e_ms < t_line2_start {
        (blink_on, false)
    } else if reveal2 < chars2 as usize {
        (false, true)
    } else {
        (false, blink_on)
    };

    egui::CentralPanel::default().show(ctx, |ui| {
        // Sirka k dispozici pro vodorovne centrovani - zjistena JEDNOU tady
        // (na urovni celeho panelu), aby na ni sla dal spolehnout uvnitr
        // `ui.vertical`/`ui.horizontal` nize beze zmeny (ty samy o sobe
        // sirku nezuzuji).
        //
        // POZNAMKA: puvodne se centrovani nechavalo na `ui.vertical_centered`
        // obalujici `ui.horizontal(...)` s textem - v praxi to ale
        // vysledne radky nechalo zarovnane doleva (zpetna vazba "text
        // animace zarovnana doleva"), protoze `ui.horizontal` bez daneho
        // rozmeru sam o sobe zabira CELOU dostupnou sirku, a `vertical_centered`
        // pak nema co centrovat (jiz "plna sirka" = zadny prostor navic na
        // stranach). Misto spolehani na tohle automaticke chovani se
        // stred kazdeho radku pocita RUCNE (zmereni skutecne sirky textu/
        // obrazku pres `ui.fonts`/`tex.size_vec2()` a pridani prislusne
        // mezery pred nej pres `ui.add_space`) - stejny princip, jaky uz
        // (kvuli chybejicimu automatickemu layoutu) musel pouzivat i
        // puvodni pixel-bufferovy `termx-splash`.
        let content_width = ui.available_width();

        ui.vertical(|ui| {
            ui.add_space((ui.available_height() / 2.0 - 140.0).max(24.0));

            if let Some(tex) = &state.logo_texture {
                let max_dim = 200.0_f32;
                let size = tex.size_vec2();
                let scale = (max_dim / size.x.max(size.y)).min(1.0);
                let img_size = size * scale;
                ui.horizontal(|ui| {
                    ui.add_space(((content_width - img_size.x) / 2.0).max(0.0));
                    ui.add(egui::Image::new((tex.id(), img_size)));
                });
                ui.add_space(14.0);
            }

            let font = FontId::new(17.0, egui::FontFamily::Name(FONT_NAME.into()));
            let shown1: String = line1_text.chars().take(reveal1).collect();
            let shown2: String = line2_text.chars().take(reveal2).collect();

            // Sirka se pocita z CELEHO (nikoliv jen prave odhaleneho)
            // textu radku, takze stred radku zustava po celou dobu psani
            // na stejnem miste - jen se do nej postupne "dopisuji" dalsi
            // znaky zleva doprava, misto aby se cely blok pri kazdem
            // znaku znovu precentroval (presne jako u puvodniho
            // `termx-splash`, ktery si layout cele radky spocital take jen
            // jednou a odhaleni pak jen omezovalo pocet vykreslenych
            // glyfu v ramci nej - viz tam).
            //
            // POZNAMKA K OVERENI: `Fonts::layout_no_wrap` (pres
            // `ui.fonts(|f| ...)`) je bezny zpusob v egui ~0.29, jak
            // zjistit skutecnou vykreslenou sirku textu v danem fontu -
            // nebylo mozne overit skutecnym `cargo build`.
            let width1 = ui.fonts(|f| f.layout_no_wrap(line1_text.clone(), font.clone(), COLOR_TEXT).size().x);
            let width2 = ui.fonts(|f| f.layout_no_wrap(line2_text.clone(), font.clone(), COLOR_TEXT).size().x);

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(((content_width - width1) / 2.0).max(0.0));
                ui.label(egui::RichText::new(shown1).font(font.clone()).color(COLOR_TEXT));
                if cursor_on_line1 {
                    ui.label(egui::RichText::new("\u{2588}").font(font.clone()).color(COLOR_CURSOR));
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(((content_width - width2) / 2.0).max(0.0));
                ui.label(egui::RichText::new(shown2).font(font.clone()).color(COLOR_TEXT));
                if cursor_on_line2 {
                    ui.label(egui::RichText::new("\u{2588}").font(font).color(COLOR_CURSOR));
                }
            });
        });
    });

    // Animace potrebuje dalsi snimek co nejdriv i bez pohnuti mysi/klavesy
    // - `eframe` jinak (mimo tento pripad) prekresluje jen na vstup.
    ctx.request_repaint();

    dismiss_requested || elapsed >= MAX_DURATION || e_ms >= close_after_ms
}
