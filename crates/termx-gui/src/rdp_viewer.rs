//! RDP prohlizec vzdalene plochy (viz `termx-rdp`) - na rozdil od
//! `sftp_browser.rs`/`ftp_browser.rs` se vlastni plocha NEVYKRESLUJE
//! primo v tabu (`TabKind::Rdp`, viz `app.rs::render_rdp`): tab jen
//! ukazuje stavovou kartu ([`RdpBrowser::render`]), samotna plocha bezi
//! v SAMOSTATNEM OS okne pres egui "deferred viewport"
//! (`egui::Context::show_viewport_deferred`, viz [`RdpBrowser::show_window`]).
//!
//! DUVOD (viz sekce RDP v `claude/roadmap-ideas.md`): uzivatel chtel jit
//! oknem se vzdalenou plochou nezavisle maximalizovat/minimalizovat, s
//! vlastni ovladaci listou (fullscreen/Ctrl+Alt+Del/odpojit) nahore.
//!
//! VERZE V3 (2026-09-29) - zmeny oproti V2:
//! - Okno se kresli z `MainApp::update` pro VSECHNY RDP relace kazdy
//!   snimek (`show_window`), ne jen z aktivniho RDP tabu - egui zavira
//!   deferred viewport, jakmile se v nekterem snimku `show_viewport_deferred`
//!   nezavola, takze v V2 okno zmizelo pri prepnuti na jiny tab.
//! - Veskery stav okna je ve sdilenem `Arc<WindowShared>` - uzaver
//!   deferred viewportu (musi byt `Fn + Send + Sync`) si sam prebira novy
//!   obraz ze `SharedFrame` a nahrava jen zmenenou oblast
//!   (`TextureHandle::set_partial`). RDP vlakno ho budi primo
//!   `request_repaint_of(viewport)` - okno plochy se tak prekresluje
//!   nezavisle na hlavnim okne.
//! - Obraz se zobrazuje 1:1 (ostry), a jen pokud je vetsi nez okno, zmensi
//!   se se zachovanim pomeru stran (V2 ho vzdy roztahovala na cele okno).
//! - Klavesnice: psany text jde jako Unicode (`Event::Text`, podle
//!   MISTNIHO rozlozeni - funguji ceske znaky i AltGr kombinace bez ohledu
//!   na rozlozeni nastavene na serveru), scancody jen pro neznakove klavesy
//!   (Enter, sipky, F1-F12...) a pro zkratky s Ctrl/Alt. Autorepeat se
//!   posila. Ctrl+C/X/V (egui je prevadi na `Event::Copy/Cut/Paste`) se
//!   prevadi zpet na klavesy. Pri ztrate fokusu okna se uvolni vsechny
//!   drzene klavesy (`RdpCommand::ReleaseAll`).
//!
//! Schranka (clipboard sync) je ZAMERNE VYNECHANA (priorita byla
//! "nejdřív funkční spojení").

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Key, Modifiers};
use termx_core::{AuthMethod, Session};
use termx_rdp::{spawn_rdp_session, RdpCommand, RdpConnectParams, RdpEvent, RdpMouseButton, RdpWake, SharedFrame};

use crate::i18n::{self, Lang};
use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RdpState {
    Connecting,
    Connected,
    Disconnected,
}

/// Vychozi velikost okna plochy (logicke body egui) pri prvnim otevreni.
const DEFAULT_WINDOW_SIZE: egui::Vec2 = egui::vec2(1280.0, 832.0);
/// Priblizna vyska horni listy okna - jen pro odhad POCATECNIHO rozliseni
/// posilaneho pri pripojeni (presna hodnota se doladi zmenou rozliseni).
const TOOLBAR_HEIGHT_ESTIMATE: f32 = 32.0;

/// Minimalni doba, po kterou se velikost plochy v okne nesmi zmenit, nez
/// se serveru posle `RdpCommand::ResizeDesktop` - tazeni za roh okna
/// generuje desitky zmen za sekundu.
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(500);

