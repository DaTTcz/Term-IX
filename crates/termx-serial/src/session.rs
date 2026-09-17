//! Interaktivni seriove/COM spojeni pro vestaveny terminal v `termx-gui` -
//! obdoba `termx_ssh::session` (`spawn_ssh_session`/`SshHandle`), jen bez
//! sitoveho protokolu okolo: `serialport` je blokujici (ne `async`)
//! knihovna, takze tu na rozdil od SSH modulu neni potreba zadny tokio
//! runtime ani jednovlaknovy trik s nim - staci obycejna dve vlakna
//! (`std::thread`) komunikujici pres `std::sync::mpsc` kanaly.
//!
//! Vstup (GUI -> seriova linka) i vystup (seriova linka -> GUI) jdou OBA
//! pres `std::sync::mpsc` (na rozdil od SSH modulu, ktery pro vstup pouziva
//! `tokio::sync::mpsc` jen kvuli tomu, ze jeho `run_session` bezi v
//! asynchronnim `tokio::select!` - tady zadna takova potreba neni,
//! obycejny synchronni kanal stejne jde poslat primo z GUI vlakna).
//!
//! Cteni ze seriove linky bezi na VLASTNIM druhem vlakne (`reader_thread`
//! v `run_session`), oddelene od hlavniho vlakna, ktere jen ceka na vstup
//! z GUI a zapisuje ho na port - `serialport::SerialPort::read` je
//! blokujici volani (s timeoutem, viz `READ_TIMEOUT`), takze bez dvou
//! vlaken by nesly obsluhovat oba smery zaroven (na rozdil od SSH modulu,
//! kde `tokio::select!` obe strany prirozene prokladá v jedine asynchronni
//! smycce).
//!
//! POZNAMKA K OVERENI: presne nazvy metod crate `serialport` (verze `4`)
//! - `serialport::new(path, baud).timeout(..).open()`, `SerialPort::try_clone()`,
//! `serialport::available_ports()` - nebylo mozne overit skutecnym `cargo
//! build` (bez pristupu na crates.io v tomto prostredi). Jde o
//! nejrozsirenejsi/nejstabilnejsi Rust knihovnu pro seriovou linku s
//! dlouhodobe stabilnim API, takze riziko by melo byt nizke, ale pokud by
//! nektery nazev/signatura po stazeni skutecne verze nesedely, jde o
//! izolovanou opravu jen v tomto souboru.
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use termx_core::{SerialDataBits, SerialFlowControl, SerialParity, SerialStopBits, Session};

/// Vychozi rychlost seriove linky, kdyz `Session::serial_baud_rate` neni
/// vyplnene - 9600 je nejcastejsi vychozi hodnota konzolovych/sitovych
/// zarizeni (Cisco, Avaya, a vetsina embedded hardwaru).
pub const DEFAULT_BAUD_RATE: u32 = 9600;

/// Jak dlouho nejdele ceka jedno cteni ze seriove linky (`reader_thread`
/// v `run_session`), nez se vrati (i kdyz zadna data nedorazila) - kratke
/// schvalne, aby `reader_thread` mohl mezitim pravidelne kontrolovat
/// `running` (viz nize) a rychle se ukoncit pri zavreni tabu, misto aby
/// se ukoncil az po prichodu dalsich dat (nebo vubec, kdyz uz zadna
/// nedorazi).
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(50);

/// Prichozi udalost ze seriove linky smerem ke GUI - podmnozina
/// `termx_ssh::SshEvent` (zadne `AwaitingCredentials`/`AuthFailed`/`Stats` -
/// seriova linka nema prihlasovaci prompt ani systemove metriky protejsi
/// strany, na rozdil od SSH shellu).
pub enum SerialEvent {
    /// Syrova data prijata z portu - k parsovani ANSI escape sekvenci na
    /// strane GUI (`alacritty_terminal`), stejne jako `SshEvent::Data`.
    Data(Vec<u8>),
    /// Port byl uspesne otevren - na rozdil od SSH tu neni zadna
    /// autentizace, takze tato udalost prijde (skoro) okamzite po
    /// zalozeni spojeni.
    Connected,
    /// Otevreni portu (nebo cteni/zapis na nem) selhalo.
    Error(String),
    /// Spojeni bylo ukonceno (zavreno GUI stranou - viz `SerialHandle`,
    /// nebo chyba pri cteni/zapisu, ktera portu predchazela).
    Closed,
}

