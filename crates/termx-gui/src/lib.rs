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
//! DULEZITY ROZSAH TETO VERZE: tab spojeni (`TabKind::Connection`) je
//! zatim JEN NAHRADNI OBRAZOVKA - ukazuje udaje o serveru, ale
//! nepripojuje se. Skutecny vestaveny emulator terminalu (bez
//! nativniho OS okna, pres `alacritty_terminal`) je navazujici krok,
//! ktery se napoji na `termx_core::ProtocolModule` (tedy i na
//! `termx-ssh`) misto puvodniho prevzeti stdin/stdout.

mod app;
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
        ..Default::default()
    };

    eframe::run_native(
        "Term-IX",
        native_options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(app::TermxApp::new(vault_path, registry)))
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