pub struct RdpBrowser {
    command_tx: Sender<RdpCommand>,
    event_rx: Receiver<RdpEvent>,
    state: RdpState,
    error: Option<String>,
    desktop_size: [u16; 2],
    /// ID viewportu okna plochy - stejne po celou dobu zivota relace.
    viewport_id: egui::ViewportId,
    window: Arc<WindowShared>,
}

/// Stav sdileny s uzaverem deferred viewportu (viz header komentar).
struct WindowShared {
    command_tx: Mutex<Sender<RdpCommand>>,
    frame: Arc<Mutex<SharedFrame>>,
    /// Okno je otevrene - krizek OS okna ho zavre BEZ ukonceni relace,
    /// stavova karta v tabu pak nabidne "Znovu otevřít okno plochy".
    open: AtomicBool,
    ui: Mutex<WindowUi>,
}

#[derive(Default)]
struct WindowUi {
    texture: Option<egui::TextureHandle>,
    /// `SharedFrame::size_generation`, pro kterou byla `texture` vytvorena.
    texture_generation: u64,
    /// Modifikatory (Shift/Ctrl/Alt) naposledy odeslane serveru.
    sent_modifiers: Modifiers,
    /// Znakove klavesy odeslane jako scancode (zkratky s Ctrl/Alt) - jejich
    /// uvolneni se musi poslat taky jako scancode.
    scancode_keys_down: HashSet<Key>,
    focused: bool,
    resize: ResizeDebounce,
}

#[derive(Default)]
struct ResizeDebounce {
    last_seen: Option<[u16; 2]>,
    changed_at: Option<Instant>,
    last_sent: Option<[u16; 2]>,
}

impl WindowShared {
    fn send(&self, cmd: RdpCommand) {
        let _ = self.command_tx.lock().unwrap().send(cmd);
    }
}

impl RdpBrowser {
    pub fn new(session: &Session, ctx: &egui::Context) -> Self {
        let (username, password) = match &session.auth {
            AuthMethod::Password { username, password } => (username.clone(), password.clone()),
            _ => (String::new(), String::new()),
        };
        let domain = session.rdp_domain.clone().filter(|d| !d.trim().is_empty());
        let viewport_id = egui::ViewportId::from_hash_of(("rdp", session.id));

        let ppp = ctx.pixels_per_point();
        let initial_width = (DEFAULT_WINDOW_SIZE.x * ppp).round() as u16;
        let initial_height = ((DEFAULT_WINDOW_SIZE.y - TOOLBAR_HEIGHT_ESTIMATE) * ppp).round() as u16;

        let waker_ctx = ctx.clone();
        let waker: termx_rdp::RdpWaker = Arc::new(move |wake| match wake {
            RdpWake::Frame => waker_ctx.request_repaint_of(viewport_id),
            RdpWake::Event => {
                waker_ctx.request_repaint_of(egui::ViewportId::ROOT);
                waker_ctx.request_repaint_of(viewport_id);
            }
        });

        let handle = spawn_rdp_session(
            RdpConnectParams {
                host: session.host.clone(),
                port: session.port,
                username,
                password,
                domain,
                initial_width,
                initial_height,
            },
            waker,
        );

        let window = Arc::new(WindowShared {
            command_tx: Mutex::new(handle.command_tx.clone()),
            frame: handle.frame,
            open: AtomicBool::new(true),
            ui: Mutex::new(WindowUi::default()),
        });

        Self {
            command_tx: handle.command_tx,
            event_rx: handle.event_rx,
            state: RdpState::Connecting,
            error: None,
            desktop_size: [initial_width, initial_height],
            viewport_id,
            window,
        }
    }

    pub fn state(&self) -> RdpState {
        self.state
    }

