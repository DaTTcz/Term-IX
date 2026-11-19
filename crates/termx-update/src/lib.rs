//! termx-update
//!
//! Sebe-aktualizace z GitHub Releases repozitare `DaTTcz/Term-IX`.
//!
//! Predpoklada, ze release proces (viz `.github/workflows/release.yml`)
//! nahra na kazdy tag `vX.Y.Z` binarky pojmenovane tak, aby obsahovaly
//! cilovy target triple, napr.:
//!   term-ix-x86_64-pc-windows-msvc.zip
//!   term-ix-x86_64-unknown-linux-gnu.tar.gz
//! `self_update` pak podle triple aktualniho OS/architektury vybere
//! spravny soubor, stahne jej a nahradi aktualne bezici binarku.
//!
//! Pouziva rustls (ne OpenSSL) - nevyzaduje tedy na Windows/Linuxu
//! zadny externi C toolchain jen kvuli TLS.
//!
//! ZPUSOB INSTALACE (prevzato z eR Commanderu, `install_mode` v jeho
//! `main.rs`, kde to uz funguje) - na Linuxu jde Term-IX ziskat tremi
//! zpusoby a kazdy se aktualizuje jinak, viz [`InstallMode`]:
//!   term-ix-x86_64-unknown-linux-gnu.tar.gz  prenosna binarka
//!   term-ix-x86_64.AppImage                  AppImage
//!   term-ix_<verze>_amd64.deb / .rpm         balicek (/usr/bin)
//! AppImage/deb/rpm ZAMERNE nemaji v nazvu cely target triple, aby si je
//! `self_update` (hleda asset podle triple) nespletl s tar.gz.

use std::path::PathBuf;

use anyhow::{Context, Result};

/// Jak je Term-IX na tomhle pocitaci "nainstalovany" - podle toho se lisi
/// automaticka aktualizace (co stahnout a kam) i zastupce v menu
/// (`termx-gui/src/linux_desktop.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallMode {
    /// Samostatna binarka (tar.gz/zip z GitHubu) - aktualizuje se vymenou
    /// souboru pres `self_update`.
    Portable,
    /// Linux AppImage - aktualizuje se vymenou celeho .AppImage souboru.
    /// Cesta je v promenne `APPIMAGE` (nastavuje ji runtime AppImage);
    /// `current_exe()` ukazuje dovnitr docasne pripojeneho read-only
    /// obrazu (`/tmp/.mount_*`), ktery prepsat nejde.
    AppImage(PathBuf),
    /// Nainstalovano balickem .deb/.rpm do /usr - soubory patri spravci
    /// balicku, appka je prepisovat nesmi; GUI misto instalace nabidne
    /// stranku s novou verzi.
    Package,
}

