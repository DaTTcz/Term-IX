//! Interaktivni SSH relace pro vestaveny terminal v `termx-gui`.
//!
//! Na rozdil od [`crate::SshModule::run`] (puvodni "TUI" cesta, ktera
//! primo prebira stdin/stdout bezicicho procesu a bezi v ramci
//! existujiciho tokio runtime) tady zadny sdileny runtime neni k
//! dispozici - `termx-gui` (eframe) bezi ve vlastni blokujici smycce na
//! hlavnim vlakne bez tokio. Kazde SSH spojeni si proto zaklada
//! VLASTNI male vlakno se svym vlastnim jednovlaknovym tokio runtime
//! (`spawn_ssh_session`) a komunikuje s GUI vlaknem pres kanaly - stejny
//! vzor (`std::thread::spawn` + kanal), jaky uz `termx-gui` pouziva pro
//! kontrolu aktualizaci na pozadi.
//!
//! Vstup (klavesnice z GUI -> SSH kanal) jde pres `tokio::sync::mpsc`
//! (`UnboundedSender::send` je bezna, ne-asynchronni metoda, jde tedy
//! volat primo z GUI vlakna bez tokio kontextu). Vystup (data ze SSH ->
//! GUI k vykresleni) jde naopak pres `std::sync::mpsc`, aby ho GUI mohlo
//! kazdy snimek nekonfliktne "vycerpat" pres `try_recv()` bez zavislosti
//! na tokio.
//!
//! POZNAMKA K OVERENI: presne nazvy metod/typu `russh` 0.44 (`Handle`,
//! `Channel`, `ChannelMsg`) vychazi z puvodniho `SshModule::run` vyse v
//! tomto crate (ten uz uzivatel jednou zkompiloval - viz historie
//! projektu), takze tato cast by mela byt spolehliva. Nove/neoverene je
//! jen samotne zapouzdreni do vlakna+kanalu zde.
//!
//! POZNAMKA K OVERENI (statistiky pro info proužek, `fetch_stats_output`):
//! oproti puvodnimu interaktivnimu kanalu (`request_pty`+`request_shell`,
//! jiz overeno) tu pribyva `Channel::exec(want_reply, command)` - pokud
//! presna signatura (napr. ocekavany typ `command`) v pouzite verzi
//! `russh` nesedi, jde o izolovanou opravu jen teto jedne funkce; zbytek
//! (parsovani v `parse_stats`) na tom nezavisi.
use std::sync::Arc;

use termx_core::{AuthMethod, Session};

use crate::handler::TofuHandler;

/// Prichozi udalost ze SSH spojeni smerem ke GUI.
pub enum SshEvent {
    /// Syrova data prijata ze vzdaleneho shellu (stdout/stderr spojene
    /// dohromady, jak je posila PTY na druhe strane) - k parsovani ANSI
    /// escape sekvenci na strane GUI (`alacritty_terminal`).
    Data(Vec<u8>),
    /// Spojeni bylo uspesne navazano a autentizovano - az od teto
    /// chvile ma smysl zacit odesilat vstup.
    Connected,
    /// Spojeni skoncilo chybou (sitova chyba, spatne heslo, ...).
    Error(String),
    /// Spojeni bylo cistě ukonceno (server zavrel kanal, apod.) bez chyby.
    Closed,
    /// Ciselne obcerstvene systemove metriky protejsi strany (CPU, RAM,
    /// sit, ...) - viz [`SystemStats`]. Posila se periodicky (kazdych
    /// nekolik sekund, viz `run_session`), ne pri kazde zmene.
    Stats(SystemStats),
}