    /// Zpracuje cekajici udalosti z bezici relace.
    fn poll(&mut self) {
        loop {
            match self.event_rx.try_recv() {
                Ok(RdpEvent::Connected { width, height }) => {
                    self.state = RdpState::Connected;
                    self.desktop_size = [width, height];
                }
                Ok(RdpEvent::Resized { width, height }) => self.desktop_size = [width, height],
                Ok(RdpEvent::Error(e)) => self.error = Some(e),
                Ok(RdpEvent::Disconnected) => self.state = RdpState::Disconnected,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.state = RdpState::Disconnected;
                    break;
                }
            }
        }
    }

    /// Stavova karta v tabu (`TabKind::Rdp`, viz `app.rs::render_rdp`).
    pub fn render(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, session_name: &str, lang: Lang) {
        self.poll();
        let tr = i18n::t(lang);
        let window_open = self.window.open.load(Ordering::Relaxed);

        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.heading(session_name);
            ui.add_space(10.0);
            match self.state {
                RdpState::Connecting => {
                    ui.add(egui::Spinner::new());
                    ui.label(tr.rdp_connecting);
                }
                RdpState::Connected => {
                    ui.colored_label(theme::ACCENT, tr.rdp_connected);
                    ui.label(format!("{}×{}", self.desktop_size[0], self.desktop_size[1]));
                }
                RdpState::Disconnected => {
                    ui.colored_label(theme::DANGER, tr.rdp_disconnected);
                }
            }
            if let Some(err) = &self.error {
                ui.add_space(6.0);
                ui.colored_label(theme::DANGER, err);
            }
            ui.add_space(16.0);
            if self.state != RdpState::Disconnected {
                if window_open {
                    if ui.button(tr.rdp_focus_window).clicked() {
                        ctx.send_viewport_cmd_to(self.viewport_id, egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd_to(self.viewport_id, egui::ViewportCommand::Focus);
                    }
                } else if ui.button(tr.rdp_reopen_window).clicked() {
                    self.window.open.store(true, Ordering::Relaxed);
                }
            }
        });
    }

    /// Okno se vzdalenou plochou - vola se z `MainApp::update` pro KAZDOU
    /// RDP relaci v KAZDEM snimku (viz header komentar, proc ne z tabu).
    pub fn show_window(&mut self, ctx: &egui::Context, session_name: &str, lang: Lang) {
        self.poll();
        if self.state == RdpState::Disconnected || !self.window.open.load(Ordering::Relaxed) {
            return;
        }
        let tr = i18n::t(lang);
        let shared = self.window.clone();
        let label_fullscreen = tr.rdp_toolbar_fullscreen;
        let label_cad = tr.rdp_toolbar_ctrl_alt_del;
        let label_disconnect = tr.rdp_toolbar_disconnect;

        let builder = egui::ViewportBuilder::default()
            .with_title(format!("{session_name} - RDP"))
            .with_inner_size(DEFAULT_WINDOW_SIZE)
            .with_min_inner_size(egui::vec2(320.0, 240.0))
            // Stejne app_id + ikona jako hlavni okno (viz `lib.rs::run_app`/
            // `linux_desktop`) - jinak by okno plochy na Waylandu melo
            // genericke "W" misto ikony Term-IX.
            .with_app_id("term-ix")
            .with_icon(crate::app_icon());

        ctx.show_viewport_deferred(self.viewport_id, builder, move |ctx, _class| {
            window_ui(ctx, &shared, label_fullscreen, label_cad, label_disconnect);
        });
    }
}

impl Drop for RdpBrowser {
    fn drop(&mut self) {
        // Zavreni tabu = konec relace (RDP vlakno skonci i samo, jakmile
        // zanikne posledni `Sender`, ale kopie v `WindowShared` muze jeste
        // chvili zit v uzaveru deferred viewportu).
        let _ = self.command_tx.send(RdpCommand::Disconnect);
    }
}

