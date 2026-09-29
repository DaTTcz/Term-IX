//! Skutecne RDP spojeni (IronRDP - blokujici, primo nad `std::net::TcpStream`) -
//! stejny vzor jako `termx-ftp::ftp`/`termx-serial`: vlastni OS vlakno +
//! `std::sync::mpsc` kanaly (`RdpCommand` smerem dovnitr, `RdpEvent` ven)
//! misto async/tokio.
//!
//! VERZE V3 (2026-09-29, obnova po prechodu na openSUSE Slowroll - V2 se
//! nikdy necommitla a zdrojaky se skladaly zpet z chatu). Oproti V2 se
//! V3 opira primo o OFICIALNIHO klienta `ironrdp-client` (soubor
//! `crates/ironrdp-client/src/rdp.rs`) ve STEJNEM vydani IronRDP, jake
//! pouzivame (souhrnny tag `ironrdp-v0.17.0` = `ironrdp-session-v0.11.0`
//! = `ironrdp-connector-v0.10.0`, vse jeden commit `11a0810c`). Hlavni
//! zmeny oproti V2:
//!
//! 1. REAKTIVACE ("Deactivation-Reactivation Sequence", MS-RDPBCGR
//!    1.3.1.3, spousti ji server po zmene rozliseni) - presne podle
//!    oficialniho klienta: po dokonceni se krome `share_id`/
//!    `enable_server_pointer` NOVE vytvori i FastPath procesor
//!    (`ActiveStage::set_fastpath_processor`) a `DecodedImage` - V2 tohle
//!    vynechavala, FastPath procesor tak dal zpracovaval obraz se STARYM
//!    `share_id`. Vstupem je `ConnectionResult::activation_factory` (v
//!    nasi verzi uz existuje, V2 si `Config` klonovala rucne). Reaktivace
//!    se spousti OBEMA cestami: (a) standardne po
//!    `ActiveStageOutput::DeactivateAll` (tak to dela oficialni klient),
//!    (b) zaloha z V2 - kdyz `active_stage.process()` selze na X.224 PDU
//!    (server poslal rovnou "Demand Active" bez "Deactivate All"), zkusi
//!    se tahle uz prectena PDU pouzit jako PRVNI vstup reaktivace.
//! 2. ZMENA ROZLISENI se posila az ve chvili, kdy server Display Control
//!    kanal SKUTECNE potvrdil (`DisplayControlClient::ready()` - server
//!    poslal "capabilities"). V2 posilala hned a pozadavek pred potvrzenim
//!    server tise zahodil (pravdepodobna pricina "pri maximalizaci se
//!    rozliseni nezmenilo"). Posledni pozadavek se ted drzi v
//!    `pending_resize` a odesle se automaticky, jakmile je kanal pripraven.
//! 3. OBRAZ se uz neposila jako cely `Vec<u8>` kanalem pri KAZDE PDU (pri
//!    1920x1080 = 8 MB kopie mnohokrat za sekundu), ale kopiruji se jen
//!    zmenene obdelniky do sdileneho bufferu [`SharedFrame`]
//!    (`Arc<Mutex<..>>`), GUI si z nej bere jen "spinavou" oblast
//!    (`TextureHandle::set_partial`) a o nove zmene se dozvi pres
//!    [`RdpWaker`] (primo `request_repaint_of` okna plochy).
//! 4. Vstup (mys/klavesnice) jde pres `ActiveStage::process_fastpath_input`
//!    (stejne jako oficialni klient) misto rucniho skladani `FastPathInput`.
//! 5. Bulk komprese se NEVYJEDNAVA (`compression_type: None`) - pri
//!    reaktivaci se FastPath procesor vytvari znovu a verejne API nedovoli
//!    predat mu novy dekompresor (oficialni klient predava `None` taky);
//!    obrazova data jsou navic komprimovana vlastnimi kodeky (RDP6/RFX...),
//!    bulk komprese by prinesla malo.
//!
//! POSTUP PRIPOJENI (viz `connect`) vychazi z oficialniho prikladu
//! `ironrdp/examples/screenshot.rs`: `connect_begin` (X.224/MCS
//! vyjednani), rucni TLS handshake (`tls_upgrade`, vlastni
//! `rustls::ClientConnection` - stejny "duveruj pri prvnim pripojeni"
//! pristup jako `termx-ftp`/`termx-ssh`, viz `AcceptAllCertVerifier`) a
//! `connect_finalize` (CredSSP/NLA prihlaseni).
//!
//! CredSSP/NLA pocita s moznosti Kerberos vymeny pres sit
//! (`sspi::network_client::NetworkClient`) - tenhle modul ji ZAMERNE
//! NEPODPORUJE (`NoNetworkClient::send` vzdy vrati chybu). Bezne
//! pripojeni jmenem+heslem (NTLM) tuto cestu vubec nevyvola.
//!
//! LADENI: `RUST_LOG=termx_rdp=debug,ironrdp=debug term-ix` vypise prubeh
//! pripojeni, reaktivace i zmen rozliseni.