/// Systemove metriky pripojeneho serveru pro MobaXterm-podobny info
/// proužek pod terminalem v `termx-gui` (viz `TerminalSession::render_status_bar`).
/// Vsechna ciselna pole jsou `Option` - kdyz se dany udaj na cilovem
/// systemu nepodari zjistit (napr. holy/kontejnerovy system bez
/// nekterych `/proc` souboru), prislusna cast panelu se proste
/// nezobrazi, misto aby to shodilo celou aktualizaci statistik.
#[derive(Debug, Clone, Default)]
pub struct SystemStats {
    /// Vytizeni CPU v procentech, odvozene z 1-minutoveho load average
    /// (`/proc/loadavg`) deleneho poctem jader - jde tedy o hrube
    /// priblizeni (ne totozne s tim, co by ukazal `top`), ale bez
    /// zavislosti na tom, jestli je `top`/`mpstat` na cilovem systemu
    /// vubec nainstalovany.
    pub cpu_percent: Option<f32>,
    pub mem_used_gb: Option<f64>,
    pub mem_total_gb: Option<f64>,
    pub uptime_days: Option<u64>,
    /// Prihlasovaci jmeno pouzite pro tuto SSH relaci (ne z cileho
    /// systemu zjistovane, ale predane pri volani `parse_stats`) -
    /// pouziva se k rozpocitani `user_sessions` z vystupu `who`.
    pub username: String,
    /// Pocet aktivnich prihlasenych relaci stejneho uzivatele (`who`) -
    /// v panelu zobrazeno jako "(xN)", kdyz je vetsi nez 1.
    pub user_sessions: usize,
    pub disk_percent: Option<u32>,
    pub net_up_mbps: Option<f64>,
    pub net_down_mbps: Option<f64>,
}

/// Odchozi prikaz od GUI smerem k bezicimu SSH spojeni.
pub enum SshInput {
    /// Bajty k odeslani do vzdaleneho shellu (napsany text, ovladaci
    /// znaky prelozene z klaves - viz `terminal.rs` v `termx-gui`).
    Data(Vec<u8>),
    /// Zmena velikosti terminaloveho okna (pocet sloupcu/radku) - preda
    /// se na PTY serveru (`window_change`), aby napr. `vim`/`htop`
    /// vedely, jak velkou obrazovku maji k dispozici.
    Resize { cols: u32, rows: u32 },
}

/// Uchyt na bezici SSH spojeni - vstupni odesilac a vystupni prijemce.
/// Zahozenim (`drop`) `input_tx` (napr. kdyz uzivatel zavre Connection
/// tab) prijemce na druhe strane vrati `None` a vlakno se samo a cistě
/// ukonci (zadne rucni "kill" vlakna neni potreba).
pub struct SshHandle {
    pub input_tx: tokio::sync::mpsc::UnboundedSender<SshInput>,
    pub output_rx: std::sync::mpsc::Receiver<SshEvent>,
}

/// Zalozi nove SSH spojeni na samostatnem vlakne a vrati uchyt pro
/// komunikaci s nim. Nikdy nepanikari - jakakoliv chyba (spatne heslo,
/// nedostupny server, nepodporovana metoda prihlaseni) se preda jako
/// [`SshEvent::Error`], aby ji GUI mohlo zobrazit primo v Connection
/// tabu.
pub fn spawn_ssh_session(session: Session, initial_cols: u16, initial_rows: u16) -> SshHandle {
    let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel::<SshInput>();
    let (output_tx, output_rx) = std::sync::mpsc::channel::<SshEvent>();

    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                let _ = output_tx.send(SshEvent::Error(format!("nepodařilo se spustit síťové vlákno: {e}")));
                return;
            }
        };

        runtime.block_on(async move {
            let mut input_rx = input_rx;
            if let Err(e) = run_session(&session, initial_cols, initial_rows, &mut input_rx, &output_tx).await {
                let _ = output_tx.send(SshEvent::Error(e.to_string()));
            }
            let _ = output_tx.send(SshEvent::Closed);
        });
    });

    SshHandle { input_tx, output_rx }
}

/// Kombinovany prikaz spousteny na pozadi (viz `fetch_stats_output`) pro
/// ziskani statistik pro info proužek pod terminalem. Zamerne stavi jen
/// na `/proc` souborech a jinak vsudypritomnych `who`/`df` (zadny
/// `top`/`free`/`vnstat`), aby fungoval i na minimalistickych/embedded
/// systemech (napr. male linuxove distribuce na sitovych zarizenich) bez
/// dodatecnych balicku. Kazdy radek vystupu je oznacen `TERMIX_*`
/// znackou, aby ho `parse_stats` slo spolehlive rozpoznat bez ohledu na
/// lokalizaci/format ostatnich nastroju.
const STATS_COMMAND: &str = r#"
echo TERMIX_LOADAVG $(cat /proc/loadavg 2>/dev/null)
echo TERMIX_NPROC $(nproc 2>/dev/null || grep -c ^processor /proc/cpuinfo 2>/dev/null)
echo TERMIX_MEMTOTAL $(grep -m1 MemTotal /proc/meminfo 2>/dev/null)
echo TERMIX_MEMAVAIL $(grep -m1 MemAvailable /proc/meminfo 2>/dev/null)
echo TERMIX_UPTIME $(cat /proc/uptime 2>/dev/null)
echo TERMIX_DISK $(df -P / 2>/dev/null | tail -n1)
echo TERMIX_WHO_BEGIN
who 2>/dev/null
echo TERMIX_WHO_END
echo TERMIX_NET_BEGIN
cat /proc/net/dev 2>/dev/null
echo TERMIX_NET_END
"#;

