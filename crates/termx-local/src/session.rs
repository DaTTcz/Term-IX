//! Spusteni shellu v PTY a prenos dat mezi nim a GUI - viz `lib.rs`.

use std::io::{Read, Write};
use std::sync::mpsc::{Receiver, Sender};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use termx_core::Session;

/// Udalost z bezici mistni relace smerem ke GUI - stejny tvar jako
/// `termx_serial::SerialEvent`.
pub enum LocalEvent {
    /// Vystup shellu (k parsovani ANSI sekvenci v GUI).
    Data(Vec<u8>),
    /// Shell byl spusten.
    Connected,
    /// Spusteni (nebo cteni/zapis) selhalo.
    Error(String),
    /// Shell skoncil (`exit`) nebo relace byla zavrena.
    Closed,
}

/// Prikaz od GUI smerem k bezicimu shellu.
pub enum LocalInput {
    Data(Vec<u8>),
    /// Zmena velikosti terminalu v GUI - posle se do PTY (SIGWINCH), aby
    /// `vim`, `htop`, `mc`... prekreslily obrazovku na novou velikost.
    Resize { cols: u16, rows: u16 },
}

/// Uchyt na bezici mistni relaci. Zahozenim `input_tx` (zavreni tabu) se
/// hlavni smycka ukonci a shell se ukonci (`kill`).
pub struct LocalHandle {
    pub input_tx: Sender<LocalInput>,
    pub output_rx: Receiver<LocalEvent>,
}

/// Cloveku citelny nazev vychoziho shellu (napr. "bash", "cmd.exe") - pro
/// nazev tabu/sessionu v GUI.
pub fn default_shell_label() -> String {
    let path = default_shell_path();
    std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or(path)
}

fn default_shell_path() -> String {
    #[cfg(windows)]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "/bin/sh".to_string())
    }
}

/// Spusti shell (`session.host`, prazdne = vychozi) v PTY o velikosti
/// `cols`x`rows` na vlastnich vlaknech.
pub fn spawn_local_session(session: Session, cols: u16, rows: u16) -> LocalHandle {
    let (input_tx, input_rx) = std::sync::mpsc::channel::<LocalInput>();
    let (output_tx, output_rx) = std::sync::mpsc::channel::<LocalEvent>();

    std::thread::Builder::new()
        .name("termx-local".into())
        .spawn(move || {
            if let Err(e) = run_session(&session, cols, rows, &input_rx, &output_tx) {
                let _ = output_tx.send(LocalEvent::Error(format!("{e:#}")));
            }
            let _ = output_tx.send(LocalEvent::Closed);
        })
        .expect("nepodarilo se spustit vlakno mistniho terminalu");

    LocalHandle { input_tx, output_rx }
}

fn pty_size(cols: u16, rows: u16) -> PtySize {
    PtySize { rows: rows.max(1), cols: cols.max(1), pixel_width: 0, pixel_height: 0 }
}

fn run_session(
    session: &Session,
    cols: u16,
    rows: u16,
    input_rx: &Receiver<LocalInput>,
    output_tx: &Sender<LocalEvent>,
) -> anyhow::Result<()> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(pty_size(cols, rows))
        .map_err(|e| anyhow::anyhow!("otevření pseudoterminálu (PTY) selhalo: {e:#}"))?;

    let program = session.host.trim();
    let program = if program.is_empty() { default_shell_path() } else { program.to_string() };
    let mut cmd = CommandBuilder::new(&program);
    // Stejny typ terminalu jako u SSH (`termx-ssh`, `xterm-256color`), pokud
    // uzivatel u session nenastavil jiny.
    let term = session.term_type.as_deref().map(str::trim).filter(|t| !t.is_empty()).unwrap_or("xterm-256color");
    cmd.env("TERM", term);
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TERM_PROGRAM", "Term-IX");
    if let Some(home) = home_dir() {
        cmd.cwd(home);
    }

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| anyhow::anyhow!("spuštění „{program}“ selhalo: {e:#}"))?;
    // Slave konec uz nepotrebujeme - kdyby zustal otevreny, cteni z masteru
    // by po skonceni shellu nikdy nedostalo EOF.
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(|e| anyhow::anyhow!("PTY (čtení): {e:#}"))?;
    let mut writer = pair.master.take_writer().map_err(|e| anyhow::anyhow!("PTY (zápis): {e:#}"))?;
    let mut killer = child.clone_killer();

    let _ = output_tx.send(LocalEvent::Connected);

    // Cteci vlakno - blokujici `read` na PTY; skonci s EOF/chybou, jakmile
    // shell skonci (nebo ho zabijeme nize).
    let reader_tx = output_tx.clone();
    let (exit_tx, exit_rx) = std::sync::mpsc::channel::<()>();
    let reader_thread = std::thread::Builder::new().name("termx-local-read".into()).spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if reader_tx.send(LocalEvent::Data(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                // Na Linuxu vraci cteni z masteru po skonceni shellu EIO -
                // normalni konec, ne chyba.
                Err(_) => break,
            }
        }
        let _ = exit_tx.send(());
    });

    // Hlavni smycka - vstup z GUI; konci, kdyz shell skonci (cteci vlakno)
    // nebo GUI zahodi `input_tx` (zavreni tabu).
    loop {
        if exit_rx.try_recv().is_ok() {
            break;
        }
        match input_rx.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(LocalInput::Data(bytes)) => {
                if let Err(e) = writer.write_all(&bytes).and_then(|_| writer.flush()) {
                    let _ = killer.kill();
                    return Err(anyhow::anyhow!("zápis do místního terminálu selhal: {e}"));
                }
            }
            Ok(LocalInput::Resize { cols, rows }) => {
                let _ = pair.master.resize(pty_size(cols, rows));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                // Tab zavren - ukoncit shell.
                let _ = killer.kill();
                break;
            }
        }
    }

    let status = child.wait().ok();
    drop(writer);
    drop(pair.master);
    if let Ok(t) = reader_thread {
        let _ = t.join();
    }
    tracing::debug!(?status, "mistni terminal skoncil");
    Ok(())
}

fn home_dir() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    let var = "USERPROFILE";
    #[cfg(not(windows))]
    let var = "HOME";
    std::env::var_os(var).filter(|v| !v.is_empty()).map(std::path::PathBuf::from)
}