use std::io::{self, Write as _};
use std::net::TcpStream;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ironrdp_connector::connection_activation::{ConnectionActivationFactory, ConnectionActivationState};
use ironrdp_connector::sspi::network_client::NetworkClient;
// `NetworkRequest` NENI verejna v `sspi::network_client` - verejna cesta je
// koren crate (`pub use generator::NetworkRequest;`), zjisteno pri buildu V1.
use ironrdp_connector::sspi::NetworkRequest;
use ironrdp_connector::{ClientConnector, Config, ConnectionResult, Credentials, DesktopSize, Sequence as _};
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_displaycontrol::pdu::MonitorLayoutEntry;
use ironrdp_dvc::DrdynvcClient;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_input::{Database, MouseButton as InputMouseButton, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp_pdu::gcc::KeyboardType;
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_pdu::input::fast_path::FastPathInputEvent;
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{fast_path, ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use tracing::{debug, info, warn};

/// TCP + TLS proud po dokoncenem "upgrade" kroku (`connect`).
type UpgradedFramed = ironrdp_blocking::Framed<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>;

/// Cteci timeout socketu v aktivni fazi - `framed.read_pdu()` se diky nemu
/// pravidelne vraci a smycka muze odbavit vstup z GUI (viz `run_session`).
const READ_TIMEOUT: Duration = Duration::from_millis(20);

/// Parametry pripojeni - viz [`spawn_rdp_session`].
#[derive(Clone)]
pub struct RdpConnectParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub domain: Option<String>,
    /// Pozadovany POCATECNI rozmer plochy (px) - typicky aktualni velikost
    /// okna plochy v GUI, aby server rovnou nabidl sedici rozliseni a
    /// nemusela hned nasledovat zmena pres Display Control kanal.
    pub initial_width: u16,
    pub initial_height: u16,
}

/// Prikaz smerem DO bezici RDP relace.
pub enum RdpCommand {
    /// Absolutni pozice kurzoru v pixelech plochy SERVERU (prevod z
    /// lokalniho okna dela `termx-gui::rdp_viewer`).
    MouseMove { x: u16, y: u16 },
    MouseButton { button: RdpMouseButton, pressed: bool },
    /// Otoceni kolecka - kladne = nahoru/doprava, 120 = jeden "zub".
    MouseWheel { vertical: bool, rotation_units: i16 },
    /// PC/AT scancode (Set 1) - viz `ironrdp_input::Scancode::from_u8`.
    /// Opakovane `pressed: true` bez uvolneni = autorepeat (server si ho
    /// sam negeneruje, posila ho klient - stejne jako mstsc).
    Key { extended: bool, code: u8, pressed: bool },
    /// Znak jako Unicode udalost (TS_FP_UNICODE_KEYBOARD_EVENT) - server ho
    /// NEPREKLADA svym rozlozenim klavesnice, dorazi presne tento znak.
    /// `termx-gui::rdp_viewer` tak posila psany text (vc. ceskych znaku a
    /// AltGr kombinaci podle MISTNIHO rozlozeni), scancody jen pro
    /// neznakove klavesy a zkratky.
    Unicode { ch: char, pressed: bool },
    /// Uvolni vsechny drzene klavesy a tlacitka mysi (okno plochy ztratilo
    /// fokus - jinak by napr. Alt z Alt+Tab zustal na serveru "viset").
    ReleaseAll,
    /// Pozadavek na zivou zmenu rozliseni (Display Control kanal,
    /// MS-RDPEDISP). `scale_percent` = meritko UI na strane serveru
    /// (100-500, `None` = nechat na serveru).
    ResizeDesktop { width: u16, height: u16, scale_percent: Option<u32> },
    /// Cisty konec relace zpusobeny uzivatelem - posle se jen
    /// `RdpEvent::Disconnected`, zadna chyba.
    Disconnect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RdpMouseButton {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

/// Udalost smerem Z bezici RDP relace do GUI. Obraz samotny chodi
/// sdilenym bufferem [`SharedFrame`], ne touto cestou.
pub enum RdpEvent {
    /// Spojeni navazano - `width`/`height` je serverem prideleny rozmer.
    Connected { width: u16, height: u16 },
    /// Rozmer plochy se zmenil (dokoncena reaktivace po zmene rozliseni).
    Resized { width: u16, height: u16 },
    /// Chyba pri navazovani NEBO za behu (nasleduje vzdy `Disconnected`).
    Error(String),
    Disconnected,
}

/// Obdelnik (vcetne okraju, stejne jako `InclusiveRectangle`) oblasti
/// `SharedFrame::rgba`, ktera se od posledniho prevzeti GUI zmenila.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirtyRect {
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
}

impl DirtyRect {
    fn union(self, other: DirtyRect) -> DirtyRect {
        DirtyRect {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
}

/// Sdileny framebuffer mezi RDP vlaknem a GUI (viz bod 3 v header
/// komentari). `rgba` ma vzdy presne `width * height * 4` bajtu.
#[derive(Default)]
pub struct SharedFrame {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
    /// Zmenena oblast od posledniho `take` v GUI; `None` = nic noveho.
    pub dirty: Option<DirtyRect>,
    /// Zvysuje se pri kazde zmene ROZMERU - GUI podle nej pozna, ze musi
    /// texturu vytvorit znovu v nove velikosti (a nahrat celou).
    pub size_generation: u64,
}

impl SharedFrame {
    fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        self.rgba = vec![0; usize::from(width) * usize::from(height) * 4];
        self.dirty = None;
        self.size_generation += 1;
    }

    /// Zkopiruje obdelnik z `image` (stejny rozmer) a oznaci ho jako zmeneny.
    fn copy_rect(&mut self, image: &DecodedImage, rect: DirtyRect) {
        if image.width() != self.width || image.height() != self.height {
            self.resize(image.width(), image.height());
        }
        if self.width == 0 || self.height == 0 {
            return;
        }
        let right = rect.right.min(self.width - 1);
        let bottom = rect.bottom.min(self.height - 1);
        if rect.left > right || rect.top > bottom {
            return;
        }
        let stride = usize::from(self.width) * 4;
        let x0 = usize::from(rect.left) * 4;
        let x1 = (usize::from(right) + 1) * 4;
        let src = image.data();
        for y in usize::from(rect.top)..=usize::from(bottom) {
            let row = y * stride;
            self.rgba[row + x0..row + x1].copy_from_slice(&src[row + x0..row + x1]);
        }
        let rect = DirtyRect { left: rect.left, top: rect.top, right, bottom };
        self.dirty = Some(match self.dirty {
            Some(d) => d.union(rect),
            None => rect,
        });
    }

    fn copy_all(&mut self, image: &DecodedImage) {
        if image.width() == 0 || image.height() == 0 {
            return;
        }
        self.copy_rect(image, DirtyRect { left: 0, top: 0, right: image.width() - 1, bottom: image.height() - 1 });
    }
}

/// Duvod probuzeni GUI - viz [`RdpWaker`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RdpWake {
    /// V `SharedFrame` je nova zmenena oblast - staci prekreslit okno plochy.
    Frame,
    /// V `event_rx` ceka nova `RdpEvent` - prekreslit hlavni okno (stavova karta).
    Event,
}

/// Zpetne volani, kterym RDP vlakno "probudi" GUI (egui jinak prekresluje
/// jen pri vstupu od uzivatele). GUI ho implementuje pres
/// `egui::Context::request_repaint_of`.
pub type RdpWaker = Arc<dyn Fn(RdpWake) + Send + Sync>;

pub struct RdpHandle {
    pub command_tx: Sender<RdpCommand>,
    pub event_rx: Receiver<RdpEvent>,
    pub frame: Arc<Mutex<SharedFrame>>,
}

/// Naveze RDP spojeni na vlastnim vlakne a vrati kanaly pro komunikaci s
/// nim - stejny vzor jako `termx_ftp::spawn_ftp_session`/`termx_serial`.
pub fn spawn_rdp_session(params: RdpConnectParams, waker: RdpWaker) -> RdpHandle {
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let frame = Arc::new(Mutex::new(SharedFrame::default()));
    let frame_thread = frame.clone();
    std::thread::Builder::new()
        .name("termx-rdp".into())
        .spawn(move || {
            let events = EventSink { tx: event_tx, waker: waker.clone() };
            let outcome = run_session(&params, &command_rx, &events, &frame_thread, &waker);
            if let Err(e) = outcome {
                warn!("RDP relace skoncila chybou: {e:#}");
                events.send(RdpEvent::Error(format!("{e:#}")));
            }
            events.send(RdpEvent::Disconnected);
        })
        .expect("nepodarilo se spustit vlakno RDP relace");
    RdpHandle { command_tx, event_rx, frame }
}

/// `Sender<RdpEvent>` + automaticke probuzeni GUI.
struct EventSink {
    tx: Sender<RdpEvent>,
    waker: RdpWaker,
}

impl EventSink {
    fn send(&self, event: RdpEvent) {
        let _ = self.tx.send(event);
        (self.waker)(RdpWake::Event);
    }
}

fn run_session(
    params: &RdpConnectParams,
    command_rx: &Receiver<RdpCommand>,
    events: &EventSink,
    frame: &Mutex<SharedFrame>,
    waker: &RdpWaker,
) -> anyhow::Result<()> {
    let (connection_result, framed, timeout_handle) = connect(params)?;
    // Cteci timeout se zapina AZ TED - cely handshake bezel na cisto
    // blokujicim socketu (IronRDP behem nej `WouldBlock`/`TimedOut` sam
    // neopakuje). `timeout_handle` je `try_clone` stejneho OS socketu.
    timeout_handle.set_read_timeout(Some(READ_TIMEOUT)).ok();
    let size = connection_result.desktop_size;
    info!(width = size.width, height = size.height, "RDP spojeni navazano");
    frame.lock().unwrap().resize(size.width, size.height);
    events.send(RdpEvent::Connected { width: size.width, height: size.height });
    active_stage_loop(connection_result, framed, command_rx, events, frame, waker)
}

/// Navaze TCP+TLS+RDP (vc. CredSSP/NLA) spojeni.
fn connect(params: &RdpConnectParams) -> anyhow::Result<(ConnectionResult, UpgradedFramed, TcpStream)> {
    use std::net::ToSocketAddrs as _;
    let host = params.host.as_str();
    let port = params.port;
    let server_addr = (host, port)
        .to_socket_addrs()
        .map_err(|e| anyhow::anyhow!("neplatná adresa {host}:{port}: {e}"))?
        .next()
        .ok_or_else(|| anyhow::anyhow!("adresu {host}:{port} se nepodařilo přeložit"))?;

    let tcp_stream = TcpStream::connect_timeout(&server_addr, Duration::from_secs(15))
        .map_err(|e| anyhow::anyhow!("TCP spojení na {server_addr} selhalo: {e}"))?;
    tcp_stream.set_nodelay(true).ok();
    // Cteci timeout se ZAMERNE nenastavuje tady (viz `run_session`);
    // `try_clone` = druhy handle na TENTYZ OS socket, pres ktery se pak
    // timeout zapne az po handshake.
    let timeout_handle = tcp_stream.try_clone().map_err(|e| anyhow::anyhow!("duplikace TCP socketu selhala: {e}"))?;
    let client_addr = tcp_stream.local_addr().map_err(|e| anyhow::anyhow!("lokální adresa socketu: {e}"))?;

    let config = build_config(params);

    // `RetryingStream` - behem handshake se (overeno v praxi, KROK 3 v
    // historii) obcas objevi EAGAIN primo z OS, ktery IronRDP neopakuje a
    // spadl by na nem cely handshake. Obal ho tise opakuje; po handshake
    // se zase vybali (viz konec funkce), aktivni smycka `WouldBlock`/
    // `TimedOut` vyuziva zamerne.
    let mut framed = ironrdp_blocking::Framed::new(RetryingStream::new(tcp_stream));

    // Display Control DVC se MUSI zaregistrovat (jako staticky kanal
    // "drdynvc") uz pred `connect_begin` - `ActiveStage::encode_resize` si
    // ho pak sam najde v `static_channels`. Stejne jako oficialni klient.
    let mut connector = ClientConnector::new(config, client_addr)
        .with_static_channel(DrdynvcClient::new().with_dynamic_channel(DisplayControlClient::new(|caps| {
            debug!(?caps, "Display Control kanal pripraven");
            Ok(Vec::new())
        })));

    // `{e:?}` misto `{e}` - Display u `ConnectorError`/`SessionError` casto
    // vypise jen "custom error", Debug obsahuje cely retezec kontextu.
    let should_upgrade = ironrdp_blocking::connect_begin(&mut framed, &mut connector)
        .map_err(|e| anyhow::anyhow!("navázání RDP spojení selhalo: {e:?}"))?;

    let initial_stream = framed.into_inner_no_leftover();
    let (upgraded_stream, server_public_key) = tls_upgrade(initial_stream, host)?;
    let upgraded = ironrdp_blocking::mark_as_upgraded(should_upgrade, &mut connector);
    let mut upgraded_framed = ironrdp_blocking::Framed::new(upgraded_stream);

    let mut network_client = NoNetworkClient;
    let connection_result = ironrdp_blocking::connect_finalize(
        upgraded,
        connector,
        &mut upgraded_framed,
        &mut network_client,
        host.to_string().into(),
        server_public_key,
        None,
    )
    .map_err(|e| anyhow::anyhow!("dokončení RDP přihlášení selhalo: {e:?}"))?;

    // Vybaleni `RetryingStream` zpet na cistou `TcpStream`
    // (`rustls::StreamOwned` ma verejna pole `conn`/`sock`).
    let (rustls_stream, leftover) = upgraded_framed.into_inner();
    let rustls::StreamOwned { conn, sock: retrying_stream } = rustls_stream;
    let plain_stream = rustls::StreamOwned { conn, sock: retrying_stream.into_inner() };
    let final_framed = ironrdp_blocking::Framed::new_with_leftover(plain_stream, leftover);

    Ok((connection_result, final_framed, timeout_handle))
}

/// Konfigurace RDP klienta - hodnoty z oficialniho `screenshot.rs`
/// prikladu, az na komentovana pole.
fn build_config(params: &RdpConnectParams) -> Config {
    let (width, height) = MonitorLayoutEntry::adjust_display_size(u32::from(params.initial_width), u32::from(params.initial_height));
    Config {
        credentials: Credentials::UsernamePassword { username: params.username.clone(), password: params.password.clone() },
        domain: params.domain.clone(),
        enable_tls: false,
        enable_credssp: true,
        keyboard_type: KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        // 0 = rozlozeni klavesnice urci server (posilame scancody, text si
        // prelozi podle vlastniho rozlozeni uctu - stejne jako mstsc).
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: DesktopSize {
            width: u16::try_from(width).unwrap_or(1280),
            height: u16::try_from(height).unwrap_or(800),
        },
        bitmap: None,
        client_build: 0,
        client_name: "term-ix".to_owned(),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),
        #[cfg(target_os = "windows")]
        platform: MajorPlatformType::WINDOWS,
        #[cfg(target_os = "macos")]
        platform: MajorPlatformType::MACINTOSH,
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        platform: MajorPlatformType::UNIX,
        // Kurzor kresli lokalni OS (okamzita odezva i pres VPN) - tvar
        // kurzoru serveru (I-beam, sipky pro zmenu velikosti...) se v teto
        // verzi neprenasi.
        enable_server_pointer: false,
        pointer_software_rendering: true,
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        // Viz bod 5 v header komentari.
        compression_type: None,
        multitransport_flags: None,
        performance_flags: PerformanceFlags::default(),
        desktop_scale_factor: 0,
        hardware_id: None,
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
    }
}

/// Obal nad `Read + Write` proudem pouzity POUZE behem handshake -
/// vnitrne opakuje cteni/zapis pri `WouldBlock`/`Interrupted` (viz
/// `connect`).
struct RetryingStream<S> {
    inner: S,
}

impl<S> RetryingStream<S> {
    fn new(inner: S) -> Self {
        Self { inner }
    }

    fn into_inner(self) -> S {
        self.inner
    }
}

fn is_retryable(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted)
}

