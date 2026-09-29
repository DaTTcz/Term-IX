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

/// Zapise `.desktop` soubor a vsechny velikosti ikon. Zapisuje se JEN kdyz
/// se obsah lisi od uz nainstalovaneho (a jen pak se obcerstvuji cache
/// desktopu) - bez zbytecne prace pri kazdem startu.
///
/// Obcerstveni cache (best-effort, na pozadi - `spawn` bez cekani, at
/// nezdrzuje start aplikace; kde nastroj neni, proste se preskoci):
/// - `update-desktop-database`/`gtk-update-icon-cache` - GNOME/Cinnamon/...
/// - `kbuildsycoca6`/`kbuildsycoca5` - KDE Plasma. POZOR (zjisteno na
///   openSUSE Slowroll + Plasma/Wayland): KWin hleda ikonu okna podle
///   app_id v databazi `.desktop` souboru "sycoca", ktera se po pridani
///   noveho souboru do `~/.local/share/applications` nemusi hned
///   obnovit - okno pak melo misto ikony Term-IX genericke "W"
///   (Wayland). `kbuildsycoca6` databazi obnovi hned.
pub fn install() {
    let Some(data_home) = xdg_data_home() else { return };

    let mut changed = false;
    match install_desktop_file(&data_home) {
        Ok(c) => changed |= c,
        Err(e) => tracing::debug!("nepodarilo se nainstalovat .desktop soubor: {e}"),
    }
    match install_icons(&data_home) {
        Ok(c) => changed |= c,
        Err(e) => tracing::debug!("nepodarilo se nainstalovat ikony aplikace: {e}"),
    }

    if changed {
        let apps = data_home.join("applications");
        let icons = data_home.join("icons/hicolor");
        spawn_quiet("update-desktop-database", &[apps.as_os_str()]);
        spawn_quiet("gtk-update-icon-cache", &[std::ffi::OsStr::new("-f"), std::ffi::OsStr::new("-t"), icons.as_os_str()]);
        if !spawn_quiet("kbuildsycoca6", &[]) {
            spawn_quiet("kbuildsycoca5", &[]);
        }
    }
}

/// Spusti prikaz na pozadi bez vystupu; `false` kdyz neexistuje.
fn spawn_quiet(program: &str, args: &[&std::ffi::OsStr]) -> bool {
    std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

/// Zapise soubor jen kdyz se jeho obsah lisi; vraci `true` pri zmene.
fn write_if_changed(path: &std::path::Path, contents: &[u8]) -> std::io::Result<bool> {
    if std::fs::read(path).is_ok_and(|old| old == contents) {
        return Ok(false);
    }
    std::fs::write(path, contents)?;
    Ok(true)
}

fn install_desktop_file(data_home: &std::path::Path) -> std::io::Result<bool> {
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
         Categories=Network;TerminalEmulator;RemoteAccess;Utility;\n\
         StartupWMClass=term-ix\n"
    );

    let apps_dir = data_home.join("applications");
    std::fs::create_dir_all(&apps_dir)?;
    write_if_changed(&apps_dir.join("term-ix.desktop"), contents.as_bytes())
}

fn install_icons(data_home: &std::path::Path) -> std::io::Result<bool> {
    let mut changed = false;
    for (size, bytes) in ICONS {
        let dir = data_home.join(format!("icons/hicolor/{size}x{size}/apps"));
        std::fs::create_dir_all(&dir)?;
        changed |= write_if_changed(&dir.join("term-ix.png"), bytes)?;
    }
    Ok(changed)
}
