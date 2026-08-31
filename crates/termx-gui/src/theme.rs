//! Vizualni "vzhledy" (temata) aplikace - viz [`Theme`] a pozadavek
//! "zkusme přidat nějaký druhý vzhled ať doladíme jejich ovládání".
//!
//! Zatim dve temata:
//! - [`Theme::Terminal`] - puvodni (a do ted jedine) tmave modro-seda +
//!   cyan akcent paleta odpovidajici logu Term-IX, MobaXterm-podobna.
//!   Zustava vychozi pro existujici uzivatele (viz `Default for Theme`).
//! - [`Theme::Modern`] - novy, svetly (bily/sedy) protejsek se stejnym
//!   cyan/tyrkysovym akcentem (jen o neco tmavsim odstinem kvuli
//!   citelnosti na svetlem pozadi), aby obe temata pusobila jako
//!   varianty JEDNE znacky, ne dva nesouvisejici vzhledy.
//!
//! POZOR NA ROZSAH: `Theme::apply` prepisuje obecne `egui::Visuals`
//! (pozadi panelu/oken, barva textu, zvyrazneni, hover/active stavy
//! tlacitek) - vsechny panely/dialogy/tlacitka v aplikaci (vc. hlavni
//! plochy `CentralPanel` a stavoveho prouzku terminalu -
//! `terminal::TerminalSession::render_status_bar`) si pozadi berou
//! DYNAMICKY primo z aktualnich `Visuals` (napr. `ui.visuals().panel_fill`),
//! ne z konstant nize, takze spravne sleduji zvolene tema (viz zpetna
//! vazba "světlé téma je nečitelné" / "ještě info panel vespod se
//! nepřepl barevně", ktera odhalila dve mista, kde to puvodne
//! neplatilo). Konstanty `ACCENT`/`DANGER`/`BG_PANEL`/`BG_DARK`/`TEXT`
//! nize zustavaji zamerne SPOLECNE pro obe temata jen tam, kde uz jde
//! primo o obsah SAMOTNEHO terminalu (vychozi ANSI popredi/pozadi
//! bufferu terminalu - `terminal::cell_colors`) - text uvnitr terminalu
//! typicky zustava tmavy i v ramci jinak svetleho OS motivu, stejne
//! jako u skutecnych terminalovych emulatoru.

use egui::{Color32, Visuals};

use crate::i18n::Lang;

/// Barvy tematu [`Theme::Terminal`] (tmave) - viz komentar u modulu.
pub const BG_DARK: Color32 = Color32::from_rgb(0x20, 0x25, 0x2b);
pub const BG_PANEL: Color32 = Color32::from_rgb(0x2a, 0x30, 0x38);
pub const ACCENT: Color32 = Color32::from_rgb(0x7f, 0xe0, 0xdc);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x3a, 0x8f, 0x8a);
pub const TEXT: Color32 = Color32::from_rgb(0xe8, 0xec, 0xef);
/// Barva pro chybove/varovne stavy (napr. odpojeny SSH terminal - viz
/// `terminal::TerminalSession::render`, obarveny tab v `tab_bar`) -
/// stejny odstin, jaky uz drive ad-hoc pouzivala chybova hlaska v
/// `app.rs`/`terminal.rs`, teď sjednoceny na jedno misto. Spolecna pro
/// obe temata (viz POZOR NA ROZSAH vyse).
pub const DANGER: Color32 = Color32::from_rgb(0xe0, 0x6c, 0x6c);

/// Barvy tematu [`Theme::Modern`] (svetle).
const BG_LIGHT: Color32 = Color32::from_rgb(0xf3, 0xf5, 0xf6);
const BG_LIGHT_PANEL: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
const ACCENT_LIGHT: Color32 = Color32::from_rgb(0x1f, 0x8f, 0x86);
const ACCENT_LIGHT_DIM: Color32 = Color32::from_rgb(0xc9, 0xed, 0xea);
const TEXT_LIGHT: Color32 = Color32::from_rgb(0x22, 0x28, 0x2c);

/// Vzhled ("tema") aplikace - viz [`crate::app::AppSettings::theme`]
/// (uklada se stejne jako ostatni `AppSettings`, preziva restart
/// aplikace) a prepinac v `MainApp::render_settings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Theme {
    Terminal,
    Modern,
}

impl Default for Theme {
    /// Puvodni (a do zavedeni [`Theme::Modern`] jedine) tema aplikace
    /// bylo tmave "terminalove" - zmena vychoziho vzhledu pro existujici
    /// uzivatele by byla nemile prekvapeni, takze vychozi tema zustava
    /// [`Theme::Terminal`] i po pridani druhe volby.
    fn default() -> Self {
        Theme::Terminal
    }
}

impl Theme {
    /// Vsechna dostupna temata, v poradi pro zobrazeni v dropdownu.
    pub const ALL: [Theme; 2] = [Theme::Terminal, Theme::Modern];

    /// Jmeno tematu v danem jazyce UI (na rozdil od `Lang::native_name`
    /// se tema PREKLADA - "Terminálové"/"Terminal" nejsou vlastni jmena).
    pub fn display_name(self, lang: Lang) -> &'static str {
        match (self, lang) {
            (Theme::Terminal, Lang::Cs) => "Terminálové (tmavé)",
            (Theme::Terminal, Lang::En) => "Terminal (dark)",
            (Theme::Modern, Lang::Cs) => "Moderní (světlé)",
            (Theme::Modern, Lang::En) => "Modern (light)",
        }
    }

    /// Nastavi `egui::Visuals` odpovidajici tomuto tematu - viz `apply`.
    fn visuals(self) -> Visuals {
        match self {
            Theme::Terminal => {
                let mut visuals = Visuals::dark();
                visuals.panel_fill = BG_PANEL;
                visuals.window_fill = BG_PANEL;
                visuals.extreme_bg_color = BG_DARK;
                visuals.faint_bg_color = BG_DARK;
                visuals.override_text_color = Some(TEXT);
                visuals.selection.bg_fill = ACCENT_DIM;
                visuals.selection.stroke.color = ACCENT;
                visuals.widgets.hovered.bg_fill = ACCENT_DIM;
                visuals.widgets.active.bg_fill = ACCENT_DIM;
                visuals
            }
            Theme::Modern => {
                let mut visuals = Visuals::light();
                visuals.panel_fill = BG_LIGHT_PANEL;
                visuals.window_fill = BG_LIGHT_PANEL;
                visuals.extreme_bg_color = BG_LIGHT;
                visuals.faint_bg_color = BG_LIGHT;
                visuals.override_text_color = Some(TEXT_LIGHT);
                visuals.selection.bg_fill = ACCENT_LIGHT_DIM;
                visuals.selection.stroke.color = ACCENT_LIGHT;
                visuals.widgets.hovered.bg_fill = ACCENT_LIGHT_DIM;
                visuals.widgets.active.bg_fill = ACCENT_LIGHT_DIM;
                visuals
            }
        }
    }
}

/// Aplikuje dane `theme` na cely `ctx` (viz `lib.rs::run_app`, volano
/// jednou pri startu s tematem nactenym z ulozenych `AppSettings`, a
/// znovu pri kazde zmene v `MainApp::render_settings`, aby se prepnuti
/// projevilo hned, bez nutnosti restartu aplikace).
pub fn apply(ctx: &egui::Context, theme: Theme) {
    ctx.set_visuals(theme.visuals());
}
