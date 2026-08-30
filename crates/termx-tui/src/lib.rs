//! termx-tui
//!
//! Terminalove uzivatelske rozhrani postavene na `ratatui` + `crossterm`.
//! Nezavisi na konkretnim protokolu (SSH/Serial/FTP/...) - pracuje jen
//! s [`termx_core::ProtocolModule`] pres [`ModuleRegistry`], takze pridani
//! dalsiho protokolu do aplikace nevyzaduje zmenu tohoto crate.

mod app;
mod registry;
mod ui;

pub use registry::ModuleRegistry;

use anyhow::Result;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use termx_vault::Vault;

use app::App;

/// Spusti hlavni smycku TUI. Vraci se az kdyz uzivatel aplikaci ukonci.
///
/// `master_password` je drzeno v pameti jen po dobu behu (potrebne pro
/// opetovne zasifrovani trezoru po kazde zmene) - viz komentar u
/// [`termx_vault::Vault`].
pub async fn run_app(vault: Vault, master_password: String, registry: ModuleRegistry) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(vault, master_password, registry);
    let result = app.run(&mut terminal).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}