impl<S: io::Read> io::Read for RetryingStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.inner.read(buf) {
                Err(e) if is_retryable(&e) => std::thread::sleep(Duration::from_millis(5)),
                other => return other,
            }
        }
    }
}

impl<S: io::Write> io::Write for RetryingStream<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        loop {
            match self.inner.write(buf) {
                Err(e) if is_retryable(&e) => std::thread::sleep(Duration::from_millis(5)),
                other => return other,
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        loop {
            match self.inner.flush() {
                Err(e) if is_retryable(&e) => std::thread::sleep(Duration::from_millis(5)),
                other => return other,
            }
        }
    }
}

/// Rucni TLS handshake + extrakce verejneho klice certifikatu (potreba pro
/// CredSSP "channel binding" v `connect_finalize`).
fn tls_upgrade<S: io::Read + io::Write>(stream: S, host: &str) -> anyhow::Result<(rustls::StreamOwned<rustls::ClientConnection, S>, Vec<u8>)> {
    // Explicitni `ring` provider (nezavisle na tom, jestli nekdo nainstaloval
    // procesovy vychozi).
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| anyhow::anyhow!("TLS konfigurace selhala: {e}"))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAllCertVerifier))
        .with_no_client_auth();
    // CredSSP nepodporuje TLS session resumption (MS-CSSP).
    config.resumption = rustls::client::Resumption::disabled();
    let config = Arc::new(config);

    let server_name: rustls::pki_types::ServerName<'static> =
        host.to_string().try_into().map_err(|e| anyhow::anyhow!("neplatný název serveru pro TLS: {e:?}"))?;

    let client = rustls::ClientConnection::new(config, server_name).map_err(|e| anyhow::anyhow!("TLS handshake selhal: {e}"))?;
    let mut tls_stream = rustls::StreamOwned::new(client, stream);
    // Flush dotlaci handshake, aby uz byl certifikat serveru k dispozici.
    tls_stream.flush().map_err(|e| anyhow::anyhow!("TLS handshake selhal: {e}"))?;

    let cert = tls_stream
        .conn
        .peer_certificates()
        .and_then(|certs| certs.first())
        .ok_or_else(|| anyhow::anyhow!("server neposlal TLS certifikát"))?;
    let server_public_key = extract_public_key(cert)?;

    Ok((tls_stream, server_public_key))
}

