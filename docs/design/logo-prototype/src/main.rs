//! PROTOTYPE (harness-7bj.5): renders docs/design/logo.txt as half-block cells,
//! downscaled live to pick a size, and hops it while "running".
//! Keys: left/right cycle widths, n toggles nearest/box scaling, space toggles running/still, q quits.
//! Throwaway; the Logo widget is the part worth folding into the port. The
//! scaler is not: the chosen size gets baked into the asset offline.
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{buffer::Buffer, layout::Rect, style::{Color, Stylize}, text::Line, widgets::Widget};

const SOURCE: &str = include_str!("../../logo.txt");
const WIDTHS: [usize; 3] = [12, 24, 48];
const TICK: Duration = Duration::from_millis(50);
/// Vertical pixel offset per tick over one hop cycle: rest is 2 pixels (one cell) down.
const HOP: [usize; 24] = [2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 1, 2];

type Px = Option<(u8, u8, u8)>;

struct Logo {
    rows: Vec<Vec<Px>>, // pixel rows, two per cell row
    truecolor: bool,
}

impl Logo {
    fn parse(src: &str, truecolor: bool) -> Self {
        let (legend, pixels) = src.split_once("\n\n").expect("blank line between legend and pixels");
        let palette: Vec<(char, (u8, u8, u8))> = legend
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, hex)| {
                let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap();
                (k.chars().next().unwrap(), ((v >> 16) as u8, (v >> 8) as u8, v as u8))
            })
            .collect();
        let rows = pixels
            .lines()
            .map(|l| l.chars().map(|c| palette.iter().find(|(k, _)| *k == c).map(|(_, p)| *p)).collect())
            .collect();
        Self { rows, truecolor }
    }

    /// Box-filter rescale to `w` pixels wide (prototype only; the port embeds the final size).
    fn scaled(&self, w: usize, nearest: bool) -> Self {
        let (sw, sh) = (self.rows[0].len(), self.rows.len());
        let h = (sh * w / sw + 1) & !1; // even, so it fills whole cells
        let (fx, fy) = (sw as f64 / w as f64, sh as f64 / h as f64);
        let rows = (0..h)
            .map(|ty| {
                (0..w)
                    .map(|tx| {
                        let (x0, y0) = ((tx as f64 * fx) as usize, (ty as f64 * fy) as usize);
                        if nearest { return self.rows[y0][x0]; }
                        let (x1, y1) = ((((tx + 1) as f64 * fx).ceil() as usize).min(sw), (((ty + 1) as f64 * fy).ceil() as usize).min(sh));
                        let (mut n, mut opaque, mut sum) = (0u32, 0u32, (0u32, 0u32, 0u32));
                        for y in y0..y1 {
                            for x in x0..x1 {
                                n += 1;
                                if let Some((r, g, b)) = self.rows[y][x] {
                                    opaque += 1;
                                    sum = (sum.0 + r as u32, sum.1 + g as u32, sum.2 + b as u32);
                                }
                            }
                        }
                        (opaque * 2 > n).then(|| ((sum.0 / opaque) as u8, (sum.1 / opaque) as u8, (sum.2 / opaque) as u8))
                    })
                    .collect()
            })
            .collect();
        Self { rows, truecolor: self.truecolor }
    }

    fn width(&self) -> u16 { self.rows[0].len() as u16 }
    fn height(&self) -> u16 { self.rows.len().div_ceil(2) as u16 }

    fn color(&self, p: (u8, u8, u8)) -> Color { paint(p, self.truecolor) }

    /// Draw shifted down by `dy` pixels (half cells), so a hop can step one pixel at a time.
    fn render_at(&self, area: Rect, dy: usize, buf: &mut Buffer) {
        let area = area.intersection(*buf.area()); // clip to the screen, never index past it
        let blank = vec![None; self.rows[0].len()];
        let rows: Vec<&Vec<Px>> = std::iter::repeat(&blank).take(dy).chain(self.rows.iter()).collect();
        for (row, pair) in rows.chunks(2).enumerate() {
            let y = area.y + row as u16;
            if y >= area.bottom() { break; }
            let (top, bottom) = (pair[0], pair.get(1));
            for x in 0..area.width.min(top.len() as u16) {
                let cell = &mut buf[(area.x + x, y)];
                let hi = top[x as usize].map(|p| self.color(p));
                let lo = bottom.and_then(|b| b[x as usize]).map(|p| self.color(p));
                match (hi, lo) {
                    (None, None) => {}
                    (Some(fg), None) => { cell.set_char('▀').set_fg(fg); }
                    (None, Some(bg)) => { cell.set_char('▄').set_fg(bg); }
                    (Some(fg), Some(bg)) => { cell.set_char('▀').set_fg(fg).set_bg(bg); }
                }
            }
        }
    }
}

fn paint((r, g, b): (u8, u8, u8), truecolor: bool) -> Color {
    if truecolor {
        Color::Rgb(r, g, b)
    } else {
        // Nearest colour in the xterm 256 cube when the terminal has no 24-bit colour.
        let q = |v: u8| ((v as u16 * 5 + 127) / 255) as u8;
        Color::Indexed(16 + 36 * q(r) + 6 * q(g) + q(b))
    }
}

