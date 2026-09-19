//! Skutecne FTP/FTPS spojeni - synchronni (blokujici) `suppaftp` klient
//! bezici ve vlastnim OS vlakne, komunikujici s GUI (`termx-gui/ftp_browser.rs`)
//! pres dvojici `std::sync::mpsc` kanalu ([`FtpCommand`] dovnitr,
//! [`FtpEvent`] ven) - obdoba `termx-ssh::sftp` (stejny "prikaz/udalost"
//! tvar rozhrani), jen bez tokia, protoze `suppaftp` samo o sobe zadny
//! async runtime nevyzaduje (stejny duvod jako u `termx-serial::session`).
//!
//! Prochazeni/prenosy zamerne sdileji stejne zjednoduseni jako
//! `termx-ssh::sftp`: existujici cile pri prenosu se vzdy proste prepisi
//! (zadna kolizni detekce) a cely soubor/adresar se stahuje/nahrava
//! postupne po jednotlivych polozkach (zadne paralelni prenosy).

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;

use suppaftp::rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use suppaftp::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use suppaftp::rustls::{self, DigitallySignedStruct, SignatureScheme};
use suppaftp::{list::ListParser, RustlsConnector, RustlsFtpStream};
use termx_core::{AuthMethod, Session};

/// Jedna polozka vypisu adresare - viz `FtpEvent::Listing`.
#[derive(Debug, Clone)]
pub struct FtpEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Prikazy posilane z GUI do bezici FTP relace (viz [`spawn_ftp_session`]).
pub enum FtpCommand {
    /// Vyplni chybejici prihlasovaci udaje, kdyz `Session::auth` nemela
    /// (nebo mela chybne) uzivatelske jmeno/heslo - viz `FtpEvent::AwaitingCredentials`/`AuthFailed`.
    Credentials { username: String, password: String },
    List(String),
    Download { remote: String, local: PathBuf },
    Upload { local: PathBuf, remote: String },
    DownloadDir { remote: String, local: PathBuf },
    UploadDir { local: PathBuf, remote: String },
    Rename { from: String, to: String },
    Delete { path: String, is_dir: bool },
    Mkdir { path: String },
}

/// Udalosti posilane z bezici FTP relace zpet do GUI.
pub enum FtpEvent {
    AwaitingCredentials,
    AuthFailed(String),
    Connected { home: String },
    Error(String),
    Listing { path: String, entries: Vec<FtpEntry> },
    Downloaded { remote: String, local: PathBuf },
    Uploaded { local: PathBuf, remote: String },
    DirProgress { done: usize, total: usize },
    DirDownloaded { remote: String, local: PathBuf, count: usize },
    DirUploaded { local: PathBuf, remote: String, count: usize },
    Renamed { from: String, to: String },
    Deleted { path: String },
    Created { path: String },
    Closed,
}

pub struct FtpHandle {
    pub cmd_tx: mpsc::Sender<FtpCommand>,
    pub event_rx: mpsc::Receiver<FtpEvent>,
}

/// Spusti FTP/FTPS relaci na vlastnim OS vlakne a vrati handle s kanaly pro
/// komunikaci s ni. Nikdy nepanikari - kazda chyba (vc. selhani samotneho
/// pripojeni) se posle jako [`FtpEvent::Error`]/[`FtpEvent::AuthFailed`] +
/// nasledne [`FtpEvent::Closed`], stejne jako u `termx_serial::spawn_serial_session`.
pub fn spawn_ftp_session(session: Session) -> FtpHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel::<FtpCommand>();
    let (event_tx, event_rx) = mpsc::channel::<FtpEvent>();

    std::thread::spawn(move || {
        run_session(session, cmd_rx, event_tx);
    });

    FtpHandle { cmd_tx, event_rx }
}

