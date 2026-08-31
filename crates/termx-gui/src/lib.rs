//! termx-gui
//!
//! Hlavni graficke uzivatelske rozhrani Term-IX (nahrazuje puvodni
//! terminalove TUI): vlastni okno s hornim menu, levym panelem se
//! stromem ulozenych serveru/slozek a hlavni plochou s taby (Home,
//! Nastaveni, jednotliva spojeni). Hlavni heslo trezoru se zadava/
//! nastavuje primo v tomto okne (uvodni "zamcena" obrazovka) - zadne
//! konzolove (cmd) okno k tomu neni potreba, viz [`run_app`].
//!
//! POZNAMKA K OVERENI: stejne jako u `termx-ssh` a `termx-splash`, ani
//! zde nebylo v tomto prostredi mozne spustit skutecny `cargo build`
//! (zadny pristup na crates.io) - `egui`/`eframe` API bylo pouzito podle
//! nejlepsiho vedomi pro verzi ~0.29, ale drobne nazvy metod se mohou
//! po prvnim buildu lisit.
//!
//! Tab spojeni (`TabKind::Connection`) ma vestaveny emulator terminalu
//! (`alacritty_terminal`, viz `terminal.rs`) napojeny primo na
//! `termx_ssh::spawn_ssh_session` (ne na obecny `termx_core::ProtocolModule::run`,
//! ktery pocita s puvodnim prevzetim stdin/stdout - viz poznamka tam) -
//! zatim jen pro SSH, dalsi protokoly (Serial/FTP/...) budou potrebovat
//! obdobnou specializovanou cestu, az pribudou.
//!
//! Uzivatelska nastaveni (`app::AppSettings` - zatim jen automaticke
//! obnoveni ztraceneho SSH spojeni) se ukladaji pres bezny eframe
//! perzistentni ulozny prostor (`cc.storage` predane sem do
//! `app::TermxApp::new`) - stejny mechanismus, jaky uz drive vyuziva
//! `persist_window` nize pro polohu/velikost okna, jen s vlastnim
//! klicem (viz `app::SETTINGS_STORAGE_KEY`).

mod app;
mod i18n;
mod terminal;
mod theme;

use std::path::PathBuf;

use termx_core::ModuleRegistry;

const ICON_BYTES: &[u8] = include_bytes!("../../../assets/icons/hicolor/128x128/apps/term-ix.png");

/// Spusti hlavni okno aplikace. Eframe si bezi ve vlastni (blokujici)
/// smycce na aktualnim vlakne - volat primo z `main()`, ne zevnitr
/// tokio `block_on`.
///
/// Na rozdil od puvodni verze uz sem `main()` nepreda uz odemceny
/// `Vault` - jen cestu k souboru trezoru (`vault_path`). Odemceni (nebo
/// nastaveni hesla pro novy trezor) resi az samotne GUI na uvodni
/// obrazovce po otevreni okna.
pub fn run_app(vault_path: PathBuf, registry: ModuleRegistry) -> anyhow::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1150.0, 720.0])
            .with_min_inner_size([760.0, 440.0])
            .with_icon(load_icon()),
        // Pri prvnim spusteni (kdyz jeste neni co obnovit) se okno
        // vycentruje na obrazovce. `persist_window` pak pri kazdem
        // dalsim spusteni (diky cargo feature "persistence" u eframe)
        // obnovi presne tu polohu a velikost, ve ktere uzivatel okno
        // naposledy zavrel - vlastni ukladaci/nacitaci logika k tomu
        // netreba, o to se stara primo eframe.
        centered: true,
        persist_window: true,
        ..Default::default()
    };

    eframe::run_native(
        "Term-IX",
        native_options,
        Box::new(move |cc| {
            let app = app::TermxApp::new(vault_path, registry, cc.storage);
            // Tema se aplikuje az PO nacteni `AppSettings` (uvnitr
            // `TermxApp::new`) - viz `TermxApp::initial_theme` - aby se
            // uz od prvniho snimku pouzilo ulozene uzivatelovo tema, ne
            // vzdy jen vychozi `Theme::Terminal`.
            theme::apply(&cc.egui_ctx, app.initial_theme());
            // `persist_window` (viz `native_options` vyse) uz sam obnovi
            // ulozenou polohu/velikost okna, ale ne jeho maximalizaci -
            // tu obnovime rucne, hned jak je okno vytvorene, podle
            // naposledy ulozene hodnoty (viz `AppSettings::window_maximized`).
            // Zamerne NEreseno pro minimalizaci - ta se vubec nesleduje
            // ani neuklada.
            if app.wants_maximized() {
                cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("nepodarilo se spustit graficke rozhrani: {e}"))
}

fn load_icon() -> egui::IconData {
    match image::load_from_memory(ICON_BYTES) {
        Ok(img) => {
            let img = img.to_rgba8();
            let (width, height) = img.dimensions();
            egui::IconData {
                rgba: img.into_raw(),
                width,
                height,
            }
        }
        Err(e) => {
            tracing::debug!("nepodarilo se nacist ikonu aplikace: {e}");
            egui::IconData::default()
        }
    }
}