fn extract_public_key(cert: &[u8]) -> anyhow::Result<Vec<u8>> {
    use x509_cert::der::Decode as _;
    let cert = x509_cert::Certificate::from_der(cert).map_err(|e| anyhow::anyhow!("neplatný TLS certifikát: {e}"))?;
    cert.tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .map(|b| b.to_owned())
        .ok_or_else(|| anyhow::anyhow!("veřejný klíč certifikátu není zarovnaný na bajty"))
}

/// Stejny bezpecnostni model jako `termx_ftp`/`termx-ssh` - viz README
/// "Known limitation": zadne overovani proti CA.
#[derive(Debug)]
struct AcceptAllCertVerifier;

impl rustls::client::danger::ServerCertVerifier for AcceptAllCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        // Sirsi seznam vc. SHA-1 - starsi Windows Server jeste SHA-1 podpisy
        // pouzivaji.
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA1,
            rustls::SignatureScheme::ECDSA_SHA1_Legacy,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

/// Kerberos (sitovy pristup behem CredSSP) zamerne nepodporovan.
struct NoNetworkClient;

impl NetworkClient for NoNetworkClient {
    fn send(&self, _request: &NetworkRequest) -> std::result::Result<Vec<u8>, ironrdp_connector::sspi::Error> {
        Err(ironrdp_connector::sspi::Error::new(
            ironrdp_connector::sspi::ErrorKind::OperationNotSupported,
            "síťový přístup (Kerberos) není v tomto RDP modulu podporován",
        ))
    }
}