fn run_session(session: Session, cmd_rx: mpsc::Receiver<FtpCommand>, event_tx: mpsc::Sender<FtpEvent>) {
    let mut stream = match connect(&session) {
        Ok(s) => s,
        Err(e) => {
            let _ = event_tx.send(FtpEvent::Error(e));
            let _ = event_tx.send(FtpEvent::Closed);
            return;
        }
    };

    let mut authenticated = false;
    if let AuthMethod::Password { username, password } = &session.auth {
        if !username.is_empty() {
            match stream.login(username.as_str(), password.as_str()) {
                Ok(()) => authenticated = true,
                Err(e) => {
                    let _ = event_tx.send(FtpEvent::AuthFailed(e.to_string()));
                }
            }
        }
    }

    if !authenticated {
        let _ = event_tx.send(FtpEvent::AwaitingCredentials);
        loop {
            match cmd_rx.recv() {
                Ok(FtpCommand::Credentials { username, password }) => match stream.login(username.as_str(), password.as_str()) {
                    Ok(()) => {
                        authenticated = true;
                        break;
                    }
                    Err(e) => {
                        let _ = event_tx.send(FtpEvent::AuthFailed(e.to_string()));
                    }
                },
                Ok(_) => { /* pred prihlasenim ostatni prikazy ignorujeme */ }
                Err(_) => {
                    let _ = event_tx.send(FtpEvent::Closed);
                    return;
                }
            }
        }
    }

    let home = stream.pwd().unwrap_or_else(|_| "/".to_string());
    let _ = event_tx.send(FtpEvent::Connected { home });

    for cmd in cmd_rx.iter() {
        match cmd {
            FtpCommand::Credentials { .. } => { /* uz prihlaseni - ignorovat */ }
            FtpCommand::List(path) => match list_dir(&mut stream, &path) {
                Ok(entries) => {
                    let _ = event_tx.send(FtpEvent::Listing { path, entries });
                }
                Err(e) => {
                    let _ = event_tx.send(FtpEvent::Error(e));
                }
            },
            FtpCommand::Download { remote, local } => match download_file(&mut stream, &remote, &local) {
                Ok(()) => {
                    let _ = event_tx.send(FtpEvent::Downloaded { remote, local });
                }
                Err(e) => {
                    let _ = event_tx.send(FtpEvent::Error(e));
                }
            },
            FtpCommand::Upload { local, remote } => match upload_file(&mut stream, &local, &remote) {
                Ok(()) => {
                    let _ = event_tx.send(FtpEvent::Uploaded { local, remote });
                }
                Err(e) => {
                    let _ = event_tx.send(FtpEvent::Error(e));
                }
            },
            FtpCommand::DownloadDir { remote, local } => match download_dir(&mut stream, &remote, &local, &event_tx) {
                Ok(count) => {
                    let _ = event_tx.send(FtpEvent::DirDownloaded { remote, local, count });
                }
                Err(e) => {
                    let _ = event_tx.send(FtpEvent::Error(e));
                }
            },
            FtpCommand::UploadDir { local, remote } => match upload_dir(&mut stream, &local, &remote, &event_tx) {
                Ok(count) => {
                    let _ = event_tx.send(FtpEvent::DirUploaded { local, remote, count });
                }
                Err(e) => {
                    let _ = event_tx.send(FtpEvent::Error(e));
                }
            },
            FtpCommand::Rename { from, to } => match stream.rename(from.as_str(), to.as_str()) {
                Ok(()) => {
                    let _ = event_tx.send(FtpEvent::Renamed { from, to });
                }
                Err(e) => {
                    let _ = event_tx.send(FtpEvent::Error(e.to_string()));
                }
            },
            FtpCommand::Delete { path, is_dir } => {
                let result = if is_dir { stream.rmdir(path.as_str()) } else { stream.rm(path.as_str()) };
                match result {
                    Ok(()) => {
                        let _ = event_tx.send(FtpEvent::Deleted { path });
                    }
                    Err(e) => {
                        let _ = event_tx.send(FtpEvent::Error(e.to_string()));
                    }
                }
            }
            FtpCommand::Mkdir { path } => match stream.mkdir(path.as_str()) {
                Ok(()) => {
                    let _ = event_tx.send(FtpEvent::Created { path });
                }
                Err(e) => {
                    let _ = event_tx.send(FtpEvent::Error(e.to_string()));
                }
            },
        }
    }

    let _ = stream.quit();
    let _ = event_tx.send(FtpEvent::Closed);
}