/// Obsah okna plochy (bezi v uzaveru deferred viewportu).
fn window_ui(ctx: &egui::Context, shared: &WindowShared, label_fullscreen: &str, label_cad: &str, label_disconnect: &str) {
    if ctx.input(|i| i.viewport().close_requested()) {
        // Krizek = jen schovat okno, relace bezi dal (viz `WindowShared::open`).
        shared.send(RdpCommand::ReleaseAll);
        shared.open.store(false, Ordering::Relaxed);
        ctx.request_repaint_of(egui::ViewportId::ROOT);
        return;
    }

    let mut ui_state = shared.ui.lock().unwrap();

    // Ztrata fokusu okna (Alt+Tab, klik jinam) - uvolnit drzene klavesy,
    // jinak by na serveru "visel" treba Alt.
    let focused = ctx.input(|i| i.focused);
    if ui_state.focused && !focused {
        shared.send(RdpCommand::ReleaseAll);
        ui_state.sent_modifiers = Modifiers::default();
        ui_state.scancode_keys_down.clear();
    }
    ui_state.focused = focused;

    egui::TopBottomPanel::top("rdp_toolbar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.button(label_fullscreen).clicked() {
                let is_fullscreen = ctx.input(|i| i.viewport().fullscreen).unwrap_or(false);
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!is_fullscreen));
            }
            if ui.button(label_cad).clicked() {
                send_ctrl_alt_del(shared);
            }
            if ui.button(label_disconnect).clicked() {
                shared.send(RdpCommand::Disconnect);
                shared.open.store(false, Ordering::Relaxed);
                ctx.request_repaint_of(egui::ViewportId::ROOT);
            }
        });
    });

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(egui::Color32::BLACK))
        .show(ctx, |ui| {
            let available = ui.available_rect_before_wrap();
            let ppp = ctx.pixels_per_point();
            poll_desktop_resize(available.size(), ppp, ctx, &mut ui_state.resize, shared);

            let desktop_size = update_texture(ctx, shared, &mut ui_state);
            let Some(texture) = ui_state.texture.clone() else {
                ui.centered_and_justified(|ui| ui.spinner());
                return;
            };
            let [w, h] = desktop_size;
            if w == 0 || h == 0 {
                return;
            }

            // 1:1 (fyzicke pixely), jen kdyz se nevejde, zmensit se
            // zachovanim pomeru stran. Zarovnano na cele pixely = ostre.
            let native = egui::vec2(f32::from(w) / ppp, f32::from(h) / ppp);
            let scale = (available.width() / native.x).min(available.height() / native.y).min(1.0);
            let size = native * scale;
            let min = available.center() - size / 2.0;
            let min = egui::pos2((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp);
            let rect = egui::Rect::from_min_size(min, size);

            let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
            ui.painter().image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            if response.hovered() {
                ctx.set_cursor_icon(egui::CursorIcon::Default);
            }
            handle_input(ui, rect, desktop_size, shared, &mut ui_state);
        });
}

/// Prenese novy obraz ze `SharedFrame` do textury (jen zmenenou oblast) a
/// vrati aktualni rozmer plochy.
fn update_texture(ctx: &egui::Context, shared: &WindowShared, ui_state: &mut WindowUi) -> [u16; 2] {
    let mut frame = shared.frame.lock().unwrap();
    let size = [frame.width, frame.height];
    if frame.width == 0 || frame.height == 0 {
        return size;
    }
    let (w, h) = (usize::from(frame.width), usize::from(frame.height));

    if ui_state.texture.is_none() || ui_state.texture_generation != frame.size_generation {
        let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &frame.rgba);
        let options = egui::TextureOptions::LINEAR;
        match &mut ui_state.texture {
            Some(tex) => tex.set(image, options),
            None => ui_state.texture = Some(ctx.load_texture("rdp-framebuffer", image, options)),
        }
        ui_state.texture_generation = frame.size_generation;
        frame.dirty = None;
        return size;
    }

    if let (Some(dirty), Some(tex)) = (frame.dirty.take(), &mut ui_state.texture) {
        let (l, t) = (usize::from(dirty.left), usize::from(dirty.top));
        let (r, b) = (usize::from(dirty.right).min(w - 1), usize::from(dirty.bottom).min(h - 1));
        if l <= r && t <= b {
            let (dw, dh) = (r - l + 1, b - t + 1);
            let mut pixels = Vec::with_capacity(dw * dh * 4);
            for y in t..=b {
                let row = y * w * 4;
                pixels.extend_from_slice(&frame.rgba[row + l * 4..row + (r + 1) * 4]);
            }
            let image = egui::ColorImage::from_rgba_unmultiplied([dw, dh], &pixels);
            tex.set_partial([l, t], image, egui::TextureOptions::LINEAR);
        }
    }
    size
}

