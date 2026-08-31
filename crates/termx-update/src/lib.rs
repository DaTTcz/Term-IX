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

use anyhow::{Context, Result};

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
