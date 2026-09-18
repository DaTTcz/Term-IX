//! termx-gui
//!
//! Hlavni graficke uzivatelske rozhrani Term-IX (nahrazuje puvodni
//! terminalove TUI): vlastni okno s hornim menu, levym panelem se
//! stromem ulozenych serveru/slozek a hlavni plochou s taby (Home,
//! Nastaveni, jednotliva spojeni). Hlavni heslo trezoru se zadava/
//! nastavuje primo v tomto okne (uvodni "zamcena" obrazovka) - zadne
//! konzolove (cmd) okno k tomu neni potreba, viz [`run_app`].
//!
//! POZNAMKA K OVERENI: stejne jako u `termx-ssh`, ani zde nebylo v tomto
//! prostredi mozne spustit skutecny `cargo build` (zadny pristup na
//! crates.io) - `egui`/`eframe` API bylo pouzito podle nejlepsiho vedomi
//! pro verzi ~0.29, ale drobne nazvy metod se mohou po prvnim buildu
//! lisit.
//!
//! Uvodni splash animace (logo + verze/autor) uz NENI samostatne okno
//! (byvaly crate `termx-splash`) - bezi primo v tomhle stejnem okne jako
//! prvni faze [`app::TermxApp`] pred zamcenou obrazovkou, viz `splash`
//! modul a duvod v jeho hlavicce.
//!
//! Tab spojeni (`TabKind::Connection`) ma vestaveny emulator terminalu
//! (`alacritty_terminal`, viz `terminal.rs`) napojeny primo na
//! `termx_ssh::spawn_ssh_session` (ne na obecny `termx_core::ProtocolModule::run`,
//! ktery pocita s puvodnim prevzetim stdin/stdout - viz poznamka tam) -
//! zatim jen pro SSH, dalsi protokoly (Serial/FTP/...) budou potrebovat
//! obdobnou specializovanou cestu, az pribudou.
//!
//! Uzivatelska nastaveni (`app::AppSettings` - zatim jen automaticke
//! obnoveni ztraceneho SSH spojeni) se ukladaji pres bezny eframe
//! perzistentni ulozny prostor (`cc.storage` predane sem do
//! `app::TermxApp::new`) - stejny mechanismus, jaky uz drive vyuziva
//! `persist_window` nize pro polohu/velikost okna, jen s vlastnim
//! klicem (viz `app::SETTINGS_STORAGE_KEY`).

mod app;
mod i18n;
#[cfg(target_os = "linux")]
mod linux_desktop;
mod sftp_browser;
mod splash;
mod terminal;
mod theme;

use std::path::PathBuf;

use termx_core::ModuleRegistry;

const ICON_BYTES: &[u8] = include_bytes!("../../../assets/icons/hicolor/128x128/apps/term-ix.png");

/// Normalni pracovni velikost hlavniho okna aplikace (mimo pocatecni
/// splash fazi, viz `show_splash` u [`run_app`]) - `pub(crate)`, protoze
/// `app::resize_to_main_window` ji potrebuje pri prechodu ze splashe na
/// zamcenou obrazovku (viz tam), aby se okno po male splash velikosti
/// (`splash::WINDOW_SIZE`) zvetsilo presne zpatky na tuto velikost.
pub(crate) const MAIN_WINDOW_SIZE: [f32; 2] = [1150.0, 720.0];

/// Normalni minimalni velikost hlavniho okna (mimo pocatecni splash fazi) -
/// oddeleno od `MAIN_WINDOW_SIZE` jako samostatna konstanta, protoze behem
/// splash faze se docasne pouzije jina, mnohem mensi minimalni velikost
/// (`splash::WINDOW_SIZE`, viz `run_app` nize) - jinak by vynucene
/// minimum 760x440 pri startu proste prebilo pozadovanou malou pocatecni
/// velikost okna a splash by se presto zobrazil velky (presne tato chyba
/// puvodne zpusobila zpetnou vazbu "splash okno je velke"). `app::resize_to_main_window`
/// tuto hodnotu obnovi zpet po dobehnuti animace.
pub(crate) const MAIN_MIN_WINDOW_SIZE: [f32; 2] = [760.0, 440.0];

