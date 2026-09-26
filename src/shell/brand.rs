//! The palette, the 256-color fallback, and the Orqadence brand: the 2×2
//! pane mark, the pixel wordmark and its cursor, and the 12×12 logo
//! (assets/brand/orqadence-logo-12x12.txt).

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

pub(crate) const PURPLE: Color = Color::Rgb(210, 90, 255);
pub(crate) const BLUE: Color = Color::Rgb(60, 180, 255);
pub(crate) const ORANGE: Color = Color::Rgb(255, 175, 60);
pub(crate) const GREEN: Color = Color::Rgb(130, 240, 120);
pub(crate) const CYAN: Color = Color::Rgb(44, 242, 250);
pub(crate) const PINK: Color = Color::Rgb(255, 120, 200);
pub(crate) const TEXT: Color = Color::Rgb(232, 240, 255);
pub(crate) const MUTED: Color = Color::Rgb(132, 147, 173);
pub(crate) const BORDER: Color = Color::Rgb(84, 98, 124);
pub(crate) const DARK_ORANGE: Color = Color::Rgb(200, 110, 20);
pub(crate) const RED: Color = Color::Rgb(255, 90, 90);
/// A Ticket's color, by its child suffix.
pub(crate) const TICKET_COLORS: [Color; 7] = [GREEN, CYAN, PURPLE, BLUE, ORANGE, PINK, MUTED];

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

/// The brand's colors, which only the header and the logos use.
pub(crate) const INK: Color = Color::Rgb(0x00, 0x0a, 0x1a);
pub(crate) const YELLOW: Color = Color::Rgb(0xfa, 0xc8, 0x00);
pub(crate) const WORDMARK: Color = Color::Rgb(0xe6, 0xf1, 0xff);
/// The boxes' borders: the brand's #1e3150 lightened, which a dark background hid.
pub(crate) const FRAME: Color = Color::Rgb(0x40, 0x64, 0x96);
/// Each pane's own color, clockwise from the top left.
pub(crate) const PANE_COLORS: [Color; 4] = [
    Color::Rgb(0x00, 0xf2, 0x7c),
    CYAN,
    Color::Rgb(0x00, 0x90, 0xfa),
    Color::Rgb(0xaa, 0x2e, 0xfa),
];
/// The pane lit while no run is live: the bottom right.
pub(crate) const REST: usize = 2;

/// orqadence in half-blocks, 62 columns by 6 rows: the d's top on the first
/// row, the letters filling rows 2 to 5 whole so text beside them lines up
/// with their tops and baseline, the q's tail on the last.
pub(crate) const WORDMARK_ROWS: [&str; 6] = [
    "                                ██                            ",
    "▄█▀▀█▄ ██▄▀▀▀ ▄█▀▀██  ▀▀▀█▄ ▄█▀▀██ ▄█▀▀█▄ ██▀▀█▄ ▄█▀▀▀▀ ▄█▀▀█▄",
    "██  ██ ██     ██  ██  ▄▄▄██ ██  ██ ██▄▄██ ██  ██ ██     ██▄▄██",
    "██  ██ ██     ██  ██ ██  ██ ██  ██ ██     ██  ██ ██     ██    ",
    "▀█▄▄█▀ ██     ▀█▄▄██ ▀█▄▄██ ▀█▄▄██ ▀█▄▄▄▄ ██  ██ ▀█▄▄▄▄ ▀█▄▄▄▄",
    "                  ██                                          ",
];
/// The cursor after the wordmark: a block on its baseline, row 5.
pub(crate) const CURSOR_ROWS: [&str; 6] = ["", "", "", "", "█████", ""];

/// The 12×12 logo's static frame for a terminal without Unicode or color.
#[allow(dead_code)] // nothing prints a standalone logo yet
pub(crate) const LOGO_ASCII: &str = "\
+---+  +---+
|   |  |   |
|   |  |   |
|   |  |   |
+---+  +---+


+---+  #####
|   |  #####
|   |  ##>##
|   |  #####
+---+  #####";

/// The header's mark, 13×6: 6×3 panes a column apart.
pub(crate) fn logo_mark(lit: usize) -> Vec<Line<'static>> {
    panes(lit, (6, 3), (1, 0))
}

/// The standalone logo, 12×12: 5×5 panes two columns and two rows apart.
#[allow(dead_code)] // nothing shows a standalone logo yet
pub(crate) fn logo_12(lit: usize) -> Vec<Line<'static>> {
    panes(lit, (5, 5), (2, 2))
}

/// Four `w`×`h` panes in a 2×2 grid, `gx` columns and `gy` rows apart,
/// numbered clockwise from the top left: `lit` filled with its color and a
/// ❯ on the middle row, the others outlined in theirs.
fn panes(lit: usize, (w, h): (usize, usize), (gx, gy): (usize, usize)) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (r, pair) in [[0, 1], [3, 2]].into_iter().enumerate() {
        if r == 1 {
            lines.extend((0..gy).map(|_| Line::raw(" ".repeat(2 * w + gx))));
        }
        for y in 0..h {
            let mut spans = pane_row(pair[0], pair[0] == lit, (w, h), y);
            spans.push(Span::raw(" ".repeat(gx)));
            spans.extend(pane_row(pair[1], pair[1] == lit, (w, h), y));
            lines.push(Line::from(spans));
        }
    }
    lines
}

/// Row `y` of pane `i`. Lit, its edges are half and quarter blocks, so the
/// fill stops mid-cell where an unlit pane's outline runs and both are one size.
/// Both have square corners: a curve can't be filled in a cell.
fn pane_row(i: usize, lit: bool, (w, h): (usize, usize), y: usize) -> Vec<Span<'static>> {
    let c = PANE_COLORS[i];
    let (edge, (l, m, r)) = match (lit, y) {
        (true, 0) => (true, ("▗", "▄", "▖")),
        (true, _) if y + 1 == h => (true, ("▝", "▀", "▘")),
        (true, _) => (false, ("▐", " ", "▌")),
        (false, 0) => (true, ("┌", "─", "┐")),
        (false, _) if y + 1 == h => (true, ("└", "─", "┘")),
        (false, _) => (true, ("│", " ", "│")),
    };
    if edge {
        return vec![Span::styled(format!("{l}{}{r}", m.repeat(w - 2)), c)];
    }
    let fill = Style::new().bg(c);
    let mut row = vec![Span::styled(l, c)];
    if y == h / 2 {
        row.push(Span::styled(" ", fill));
        row.push(Span::styled("❯", fill.fg(INK).bold()));
        row.push(Span::styled(" ".repeat(w - 4), fill));
    } else {
        row.push(Span::styled(" ".repeat(w - 2), fill));
    }
    row.push(Span::styled(r, c));
    row
}
