//! termx-splash
//!
//! Male samostatne graficke okno (mimo terminal), ktere se kratce zobrazi
//! PRED tim, nez aplikace prevezme terminal a spusti TUI - obdoba splash
//! screen znameho z klasickych desktopovych aplikaci. Ukazuje logo
//! (`assets/term-ix_logo.png`) a pod nim se "terminalovym" zpusobem
//! (pismenko po pismenku, s blikajicim kurzorem) vypise verze a autor -
//! font DejaVu Sans Mono Bold, `assets/fonts/`, viz tamni
//! LICENSE-DejaVu.txt. Samotne psani textu tak prirozene vytvori kratkou
//! "startovaci" pauzu, aniz by pusobila zdlouhave - jakmile dopise a
//! kurzor par-krat blikne, okno se samo zavre.
//!
//! POZNAMKA K OVERENI: podobne jako `termx-ssh`, i tento crate pouziva
//! zavislosti (`minifb`, `fontdue`), jejichz presne API nebylo mozne v
//! tomto prostredi overit skutecnym `cargo build` (zadny pristup na
//! crates.io - viz README). Logika je napsana konzervativne a defenzivne
//! (viz `try_show_splash`), ale drobne doladeni casovani/zarovnani muze
//! byt po prvnim spusteni potreba.
//!
//! DULEZITE: splash je kosmeticky bonus, nikdy nesmi zabranit spusteni
//! aplikace. Pokud se graficke okno nepodari otevrit (napr. beh pres SSH
//! bez X11/Wayland, headless server, chybejici grafika), [`show_splash`]
//! chybu jen zaloguje a tise pokracuje dal.

use std::time::{Duration, Instant};

use minifb::{Key, MouseButton, Window, WindowOptions};

const LOGO_BYTES: &[u8] = include_bytes!("../../../assets/term-ix_logo.png");
const FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSansMono-Bold.ttf");

/// Kolik milisekund trva "napsani" jednoho znaku.
const TYPE_INTERVAL_MS: u128 = 32;
/// Pauza mezi prvnim (verze) a druhym (autor) radkem - jako novy radek
/// v terminalu, kdyz uzivatel na chvili zastavi. Zamerne kratka - je to
/// jediny usek celeho zobrazeni, kdy je videt jen prvni radek (druhy
/// jeste nezacal), takze cim kratsi pauza, tim mensi sance, ze prave v
/// tomto kratkem okamziku nekdo stihne udelat screenshot/mrknout a
/// bude si myslet, ze se autor "ztratil".
const LINE_PAUSE_MS: u128 = 100;
/// Jak dlouho jeste zustane okno otevrene po dopsani textu (kurzor
/// mezitim par-krat blikne), nez se samo zavre - dost casu, aby si
/// clovek stihl precist verzi/autora.
const IDLE_HOLD_MS: u128 = 2000;
/// Jak casto blika kurzor, kdyz zrovna nic nepise (viditelny/skryty).
const CURSOR_BLINK_MS: u128 = 500;
/// Bezpecnostni strop celkove doby zobrazeni, i kdyby vypocet casovani
/// z nejakeho duvodu vysel mnohem delsi, nez se cekalo.
const MAX_DURATION: Duration = Duration::from_millis(5000);

/// Nejvetsi povoleny rozmer (sirka/vyska, cokoliv je vetsi) splash okna
/// v pixelech. Zdrojove logo (`assets/term-ix_logo.png`) je samo o sobe
/// 768x768 - to je v poradku pro ikony/vetsi graziku, ale jako okno by
/// to bylo obri pres pulku obrazovky. Puvodni DPI-awareness fix (viz
/// `main.rs`) resil jen to, ze Windows uz takove okno dal nezvetsoval -
/// tohle je skutecna oprava zakladni velikosti: obrazek (a s nim
/// umerne i velikost pisma/okrajů pro text) se pred vytvorenim okna
/// zmensi na rozumnou "splash" velikost.
const TARGET_MAX_DIM: u32 = 400;

const COLOR_TEXT: u32 = 0xE8ECEF;
const COLOR_CURSOR: u32 = 0x7FE0DC;

pub struct SplashInfo<'a> {
    pub version: &'a str,
    pub author: &'a str,
}

/// Zobrazi splash okno. Nikdy nepanikari a nikdy neprerusi start aplikace -
/// jakoukoliv chybu (chybejici graficke prostredi apod.) jen zaloguje.
pub fn show_splash(info: SplashInfo) {
    if let Err(e) = try_show_splash(&info) {
        tracing::debug!("splash se nepodarilo zobrazit, pokracuji bez nej: {e}");
    }
}