/// Otevre SAMOSTATNY (docasny) SSH kanal - nezavisly na interaktivnim
/// kanalu shellu v `run_session` - spusti na nem [`STATS_COMMAND`] a
/// pockej na jeho cely vystup. Samostatny kanal proto, aby se nemichal
/// s vystupem interaktivniho shellu (ktery cte `alacritty_terminal` v
/// `termx-gui`).
async fn fetch_stats_output(handle: &mut russh::client::Handle<TofuHandler>) -> anyhow::Result<String> {
    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| anyhow::anyhow!("nelze otevřít kanál pro statistiky: {e}"))?;

    channel
        .exec(true, STATS_COMMAND.as_bytes())
        .await
        .map_err(|e| anyhow::anyhow!("spuštění příkazu pro statistiky selhalo: {e}"))?;

    let mut output = Vec::new();
    loop {
        match channel.wait().await {
            Some(russh::ChannelMsg::Data { data }) => output.extend_from_slice(&data),
            Some(russh::ChannelMsg::Eof) | Some(russh::ChannelMsg::Close) | None => break,
            _ => {}
        }
    }

    Ok(String::from_utf8_lossy(&output).into_owned())
}

/// Rozparsuje vystup [`STATS_COMMAND`] (viz `fetch_stats_output`) do
/// [`SystemStats`]. Cistá (bez I/O) funkce - snadno se da overit i bez
/// skutecneho SSH spojeni. `prev_net` drzi casovou znacku a soucet
/// bajtu z predchoziho volani, aby slo dopocitat rychlost site (Mb/s) z
/// rozdilu kumulativnich citacu `/proc/net/dev` - pri prvnim volani (kdy
/// `prev_net` je `None`) proto `net_up_mbps`/`net_down_mbps` zustanou
/// `None` (neni jeste s cim porovnat).
fn parse_stats(
    output: &str,
    username: &str,
    prev_net: &mut Option<(std::time::Instant, u64, u64)>,
) -> SystemStats {
    let mut stats = SystemStats {
        username: username.to_string(),
        ..Default::default()
    };

    let mut nproc: f32 = 1.0;
    let mut loadavg1: Option<f32> = None;
    let mut mem_total_kb: Option<f64> = None;
    let mut mem_avail_kb: Option<f64> = None;
    let mut rx_total: u64 = 0;
    let mut tx_total: u64 = 0;
    let mut in_who = false;
    let mut in_net = false;

    for raw_line in output.lines() {
        let line = raw_line.trim_end();

        if line == "TERMIX_WHO_BEGIN" {
            in_who = true;
            continue;
        }
        if line == "TERMIX_WHO_END" {
            in_who = false;
            continue;
        }
        if line == "TERMIX_NET_BEGIN" {
            in_net = true;
            continue;
        }
        if line == "TERMIX_NET_END" {
            in_net = false;
            continue;
        }

        if in_who {
            // Kazdy radek `who` zacina prihlasovacim jmenem - pocitame
            // vsechny radky patrici prihlasenemu uzivateli.
            if line.split_whitespace().next() == Some(username) {
                stats.user_sessions += 1;
            }
            continue;
        }

        if in_net {
            // `/proc/net/dev`: "  eth0: 12345 ... 67890 ..." - 1. cislo
            // za dvojteckou jsou prijate bajty, 9. odeslane bajty.
            // Hlavickove radky nemaji dvojtecku vubec, vypadnou tak samy
            // (`split_once` vrati `None`). Loopback (`lo`) se zamerne
            // vynechava, at neovlivnuje rychlost "site".
            if let Some((iface, rest)) = line.split_once(':') {
                let iface = iface.trim();
                if !iface.is_empty() && iface != "lo" {
                    let cols: Vec<&str> = rest.split_whitespace().collect();
                    if let (Some(rx), Some(tx)) = (cols.first(), cols.get(8)) {
                        if let (Ok(rx), Ok(tx)) = (rx.parse::<u64>(), tx.parse::<u64>()) {
                            rx_total += rx;
                            tx_total += tx;
                        }
                    }
                }
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("TERMIX_LOADAVG ") {
            loadavg1 = rest.split_whitespace().next().and_then(|v| v.parse::<f32>().ok());
        } else if let Some(rest) = line.strip_prefix("TERMIX_NPROC ") {
            if let Ok(n) = rest.trim().parse::<f32>() {
                if n > 0.0 {
                    nproc = n;
                }
            }
        } else if let Some(rest) = line.strip_prefix("TERMIX_MEMTOTAL ") {
            mem_total_kb = first_number(rest);
        } else if let Some(rest) = line.strip_prefix("TERMIX_MEMAVAIL ") {
            mem_avail_kb = first_number(rest);
        } else if let Some(rest) = line.strip_prefix("TERMIX_UPTIME ") {
            if let Some(secs) = rest.split_whitespace().next().and_then(|v| v.parse::<f64>().ok()) {
                stats.uptime_days = Some((secs / 86400.0).floor() as u64);
            }
        } else if let Some(rest) = line.strip_prefix("TERMIX_DISK ") {
            // `df -P /` vystup: "Filesystem 1024-blocks Used Available Capacity Mounted"
            // - 5. sloupec (index 4) je vyuziti v procentech vc. "%".
            if let Some(pct) = rest.split_whitespace().nth(4) {
                stats.disk_percent = pct.trim_end_matches('%').parse::<u32>().ok();
            }
        }
    }

    if let Some(load1) = loadavg1 {
        stats.cpu_percent = Some((load1 / nproc * 100.0).clamp(0.0, 100.0));
    }

    if let (Some(total_kb), Some(avail_kb)) = (mem_total_kb, mem_avail_kb) {
        let used_kb = (total_kb - avail_kb).max(0.0);
        stats.mem_total_gb = Some(total_kb / 1024.0 / 1024.0);
        stats.mem_used_gb = Some(used_kb / 1024.0 / 1024.0);
    }

    let now = std::time::Instant::now();
    if let Some((prev_time, prev_rx, prev_tx)) = prev_net.as_ref() {
        let elapsed = now.duration_since(*prev_time).as_secs_f64();
        if elapsed > 0.5 {
            let rx_delta = rx_total.saturating_sub(*prev_rx);
            let tx_delta = tx_total.saturating_sub(*prev_tx);
            // bajty/s -> megabity/s (1 Mb = 1_000_000 b, konvence
            // pouzita i pri udavani rychlosti pripojeni typu "100 Mb/s").
            stats.net_down_mbps = Some((rx_delta as f64 * 8.0) / elapsed / 1_000_000.0);
            stats.net_up_mbps = Some((tx_delta as f64 * 8.0) / elapsed / 1_000_000.0);
        }
    }
    *prev_net = Some((now, rx_total, tx_total));

    stats
}

/// Najde prvni token v retezci, ktery jde rozparsovat jako cislo -
/// vyuzivano pro radky `/proc/meminfo` (napr. `"MemTotal: 16384000 kB"`,
/// kde po sceleni pres `echo`/`$()` jednotlive "sloupce" oddeli mezery).
fn first_number(text: &str) -> Option<f64> {
    text.split_whitespace().find_map(|tok| tok.parse::<f64>().ok())
}

async fn run_session(
    session: &Session,
    cols: u16,
    rows: u16,
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<SshInput>,
    output_tx: &std::sync::mpsc::Sender<SshEvent>,
) -> anyhow::Result<()> {
    let (username, password) = match &session.auth {
        AuthMethod::Password { username, password } => (username.clone(), password.clone()),
        AuthMethod::PrivateKey { .. } => {
            anyhow::bail!("přihlášení privátním klíčem zatím není v SSH modulu implementováno")
        }
        AuthMethod::Agent { .. } => {
            anyhow::bail!("přihlášení přes ssh-agent zatím není v SSH modulu implementováno")
        }
        AuthMethod::None => anyhow::bail!("SSH vyžaduje přihlašovací údaje"),
    };

    let config = Arc::new(russh::client::Config::default());
    let addr = (session.host.as_str(), session.port);

    let mut handle = russh::client::connect(config, addr, TofuHandler)
        .await
        .map_err(|e| anyhow::anyhow!("SSH připojení selhalo: {e}"))?;

    let authenticated = handle
        .authenticate_password(&username, &password)
        .await
        .map_err(|e| anyhow::anyhow!("SSH autentizace selhala: {e}"))?;

    if !authenticated {
        anyhow::bail!("SSH autentizace odmítnuta - zkontrolujte uživatelské jméno a heslo");
    }

    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| anyhow::anyhow!("nelze otevřít SSH kanál: {e}"))?;

    channel
        .request_pty(false, "xterm-256color", cols as u32, rows as u32, 0, 0, &[])
        .await
        .map_err(|e| anyhow::anyhow!("požadavek na pty selhal: {e}"))?;
    channel
        .request_shell(true)
        .await
        .map_err(|e| anyhow::anyhow!("požadavek na shell selhal: {e}"))?;

    let _ = output_tx.send(SshEvent::Connected);

    // Periodicke nacitani statistik (CPU/RAM/sit/disk/...) pro info
    // proužek pod terminalem - viz `fetch_stats_output`/`parse_stats`.
    // Bezi jako dalsi vetev stejneho `tokio::select!` (jednoduchsi nez
    // samostatny task se sdilenym stavem pres `Arc<Mutex<...>>`), takze
    // kazdych 5 sekund na chvili (ohraniceno `tokio::time::timeout` na 3s,
    // aby pripadny vypadek/pomaly server neuvazl na neurcito) pozastavi
    // obsluhu interaktivniho kanalu - vedomy kompromis, zadouci hodnoty
    // (5s interval, 3s timeout) jsou dost male, aby to na interaktivite
    // terminalu nemelo znatelny dopad.
    let mut stats_interval = tokio::time::interval(std::time::Duration::from_secs(5));
    stats_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut prev_net: Option<(std::time::Instant, u64, u64)> = None;

    loop {
        tokio::select! {
            input = input_rx.recv() => {
                match input {
                    Some(SshInput::Data(bytes)) => {
                        channel.data(&bytes[..]).await.map_err(|e| anyhow::anyhow!("odeslani dat selhalo: {e}"))?;
                    }
                    Some(SshInput::Resize { cols, rows }) => {
                        channel
                            .window_change(cols, rows, 0, 0)
                            .await
                            .map_err(|e| anyhow::anyhow!("zmena velikosti terminalu selhala: {e}"))?;
                    }
                    // GUI strana zahodila `input_tx` (zavreny tab) - cas
                    // se cistě odpojit, nejde o chybu.
                    None => break,
                }
            }
            msg = channel.wait() => {
                match msg {
                    Some(russh::ChannelMsg::Data { data }) => {
                        // Prijemce (GUI vlakno) uz nemusi poslouchat
                        // (napr. tab byl mezitim zavreny) - poslani se
                        // pak proste nezdari, to neni duvod koncit s
                        // chybou.
                        let _ = output_tx.send(SshEvent::Data(data.to_vec()));
                    }
                    Some(russh::ChannelMsg::Eof) | Some(russh::ChannelMsg::Close) | None => break,
                    _ => {}
                }
            }
            _ = stats_interval.tick() => {
                let fetched = tokio::time::timeout(
                    std::time::Duration::from_secs(3),
                    fetch_stats_output(&mut handle),
                ).await;

                match fetched {
                    Ok(Ok(output)) => {
                        let stats = parse_stats(&output, &username, &mut prev_net);
                        let _ = output_tx.send(SshEvent::Stats(stats));
                    }
                    // Chyba nebo timeout pri nacitani statistik neni
                    // duvod koncit celou relaci - proste se pri tomto
                    // cyklu info proužek neaktualizuje, zkusi se to znovu
                    // za dalsich 5 sekund.
                    Ok(Err(_)) | Err(_) => {}
                }
            }
        }
    }

    Ok(())
}
