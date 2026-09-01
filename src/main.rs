//! Term-IX - binarni crate, ktery propojuje vsechny moduly dohromady:
//! zaregistruje dostupne protokolove moduly (zatim termx-ssh) do GUI
//! (termx-gui). Odemceni/vytvoreni sifrovaneho trezoru (termx-vault) uz
//! resi az samotne GUI po otevreni hlavniho okna - zadne cmd okno k
//! tomu neni potreba.
//!
//! Kontrola/instalace aktualizaci (`termx-update`) uz NENI soucasti
//! tohoto souboru - drive se delala VZDY automaticky a potichu tady,
//! jeste pred otevrenim hlavniho okna (`check_for_updates`, ted uz
//! smazano), coz podle zpetne vazby "spustil jsem program, startokno a
//! konec nic mi to neřeklo a updatlo to samo" nedavalo uzivateli zadnou
//! zpetnou vazbu ani moznost novou verzi rovnou spustit. Cely
//! mechanismus je ted vestaveny primo v GUI (Home tab, viz
//! `termx-gui/src/app.rs::render_update_check_status`) jako viditelny/
//! ovladany krok - `--no-update` nize uz tedy vypina AUTOMATICKOU
//! kontrolu PRI STARTU TAM (rucni tlacitko v dialogu "O programu" jde
//! pouzit i tak), ne puvodni blokujici instalaci zde.
//!
//! Pridani noveho protokolu do aplikace = novy crate `termx-<protokol>`
//! implementujici `termx_core::ProtocolModule` + jeden radek
//! `registry.register(...)` zde.

// Na Windows by binarka jinak i jako cistokrevni GUI program dostala
// vlastni "console" subsystem, a pri kazdem spusteni by se tak na
// pozadi otevrelo prazdne cmd okno (i kdyz uz do nej nic neprosime,
// jako drive pres `rpassword`). Tento atribut na Windows prepne
// binarku na "windows" subsystem, ktery zadne konzolove okno neotevira
// - jen v ladicim (`cargo build`/`cargo run` bez `--release`) buildu ho
// zamerne necháváme, aby byly bezne bĕhem vyvoje videt println!/log
// hlasky z konzole. Na jinych platformach je atribut bez efektu.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use clap::Parser;
use termx_core::{AppPaths, ModuleRegistry};

#[derive(Parser)]
#[command(name = "term-ix", version, about = "Term-IX - modularni terminalovy klient (SSH/Serial/FTP...)")]
struct Cli {
    /// Preskoci AUTOMATICKOU kontrolu aktualizaci pri startu (Home tab v
    /// GUI, viz `termx-gui/src/app.rs::UpdateCheck`) - rucni tlacitko
    /// "Zkontrolovat aktualizace" v dialogu "O programu" jde pouzit i
    /// tak.
    #[arg(long)]
    no_update: bool,

    /// Preskoci uvodni splash obrazovku s logem.
    #[arg(long)]
    no_splash: bool,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::try_init().ok();
    // Bez tohoto volani Windows u DPI-nevedomych aplikaci sam "roztahuje"
    // vykreslene okno podle systemoveho meritka (typicky 125/150/200 %),
    // takze napr. splash okno vypada 2x vetsi a rozmazane, nez jak bylo
    // vykresleno. Musi se volat driv, nez se otevre prvni okno.
    #[cfg(target_os = "windows")]
    set_process_dpi_aware();

    let cli = Cli::parse();

    if !cli.no_splash {
        termx_splash::show_splash(termx_splash::SplashInfo {
            version: env!("CARGO_PKG_VERSION"),
            author: "DaTTcz",
        });
    }

    let paths = AppPaths::new()?;
    paths.ensure_dirs()?;
    let vault_path = paths.vault_path();

    let mut registry = ModuleRegistry::new();
    registry.register(Arc::new(termx_ssh::SshModule::new()));

    // GUI (termx-gui, postavene na egui/eframe) si bezi ve vlastni
    // blokujici smycce na aktualnim vlakne - na rozdil od puvodniho TUI
    // uz zde neni potreba tokio runtime zalozeny primo v main(); pripadne
    // asynchronni sitove operace protokolovych modulu si spousti az GUI
    // vrstva/moduly samy, az bude vestaveny terminal skutecne pripojeny.
    // Samotne odemceni/vytvoreni trezoru (drive `prompt_master_password`
    // pres cmd konzoli) resi az GUI na uvodni obrazovce po otevreni okna.
    termx_gui::run_app(vault_path, registry, cli.no_update)
}

/// Rekne Windows, ze si o vlastni vykreslovani/meritko umime rozhodnout
/// sami (DPI-aware), aby nas nezacal automaticky "roztahovat" bitmapove
/// podle systemoveho meritka zobrazeni. Volana FFI funkce (`user32.dll`)
/// je soucasti Win32 API od Windows Vista, zadna dalsi zavislost neni
/// potreba.
#[cfg(target_os = "windows")]
fn set_process_dpi_aware() {
    #[link(name = "user32")]
    extern "system" {
        fn SetProcessDPIAware() -> i32;
    }
    unsafe {
        SetProcessDPIAware();
    }
}