/// Po odezneni `RESIZE_DEBOUNCE` posle serveru pozadavek na rozliseni
/// odpovidajici aktualni velikosti okna (fyzicke pixely). Server ho
/// provede az bude Display Control kanal pripraveny (resi `termx-rdp`).
fn poll_desktop_resize(available: egui::Vec2, ppp: f32, ctx: &egui::Context, state: &mut ResizeDebounce, shared: &WindowShared) {
    let current = [(available.x * ppp).round().max(0.0) as u16, (available.y * ppp).round().max(0.0) as u16];
    // Minimalizace/sbaleni - MS-RDPEDISP stejne pod 200 px nedovoli.
    if current[0] < 200 || current[1] < 200 {
        return;
    }
    if state.last_seen != Some(current) {
        state.last_seen = Some(current);
        state.changed_at = Some(Instant::now());
    }
    let Some(changed_at) = state.changed_at else {
        return;
    };
    let elapsed = changed_at.elapsed();
    if elapsed < RESIZE_DEBOUNCE {
        // Bez tohohle by se po dotazeni okna uz nic neprekreslilo a
        // pozadavek by se nikdy neodeslal.
        ctx.request_repaint_after(RESIZE_DEBOUNCE - elapsed);
        return;
    }
    if state.last_sent != Some(current) {
        let scale_percent = ((ppp * 100.0).round() as u32).clamp(100, 500);
        shared.send(RdpCommand::ResizeDesktop { width: current[0], height: current[1], scale_percent: Some(scale_percent) });
        state.last_sent = Some(current);
    }
}

/// Ctrl+Alt+Del z tlacitka v liste - klavesovou zkratku by odchytil uz
/// mistni OS.
fn send_ctrl_alt_del(shared: &WindowShared) {
    // Scancode Set 1: Ctrl(leve)=0x1D, Alt(leve)=0x38, Delete(extended)=0x53.
    for (extended, code, pressed) in
        [(false, 0x1D, true), (false, 0x38, true), (true, 0x53, true), (true, 0x53, false), (false, 0x38, false), (false, 0x1D, false)]
    {
        shared.send(RdpCommand::Key { extended, code, pressed });
    }
}

