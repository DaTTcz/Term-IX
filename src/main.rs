//! Term-IX - binarni crate, ktery propojuje vsechny moduly dohromady:
//! nacte/vytvori sifrovany trezor (termx-vault), zaregistruje dostupne
//! protokolove moduly (zatim termx-ssh) do TUI (termx-tui) a pred startem
//! zkontroluje aktualizace (termx-update).
//!
//! Pridani noveho protokolu do aplikace = novy crate `termx-<protokol>`
//! implementujici `termx_core::ProtocolModule` + jeden radek
//! `registry.register(...)` zde.

use std::sync::Arc;

use clap::Parser;
use termx_core::AppPaths;
use termx_tui::ModuleRegistry;

#[derive(Parser)]
#[command(name = "term-ix", version, about = "Term-IX - modularni terminalovy klient (SSH/Serial/FTP...)")]
struct Cli {
    /// Preskoci kontrolu aktualizaci pri startu.
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

    if !cli.no_update {
        check_for_updates();
    }

    let paths = AppPaths::new()?;
    paths.ensure_dirs()?;
    let vault_path = paths.vault_path();

    let master_password = prompt_master_password(&vault_path)?;

    let vault = if vault_path.exists() {
        termx_vault::Vault::unlock(&vault_path, &master_password)
            .map_err(|e| anyhow::anyhow!("Nepodarilo se odemknout trezor ({e}). Pri zapomenutem hesle bohuzel neexistuje zadny zpusob obnovy."))?
    } else {
        println!("Zadny trezor jeste neexistuje, vytvarim novy: {}", vault_path.display());
        termx_vault::Vault::create(&vault_path, &master_password)?
    };

    let mut registry = ModuleRegistry::new();
    registry.register(Arc::new(termx_ssh::SshModule::new()));

    // Async cast aplikace (TUI smycka + sitove operace modulu) bezi ve
    // vlastnim tokio runtime, ktery zakladame az TADY - self-update a
    // prace s trezorem vyse jsou zamerne cist synchronni, aby se
    // predeslo problemum s vnorenymi/blokujicimi volanimi uvnitr
    // async kontextu.
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(termx_tui::run_app(vault, master_password, registry))
}

fn check_for_updates() {
    match termx_update::self_update(env!("CARGO_PKG_VERSION")) {
        Ok(outcome) if outcome.updated => {
            println!(
                "Term-IX byl aktualizovan na verzi {} - spustte aplikaci prosim znovu.",
                outcome.version
            );
            std::process::exit(0);
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!("Kontrola aktualizaci se nezdarila, pokracuji bez ni ({e}).");
        }
    }
}

fn prompt_master_password(vault_path: &std::path::Path) -> anyhow::Result<String> {
    if vault_path.exists() {
        let password = rpassword::prompt_password("Hlavni heslo trezoru: ")?;
        Ok(password)
    } else {
        println!("Nastavujete hlavni heslo noveho trezoru.");
        println!("POZOR: pri zapomenuti tohoto hesla se k ulozenym udajum jiz NIKDO nedostane.");
        let password1 = rpassword::prompt_password("Nove hlavni heslo: ")?;
        let password2 = rpassword::prompt_password("Zopakujte heslo: ")?;
        if password1 != password2 {
            anyhow::bail!("Zadana hesla se neshoduji");
        }
        if password1.is_empty() {
            anyhow::bail!("Hlavni heslo nesmi byt prazdne");
        }
        Ok(password1)
    }
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