/// Zjisti [`InstallMode`] aktualne bezici appky.
pub fn install_mode() -> InstallMode {
    if cfg!(target_os = "linux") {
        if let Some(p) = std::env::var_os("APPIMAGE") {
            if !p.is_empty() {
                return InstallMode::AppImage(PathBuf::from(p));
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            if exe.starts_with("/usr/") || exe.starts_with("/opt/") {
                return InstallMode::Package;
            }
        }
    }
    InstallMode::Portable
}

/// Cesta, pres kterou se appka spousti "zvenku" - pro zastupce v menu a
/// pro spusteni nove verze po aktualizaci. U AppImage je to samotny
/// .AppImage soubor (ne binarka uvnitr docasneho pripojeni), jinak
/// `current_exe()`.
pub fn launch_path() -> Option<PathBuf> {
    match install_mode() {
        InstallMode::AppImage(p) => Some(p),
        _ => std::env::current_exe().ok(),
    }
}

pub struct UpdateOutcome {
    pub updated: bool,
    pub version: String,
}

/// Informace o nejnovejsim GitHub Release, kdyz je novejsi nez
/// `current_version` predana do [`check_latest_version`].
pub struct LatestRelease {
    pub version: String,
    /// Odkaz na "releases" stranku repozitare (vzdy funkcni, i kdyby se
    /// presny format tagu casem zmenil - misto skladani konkretniho
    /// tagu primo).
    pub url: String,
}

/// Zkontroluje nejnovejsi GitHub Release a pripadne aktualizuje bezici
/// binarku. `current_version` se preda typicky jako
/// `env!("CARGO_PKG_VERSION")` z binarniho crate.
pub fn self_update(current_version: &str) -> Result<UpdateOutcome> {
    match install_mode() {
        InstallMode::AppImage(path) => return update_appimage(&path, current_version),
        InstallMode::Package => anyhow::bail!(
            "Term-IX je nainstalovany balickem (.deb/.rpm) - novou verzi stahni a nainstaluj ze stranky releasu"
        ),
        InstallMode::Portable => {}
    }
    let status = self_update::backends::github::Update::configure()
        .repo_owner("DaTTcz")
        .repo_name("Term-IX")
        .bin_name("term-ix")
        .show_download_progress(true)
        .no_confirm(true)
        .current_version(current_version)
        .build()
        .context("nepodarilo se pripravit self-update")?
        .update()
        .context("self-update selhal")?;

    Ok(UpdateOutcome {
        updated: status.updated(),
        version: status.version().to_string(),
    })
}

/// Jen zjisti, jestli je na GitHubu dostupna novejsi verze, nez
/// `current_version` - bez stahovani/instalace ceho koliv (na rozdil
/// od [`self_update`]). Urceno pro zobrazeni v UI (napr. "Home" tab v
/// `termx-gui`) - vraci `None`, kdyz uzivatel uz ma nejnovejsi verzi.
///
/// Bezi synchronne (blokujici sitovy pozadavek) - volajici (GUI) by
/// mel tuto funkci volat na samostatnem vlakne, ne primo v render
/// smycce.
pub fn check_latest_version(current_version: &str) -> Result<Option<LatestRelease>> {
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("DaTTcz")
        .repo_name("Term-IX")
        .build()
        .context("nepodarilo se pripravit kontrolu aktualizaci")?
        .fetch()
        .context("nepodarilo se nacist seznam GitHub releases")?;

    let Some(latest) = releases.first() else {
        // Zadny release jeste na GitHubu neexistuje - neni co porovnavat,
        // to neni chyba.
        return Ok(None);
    };

    if is_newer_version(&latest.version, current_version) {
        Ok(Some(LatestRelease {
            version: strip_v_prefix(&latest.version).to_string(),
            url: "https://github.com/DaTTcz/Term-IX/releases/latest".to_string(),
        }))
    } else {
        Ok(None)
    }
}

/// Aktualizace AppImage (stejny postup jako v eR Commanderu): stahne
/// `*.AppImage` z nejnovejsiho releasu vedle puvodniho souboru jako
/// `.AppImage.new`, nastavi spustitelnost a prejmenovanim nahradi puvodni
/// soubor. Bezici proces si drzi starou inode, takze to jde za behu; novou
/// verzi pak GUI spusti pres [`launch_path`].
///
/// Stahuje se z verejne adresy `.../releases/download/<tag>/<soubor>`, ne
/// z `ReleaseAsset::download_url` - to je GitHub API endpoint, ktery bez
/// hlavicky `Accept: application/octet-stream` vrati JSON misto souboru
/// (poucka z eR Commanderu).
fn update_appimage(appimage: &std::path::Path, current_version: &str) -> Result<UpdateOutcome> {
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner("DaTTcz")
        .repo_name("Term-IX")
        .build()
        .context("nepodarilo se pripravit kontrolu aktualizaci")?
        .fetch()
        .context("nepodarilo se nacist seznam GitHub releases")?;
    let latest = releases.first().context("na GitHubu zatim neexistuje zadny release")?;
    let version = strip_v_prefix(&latest.version).to_string();
    if !is_newer_version(&version, current_version) {
        return Ok(UpdateOutcome { updated: false, version });
    }
    let asset = latest
        .assets
        .iter()
        .find(|a| a.name.ends_with(".AppImage"))
        .context("nejnovejsi release neobsahuje AppImage")?;
    let url = format!("https://github.com/DaTTcz/Term-IX/releases/download/v{version}/{}", asset.name);

    let new_path = appimage.with_extension("AppImage.new");
    let result = (|| -> Result<()> {
        let file = std::fs::File::create(&new_path)
            .with_context(|| format!("nelze zapsat {}", new_path.display()))?;
        self_update::Download::from_url(&url)
            .download_to(&file)
            .context("stazeni nove AppImage selhalo")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&new_path, std::fs::Permissions::from_mode(0o755))
                .context("nastaveni spustitelnosti nove AppImage selhalo")?;
        }
        std::fs::rename(&new_path, appimage)
            .with_context(|| format!("nahrazeni {} selhalo", appimage.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&new_path);
    }
    result?;
    Ok(UpdateOutcome { updated: true, version })
}

fn strip_v_prefix(v: &str) -> &str {
    v.strip_prefix('v').or_else(|| v.strip_prefix('V')).unwrap_or(v)
}

/// Jednoduche (major, minor, patch) porovnani verzi - zamerne bez
/// zavislosti na `semver` crate (ktery zatim workspace nema) a bez
/// spolehani na presne API `self_update::version` (ktere zde nebylo
/// mozne overit skutecnym buildem). Nerozumi pre-release/build
/// metadata (`-beta`, `+build...`) - u tech se jednoduse porovnaji
/// jen ciselne casti pred prvni nenumerickou znackou, coz pro releasy
/// tvaru `vX.Y.Z` tohoto projektu plne stac.
fn is_newer_version(remote: &str, current: &str) -> bool {
    match (parse_version(remote), parse_version(current)) {
        (Some(r), Some(c)) => r > c,
        // Kdyz se nektera verze neda rozparsovat, radeji nic
        // nehlasit, nez uzivatele zbytecne strasit chybnou "novou"
        // verzi.
        _ => false,
    }
}

fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let v = strip_v_prefix(v.trim());
    let mut parts = v.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch_part = parts.next().unwrap_or("0");
    let patch_digits: String = patch_part.chars().take_while(|c| c.is_ascii_digit()).collect();
    let patch = if patch_digits.is_empty() { 0 } else { patch_digits.parse().ok()? };
    Some((major, minor, patch))
}