/// Naváže TCP spojení a - kdyz `session.ftp_use_tls` - ho hned povysi na
/// FTPS explicitnim "AUTH TLS" prikazem (`into_secure`, ne implicitni FTPS
/// na portu 990 - stejny vychozi rezim, jaky doporucuje/pouziva napr.
/// FileZilla). Pasivni rezim prenosu (`PASV`) je vychozi chovani
/// `suppaftp`, dokud se explicitne nezavola `active_mode` - zadne dalsi
/// nastaveni aktivni/pasivni v UI tedy zatim neni potreba.
fn connect(session: &Session) -> Result<RustlsFtpStream, String> {
    let addr = format!("{}:{}", session.host, session.port);
    let mut stream = RustlsFtpStream::connect(addr.as_str()).map_err(|e| e.to_string())?;

    if session.ftp_use_tls {
        // `rustls-ring` (viz Cargo.toml) vyzaduje pred prvnim pouzitim
        // `ClientConfig::builder()` nainstalovany vychozi kryptograficky
        // "provider" pro cely proces - `install_default()` vraci `Err`,
        // kdyz uz nainstalovany je (napr. z jineho drivejsiho FTP spojeni
        // behem tehoz behu aplikace), coz je v poradku a zamerne se
        // ignoruje.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let verifier: Arc<dyn ServerCertVerifier> = Arc::new(AcceptAllCertVerifier);
        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();

        stream = stream
            .into_secure(RustlsConnector::from(Arc::new(config)), &session.host)
            .map_err(|e| e.to_string())?;
    }

    Ok(stream)
}

/// Prijme JAKYKOLIV TLS certifikat serveru bez overeni (vc. samopodepsanych) -
/// stejny zamerny kompromis "duveruj pri prvnim pripojeni" jako uz drive u
/// SSH modulu (viz README.md, "Known limitation": `termx-ssh` take
/// neoveruje otisk host klice proti znamym hostitelum). FTPS servery casto
/// bezi na samopodepsanych certifikatech a bezne FTP klienty (napr.
/// FileZilla) je proto take standardne prijimaji bez varovani.
#[derive(Debug)]
struct AcceptAllCertVerifier;

impl ServerCertVerifier for AcceptAllCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA1,
            SignatureScheme::ECDSA_SHA1_Legacy,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
            SignatureScheme::ED448,
        ]
    }
}

fn parse_list_line(line: &str) -> Option<suppaftp::list::File> {
    ListParser::parse_posix(line).ok().or_else(|| ListParser::parse_dos(line).ok())
}

fn list_dir(stream: &mut RustlsFtpStream, path: &str) -> Result<Vec<FtpEntry>, String> {
    let lines = stream.list(Some(path)).map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for line in &lines {
        if let Some(file) = parse_list_line(line) {
            let name = file.name();
            if name == "." || name == ".." {
                continue;
            }
            entries.push(FtpEntry {
                name: name.to_string(),
                is_dir: file.is_directory(),
                size: file.size() as u64,
            });
        }
    }
    Ok(entries)
}

fn download_file(stream: &mut RustlsFtpStream, remote: &str, local: &Path) -> Result<(), String> {
    let mut cursor: Cursor<Vec<u8>> = stream.retr_as_buffer(remote).map_err(|e| e.to_string())?;
    if let Some(parent) = local.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut file = std::fs::File::create(local).map_err(|e| e.to_string())?;
    std::io::copy(&mut cursor, &mut file).map_err(|e| e.to_string())?;
    Ok(())
}

fn upload_file(stream: &mut RustlsFtpStream, local: &Path, remote: &str) -> Result<(), String> {
    let mut file = std::fs::File::open(local).map_err(|e| e.to_string())?;
    stream.put_file(remote, &mut file).map_err(|e| e.to_string())?;
    Ok(())
}

/// Slozi lokalni cestu ze "/"-oddeleneho relativniho retezce (`rel`) -
/// pouzivane pro polozky nasbirane pomoci [`collect_remote_files`]/
/// [`collect_local_files`] nize, aby fungovalo shodne na Linuxu i Windows
/// (FTP cesty vzdy pouzivaji "/", bez ohledu na OS klienta).
fn local_path_for(local_root: &Path, rel: &str) -> PathBuf {
    if rel.is_empty() {
        return local_root.to_path_buf();
    }
    let mut p = local_root.to_path_buf();
    for seg in rel.split('/') {
        p.push(seg);
    }
    p
}