/// Posledni pozadovana zmena rozliseni, ktera jeste nebyla odeslana.
#[derive(Clone, Copy)]
struct PendingResize {
    width: u32,
    height: u32,
    scale_percent: Option<u32>,
}

/// Stav aktivni faze relace.
struct Session<'a> {
    framed: UpgradedFramed,
    active_stage: ActiveStage,
    image: DecodedImage,
    input_db: Database,
    activation_factory: ConnectionActivationFactory,
    pending_resize: Option<PendingResize>,
    events: &'a EventSink,
    frame: &'a Mutex<SharedFrame>,
    waker: &'a RdpWaker,
}

fn active_stage_loop(
    connection_result: ConnectionResult,
    framed: UpgradedFramed,
    command_rx: &Receiver<RdpCommand>,
    events: &EventSink,
    frame: &Mutex<SharedFrame>,
    waker: &RdpWaker,
) -> anyhow::Result<()> {
    let image = DecodedImage::new(
        PixelFormat::RgbA32,
        connection_result.desktop_size.width,
        connection_result.desktop_size.height,
    );
    let active_stage = ActiveStageBuilder {
        static_channels: connection_result.static_channels,
        user_channel_id: connection_result.user_channel_id,
        io_channel_id: connection_result.io_channel_id,
        message_channel_id: connection_result.message_channel_id,
        share_id: connection_result.share_id,
        compression_type: connection_result.compression_type,
        enable_server_pointer: connection_result.enable_server_pointer,
        pointer_software_rendering: connection_result.pointer_software_rendering,
    }
    .build();

    let mut s = Session {
        framed,
        active_stage,
        image,
        input_db: Database::new(),
        activation_factory: connection_result.activation_factory,
        pending_resize: None,
        events,
        frame,
        waker,
    };

    loop {
        // (a) vstup z GUI
        let mut input_ops: Vec<Operation> = Vec::new();
        loop {
            match command_rx.try_recv() {
                Ok(RdpCommand::Disconnect) => {
                    info!("RDP relace ukoncena uzivatelem");
                    return Ok(());
                }
                Ok(RdpCommand::ResizeDesktop { width, height, scale_percent }) => {
                    let (width, height) = MonitorLayoutEntry::adjust_display_size(u32::from(width), u32::from(height));
                    s.pending_resize = Some(PendingResize { width, height, scale_percent });
                }
                Ok(RdpCommand::ReleaseAll) => {
                    s.send_input_ops(std::mem::take(&mut input_ops))?;
                    let release: Vec<FastPathInputEvent> = s.input_db.release_all().into_iter().collect();
                    s.send_input_events(&release)?;
                }
                Ok(cmd) => {
                    if let Some(op) = command_to_operation(cmd) {
                        input_ops.push(op);
                    }
                }
                Err(TryRecvError::Empty) => break,
                // GUI strana (`RdpHandle`) zanikla (tab zavren) - cisty konec.
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        s.send_input_ops(input_ops)?;

        // (b) cekajici zmena rozliseni, jakmile je Display Control pripraven
        s.try_send_pending_resize()?;

        // (c) data ze serveru
        let (action, payload) = match s.framed.read_pdu() {
            Ok(v) => v,
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => continue,
            Err(e) => return Err(anyhow::anyhow!("čtení z RDP spojení selhalo: {e}")),
        };

        let outputs = match s.active_stage.process(&mut s.image, action, &payload) {
            Ok(outputs) => outputs,
            Err(process_err) => {
                // Zaloha z V2 (viz bod 1 v header komentari): nektere servery
                // (Daviduv Windows) poslou po zmene rozliseni rovnou "Server
                // Demand Active" BEZ predchoziho "Deactivate All" - to
                // `decode_io_channel` strukturalne odmitne. Zkusime tu uz
                // prectenou PDU pouzit jako prvni krok reaktivace; kdyz to
                // nevyjde, vratime PUVODNI chybu.
                if action != ironrdp_pdu::Action::X224 {
                    return Err(anyhow::anyhow!("zpracování RDP dat selhalo: {process_err:?}"));
                }
                debug!("active_stage.process selhal na X.224 PDU, zkousim reaktivaci: {process_err:?}");
                match s.reactivate(Some(&payload)) {
                    Ok(()) => continue,
                    Err(react_err) => {
                        debug!("zalozni reaktivace take selhala: {react_err:#}");
                        return Err(anyhow::anyhow!("zpracování RDP dat selhalo: {process_err:?}"));
                    }
                }
            }
        };

        if s.handle_outputs(outputs)? {
            info!("server ukoncil RDP relaci");
            return Ok(());
        }
    }
}

impl Session<'_> {
    /// Zpracuje vystupy `ActiveStage` - vraci `true`, pokud server relaci ukoncil.
    fn handle_outputs(&mut self, outputs: Vec<ActiveStageOutput>) -> anyhow::Result<bool> {
        let mut dirty: Option<DirtyRect> = None;
        for out in outputs {
            match out {
                ActiveStageOutput::ResponseFrame(bytes) => {
                    self.framed.write_all(&bytes).map_err(|e| anyhow::anyhow!("odeslání RDP odpovědi selhalo: {e}"))?;
                }
                ActiveStageOutput::GraphicsUpdate(rect) => {
                    let rect = to_dirty(&rect);
                    dirty = Some(match dirty {
                        Some(d) => d.union(rect),
                        None => rect,
                    });
                }
                ActiveStageOutput::DeactivateAll => {
                    // Standardni cesta (oficialni klient): server ohlasil
                    // Deactivation-Reactivation sekvenci.
                    self.flush_dirty(dirty.take());
                    self.reactivate(None)?;
                }
                ActiveStageOutput::Terminate(reason) => {
                    info!(?reason, "RDP relace ukoncena serverem");
                    self.flush_dirty(dirty.take());
                    return Ok(true);
                }
                // Kurzor serveru (vypnuto, viz `build_config`), multitransport
                // (UDP neimplementovano), autodetekce site - bez efektu.
                _ => {}
            }
        }
        self.flush_dirty(dirty);
        Ok(false)
    }

    /// Prenese zmenenou oblast do `SharedFrame` a probudi okno plochy.
    fn flush_dirty(&mut self, dirty: Option<DirtyRect>) {
        if let Some(rect) = dirty {
            self.frame.lock().unwrap().copy_rect(&self.image, rect);
            (self.waker)(RdpWake::Frame);
        }
    }

    fn send_input_ops(&mut self, ops: Vec<Operation>) -> anyhow::Result<()> {
        if ops.is_empty() {
            return Ok(());
        }
        let events: Vec<FastPathInputEvent> = self.input_db.apply(ops).into_iter().collect();
        self.send_input_events(&events)
    }

    fn send_input_events(&mut self, events: &[FastPathInputEvent]) -> anyhow::Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let outputs = self
            .active_stage
            .process_fastpath_input(&mut self.image, events)
            .map_err(|e| anyhow::anyhow!("odeslání vstupu (myš/klávesnice) selhalo: {e:?}"))?;
        self.handle_outputs(outputs)?;
        Ok(())
    }

    /// Je Display Control kanal otevreny A server uz poslal "capabilities"?
    /// Pred tim server pozadavky na zmenu rozliseni tise ignoruje.
    fn display_control_ready(&mut self) -> bool {
        self.active_stage.get_dvc::<DisplayControlClient>().is_some_and(|dvc| {
            dvc.is_open() && dvc.channel_processor_downcast_ref::<DisplayControlClient>().is_some_and(|c| c.ready())
        })
    }

    fn try_send_pending_resize(&mut self) -> anyhow::Result<()> {
        let Some(req) = self.pending_resize else {
            return Ok(());
        };
        if req.width == u32::from(self.image.width()) && req.height == u32::from(self.image.height()) {
            debug!(req.width, req.height, "zmena rozliseni neni potreba - plocha uz ma tuto velikost");
            self.pending_resize = None;
            return Ok(());
        }
        if !self.display_control_ready() {
            // Zkusi se znovu v dalsim kole smycky (nejpozdeji za READ_TIMEOUT).
            return Ok(());
        }
        self.pending_resize = None;
        debug!(req.width, req.height, ?req.scale_percent, "odesilam pozadavek na zmenu rozliseni");
        match self.active_stage.encode_resize(req.width, req.height, req.scale_percent, None) {
            Some(Ok(bytes)) => self
                .framed
                .write_all(&bytes)
                .map_err(|e| anyhow::anyhow!("odeslání požadavku na změnu rozlišení selhalo: {e}")),
            Some(Err(e)) => Err(anyhow::anyhow!("zakódování požadavku na změnu rozlišení selhalo: {e:?}")),
            None => {
                // Nemelo by nastat (ready() vyse) - radeji zkusime pozdeji.
                self.pending_resize = Some(req);
                Ok(())
            }
        }
    }

    /// RDP "Deactivation-Reactivation" sekvence - postup presne podle
    /// oficialniho `ironrdp-client` (viz bod 1 v header komentari).
    /// `first_frame` = uz prectena PDU, ktera se pouzije jako PRVNI vstup
    /// (zalozni cesta, server poslal Demand Active bez Deactivate All).
    fn reactivate(&mut self, first_frame: Option<&[u8]>) -> anyhow::Result<()> {
        debug!(from_error_path = first_frame.is_some(), "spoustim Deactivation-Reactivation sekvenci");
        let mut sequence = self.activation_factory.create();
        let mut buf = ironrdp_core::WriteBuf::new();
        let mut pending_first = first_frame.map(|f| f.to_vec());

        loop {
            buf.clear();
            let written = if let Some(hint) = sequence.next_pdu_hint() {
                let pdu = match pending_first.take() {
                    Some(pdu) => pdu,
                    None => loop {
                        // Socket ma porad kratky cteci timeout - neni to chyba.
                        match self.framed.read_by_hint(hint) {
                            Ok(pdu) => break pdu.to_vec(),
                            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => continue,
                            Err(e) => return Err(anyhow::anyhow!("čtení dat při reaktivaci RDP relace selhalo: {e}")),
                        }
                    },
                };
                sequence
                    .step(&pdu, &mut buf)
                    .map_err(|e| anyhow::anyhow!("zpracování reaktivace RDP relace selhalo: {e:?}"))?
            } else {
                sequence
                    .step_no_input(&mut buf)
                    .map_err(|e| anyhow::anyhow!("zpracování reaktivace RDP relace selhalo: {e:?}"))?
            };

            if let Some(len) = written.size() {
                self.framed
                    .write_all(&buf[..len])
                    .map_err(|e| anyhow::anyhow!("odeslání dat při reaktivaci RDP relace selhalo: {e}"))?;
            }

            if let ConnectionActivationState::Finalized { desktop_size, share_id, enable_server_pointer, pointer_software_rendering } =
                sequence.connection_activation_state()
            {
                info!(width = desktop_size.width, height = desktop_size.height, "reaktivace dokoncena");
                self.image = DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
                self.active_stage.set_fastpath_processor(
                    fast_path::ProcessorBuilder {
                        io_channel_id: self.activation_factory.io_channel_id(),
                        user_channel_id: self.activation_factory.user_channel_id(),
                        share_id,
                        enable_server_pointer,
                        pointer_software_rendering,
                        bulk_decompressor: None,
                    }
                    .build(),
                );
                self.active_stage.set_share_id(share_id);
                self.active_stage.set_enable_server_pointer(enable_server_pointer);

                {
                    let mut frame = self.frame.lock().unwrap();
                    if frame.width != desktop_size.width || frame.height != desktop_size.height {
                        frame.resize(desktop_size.width, desktop_size.height);
                    }
                    frame.copy_all(&self.image);
                }
                (self.waker)(RdpWake::Frame);
                self.events.send(RdpEvent::Resized { width: desktop_size.width, height: desktop_size.height });
                return Ok(());
            }
        }
    }
}