/// Odchozi prikaz od GUI smerem k bezicimu seriovemu spojeni. Na rozdil
/// od `termx_ssh::SshInput` zadne `Resize` (seriova linka nema pojem PTY
/// velikosti - zmena velikosti terminaloveho okna v GUI je cistě lokalni
/// zalezitost, viz `TerminalSession::resize` v `termx-gui`) ani
/// `Credentials` (zadne prihlasovani).
pub enum SerialInput {
    Data(Vec<u8>),
}

/// Uchyt na bezici seriove spojeni - vstupni odesilac a vystupni prijemce,
/// stejny vzor jako `termx_ssh::SshHandle`. Zahozenim (`drop`) `input_tx`
/// (napr. kdyz uzivatel zavre Connection tab) hlavni vlakno v `run_session`
/// vrati `Err` z `input_rx.recv()` a cele spojeni (vc. `reader_thread`) se
/// samo a cistě ukonci.
pub struct SerialHandle {
    pub input_tx: std::sync::mpsc::Sender<SerialInput>,
    pub output_rx: std::sync::mpsc::Receiver<SerialEvent>,
}

/// Vypise vsechny aktualne pripojene seriove porty (napr. `/dev/ttyUSB0`
/// na Linuxu, `COM3` na Windows) - pouziva se v `termx-gui` jako napoveda
/// (ne vynucene omezeni) v poli "Port:" nového/upravovaného seriového
/// spojeni (zpetna vazba "pokud je připojeno tak se ukáže v nápovědě ale
/// můžu uložit jiný port když vím že tam někdy byl/bude" - proto tu jen
/// vracime seznam, samotne pole zustava normalne editovatelny text, viz
/// `grid_field_with_suggestions` v `app.rs`). Prazdny seznam (napr. kdyz
/// se vypis nezdari, nebo zadny port neni pripojeny) neni chyba - jen
/// nabidka poradi.
pub fn available_ports() -> Vec<String> {
    serialport::available_ports().map(|ports| ports.into_iter().map(|p| p.port_name).collect()).unwrap_or_default()
}

/// Prevody `termx_core::Serial*` (nezavisle na konkretni knihovne, viz
/// doc-komentare u techto typu v `termx-core`) na odpovidajici typy
/// crate `serialport` - jednoduche 1:1 mapovani.
fn map_data_bits(v: SerialDataBits) -> serialport::DataBits {
    match v {
        SerialDataBits::Five => serialport::DataBits::Five,
        SerialDataBits::Six => serialport::DataBits::Six,
        SerialDataBits::Seven => serialport::DataBits::Seven,
        SerialDataBits::Eight => serialport::DataBits::Eight,
    }
}

fn map_parity(v: SerialParity) -> serialport::Parity {
    match v {
        SerialParity::None => serialport::Parity::None,
        SerialParity::Odd => serialport::Parity::Odd,
        SerialParity::Even => serialport::Parity::Even,
    }
}

fn map_stop_bits(v: SerialStopBits) -> serialport::StopBits {
    match v {
        SerialStopBits::One => serialport::StopBits::One,
        SerialStopBits::Two => serialport::StopBits::Two,
    }
}

fn map_flow_control(v: SerialFlowControl) -> serialport::FlowControl {
    match v {
        SerialFlowControl::None => serialport::FlowControl::None,
        SerialFlowControl::Software => serialport::FlowControl::Software,
        SerialFlowControl::Hardware => serialport::FlowControl::Hardware,
    }
}

/// Zalozi nove seriove spojeni na samostatnem vlakne a vrati uchyt pro
/// komunikaci s nim. Nikdy nepanikari - jakakoliv chyba (neexistujici/
/// jiz obsazeny port apod.) se preda jako [`SerialEvent::Error`], aby ji
/// GUI mohlo zobrazit primo v Connection tabu, stejne jako
/// `termx_ssh::spawn_ssh_session`.
///
/// `session.host` se pouziva jako CESTA/JMENO PORTU (napr.
/// `/dev/ttyUSB0`, `COM3`) - viz doc-komentar u `Session::serial_baud_rate`
/// v `termx-core`, ktery vysvetluje, proc port nema vlastni pole.
pub fn spawn_serial_session(session: Session) -> SerialHandle {
    let (input_tx, input_rx) = std::sync::mpsc::channel::<SerialInput>();
    let (output_tx, output_rx) = std::sync::mpsc::channel::<SerialEvent>();

    std::thread::spawn(move || {
        run_session(&session, &input_rx, &output_tx);
    });

    SerialHandle { input_tx, output_rx }
}