/// Jeden rasterizovany znak pripraveny predem, aby se pri kazdem snimku
/// animace nemusel font znovu rasterizovat.
struct Glyph {
    metrics: fontdue::Metrics,
    bitmap: Vec<u8>,
}

/// Radek textu s predpocitanym rozlozenim (start_x pro horizontalni
/// centrovani vuci CELE finalni delce radku - diky tomu text pri psani
/// "roste" doprava z pevneho zacatku a neposkakuje).
struct LineLayout {
    glyphs: Vec<Glyph>,
    start_x: f32,
    baseline_y: usize,
}

fn layout_line(font: &fontdue::Font, text: &str, px: f32, img_w: usize, baseline_y: usize) -> LineLayout {
    let glyphs: Vec<Glyph> = text
        .chars()
        .map(|ch| {
            let (metrics, bitmap) = font.rasterize(ch, px);
            Glyph { metrics, bitmap }
        })
        .collect();

    let total_advance: f32 = glyphs.iter().map(|g| g.metrics.advance_width).sum();
    let start_x = (img_w as f32 - total_advance) / 2.0;

    LineLayout { glyphs, start_x, baseline_y }
}

/// Vykresli prvnich `reveal_count` znaku radku a vrati x-pozici, kam patri
/// kurzor (za poslednim vykreslenym znakem).
fn draw_line_partial(buffer: &mut [u32], img_w: usize, img_h: usize, line: &LineLayout, reveal_count: usize) -> f32 {
    let mut cursor_x = line.start_x;

    for glyph in line.glyphs.iter().take(reveal_count) {
        let m = &glyph.metrics;
        let glyph_left = cursor_x as i32 + m.xmin;
        let glyph_top = line.baseline_y as i32 - m.ymin - m.height as i32;

        for row in 0..m.height {
            for col in 0..m.width {
                let coverage = glyph.bitmap[row * m.width + col] as u32;
                if coverage == 0 {
                    continue;
                }
                let x = glyph_left + col as i32;
                let y = glyph_top + row as i32;
                if x < 0 || y < 0 || x as usize >= img_w || y as usize >= img_h {
                    continue;
                }
                blend_pixel(buffer, img_w, x as usize, y as usize, COLOR_TEXT, coverage.min(255));
            }
        }

        cursor_x += m.advance_width;
    }

    cursor_x
}

/// Nakresli jednoduchy obdelnikovy blokovy kurzor (jako v terminalu) na
/// dane pozici zakladni linky.
fn draw_cursor_block(buffer: &mut [u32], img_w: usize, img_h: usize, x: f32, baseline_y: usize, px: f32) {
    let width = (px * 0.55).round() as i32;
    let height = (px * 0.85).round() as i32;
    let top = baseline_y as i32 - height;

    for row in 0..height {
        for col in 0..width {
            let px_x = x as i32 + col;
            let px_y = top + row;
            if px_x < 0 || px_y < 0 || px_x as usize >= img_w || px_y as usize >= img_h {
                continue;
            }
            blend_pixel(buffer, img_w, px_x as usize, px_y as usize, COLOR_CURSOR, 235);
        }
    }
}

fn blend_pixel(buffer: &mut [u32], img_w: usize, x: usize, y: usize, color_rgb: u32, alpha: u32) {
    let idx = y * img_w + x;
    let bg = buffer[idx];
    let (cr, cg, cb) = ((color_rgb >> 16) & 0xff, (color_rgb >> 8) & 0xff, color_rgb & 0xff);
    let (br, bg_g, bb) = ((bg >> 16) & 0xff, (bg >> 8) & 0xff, bg & 0xff);
    let a = alpha.min(255);
    let nr = (cr * a + br * (255 - a)) / 255;
    let ng = (cg * a + bg_g * (255 - a)) / 255;
    let nb = (cb * a + bb * (255 - a)) / 255;
    buffer[idx] = (nr << 16) | (ng << 8) | nb;
}

