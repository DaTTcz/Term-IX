//! Barevna paleta odpovidajici logu Term-IX (tmave modro-seda + cyan
//! akcent). Zatim jedine dostupne "terminalove" tema - MobaXterm-podobne
//! modernejsi tema je pripraveno jako budouci volba v menu Settings, az
//! bude v `termx-gui` vice nez jeden vzhled.

use egui::{Color32, Visuals};

pub const BG_DARK: Color32 = Color32::from_rgb(0x20, 0x25, 0x2b);
pub const BG_PANEL: Color32 = Color32::from_rgb(0x2a, 0x30, 0x38);
pub const ACCENT: Color32 = Color32::from_rgb(0x7f, 0xe0, 0xdc);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x3a, 0x8f, 0x8a);
pub const TEXT: Color32 = Color32::from_rgb(0xe8, 0xec, 0xef);

pub fn apply(ctx: &egui::Context) {
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

    ctx.set_visuals(visuals);
}