fn to_dirty(rect: &InclusiveRectangle) -> DirtyRect {
    DirtyRect { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom }
}

/// Prevede vstupni `RdpCommand` na operaci pro `ironrdp_input::Database`
/// (ta drzi stav stisknutych klaves/tlacitek a z rozdilu sestavi spravne
/// FastPath udalosti).
fn command_to_operation(cmd: RdpCommand) -> Option<Operation> {
    Some(match cmd {
        RdpCommand::MouseMove { x, y } => Operation::MouseMove(MousePosition { x, y }),
        RdpCommand::MouseButton { button, pressed } => {
            let b = match button {
                RdpMouseButton::Left => InputMouseButton::Left,
                RdpMouseButton::Middle => InputMouseButton::Middle,
                RdpMouseButton::Right => InputMouseButton::Right,
                RdpMouseButton::X1 => InputMouseButton::X1,
                RdpMouseButton::X2 => InputMouseButton::X2,
            };
            if pressed {
                Operation::MouseButtonPressed(b)
            } else {
                Operation::MouseButtonReleased(b)
            }
        }
        RdpCommand::MouseWheel { vertical, rotation_units } => {
            Operation::WheelRotations(WheelRotations { is_vertical: vertical, rotation_units })
        }
        RdpCommand::Key { extended, code, pressed } => {
            let sc = Scancode::from_u8(extended, code);
            if pressed {
                Operation::KeyPressed(sc)
            } else {
                Operation::KeyReleased(sc)
            }
        }
        RdpCommand::Unicode { ch, pressed } => {
            if pressed {
                Operation::UnicodeKeyPressed(ch)
            } else {
                Operation::UnicodeKeyReleased(ch)
            }
        }
        RdpCommand::ReleaseAll | RdpCommand::ResizeDesktop { .. } | RdpCommand::Disconnect => return None,
    })
}