/// Rozmery hlavni obrazovky (primarniho monitoru) ve fyzickych pixelech -
/// viz `force_borderless_and_center`. Stejny zpusob (primy Win32 FFI
/// dotaz) jako `set_process_dpi_aware` v `main.rs`, aby se kvuli jedne
/// funkci nemusela pridavat dalsi zavislost (`winapi`/`windows` crate).
#[cfg(target_os = "windows")]
fn primary_screen_size() -> Option<(i32, i32)> {
    #[link(name = "user32")]
    extern "system" {
        fn GetSystemMetrics(n_index: i32) -> i32;
    }
    const SM_CXSCREEN: i32 = 0;
    const SM_CYSCREEN: i32 = 1;
    let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    if w > 0 && h > 0 {
        Some((w, h))
    } else {
        None
    }
}

/// Na Windows nekdy `minifb`'s `borderless: true` + `title: false` (viz
/// nastaveni okna nize) samo o sobe nestaci a titulkovy pruh zustane
/// viditelny (potvrzeno screenshotem od uzivatele). Jako spolehlivejsi
/// pojistka se okno hned po vytvoreni jeste najde podle sveho titulku
/// pres primy Win32 dotaz (`FindWindowW`) a jeho styl se natvrdo
/// prepne na `WS_POPUP` (cisty "popup" bez jakehokoliv systemoveho
/// ramecku/titulku) - stejny pristup (primy FFI, zadna dalsi
/// zavislost) jako `set_process_dpi_aware` v `main.rs`. Stejnym
/// volanim (`SetWindowPos`) se zaroven okno vycentruje na obrazovce -
/// je to spolehlivejsi nez puvodni `Window::set_position` z `minifb`,
/// protoze `SetWindowPos` s `SWP_FRAMECHANGED` donuti Windows prepocitat
/// ramecek podle noveho stylu ve stejnem kroku.
#[cfg(target_os = "windows")]
fn force_borderless_and_center(title: &str, width: i32, height: i32) {
    use std::ffi::c_void;

    #[link(name = "user32")]
    extern "system" {
        fn FindWindowW(lp_class_name: *const u16, lp_window_name: *const u16) -> *mut c_void;
        fn SetWindowLongPtrW(hwnd: *mut c_void, n_index: i32, dw_new_long: isize) -> isize;
        fn SetWindowPos(hwnd: *mut c_void, hwnd_insert_after: *mut c_void, x: i32, y: i32, cx: i32, cy: i32, u_flags: u32) -> i32;
    }

    const GWL_STYLE: i32 = -16;
    const WS_POPUP: isize = 0x8000_0000_u32 as i32 as isize;
    const WS_VISIBLE: isize = 0x1000_0000;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_FRAMECHANGED: u32 = 0x0020;
    const SWP_SHOWWINDOW: u32 = 0x0040;

    let wide_title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), wide_title.as_ptr());
        if hwnd.is_null() {
            // Okno se nenaslo (napr. jina verze Windows hlasi titulek
            // jinak) - splash zustane zobrazeny aspon tak, jak ho
            // vytvorilo minifb samo, jen bez vycentrovani/tvrdeho
            // borderless. Nikdy nesmi kvuli tomu spadnout cela aplikace.
            return;
        }

        SetWindowLongPtrW(hwnd, GWL_STYLE, WS_POPUP | WS_VISIBLE);

        let (x, y) = match primary_screen_size() {
            Some((screen_w, screen_h)) => (((screen_w - width) / 2).max(0), ((screen_h - height) / 2).max(0)),
            None => (0, 0),
        };
        SetWindowPos(hwnd, std::ptr::null_mut(), x, y, width, height, SWP_NOZORDER | SWP_FRAMECHANGED | SWP_SHOWWINDOW);
    }
}

