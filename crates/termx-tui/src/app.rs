use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::widgets::ListState;
use ratatui::Terminal;
use std::time::Duration;

use termx_core::{AuthMethod, ConnectionContext, Protocol, Session};
use termx_vault::Vault;

use crate::registry::ModuleRegistry;
use crate::ui;

/// Pole formulare pro pridani noveho serveru - MVP podporuje jen SSH
/// s prihlasenim jmenem/heslem, dalsi protokoly pribudou spolu s jejich
/// moduly.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Host,
    Port,
    Username,
    Password,
}

impl Field {
    pub const ALL: [Field; 5] = [Field::Name, Field::Host, Field::Port, Field::Username, Field::Password];

    pub fn label(&self) -> &'static str {
        match self {
            Field::Name => "Nazev",
            Field::Host => "Host",
            Field::Port => "Port",
            Field::Username => "Uzivatel",
            Field::Password => "Heslo",
        }
    }
}

pub enum Mode {
    Normal,
    AddForm { field: usize, values: [String; 5] },
    ConfirmDelete,
    Message(String),
}

pub struct App {
    vault: Vault,
    master_password: String,
    registry: ModuleRegistry,
    pub list_state: ListState,
    pub mode: Mode,
    pub should_quit: bool,
}

impl App {
    pub fn new(vault: Vault, master_password: String, registry: ModuleRegistry) -> Self {
        let mut list_state = ListState::default();
        if !vault.data.servers.is_empty() {
            list_state.select(Some(0));
        }
        Self {
            vault,
            master_password,
            registry,
            list_state,
            mode: Mode::Normal,
            should_quit: false,
        }
    }

    pub fn sessions(&self) -> &[Session] {
        &self.vault.data.servers
    }

    pub async fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
        while !self.should_quit {
            terminal.draw(|f| ui::draw(f, self))?;

            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.on_key(key.code, terminal).await?;
                    }
                }
            }
        }
        Ok(())
    }

    async fn on_key(&mut self, code: KeyCode, terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => self.on_key_normal(code, terminal).await?,
            Mode::AddForm { field, values } => self.on_key_add_form(code, field, values),
            Mode::ConfirmDelete => self.on_key_confirm_delete(code),
            Mode::Message(_) => {
                // libovolna klavesa zavre zpravu
                self.mode = Mode::Normal;
            }
        }
        Ok(())
    }

    async fn on_key_normal(&mut self, code: KeyCode, terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_prev(),
            KeyCode::Char('a') => {
                self.mode = Mode::AddForm {
                    field: 0,
                    values: Default::default(),
                };
            }
            KeyCode::Char('d') => {
                if self.list_state.selected().is_some() {
                    self.mode = Mode::ConfirmDelete;
                }
            }
            KeyCode::Enter => self.connect_selected(terminal).await?,
            _ => {}
        }
        Ok(())
    }

    fn on_key_add_form(&mut self, code: KeyCode, mut field: usize, mut values: [String; 5]) {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Backspace => {
                values[field].pop();
            }
            KeyCode::Char(c) => {
                values[field].push(c);
            }
            KeyCode::Tab | KeyCode::Down => {
                field = (field + 1) % Field::ALL.len();
            }
            KeyCode::Up => {
                field = (field + Field::ALL.len() - 1) % Field::ALL.len();
            }
            KeyCode::Enter => {
                if field + 1 < Field::ALL.len() {
                    field += 1;
                } else {
                    self.submit_add_form(&values);
                    return;
                }
            }
            _ => {}
        }
        self.mode = Mode::AddForm { field, values };
    }

    fn submit_add_form(&mut self, values: &[String; 5]) {
        let name = if values[0].trim().is_empty() { values[1].clone() } else { values[0].clone() };
        let port: u16 = values[2].trim().parse().unwrap_or(22);

        let session = Session::new(
            name,
            Protocol::Ssh,
            values[1].clone(),
            port,
            AuthMethod::Password {
                username: values[3].clone(),
                password: values[4].clone(),
            },
        );

        self.vault.data.servers.push(session);
        if self.list_state.selected().is_none() {
            self.list_state.select(Some(0));
        }
        self.mode = match self.vault.save(&self.master_password) {
            Ok(()) => Mode::Normal,
            Err(e) => Mode::Message(format!("Server pridan, ale ulozeni trezoru selhalo: {e}")),
        };
    }

    fn on_key_confirm_delete(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(i) = self.list_state.selected() {
                    if i < self.vault.data.servers.len() {
                        self.vault.data.servers.remove(i);
                        let len = self.vault.data.servers.len();
                        if len == 0 {
                            self.list_state.select(None);
                        } else if i >= len {
                            self.list_state.select(Some(len - 1));
                        }
                        if let Err(e) = self.vault.save(&self.master_password) {
                            self.mode = Mode::Message(format!("Smazano, ale ulozeni trezoru selhalo: {e}"));
                            return;
                        }
                    }
                }
                self.mode = Mode::Normal;
            }
            _ => self.mode = Mode::Normal,
        }
    }

    fn select_next(&mut self) {
        let len = self.vault.data.servers.len();
        if len == 0 {
            return;
        }
        let next = match self.list_state.selected() {
            Some(i) => (i + 1) % len,
            None => 0,
        };
        self.list_state.select(Some(next));
    }

    fn select_prev(&mut self) {
        let len = self.vault.data.servers.len();
        if len == 0 {
            return;
        }
        let prev = match self.list_state.selected() {
            Some(0) | None => len - 1,
            Some(i) => i - 1,
        };
        self.list_state.select(Some(prev));
    }

    /// Docasne opusti alternate screen / raw mode TUI, preda rizeni
    /// modulu protokolu (napr. SSH) a po navratu obnovi TUI obrazovku.
    async fn connect_selected(&mut self, terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
        let Some(i) = self.list_state.selected() else { return Ok(()) };
        let Some(session) = self.vault.data.servers.get(i).cloned() else { return Ok(()) };

        let Some(module) = self.registry.get(session.protocol.key()).cloned() else {
            self.mode = Mode::Message(format!("Protokol {} zatim nema modul", session.protocol));
            return Ok(());
        };

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

        let outcome = module.run(ConnectionContext { session: &session }).await;

        enable_raw_mode()?;
        execute!(terminal.backend_mut(), EnterAlternateScreen)?;
        terminal.clear()?;

        self.mode = match outcome {
            Ok(()) => Mode::Normal,
            Err(e) => Mode::Message(format!("Spojeni skoncilo chybou: {e}")),
        };

        Ok(())
    }
}