fn remote_path_for(remote_root: &str, rel: &str) -> String {
    if rel.is_empty() {
        remote_root.to_string()
    } else {
        format!("{}/{}", remote_root.trim_end_matches('/'), rel)
    }
}

/// Iterativne (vlastni zasobnik, ne rekurzivni funkce) projde cely
/// vzdaleny podstrom pod `remote_root` a vrati vsechny nalezene polozky
/// jako dvojice (cesta relativni k `remote_root` oddelena "/", je_slozka).
/// Slozka se v seznamu vzdy objevi driv nez jeji obsah - dulezite pro
/// [`download_dir`], ktery podle tohoto poradi lokalni slozky vytvari.
fn collect_remote_files(stream: &mut RustlsFtpStream, remote_root: &str) -> Result<Vec<(String, bool)>, String> {
    let mut result = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(rel) = stack.pop() {
        let remote_path = remote_path_for(remote_root, &rel);
        let lines = stream.list(Some(&remote_path)).map_err(|e| e.to_string())?;
        for line in &lines {
            let Some(file) = parse_list_line(line) else { continue };
            let name = file.name();
            if name == "." || name == ".." {
                continue;
            }
            let child_rel = if rel.is_empty() { name.to_string() } else { format!("{}/{}", rel, name) };
            if file.is_directory() {
                result.push((child_rel.clone(), true));
                stack.push(child_rel);
            } else {
                result.push((child_rel, false));
            }
        }
    }
    Ok(result)
}

/// Obdoba [`collect_remote_files`], jen pro lokalni slozku (`std::fs::read_dir`
/// misto FTP `LIST`).
fn collect_local_files(local_root: &Path) -> Result<Vec<(String, bool)>, String> {
    let mut result = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(rel) = stack.pop() {
        let dir_path = local_path_for(local_root, &rel);
        let read_dir = std::fs::read_dir(&dir_path).map_err(|e| e.to_string())?;
        for entry in read_dir {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().to_string();
            let child_rel = if rel.is_empty() { name.clone() } else { format!("{}/{}", rel, name) };
            let file_type = entry.file_type().map_err(|e| e.to_string())?;
            if file_type.is_dir() {
                result.push((child_rel.clone(), true));
                stack.push(child_rel);
            } else if file_type.is_file() {
                result.push((child_rel, false));
            }
        }
    }
    Ok(result)
}

fn download_dir(stream: &mut RustlsFtpStream, remote_root: &str, local_root: &Path, event_tx: &mpsc::Sender<FtpEvent>) -> Result<usize, String> {
    let entries = collect_remote_files(stream, remote_root)?;
    let total = entries.len();
    let mut done = 0usize;
    for (rel, is_dir) in &entries {
        let local_path = local_path_for(local_root, rel);
        if *is_dir {
            std::fs::create_dir_all(&local_path).map_err(|e| e.to_string())?;
        } else {
            let remote_path = remote_path_for(remote_root, rel);
            download_file(stream, &remote_path, &local_path)?;
        }
        done += 1;
        let _ = event_tx.send(FtpEvent::DirProgress { done, total });
    }
    Ok(total)
}

fn upload_dir(stream: &mut RustlsFtpStream, local_root: &Path, remote_root: &str, event_tx: &mpsc::Sender<FtpEvent>) -> Result<usize, String> {
    // Cilova slozka na serveru uz existovat muze - chyba se zamerne
    // ignoruje (stejne zjednoduseni "existujici cile se vzdy prepisuji"
    // jako u `termx-ssh::sftp::upload_dir`).
    let _ = stream.mkdir(remote_root);

    let entries = collect_local_files(local_root)?;
    let total = entries.len();
    let mut done = 0usize;
    for (rel, is_dir) in &entries {
        let remote_path = remote_path_for(remote_root, rel);
        if *is_dir {
            let _ = stream.mkdir(remote_path.as_str());
        } else {
            let local_path = local_path_for(local_root, rel);
            upload_file(stream, &local_path, &remote_path)?;
        }
        done += 1;
        let _ = event_tx.send(FtpEvent::DirProgress { done, total });
    }
    Ok(total)
}
