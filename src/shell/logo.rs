//! The palette, the half-block logo with its hop, the THE HARNESS banner, and
//! the 256-color fallback (docs/design/status-panel.rs, logo.txt, the logo
//! prototype).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

// Palette from docs/design/status-panel.rs.
pub(crate) const PURPLE: Color = Color::Rgb(210, 90, 255);
pub(crate) const BLUE: Color = Color::Rgb(60, 180, 255);
pub(crate) const ORANGE: Color = Color::Rgb(255, 175, 60);
pub(crate) const GREEN: Color = Color::Rgb(130, 240, 120);
pub(crate) const CYAN: Color = Color::Rgb(44, 242, 250);
pub(crate) const PINK: Color = Color::Rgb(255, 120, 200);
pub(crate) const TEXT: Color = Color::Rgb(232, 240, 255);
pub(crate) const MUTED: Color = Color::Rgb(132, 147, 173);
pub(crate) const BORDER: Color = Color::Rgb(35, 49, 74);
pub(crate) const GRAY: Color = Color::Rgb(160, 160, 160);
/// A Ticket's color, by its child suffix.
pub(crate) const TICKET_COLORS: [Color; 7] = [GREEN, CYAN, PURPLE, BLUE, ORANGE, PINK, MUTED];

const SOURCE: &str = include_str!("../../docs/design/logo.txt");

/// Vertical pixel offset per 50 ms tick over one 1.2 s hop: rest is 2 pixels
/// (one cell) down, then up one pixel, up a full cell, hold, and back.
pub(crate) const HOP: [usize; 24] = [
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 1, 2,
];
/// Where the logo rests when no run is live.
pub(crate) const REST: usize = 2;

type Px = Option<(u8, u8, u8)>;

/// The logo as pixel rows, two per cell row.
pub(crate) struct Logo {
    rows: Vec<Vec<Px>>,
}

impl Logo {
    pub(crate) fn embedded() -> Self {
        Self::parse(SOURCE)
    }

    fn parse(src: &str) -> Self {
        let (legend, pixels) = src
            .split_once("\n\n")
            .expect("blank line between legend and pixels");
        let palette: Vec<(char, (u8, u8, u8))> = legend
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, hex)| {
                let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap();
                (
                    k.chars().next().unwrap(),
                    ((v >> 16) as u8, (v >> 8) as u8, v as u8),
                )
            })
            .collect();
        let rows = pixels
            .lines()
            .map(|l| {
                l.chars()
                    .map(|c| palette.iter().find(|(k, _)| *k == c).map(|(_, p)| *p))
                    .collect()
            })
            .collect();
        Self { rows }
    }

    pub(crate) fn width(&self) -> u16 {
        self.rows[0].len() as u16
    }

    pub(crate) fn height(&self) -> u16 {
        self.rows.len().div_ceil(2) as u16
    }

    /// Draws shifted down by `dy` pixels (half cells), so a hop can step one
    /// pixel at a time. Transparent pixels leave the cell as it is.
    pub(crate) fn render_at(&self, area: Rect, dy: usize, buf: &mut Buffer) {
        let area = area.intersection(*buf.area());
        let blank = vec![None; self.rows[0].len()];
        let rows: Vec<&Vec<Px>> = std::iter::repeat_n(&blank, dy)
            .chain(self.rows.iter())
            .collect();
        for (row, pair) in rows.chunks(2).enumerate() {
            let y = area.y + row as u16;
            if y >= area.bottom() {
                break;
            }
            let (top, bottom) = (pair[0], pair.get(1));
            for x in 0..area.width.min(top.len() as u16) {
                let cell = &mut buf[(area.x + x, y)];
                let hi = top[x as usize].map(rgb);
                let lo = bottom.and_then(|b| b[x as usize]).map(rgb);
                match (hi, lo) {
                    (None, None) => {}
                    (Some(fg), None) => {
                        cell.set_char('▀').set_fg(fg);
                    }
                    (None, Some(bg)) => {
                        cell.set_char('▄').set_fg(bg);
                    }
                    (Some(fg), Some(bg)) => {
                        cell.set_char('▀').set_fg(fg).set_bg(bg);
                    }
                }
            }
        }
    }
}

fn rgb((r, g, b): (u8, u8, u8)) -> Color {
    Color::Rgb(r, g, b)
}

/// The nearest color in the xterm 256 cube, for a terminal without 24-bit
/// color (COLORTERM unset): a wrong Rgb looks broken, a wrong Indexed only flat.
pub(crate) fn quantize(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => {
            let q = |v: u8| ((v as u16 * 5 + 127) / 255) as u8;
            Color::Indexed(16 + 36 * q(r) + 6 * q(g) + q(b))
        }
        other => other,
    }
}

/// Straight-line blend between two colours, `t` in 0..=1.
pub(crate) fn lerp((a, b): (Color, Color), t: f32) -> Color {
    let (Color::Rgb(r0, g0, b0), Color::Rgb(r1, g1, b1)) = (a, b) else {
        return a;
    };
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color::Rgb(mix(r0, r1), mix(g0, g1), mix(b0, b1))
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

fn hsv((h, s, v): (f32, f32, f32)) -> Color {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as u32 % 6 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    Color::Rgb(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

/// Block-letter banner, hue drifting along the text and brightness pulsing with the ticks.
pub(crate) fn banner(text: &str, area: Rect, ticks: u64, buf: &mut Buffer) {
    let area = area.intersection(*buf.area());
    let t = ticks as f32;
    let pulse = 0.7 + 0.3 * (t / 25.0).sin();
    for (i, ch) in text.chars().enumerate() {
        let Some((_, glyph)) = FONT.iter().find(|(k, _)| *k == ch) else {
            continue;
        };
        for (row, pair) in glyph.chunks(2).enumerate() {
            let y = area.y + row as u16;
            if y >= area.bottom() {
                break;
            }
            for col in 0..3 {
                let x = area.x + (i * 4 + col) as u16;
                if x >= area.right() {
                    continue;
                }
                let hi = pair[0].as_bytes()[col] == b'#';
                let lo = pair.get(1).is_some_and(|l| l.as_bytes()[col] == b'#');
                let ch = match (hi, lo) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    _ => continue,
                };
                let hue = ((i * 4 + col) as f32 * 6.0 + t * 1.2) % 360.0;
                buf[(x, y)].set_char(ch).set_fg(hsv((hue, 0.85, pulse)));
            }
        }
    }
}