fn try_show_splash(info: &SplashInfo) -> anyhow::Result<()> {
    let img = image::load_from_memory(LOGO_BYTES)?.to_rgba8();

    // Zdrojove logo zmensit na rozumnou velikost splash okna (viz
    // `TARGET_MAX_DIM`) - beze zmeny pomeru stran, a nikdy needly
    // (kdyby uz nekdy byl zdroj mensi, nez cilova velikost).
    let longer_side = img.width().max(img.height());
    let scale = if longer_side > TARGET_MAX_DIM { TARGET_MAX_DIM as f32 / longer_side as f32 } else { 1.0 };
    let new_w = ((img.width() as f32 * scale).round() as u32).max(1);
    let new_h = ((img.height() as f32 * scale).round() as u32).max(1);
    let img = if scale < 1.0 {
        image::imageops::resize(&img, new_w, new_h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let (width, height) = (img.width() as usize, img.height() as usize);

    let base_buffer: Vec<u32> = img
        .pixels()
        .map(|p| {
            let [r, g, b, _a] = p.0;
            ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
        })
        .collect();

    let font = fontdue::Font::from_bytes(FONT_BYTES, fontdue::FontSettings::default())
        .map_err(|e| anyhow::anyhow!("nelze nacist font pro splash: {e}"))?;

    let line1_text = format!("Term-IX v{}", info.version);
    let line2_text = info.author.to_string();
    // Velikost pisma a okraje textu se skaluji spolu s obrazkem, aby text
    // zustal na spravnem miste a v odpovidajicim pomeru i po zmenseni loga.
    let text_px = 30.0 * scale;
    let margin_line1 = (92.0 * scale).round() as usize;
    let margin_line2 = (48.0 * scale).round() as usize;

    let line1 = layout_line(&font, &line1_text, text_px, width, height.saturating_sub(margin_line1));
    let line2 = layout_line(&font, &line2_text, text_px, width, height.saturating_sub(margin_line2));

    let chars1 = line1.glyphs.len() as u128;
    let chars2 = line2.glyphs.len() as u128;

    let t_line1_end = chars1 * TYPE_INTERVAL_MS;
    let t_line2_start = t_line1_end + LINE_PAUSE_MS;
    let t_line2_end = t_line2_start + chars2 * TYPE_INTERVAL_MS;
    let close_after_ms = t_line2_end + IDLE_HOLD_MS;

    let mut window = Window::new(
        "Term-IX",
        width,
        height,
        WindowOptions {
            borderless: true,
            // `borderless` samo o sobe u minifb na Windows nestaci - bez
            // `title: false` se muze porad zobrazit titulkovy pruh
            // (caption bar), coz je presne ta chyba, kterou uzivatel
            // hlasil. Obe nastaveni dohromady = zadne okenni orameni.
            title: false,
            resize: false,
            topmost: true,
            ..WindowOptions::default()
        },
    )?;

    // Na Windows natvrdo zajistit borderless (viz komentar u funkce) a
    // vycentrovat na obrazovce. Na jinych platformach necha aplikace
    // umisteni na okennim manageru - `borderless`/`title` z
    // `WindowOptions` vyse tam plati primo.
    #[cfg(target_os = "windows")]
    force_borderless_and_center("Term-IX", width as i32, height as i32);

    let start = Instant::now();

    loop {
        if !window.is_open() {
            break;
        }

        let elapsed = start.elapsed();
        let e_ms = elapsed.as_millis();

        if elapsed >= MAX_DURATION || e_ms >= close_after_ms {
            break;
        }

        let dismissed = window.is_key_down(Key::Escape)
            || window.is_key_down(Key::Enter)
            || window.is_key_down(Key::Space)
            || window.get_mouse_down(MouseButton::Left);
        if dismissed {
            break;
        }

        let mut frame = base_buffer.clone();

        let reveal1 = ((e_ms / TYPE_INTERVAL_MS) as usize).min(chars1 as usize);
        let reveal2 = if e_ms < t_line2_start {
            0
        } else {
            (((e_ms - t_line2_start) / TYPE_INTERVAL_MS) as usize).min(chars2 as usize)
        };

        let cursor1_x = draw_line_partial(&mut frame, width, height, &line1, reveal1);
        let cursor2_x = draw_line_partial(&mut frame, width, height, &line2, reveal2);

        // kurzor: dokud se aktivne pise, je vzdy viditelny (jako pisici se
        // kurzor v terminalu); v pauze mezi radky a po dopsani obou radku
        // blika.
        let (cursor_x, cursor_baseline, actively_typing) = if reveal1 < chars1 as usize {
            (cursor1_x, line1.baseline_y, true)
        } else if e_ms < t_line2_start {
            (cursor1_x, line1.baseline_y, false)
        } else if reveal2 < chars2 as usize {
            (cursor2_x, line2.baseline_y, true)
        } else {
            (cursor2_x, line2.baseline_y, false)
        };

        let cursor_visible = actively_typing || (e_ms / CURSOR_BLINK_MS) % 2 == 0;
        if cursor_visible {
            draw_cursor_block(&mut frame, width, height, cursor_x, cursor_baseline, text_px);
        }

        // update_with_buffer jak vykresli novy snimek, tak zaroven "pumpuje"
        // udalosti okna (klavesnice/mys) - samostatne window.update() navic
        // netreba.
        window.update_with_buffer(&frame, width, height)?;
        std::thread::sleep(Duration::from_millis(16));
    }

    Ok(())
}