/// Spusti hlavni okno aplikace. Eframe si bezi ve vlastni (blokujici)
/// smycce na aktualnim vlakne - volat primo z `main()`, ne zevnitr
/// tokio `block_on`.
///
/// Na rozdil od puvodni verze uz sem `main()` nepreda uz odemceny
/// `Vault` - jen cestu k souboru trezoru (`vault_path`). Odemceni (nebo
/// nastaveni hesla pro novy trezor) resi az samotne GUI na uvodni
/// obrazovce po otevreni okna.
///
/// `skip_update_check` odpovida CLI prepinaci `--no-update` (`main.rs`) -
/// preda se dal do [`app::TermxApp::new`], ktere ho pouzije pro
/// pocatecni stav kontroly aktualizace v Home tabu (viz tam).
///
/// `show_splash` odpovida (obracene) CLI prepinaci `--no-splash`
/// (`main.rs`) - kdyz `true`, `TermxApp` pri startu nejdriv na par
/// sekund zobrazi uvodni animaci (viz `splash` modul a `app::LockState::Splash`)
/// primo v tomhle stejnem okne, driv nez prejde na zamcenou obrazovku.
pub fn run_app(vault_path: PathBuf, registry: ModuleRegistry, skip_update_check: bool, show_splash: bool) -> anyhow::Result<()> {
    // Sebeinstalace .desktop souboru + ikon do XDG slozek uzivatele -
    // zpetna vazba "nemáme nikde ikonku aplikace ani v liště ani na
    // ploše" (viz `linux_desktop`). Na jinych platformach netreba - tam
    // uz ikonu resi bud instalator (Windows, zabudovana primo do .exe),
    // nebo .app balicek (macOS).
    #[cfg(target_os = "linux")]
    linux_desktop::install();

    // Kdyz se ma zobrazit uvodni splash animace (`show_splash`), okno
    // pri startu zamerne otevreme rovnou v jeji male velikosti
    // (`splash::WINDOW_SIZE`) misto plne pracovni velikosti aplikace
    // (`MAIN_WINDOW_SIZE`) - zpetna vazba "splash okno bychom meli
    // zmensit" (drive, nez presel na tohle spolecne okno, byval splash
    // samostatne male okno, viz `splash.rs`). Po dobehnuti animace se
    // okno zvetsi zpatky na `MAIN_WINDOW_SIZE` (viz
    // `app::resize_to_main_window`, volano z `app::TermxApp::update`).
    // Minimalni velikost MUSI behem splash faze klesnout spolu s
    // pocatecni velikosti (`initial_size` nize) - `with_min_inner_size`
    // by jinak (viz komentar u `MAIN_MIN_WINDOW_SIZE`) vynutil aspon
    // 760x440 uz na prvnim snimku, bez ohledu na to, jak malou
    // `initial_size` pozadujeme.
    //
    // POZNAMKA K WAYLANDU: umisteni/velikost tohohle maleho splash okna
    // se na Waylandu (pozorovano na Cinnamon/Muffin) chova nespolehlive/
    // nekonzistentne navzdory ruznym vyzkousenym pristupum (viz historie
    // v `app.rs` u `PendingResize::ToSplash`/`shrink_to_splash_window`) -
    // zamerne se to dal neresi a nechava se to spolecne (jednoduche)
    // chovani pro vsechny platformy, na Waylandu tedy splash muze
    // pusobit "podivne", dokud se situace nezlepsi na strane
    // Cinnamonu/Muffinu.
    let (initial_size, initial_min_size): ([f32; 2], [f32; 2]) =
        if show_splash { (splash::WINDOW_SIZE, splash::WINDOW_SIZE) } else { (MAIN_WINDOW_SIZE, MAIN_MIN_WINDOW_SIZE) };

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(initial_size)
            .with_min_inner_size(initial_min_size)
            // Kdyz se ma zobrazit splash (`show_splash`), okno pri
            // vytvoreni ZUSTANE SCHOVANE (neviditelne) - `app.rs` ho
            // po `WINDOW_REVEAL_DELAY_FRAMES` snimcich, az uz jsou
            // pozadavky na zmenseni/vycentrovani (viz
            // `shrink_to_splash_window`) skutecne odeslane, zase
            // zviditelni (`egui::ViewportCommand::Visible(true)`, viz
            // `app::TermxApp::update`/`window_reveal_countdown`).
            // DUVOD: `persist_window` (nize) na X11 chvilku obnovuje
            // POLOHU z minuleho spusteni, driv nez ji nas vlastni kod
            // prepise na vycentrovanou - bez schovani okna byl kratce
            // viditelny "zablesk" na te stare pozici, nez se okno
            // presunulo doprostred. OVERENO uzivatelem na X11/Cinnamonu:
            // se schovanim a dostatecne dlouhym `WINDOW_REVEAL_DELAY_FRAMES`
            // uz zablesk videt neni. Kdyz `show_splash == false`, okno
            // je viditelne rovnou (zadne dalsi prepocitavani polohy v
            // `app.rs` v tom pripade neprobiha).
            .with_visible(!show_splash)
            .with_icon(load_icon())
            // Wayland (napr. GNOME): bez shodneho app_id se ikonka v
            // panelu/doku nezobrazi vubec, i kdyz jsou `.desktop` soubor
            // i ikony spravne nainstalovane (viz `linux_desktop`, jehoz
            // `StartupWMClass`/`Icon` pouzivaji stejny nazev "term-ix").
            // POZNAMKA: podle dokumentace `ViewportBuilder::with_app_id`
            // tohle u eframe zaroven urcuje umisteni uloziste pro
            // `cc.storage` (`persist_window`/`AppSettings` nize) - starsi
            // Linuxovi uzivatele tak jednorazove prijdou o ulozenou
            // polohu okna/nastaveni (jazyk, tema, ...), nic
            // bezpecnostne/datove kriticke (trezor je samostatny soubor,
            // viz `vault_path`) - prijatelna cena za funkcni ikonu.
            .with_app_id("term-ix"),
        // Pri prvnim spusteni (kdyz jeste neni co obnovit) se okno
        // vycentruje na obrazovce. `persist_window` pak pri kazdem
        // dalsim spusteni (diky cargo feature "persistence" u eframe)
        // obnovi presne tu polohu a velikost, ve ktere uzivatel okno
        // naposledy zavrel - na X11/Windows spolehlive; na Waylandu se
        // to (spolu s dalsimi souvisejicimi drobnostmi - viz
        // `app::AppSettings::window_size`/`window_pos`/`is_wayland_session`)
        // chova hur, ale zamerne se to prozatim neresi zvlast (viz
        // poznamka u `initial_size` vyse).
        centered: true,
        persist_window: true,
        ..Default::default()
    };

    eframe::run_native(
        "Term-IX",
        native_options,
        Box::new(move |cc| {
            // Font pro splash animaci (viz `splash::install_font`) se musi
            // vlozit driv, nez se vykresli prvni snimek - jinak by prvni
            // snimek splashe (pokud se vubec zobrazuje, viz `show_splash`)
            // pouzil vychozi font misto DejaVu Sans Mono Bold.
            splash::install_font(&cc.egui_ctx);
            let app = app::TermxApp::new(vault_path, registry, cc.storage, skip_update_check, show_splash);
            // Tema se aplikuje az PO nacteni `AppSettings` (uvnitr
            // `TermxApp::new`) - viz `TermxApp::initial_theme` - aby se
            // uz od prvniho snimku pouzilo ulozene uzivatelovo tema, ne
            // vzdy jen vychozi `Theme::Terminal`.
            theme::apply(&cc.egui_ctx, app.initial_theme());
            // `persist_window` (viz `native_options` vyse) uz sam obnovi
            // ulozenou polohu/velikost okna, ale ne jeho maximalizaci -
            // tu obnovime rucne podle naposledy ulozene hodnoty (viz
            // `AppSettings::window_maximized`). Zamerne NEreseno pro
            // minimalizaci - ta se vubec nesleduje ani neuklada.
            //
            // Kdyz se ma zobrazit splash (`show_splash`), maximalizaci
            // ZDE schvalne NEprovadime - okno je v tuto chvili jeste v
            // male splash velikosti (viz `initial_size` vyse) a okamzita
            // maximalizace by ho rovnou roztahla na celou obrazovku, cimz
            // by cely smysl mensiho splash okna zmizel. Misto toho se
            // maximalizace (pripadne) provede az PO dobehnuti animace,
            // viz `app::TermxApp::update`/`app::resize_to_main_window`.
            if !show_splash && app.wants_maximized() {
                cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            }
            // Bez tohoto nemusi cerstve vytvorene okno na startu dostat
            // OS-level klavesovy fokus (potvrzeno na Linuxu/X11, typicky
            // kdyz se binarka spousti z terminalu - `cargo run`) - window
            // manager novemu oknu fokus jednoduse sam neda.
            // `LockScreen::focus_attempts`/`splash::render` (viz `app.rs`
            // a `splash.rs`) samy o sobe resi jen fokus WIDGETU uvnitr
            // egui, ne uz OS-level fokus cele aplikace/okna - to prvni bez
            // druheho k nicemu neni, klavesove udalosti se do aplikace
            // vubec nedostanou. Explicitni pozadavek hned po vytvoreni
            // okna (predtim, nez se vykresli prvni snimek) tohle resi
            // primo. PUVODNE (pred prechodem na jedno spolecne okno pro
            // splash i zbytek aplikace - viz `splash.rs`) tenhle problem
            // zpusobovalo hlavne to, ze splash bezel v samostatnem
            // minifb/X11 okne oddelenem od tohohle - i po tomto presunu
            // volani zustava jako dalsi pojistka (viz i opakovane pokusy
            // v `LockScreen::focus_attempts`/`splash::render`).
            cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("nepodarilo se spustit graficke rozhrani: {e}"))
}

fn load_icon() -> egui::IconData {
    match image::load_from_memory(ICON_BYTES) {
        Ok(img) => {
            let img = img.to_rgba8();
            let (width, height) = img.dimensions();
            egui::IconData {
                rgba: img.into_raw(),
                width,
                height,
            }
        }
        Err(e) => {
            tracing::debug!("nepodarilo se nacist ikonu aplikace: {e}");
            egui::IconData::default()
        }
    }
}
