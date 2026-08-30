//! termx-splash
//!
//! Male samostatne graficke okno (mimo terminal), ktere se kratce zobrazi
//! PRED tim, nez aplikace prevezme terminal a spusti TUI - obdoba splash
//! screen znameho z klasickych desktopovych aplikaci. Ukazuje logo
//! (`assets/term-ix_logo.png`), verzi a autora vypalene do obrazku pres
//! font DejaVu Sans Bold (`assets/fonts/`, viz tamni LICENSE-DejaVu.txt).
//!
//! POZNAMKA K OVERENI: podobne jako `termx-ssh`, i tento crate pouziva
//! zavislosti (`minifb`, `fontdue`), jejichz presne API nebylo mozne v
//! tomto prostredi overit skutecnym `cargo build` (zadny pristup na
//! crates.io - viz README). Logika je napsana konzervativne a defenzivne
//! (viz `try_show_splash`), ale drobne doladeni (napr. presne zarovnani
//! textu na zakladni linku pisma) muze byt po prvnim spusteni potreba.
//!
//! DULEZITE: splash je kosmeticky bonus, nikdy nesmi zabranit spusteni
//! aplikace. Pokud se graficke okno nepodari otevrit (napr. beh pres SSH
//! bez X11/Wayland, headless server, chybejici grafika), [`show_splash`]
//! chybu jen zaloguje a tise pokracuje dal.

use std::time::{Duration, Instant};

use minifb::{Key, MouseButton, Window, WindowOptions};

const LOGO_BYTES: &[u8] = include_bytes!("../../../assets/term-ix_logo.png");
const FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSans-Bold.ttf");

/// Jak dlouho se splash maximalne zobrazuje, pokud jej uzivatel drive
/// nezavre klavesou/klikem.
const MAX_DURATION: Duration = Duration::from_millis(1800);

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

fn try_show_splash(info: &SplashInfo) -> anyhow::Result<()> {
    let img = image::load_from_memory(LOGO_BYTES)?.to_rgba8();
    let (width, height) = (img.width() as usize, img.height() as usize);

    let mut buffer: Vec<u32> = img
        .pixels()
        .map(|p| {
            let [r, g, b, _a] = p.0;
            ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
        })
        .collect();

    if let Ok(font) = fontdue::Font::from_bytes(FONT_BYTES, fontdue::FontSettings::default()) {
        let title = format!("Term-IX v{}", info.version);
        // barvy odpovidaji palete loga (svetle seda / cyan akcent)
        draw_centered_text(&mut buffer, width, height, &font, &title, 34.0, height - 92, 0xE8ECEF);
        draw_centered_text(&mut buffer, width, height, &font, info.author, 22.0, height - 48, 0x8FE0DC);
    } else {
        tracing::debug!("nepodarilo se nacist font pro splash - zobrazim jen logo bez textu");
    }

    let mut window = Window::new(
        "Term-IX",
        width,
        height,
        WindowOptions {
            borderless: true,
            resize: false,
            topmost: true,
            ..WindowOptions::default()
        },
    )?;

    window.update_with_buffer(&buffer, width, height)?;

    let start = Instant::now();
    while window.is_open() && start.elapsed() < MAX_DURATION {
        let dismissed = window.is_key_down(Key::Escape)
            || window.is_key_down(Key::Enter)
            || window.is_key_down(Key::Space)
            || window.get_mouse_down(MouseButton::Left);
        if dismissed {
            break;
        }
        window.update();
        std::thread::sleep(Duration::from_millis(16));
    }

    Ok(())
}

/// Vykresli horizontalne centrovany radek textu do RGB (0x00RRGGBB) bufferu
/// pomoci `fontdue` (rasterizace jednotlivych znaku + rucni alpha-blend
/// blit podle pokryti pixelu). `baseline_y` je y-souradnice zakladni linky
/// pisma v cilovem obrazku.
fn draw_centered_text(
    buffer: &mut [u32],
    img_w: usize,
    img_h: usize,
    font: &fontdue::Font,
    text: &str,
    px: f32,
    baseline_y: usize,
    color_rgb: u32,
) {
    let glyphs: Vec<(fontdue::Metrics, Vec<u8>)> =
        text.chars().map(|ch| font.rasterize(ch, px)).collect();

    let total_advance: f32 = glyphs.iter().map(|(m, _)| m.advance_width).sum();
    let mut cursor_x = (img_w as f32 - total_advance) / 2.0;

    let (cr, cg, cb) = (
        (color_rgb >> 16) & 0xff,
        (color_rgb >> 8) & 0xff,
        color_rgb & 0xff,
    );

    for (metrics, bitmap) in &glyphs {
        let glyph_left = cursor_x as i32 + metrics.xmin;
        // horni rada bitmapy = baseline - ymin - height (ymin muze byt zaporne)
        let glyph_top = baseline_y as i32 - metrics.ymin - metrics.height as i32;

        for row in 0..metrics.height {
            for col in 0..metrics.width {
                let coverage = bitmap[row * metrics.width + col] as u32;
                if coverage == 0 {
                    continue;
                }
                let x = glyph_left + col as i32;
                let y = glyph_top + row as i32;
                if x < 0 || y < 0 || x as usize >= img_w || y as usize >= img_h {
                    continue;
                }
                let idx = y as usize * img_w + x as usize;
                let bg = buffer[idx];
                let (br, bg_g, bb) = ((bg >> 16) & 0xff, (bg >> 8) & 0xff, bg & 0xff);
                let a = coverage.min(255);
                let nr = (cr * a + br * (255 - a)) / 255;
                let ng = (cg * a + bg_g * (255 - a)) / 255;
                let nb = (cb * a + bb * (255 - a)) / 255;
                buffer[idx] = (nr << 16) | (ng << 8) | nb;
            }
        }

        cursor_x += metrics.advance_width;
    }
}
