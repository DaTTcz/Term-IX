use std::path::PathBuf;

use directories::ProjectDirs;

use crate::CoreError;

/// Cross-platform cesty aplikace (Windows + Linux).
///
/// Windows: `%APPDATA%\DaTTcz\Term-IX\...`
/// Linux:   `~/.config/term-ix/...` (resp. `~/.local/share/term-ix` pro data)
pub struct AppPaths {
    dirs: ProjectDirs,
}

impl AppPaths {
    pub fn new() -> std::result::Result<Self, CoreError> {
        let dirs = ProjectDirs::from("cz", "DaTTcz", "Term-IX")
            .ok_or_else(|| CoreError::Config("nepodarilo se zjistit domovsky adresar".into()))?;
        Ok(Self { dirs })
    }

    pub fn config_dir(&self) -> &std::path::Path {
        self.dirs.config_dir()
    }

    pub fn data_dir(&self) -> &std::path::Path {
        self.dirs.data_dir()
    }

    /// Vychozi umisteni sifrovaneho trezoru ulozenych serveru.
    pub fn vault_path(&self) -> PathBuf {
        self.data_dir().join("vault.termx")
    }

    /// Zajisti, ze potrebne adresare existuji.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.config_dir())?;
        std::fs::create_dir_all(self.data_dir())?;
        Ok(())
    }
}