/// Mys + klavesnice nad obrazem plochy (`rect` = skutecne vykresleny
/// obdelnik obrazu, `desktop_size` = rozmer plochy serveru).
fn handle_input(ui: &egui::Ui, rect: egui::Rect, desktop_size: [u16; 2], shared: &WindowShared, st: &mut WindowUi) {
    let [desktop_w, desktop_h] = desktop_size;
    let to_rdp_coords = |pos: egui::Pos2| -> (u16, u16) {
        let rel_x = ((pos.x - rect.min.x) / rect.width().max(1.0)).clamp(0.0, 1.0);
        let rel_y = ((pos.y - rect.min.y) / rect.height().max(1.0)).clamp(0.0, 1.0);
        let x = (rel_x * f32::from(desktop_w)).floor().min(f32::from(desktop_w.saturating_sub(1)));
        let y = (rel_y * f32::from(desktop_h)).floor().min(f32::from(desktop_h.saturating_sub(1)));
        (x as u16, y as u16)
    };

    let (events, frame_modifiers) = ui.input(|i| (i.events.clone(), i.modifiers));
    // AltGr (hlavne na Windows hostiteli hlasi Ctrl+Alt) - pokud v tomto
    // snimku prisel text, znakova klavesa byla psani, ne zkratka.
    let frame_has_text = events.iter().any(|e| matches!(e, egui::Event::Text(t) if !t.is_empty()));

    for event in &events {
        match event {
            egui::Event::PointerMoved(pos) if rect.contains(*pos) => {
                let (x, y) = to_rdp_coords(*pos);
                shared.send(RdpCommand::MouseMove { x, y });
            }
            egui::Event::PointerButton { pos, button, pressed, modifiers } => {
                // Stisk jen nad obrazem, uvolneni vzdy (tazeni mimo okraj).
                if *pressed && !rect.contains(*pos) {
                    continue;
                }
                sync_modifiers(shared, st, *modifiers);
                let (x, y) = to_rdp_coords(*pos);
                shared.send(RdpCommand::MouseMove { x, y });
                let button = match button {
                    egui::PointerButton::Primary => RdpMouseButton::Left,
                    egui::PointerButton::Secondary => RdpMouseButton::Right,
                    egui::PointerButton::Middle => RdpMouseButton::Middle,
                    egui::PointerButton::Extra1 => RdpMouseButton::X1,
                    egui::PointerButton::Extra2 => RdpMouseButton::X2,
                };
                shared.send(RdpCommand::MouseButton { button, pressed: *pressed });
            }
            egui::Event::MouseWheel { unit, delta, modifiers } => {
                if !ui.rect_contains_pointer(rect) {
                    continue;
                }
                sync_modifiers(shared, st, *modifiers);
                // 120 = jeden "zub" kolecka (WHEEL_DELTA).
                let per_unit = match unit {
                    egui::MouseWheelUnit::Line => 120.0,
                    egui::MouseWheelUnit::Page => 360.0,
                    egui::MouseWheelUnit::Point => 3.0,
                };
                for (vertical, amount) in [(true, delta.y), (false, -delta.x)] {
                    let units = (amount * per_unit).round().clamp(-255.0, 255.0) as i16;
                    if units != 0 {
                        shared.send(RdpCommand::MouseWheel { vertical, rotation_units: units });
                    }
                }
            }
            egui::Event::Key { key, physical_key, pressed, modifiers, .. } => {
                let key = physical_key.unwrap_or(*key);
                sync_modifiers(shared, st, *modifiers);
                let Some((extended, code)) = key_to_scancode(key) else {
                    continue;
                };
                if is_text_key(key) {
                    // Znakove klavesy jdou jako Unicode text (viz header
                    // komentar) - scancode jen pro zkratky s Ctrl/Alt.
                    if *pressed {
                        let shortcut = (modifiers.ctrl || modifiers.alt || modifiers.command) && !frame_has_text;
                        if !shortcut {
                            continue;
                        }
                        st.scancode_keys_down.insert(key);
                    } else if !st.scancode_keys_down.remove(&key) {
                        continue;
                    }
                }
                // `repeat` (drzena klavesa) se posila jako dalsi stisk -
                // autorepeat na serveru generuje klient.
                shared.send(RdpCommand::Key { extended, code, pressed: *pressed });
            }
            egui::Event::Text(text) => {
                // Mezera jde scancodem (viz `is_text_key`), ne podruhe jako text.
                for ch in text.chars().filter(|c| !c.is_control() && *c != ' ') {
                    shared.send(RdpCommand::Unicode { ch, pressed: true });
                    shared.send(RdpCommand::Unicode { ch, pressed: false });
                }
            }
            // egui-winit prevadi Ctrl+C/X/V (a Ctrl/Shift+Insert,
            // Shift+Delete) na tyto udalosti MISTO `Event::Key` stisku -
            // uvolneni pak prijde normalne jako `Event::Key`.
            egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_) => {
                sync_modifiers(shared, st, frame_modifiers);
                let shift_only = frame_modifiers.shift && !frame_modifiers.ctrl;
                let key = match event {
                    egui::Event::Copy if !frame_modifiers.ctrl => continue,
                    egui::Event::Copy => Key::C,
                    egui::Event::Cut if shift_only => Key::Delete,
                    egui::Event::Cut => Key::X,
                    _ if shift_only => Key::Insert,
                    _ => Key::V,
                };
                if let Some((extended, code)) = key_to_scancode(key) {
                    if is_text_key(key) {
                        st.scancode_keys_down.insert(key);
                    }
                    shared.send(RdpCommand::Key { extended, code, pressed: true });
                }
            }
            _ => {}
        }
    }

    if st.focused {
        sync_modifiers(shared, st, frame_modifiers);
    }
}

/// egui nevydava Shift/Ctrl/Alt jako `Event::Key` - zmena oproti naposledy
/// odeslanemu stavu se prelozi na stisk/uvolneni LEVE varianty klavesy
/// (egui levou/pravou nerozlisuje).
fn sync_modifiers(shared: &WindowShared, st: &mut WindowUi, new: Modifiers) {
    const SHIFT: (bool, u8) = (false, 0x2A);
    const CTRL: (bool, u8) = (false, 0x1D);
    const ALT: (bool, u8) = (false, 0x38);
    let old = st.sent_modifiers;
    for (was, is, (extended, code)) in [(old.shift, new.shift, SHIFT), (old.ctrl, new.ctrl, CTRL), (old.alt, new.alt, ALT)] {
        if was != is {
            shared.send(RdpCommand::Key { extended, code, pressed: is });
        }
    }
    st.sent_modifiers = new;
}