/// Hlavni smycka bezici na vlastnim vlakne (viz `spawn_serial_session`).
/// Nikdy nevraci chybu primo - vsechny chybove stavy se posilaji jako
/// [`SerialEvent::Error`]/[`SerialEvent::Closed`], aby volajici vlakno
/// (`spawn_serial_session`) nemuselo resit zadne dalsi predavani chyby.
fn run_session(session: &Session, input_rx: &std::sync::mpsc::Receiver<SerialInput>, output_tx: &std::sync::mpsc::Sender<SerialEvent>) {
    let baud_rate = session.serial_baud_rate.unwrap_or(DEFAULT_BAUD_RATE);
    // Vsechny 4 parametry seriove linky jsou nastavitelne (pozadavek
    // "chci komplet nastavení portu") - kdyz uzivatel nic nezmenil (`None`
    // v `Session`), pouzije se vychozi "8-N-1" bez rizeni toku (vychozi
    // hodnoty `serialport` crate se stejne shoduji), coz odpovida
    // typickemu primemu konzolovemu pristupu (napr. k Avaya CM SAT pres
    // null-modem kabel) - viz `Session::serial_baud_rate` v `termx-core`.
    let data_bits = session.serial_data_bits.map(map_data_bits).unwrap_or(serialport::DataBits::Eight);
    let parity = session.serial_parity.map(map_parity).unwrap_or(serialport::Parity::None);
    let stop_bits = session.serial_stop_bits.map(map_stop_bits).unwrap_or(serialport::StopBits::One);
    let flow_control = session.serial_flow_control.map(map_flow_control).unwrap_or(serialport::FlowControl::None);
    let port = match serialport::new(session.host.as_str(), baud_rate)
        .data_bits(data_bits)
        .parity(parity)
        .stop_bits(stop_bits)
        .flow_control(flow_control)
        .timeout(READ_TIMEOUT)
        .open()
    {
        Ok(p) => p,
        Err(e) => {
            let _ = output_tx.send(SerialEvent::Error(format!("otevření sériového portu „{}“ selhalo: {e}", session.host)));
            let _ = output_tx.send(SerialEvent::Closed);
            return;
        }
    };

    let mut reader = match port.try_clone() {
        Ok(p) => p,
        Err(e) => {
            let _ = output_tx.send(SerialEvent::Error(format!("nelze duplikovat handle portu: {e}")));
            let _ = output_tx.send(SerialEvent::Closed);
            return;
        }
    };
    let mut writer = port;

    let _ = output_tx.send(SerialEvent::Connected);

    // `running` rika `reader_thread` nize, kdy ma prestat cist a skoncit -
    // nastavuje se na `false` az PO opusteni hlavni smycky (zavreni tabu
    // NEBO chyba pri zapisu), viz konec teto funkce.
    let running = Arc::new(AtomicBool::new(true));
    let reader_running = Arc::clone(&running);
    let reader_output_tx = output_tx.clone();
    let reader_thread = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while reader_running.load(Ordering::Relaxed) {
            match reader.read(&mut buf) {
                // `serialport`uv timeout (viz `READ_TIMEOUT` vyse) se
                // hlasi jako `Ok(0)` NEBO jako `Err(TimedOut)` v
                // zavislosti na platforme/backendu - oba pripady tu
                // proto jen znamenaji "zkus znovu", ne chybu/konec.
                Ok(0) => continue,
                Ok(n) => {
                    // GUI vlakno uz nemusi poslouchat (tab mezitim
                    // zavreny) - poslani se pak proste nezdari, to samo
                    // o sobe neni duvod cteci smycku ukoncovat (o to se
                    // postara `running` nastavene hlavni smyckou nize).
                    let _ = reader_output_tx.send(SerialEvent::Data(buf[..n].to_vec()));
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(e) => {
                    let _ = reader_output_tx.send(SerialEvent::Error(format!("čtení ze sériového portu selhalo: {e}")));
                    break;
                }
            }
        }
    });

    loop {
        match input_rx.recv() {
            Ok(SerialInput::Data(bytes)) => {
                if let Err(e) = writer.write_all(&bytes) {
                    let _ = output_tx.send(SerialEvent::Error(format!("zápis na sériový port selhal: {e}")));
                    break;
                }
            }
            // GUI strana zahodila `input_tx` (zavreny tab) - cas se
            // cistě odpojit, nejde o chybu.
            Err(_) => break,
        }
    }

    running.store(false, Ordering::Relaxed);
    // Cekame na dokonceni cteciho vlakna, aby `port`/`writer` (a tim i
    // jeho klon `reader`) nebyly zahozeny driv, nez cteci vlakno samo
    // skonci svuj (nejvyse `READ_TIMEOUT` dlouhy) posledni pokus o cteni.
    let _ = reader_thread.join();
    let _ = output_tx.send(SerialEvent::Closed);
}
