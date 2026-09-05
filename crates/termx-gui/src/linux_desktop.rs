//! Sebeinstalace `.desktop` souboru a sady ikon do XDG slozek uzivatele
//! na Linuxu - zpetna vazba "zjistil jsem že nemáme nikde ikonku
//! aplikace ani v liště ani na ploše".
//!
//! Term-IX se distribuuje jako samostatna binarka (zadny .deb/.rpm
//! balicek, zadny instalator) - `packaging/term-ix.desktop` a ikony v
//! `assets/icons/hicolor/...` v repu sice existovaly uz drive, ale
//! nikam se neinstalovaly, takze se k bezicimu uzivateli vubec
//! nedostaly. `install()` (volano jednou z `run_app` pri kazdem startu)
//! tohle napravi rucne za behu - zapise `.desktop` soubor (s `Exec`
//! ukazujicim na SKUTECNOU cestu prave bezici binarky, viz
//! `current_exe`) a celou sadu ikon do `~/.local/share/applications`/
//! `~/.local/share/icons/hicolor/...` (respektuje `$XDG_DATA_HOME`,
//! pokud je nastaveny). Bezi na kazdem spusteni - levne (par malych
//! zapisu) a samo-opravne, kdyby si uzivatel soubory omylem smazal.
//!
//! Doplnuje se `ViewportBuilder::with_app_id("term-ix")` v `run_app`
//! (viz tamni komentar) - na Waylandu (napr. GNOME) se bez shodneho
//! app_id ikona v panelu/doku nezobrazi vubec, i kdyby `.desktop`
//! soubor a ikony byly nainstalovane spravne.
//!
//! Chyby (chybejici `$HOME`, nezapisovatelny adresar, ...) se jen tise
//! zaloguji - chybejici ikonka v panelu neni duvod aplikaci vubec
//! nespustit.

const ICON_16: &[u8] = include_bytes!("../../../assets/icons/hicolor/16x16/apps/term-ix.png");
const ICON_32: &[u8] = include_bytes!("../../../assets/icons/hicolor/32x32/apps/term-ix.png");
const ICON_48: &[u8] = include_bytes!("../../../assets/icons/hicolor/48x48/apps/term-ix.png");
const ICON_64: &[u8] = include_bytes!("../../../assets/icons/hicolor/64x64/apps/term-ix.png");
const ICON_128: &[u8] = include_bytes!("../../../assets/icons/hicolor/128x128/apps/term-ix.png");
const ICON_256: &[u8] = include_bytes!("../../../assets/icons/hicolor/256x256/apps/term-ix.png");
const ICON_512: &[u8] = include_bytes!("../../../assets/icons/hicolor/512x512/apps/term-ix.png");

const ICONS: &[(u32, &[u8])] =
    &[(16, ICON_16), (32, ICON_32), (48, ICON_48), (64, ICON_64), (128, ICON_128), (256, ICON_256), (512, ICON_512)];

/// XDG data adresar uzivatele (`$XDG_DATA_HOME`, jinak `~/.local/share`) -
/// stejna konvence, jakou pro tento ucel pouziva vetsina linuxovych
/// desktopu/specifikace freedesktop.org.
fn xdg_data_home() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("XDG_DATA_HOME") {
        if !dir.trim().is_empty() {
            return Some(std::path::PathBuf::from(dir));
        }
    }
    let home = std::env::var("HOME").ok()?;
    Some(std::path::PathBuf::from(home).join(".local/share"))
}

/// Zapise `.desktop` soubor a vsechny velikosti ikon; zkusi (best-effort)
/// obcerstvit `update-desktop-database`/`gtk-update-icon-cache`, at se
/// zmena projevi hned bez nutnosti odhlaseni - kde tyto nastroje
/// nejsou k dispozici (ne kazde DE je ma), zmena se stejne projevi pri
/// pristim prihlaseni/restartu panelu.
pub fn install() {
    let Some(data_home) = xdg_data_home() else { return };

    if let Err(e) = install_desktop_file(&data_home) {
        tracing::debug!("nepodarilo se nainstalovat .desktop soubor: {e}");
    }
    if let Err(e) = install_icons(&data_home) {
        tracing::debug!("nepodarilo se nainstalovat ikony aplikace: {e}");
    }

    let _ = std::process::Command::new("update-desktop-database").arg(data_home.join("applications")).output();
    let _ = std::process::Command::new("gtk-update-icon-cache").arg(data_home.join("icons/hicolor")).output();
}

fn install_desktop_file(data_home: &std::path::Path) -> std::io::Result<()> {
    // `Exec`/`TryExec` musi ukazovat na SKUTECNOU absolutni cestu bezici
    // binarky - zadna pevna instalacni cesta jako `/usr/bin/term-ix`
    // tu nedava smysl (aplikace nema instalator, uzivatel si ji
    // rozbaluje kamkoliv), takze se zjistuje za behu.
    let exe = std::env::current_exe()?;
    let exe = exe.to_string_lossy();

    let contents = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Term-IX\n\
         GenericName=Terminálový klient\n\
         Comment=Modulární terminálový klient (SSH/Serial/FTP...)\n\
         Exec={exe}\n\
         TryExec={exe}\n\
         Icon=term-ix\n\
         Terminal=false\n\
         Categories=Network;TerminalEmulator;RemoteAccess;\n\
         StartupWMClass=term-ix\n"
    );

    let apps_dir = data_home.join("applications");
    std::fs::create_dir_all(&apps_dir)?;
    std::fs::write(apps_dir.join("term-ix.desktop"), contents)
}

fn install_icons(data_home: &std::path::Path) -> std::io::Result<()> {
    for (size, bytes) in ICONS {
        let dir = data_home.join(format!("icons/hicolor/{size}x{size}/apps"));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("term-ix.png"), bytes)?;
    }
    Ok(())
}