/// 3x5 block font, only the letters the banner needs; drawn in half-blocks, so 3 rows tall.
const FONT: &[(char, [&str; 5])] = &[
    ('T', ["###", ".#.", ".#.", ".#.", ".#."]),
    ('H', ["#.#", "#.#", "###", "#.#", "#.#"]),
    ('E', ["###", "#..", "##.", "#..", "###"]),
    ('A', [".#.", "#.#", "###", "#.#", "#.#"]),
    ('R', ["##.", "#.#", "##.", "#.#", "#.#"]),
    ('N', ["##.", "#.#", "#.#", "#.#", "#.#"]),
    ('S', ["###", "#..", "###", "..#", "###"]),
    (' ', ["...", "...", "...", "...", "..."]),
];

fn hsv((h, s, v): (f32, f32, f32)) -> (u8, u8, u8) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as u32 % 6 {
        0 => (c, x, 0.0), 1 => (x, c, 0.0), 2 => (0.0, c, x),
        3 => (0.0, x, c), 4 => (x, 0.0, c), _ => (c, 0.0, x),
    };
    let m = v - c;
    (((r + m) * 255.0) as u8, ((g + m) * 255.0) as u8, ((b + m) * 255.0) as u8)
}

/// Block-letter banner, hue drifting along the text and brightness pulsing with the ticks.
fn banner(text: &str, area: Rect, ticks: usize, truecolor: bool, buf: &mut Buffer) {
    let area = area.intersection(*buf.area());
    let t = ticks as f32;
    let pulse = 0.7 + 0.3 * (t / 25.0).sin();
    for (i, ch) in text.chars().enumerate() {
        let Some((_, glyph)) = FONT.iter().find(|(k, _)| *k == ch) else { continue };
        for (row, pair) in glyph.chunks(2).enumerate() {
            let y = area.y + row as u16;
            if y >= area.bottom() { break; }
            for col in 0..3 {
                let x = area.x + (i * 4 + col) as u16;
                if x >= area.right() { continue; }
                let hi = pair[0].as_bytes()[col] == b'#';
                let lo = pair.get(1).map_or(false, |l| l.as_bytes()[col] == b'#');
                let ch = match (hi, lo) { (true, true) => '\u{2588}', (true, false) => '\u{2580}', (false, true) => '\u{2584}', _ => continue };
                let hue = ((i * 4 + col) as f32 * 6.0 + t * 1.2) % 360.0;
                buf[(x, y)].set_char(ch).set_fg(paint(hsv((hue, 0.85, pulse)), truecolor));
            }
        }
    }
}

impl Widget for &Logo {
    fn render(self, area: Rect, buf: &mut Buffer) { self.render_at(area, 0, buf) }
}

fn main() -> std::io::Result<()> {
    let truecolor = std::env::var("COLORTERM").map(|v| v == "truecolor" || v == "24bit").unwrap_or(false);
    let source = Logo::parse(SOURCE, truecolor);
    let mut nearest = true;
    let build = |nearest: bool| -> Vec<Logo> { WIDTHS.iter().map(|&w| source.scaled(w, nearest)).collect() };
    let mut logos = build(nearest);
    let folder = std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default();
    let folder = match std::env::var("HOME") {
        Ok(h) if folder.starts_with(&h) => folder.replacen(&h, "~", 1),
        _ => folder,
    };
    let mut terminal = ratatui::init();
    let (mut running, mut ticks, mut which) = (true, 0usize, 1usize);
    let mut last = Instant::now();
    loop {
        let logo = &logos[which];
        let dy = if running { HOP[ticks % HOP.len()] } else { 2 };
        terminal.draw(|f| {
            let a = f.area();
            logo.render_at(Rect::new(2, 1, logo.width(), logo.height() + 1), dy, f.buffer_mut());
            // Header beside the logo: banner, version in light gray, then the folder.
            let x = 2 + logo.width() + 3;
            let w = a.width.saturating_sub(x);
            banner("THE HARNESS", Rect::new(x, 2, w, 3), ticks, truecolor, f.buffer_mut());
            f.render_widget(Line::from("v0.1.0".fg(Color::Rgb(160, 160, 160))), Rect::new(x, 6, w, 1));
            f.render_widget(Line::from(folder.as_str()), Rect::new(x, 7, w, 1));
            let status = format!(
                "{}  |  {} px wide = {}x{} cells, {} scaling  |  terminal {}x{}  |  {}  |  left/right: size, n: scaling, space: toggle, q: quit",
                if running { "RUNNING (hopping)" } else { "STILL" },
                WIDTHS[which], logo.width(), logo.height(), if nearest { "nearest" } else { "box" }, a.width, a.height,
                if truecolor { "24-bit" } else { "256-colour fallback (COLORTERM unset)" },
            );
            f.render_widget(Line::from(status), Rect::new(0, a.height.saturating_sub(1), a.width, 1));
        })?;
        if !event::poll(TICK.saturating_sub(last.elapsed()))? {
            ticks += 1;
            last = Instant::now();
            continue;
        }
        if let Event::Key(k) = event::read()? {
            if k.kind != KeyEventKind::Press { continue; }
            match k.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char(' ') => running = !running,
                KeyCode::Char('n') => { nearest = !nearest; logos = build(nearest); }
                KeyCode::Right => which = (which + 1) % WIDTHS.len(),
                KeyCode::Left => which = (which + WIDTHS.len() - 1) % WIDTHS.len(),
                _ => {}
            }
        }
    }
    ratatui::restore();
    Ok(())
}

#[test]
fn asset_parses_and_scales() {
    let logo = Logo::parse(SOURCE, true);
    assert_eq!((logo.width(), logo.height()), (24, 13));
    let small = logo.scaled(48, false);
    assert_eq!(logo.scaled(12, true).height(), 6);
    assert_eq!((small.width(), small.height()), (48, 25));
    assert!(small.rows.iter().flatten().any(|p| p.is_some()));
}
