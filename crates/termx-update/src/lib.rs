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
