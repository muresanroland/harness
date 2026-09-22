//! PROTOTYPE (harness-7bj.5): renders docs/design/logo.txt as half-block cells
//! and bobs it one row while "running". Space toggles running/still, q quits.
//! Throwaway; the Logo widget is the part worth folding into the port.
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{buffer::Buffer, layout::Rect, style::Color, text::Line, widgets::Widget};

const LOGO: &str = include_str!("../../logo.txt");

struct Logo {
    rows: Vec<Vec<Option<Color>>>, // pixel rows, two per cell row
}

impl Logo {
    fn parse(src: &str, truecolor: bool) -> Self {
        let (legend, pixels) = src.split_once("\n\n").expect("blank line between legend and pixels");
        let palette: Vec<(char, Color)> = legend
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, hex)| {
                let rgb = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap();
                (k.chars().next().unwrap(), paint(Color::from_u32(rgb), truecolor))
            })
            .collect();
        let rows = pixels
            .lines()
            .map(|l| l.chars().map(|c| palette.iter().find(|(k, _)| *k == c).map(|(_, col)| *col)).collect())
            .collect();
        Self { rows }
    }
    fn width(&self) -> u16 { self.rows[0].len() as u16 }
    fn height(&self) -> u16 { self.rows.len().div_ceil(2) as u16 }
}

/// Nearest colour in the xterm 256 cube when the terminal has no 24-bit colour.
fn paint(c: Color, truecolor: bool) -> Color {
    match c {
        Color::Rgb(r, g, b) if !truecolor => {
            let q = |v: u8| ((v as u16 * 5 + 127) / 255) as u8;
            Color::Indexed(16 + 36 * q(r) + 6 * q(g) + q(b))
        }
        c => c,
    }
}

impl Widget for &Logo {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for (row, pair) in self.rows.chunks(2).enumerate() {
            let y = area.y + row as u16;
            if y >= area.bottom() { break; }
            let (top, bottom) = (&pair[0], pair.get(1));
            for x in 0..area.width.min(top.len() as u16) {
                let cell = &mut buf[(area.x + x, y)];
                let hi = top[x as usize];
                let lo = bottom.and_then(|b| b.get(x as usize).copied().flatten());
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

fn main() -> std::io::Result<()> {
    let truecolor = std::env::var("COLORTERM").map(|v| v == "truecolor" || v == "24bit").unwrap_or(false);
    let logo = Logo::parse(LOGO, truecolor);
    let mut terminal = ratatui::init();
    let tick = Duration::from_millis(100);
    let mut running = true;
    let mut ticks: u32 = 0;
    let mut last = Instant::now();
    loop {
        // Rest one row down; hop up for 2 of every 12 ticks while running.
        let up = running && ticks % 12 >= 10;
        let dy = if up { 0 } else { 1 };
        terminal.draw(|f| {
            let a = f.area();
            let logo_area = Rect::new(2, dy, logo.width().min(a.width.saturating_sub(2)), logo.height());
            f.render_widget(&logo, logo_area);
            let status = format!(
                "{}  |  logo {}x{} cells  |  terminal {}x{}  |  {}  |  space: toggle, q: quit",
                if running { "RUNNING (bobbing)" } else { "STILL" },
                logo.width(), logo.height(), a.width, a.height,
                if truecolor { "24-bit" } else { "256-colour fallback (COLORTERM unset)" },
            );
            f.render_widget(Line::from(status), Rect::new(0, a.height.saturating_sub(1), a.width, 1));
        })?;
        let timeout = tick.saturating_sub(last.elapsed());
        if !event::poll(timeout)? {
            ticks += 1;
            last = Instant::now();
            continue;
        }
        if let Event::Key(k) = event::read()? {
            if k.kind != KeyEventKind::Press { continue; }
            match k.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char(' ') => running = !running,
                _ => {}
            }
        }
    }
    ratatui::restore();
    Ok(())
}

#[test]
fn asset_is_144_by_64_cells() {
    let logo = Logo::parse(LOGO, true);
    assert_eq!((logo.width(), logo.height()), (144, 64));
    assert!(logo.rows.iter().all(|r| r.len() == 144));
}