/// Klavesy, ktere pri psani produkuji znak (jdou jako Unicode `Event::Text`).
/// Mezernik zamerne NE - Windows na VK_SPACE reaguje u tlacitek/checkboxu,
/// Unicode udalost by je neaktivovala.
fn is_text_key(key: Key) -> bool {
    use Key::*;
    matches!(
        key,
        A | B | C | D | E | F | G | H | I | J | K | L | M | N | O | P | Q | R | S | T | U | V | W | X | Y | Z
            | Num0 | Num1 | Num2 | Num3 | Num4 | Num5 | Num6 | Num7 | Num8 | Num9
            | Minus | Equals | Plus | OpenBracket | CloseBracket | Semicolon | Colon | Quote | Backtick
            | Backslash | Pipe | Comma | Period | Slash | Questionmark
    )
}

/// `egui::Key` (fyzicka pozice na US klavesnici) -> PC/AT scancode Set 1
/// jako `(extended, code)`. Nepokryta klavesa (`None`) se ignoruje.
fn key_to_scancode(key: Key) -> Option<(bool, u8)> {
    use Key::*;
    Some(match key {
        Escape => (false, 0x01),
        Num1 => (false, 0x02),
        Num2 => (false, 0x03),
        Num3 => (false, 0x04),
        Num4 => (false, 0x05),
        Num5 => (false, 0x06),
        Num6 => (false, 0x07),
        Num7 => (false, 0x08),
        Num8 => (false, 0x09),
        Num9 => (false, 0x0A),
        Num0 => (false, 0x0B),
        Minus => (false, 0x0C),
        Equals | Plus => (false, 0x0D),
        Backspace => (false, 0x0E),
        Tab => (false, 0x0F),
        Q => (false, 0x10),
        W => (false, 0x11),
        E => (false, 0x12),
        R => (false, 0x13),
        T => (false, 0x14),
        Y => (false, 0x15),
        U => (false, 0x16),
        I => (false, 0x17),
        O => (false, 0x18),
        P => (false, 0x19),
        OpenBracket => (false, 0x1A),
        CloseBracket => (false, 0x1B),
        Enter => (false, 0x1C),
        A => (false, 0x1E),
        S => (false, 0x1F),
        D => (false, 0x20),
        F => (false, 0x21),
        G => (false, 0x22),
        H => (false, 0x23),
        J => (false, 0x24),
        K => (false, 0x25),
        L => (false, 0x26),
        Semicolon | Colon => (false, 0x27),
        Quote => (false, 0x28),
        Backtick => (false, 0x29),
        Backslash | Pipe => (false, 0x2B),
        Z => (false, 0x2C),
        X => (false, 0x2D),
        C => (false, 0x2E),
        V => (false, 0x2F),
        B => (false, 0x30),
        N => (false, 0x31),
        M => (false, 0x32),
        Comma => (false, 0x33),
        Period => (false, 0x34),
        Slash | Questionmark => (false, 0x35),
        Space => (false, 0x39),
        F1 => (false, 0x3B),
        F2 => (false, 0x3C),
        F3 => (false, 0x3D),
        F4 => (false, 0x3E),
        F5 => (false, 0x3F),
        F6 => (false, 0x40),
        F7 => (false, 0x41),
        F8 => (false, 0x42),
        F9 => (false, 0x43),
        F10 => (false, 0x44),
        F11 => (false, 0x57),
        F12 => (false, 0x58),
        Insert => (true, 0x52),
        Delete => (true, 0x53),
        Home => (true, 0x47),
        End => (true, 0x4F),
        PageUp => (true, 0x49),
        PageDown => (true, 0x51),
        ArrowUp => (true, 0x48),
        ArrowDown => (true, 0x50),
        ArrowLeft => (true, 0x4B),
        ArrowRight => (true, 0x4D),
        _ => return None,
    })
}
