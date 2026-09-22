//! PROTOTYPE (harness-7bj.8): the Shell screen over a fake run. The header (logo,
//! banner, version, folder), the status row (spinner, RUNNING, counts), the
//! Overall bar shifting purple to green with progress, the boxed Ticket table,
//! the boxed RECENT panel newest first, and the input line. Layout A of three; B and C
//! (side by side, log first) are in this branch's history.
//! Keys: s clamps the view to 80x24, space pauses the animation and the fake
//! events, q quits.
//! Throwaway. Logo and banner code copied from docs/design/logo-prototype.
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Cell, Padding, Paragraph, Row, Table, Widget},
    Frame,
};

const SOURCE: &str = include_str!("../../logo.txt");
const TICK: Duration = Duration::from_millis(50);
const HOP: [usize; 24] = [2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 1, 2];
const SPINNER: &[&str] = &["|", "/", "—", "\\"];
const VERSION: &str = "v0.1.0";

// Palette from docs/design/status-panel.rs.
const PURPLE: Color = Color::Rgb(210, 90, 255);
const BLUE: Color = Color::Rgb(60, 180, 255);
const ORANGE: Color = Color::Rgb(255, 175, 60);
const GREEN: Color = Color::Rgb(130, 240, 120);
const CYAN: Color = Color::Rgb(44, 242, 250);
const PINK: Color = Color::Rgb(255, 120, 200);
const TEXT: Color = Color::Rgb(232, 240, 255);
const MUTED: Color = Color::Rgb(132, 147, 173);
const BORDER: Color = Color::Rgb(35, 49, 74);
const TICKET_COLORS: [Color; 7] = [GREEN, CYAN, PURPLE, BLUE, ORANGE, PINK, MUTED];

// ---------- fake run ----------

#[derive(Clone, Copy, PartialEq)]
enum Status { Active, Blocked, Done, Parked, Queued }

struct Ticket { n: usize, title: &'static str, stage: &'static str, status: Status, phase: u64 }

struct Ev { t: u32, ticket: Option<usize>, text: String }

struct App {
    logo: Logo,
    truecolor: bool,
    folder: String,
    tickets: Vec<Ticket>,
    events: Vec<Ev>, // oldest first
    script: Vec<(Option<usize>, &'static str, Option<(usize, &'static str, Status)>)>,
    ticks: u64,
    running: bool,
    small: bool,
}

const T0: u32 = 12 * 3600 + 4 * 60 + 44;

fn tk(n: usize, title: &'static str, stage: &'static str, status: Status) -> Ticket {
    Ticket { n, title, stage, status, phase: (n as u64) * 3 }
}

impl App {
    fn new(truecolor: bool) -> Self {
        let folder = std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default();
        let folder = match std::env::var("HOME") {
            Ok(h) if folder.starts_with(&h) => folder.replacen(&h, "~", 1),
            _ => folder,
        };
        let tickets = vec![
            tk(1, "Rust workspace and the Tools seam", "merged", Status::Done),
            tk(2, "Port the fake herdr/bd/gh world", "PR #12 open", Status::Done),
            tk(3, "Port the Orchestrator and scheduler", "fix 1", Status::Active),
            tk(4, "The Shell screen", "implement", Status::Active),
            tk(5, "Silent self-update", "waiting for PR #12", Status::Blocked),
            tk(6, "The init gate", "review 1", Status::Blocked),
            tk(7, "Plan mode on Implement", "", Status::Queued),
        ];
        let seed: [(i32, Option<usize>, &str); 12] = [
            (-281, None, "started Epic harness-8: 7 Tickets"),
            (-184, Some(1), "merged, Ticket closed"),
            (-155, Some(4), "branch harness-8-4 created"),
            (-154, Some(4), "implement started: claude (pane 4-1)"),
            (-55, Some(6), "stuck in review 1: went idle without a result (pane 6-2)"),
            (-54, Some(6), "judged: nudge 0.31, retry 0.22, park 0.47"),
            (-54, Some(6), "asking you: stuck in review 1"),
            (-32, Some(3), "debate 1 settled: 3 to fix, 1 skipped"),
            (-31, Some(3), "fix 1 started: claude (pane 3-4)"),
            (-13, Some(2), "PR #12 opened after 2 rounds"),
            (-13, Some(5), "waiting for PR #12 to merge (Ticket 2)"),
            (0, Some(4), "implemented"),
        ];
        let events = seed.iter().map(|(dt, t, s)| Ev { t: (T0 as i32 + dt) as u32, ticket: *t, text: s.to_string() }).collect();
        // What the fake run does next, one step every 4 s: (ticket, event line, ticket mutation).
        let mut script = vec![
            (Some(4), "review 1 started: codex (pane 4-2)", Some((4, "review 1", Status::Active))),
            (Some(6), "you answered: park", None),
            (Some(6), "parked: went idle without a result", Some((6, "review 1", Status::Parked))),
            (Some(3), "fix 1 done", None),
            (Some(3), "review 2 started: codex (pane 3-5)", Some((3, "review 2", Status::Active))),
            (Some(2), "merged, Ticket closed", Some((2, "merged", Status::Done))),
            (Some(5), "branch harness-8-5 created", None),
            (Some(5), "implement started: claude (pane 5-1)", Some((5, "implement", Status::Active))),
            (Some(4), "review 1 found 2 findings", None),
            (Some(4), "debate 1 started: claude (pane 4-3)", Some((4, "debate 1", Status::Active))),
            (Some(3), "review 2 found 0 findings", None),
            (Some(3), "PR #14 opened after 2 rounds", Some((3, "PR #14 open", Status::Done))),
            (Some(7), "waiting for PR #14 to merge (Ticket 3)", Some((7, "waiting for PR #14", Status::Blocked))),
            (None, "state not saved: permission denied", None),
            (Some(4), "debate 1 settled: 1 to fix, 1 skipped", None),
            (Some(4), "fix 1 done", None),
            (Some(4), "PR #15 opened after 1 round", Some((4, "PR #15 open", Status::Done))),
            (Some(6), "retrying review 1 with a fresh session (pane 6-3)", Some((6, "review 1", Status::Active))),
            (Some(5), "implemented", Some((5, "review 1", Status::Active))),
            (Some(3), "merged, Ticket closed", Some((3, "merged", Status::Done))),
            (Some(7), "implement started: claude (pane 7-1)", Some((7, "implement", Status::Active))),
            (Some(6), "review 1 found 0 findings", None),
            (Some(6), "PR #16 opened after 1 round", Some((6, "PR #16 open", Status::Done))),
            (Some(5), "PR #17 opened after 1 round", Some((5, "PR #17 open", Status::Done))),
            (Some(7), "PR #18 opened after 1 round", Some((7, "PR #18 open", Status::Done))),
            (None, "Epic done, every Ticket closed", None),
        ];
        script.reverse();
        App { logo: Logo::parse(SOURCE, truecolor), truecolor, folder, tickets, events, script, ticks: 0, running: true, small: false }
    }

    fn now(&self) -> u32 { T0 + (self.ticks / 20) as u32 }

    fn tick(&mut self) {
        if !self.running { return; }
        self.ticks += 1;
        if self.ticks % 80 == 0 {
            if let Some((ticket, text, change)) = self.script.pop() {
                self.events.push(Ev { t: self.now(), ticket, text: text.to_string() });
                if let Some((n, stage, status)) = change {
                    let t = &mut self.tickets[n - 1];
                    t.stage = stage;
                    t.status = status;
                }
            }
        }
    }

    fn count(&self, s: Status) -> usize { self.tickets.iter().filter(|t| t.status == s).count() }
    fn prs(&self) -> usize { self.tickets.iter().filter(|t| t.stage.starts_with("PR #") || t.stage == "merged").count() }
    fn color(&self, n: usize) -> Color { TICKET_COLORS[(n - 1) % TICKET_COLORS.len()] }
}

// ---------- pieces ----------

fn hms(t: u32) -> String { format!("{:02}:{:02}:{:02}", t / 3600 % 24, t / 60 % 60, t % 60) }

fn status_line(app: &App) -> Line<'static> {
    let spinner = SPINNER[(app.ticks / 4) as usize % SPINNER.len()];
    let bold = |c: Color| Style::default().fg(c).add_modifier(Modifier::BOLD);
    let mut spans = vec![
        Span::styled(spinner.to_string(), bold(PURPLE)),
        Span::styled(" RUNNING", bold(PURPLE)),
        Span::styled(format!("    {} active", app.count(Status::Active)), Style::default().fg(TEXT)),
        Span::styled("  ·  ", Style::default().fg(BORDER)),
        Span::styled(format!("{} blocked", app.count(Status::Blocked)), Style::default().fg(ORANGE)),
        Span::styled("  ·  ", Style::default().fg(BORDER)),
        Span::styled(format!("{} complete", app.count(Status::Done)), Style::default().fg(GREEN)),
    ];
    let parked = app.count(Status::Parked);
    if parked > 0 {
        spans.push(Span::styled("  ·  ", Style::default().fg(BORDER)));
        spans.push(Span::styled(format!("{parked} parked"), Style::default().fg(MUTED)));
    }
    Line::from(spans)
}

/// Logo left, hopping while running; beside it the status line, the banner, the version and the folder.
/// Only a tiny terminal (under 64 columns or 18 rows) drops the logo and banner for two plain lines.
fn compact(area: Rect) -> bool { area.width < 64 || area.height < 18 }
fn header_height(area: Rect) -> u16 { if compact(area) { 2 } else { 8 } }

fn header(f: &mut Frame, area: Rect, app: &App) {
    if area.height < 8 || area.width < 64 { // given the compact slot by header_height
        let l = Line::from(vec!["HARNESS ".fg(TEXT).bold(), VERSION.fg(Color::Rgb(160, 160, 160)), "  ".into(), app.folder.as_str().fg(MUTED)]);
        f.render_widget(l, Rect::new(area.x, area.y, area.width, 1));
        return;
    }
    let dy = if app.running { HOP[app.ticks as usize % HOP.len()] } else { 2 };
    app.logo.render_at(Rect::new(area.x + 1, area.y, app.logo.width(), app.logo.height() + 1), dy, f.buffer_mut());
    let x = area.x + app.logo.width() + 4;
    let w = area.right().saturating_sub(x);
    banner("THE HARNESS", Rect::new(x, area.y + 1, w, 3), app.ticks as usize, app.truecolor, f.buffer_mut());
    f.render_widget(Line::from(VERSION.fg(Color::Rgb(160, 160, 160))), Rect::new(x, area.y + 4, w, 1));
    f.render_widget(Line::from(app.folder.as_str().fg(MUTED)), Rect::new(x, area.y + 5, w, 1));
}

fn indicator(t: &Ticket, ticks: u64) -> (&'static str, Color, &'static str, Color) {
    let pulsing = (ticks / 6 + t.phase) % 12 < 6;
    match t.status {
        Status::Active => (if pulsing { "●" } else { "◉" }, if pulsing { TICKET_COLORS[(t.n - 1) % 7] } else { BORDER }, "ACTIVE", TICKET_COLORS[(t.n - 1) % 7]),
        Status::Blocked => ("◆", ORANGE, "BLOCKED", ORANGE),
        Status::Done => ("✓", GREEN, "DONE", GREEN),
        Status::Parked => ("◌", MUTED, "PARKED", MUTED),
        Status::Queued => ("·", BORDER, "", BORDER),
    }
}

fn ticket_table(app: &App, with_header: bool, width: u16) -> Table<'static> {
    let narrow = width < 60;
    let rows = app.tickets.iter().map(|t| {
        let (ind, ic, label, lc) = indicator(t, app.ticks);
        let bold = |c: Color| Style::default().fg(c).add_modifier(Modifier::BOLD);
        Row::new(vec![
            Cell::from(Span::styled(ind, bold(ic))),
            Cell::from(Span::styled(format!("{} {}", t.n, t.title), Style::default().fg(TEXT))),
            Cell::from(Span::styled(t.stage, Style::default().fg(MUTED))),
            Cell::from(Span::styled(if narrow { "" } else { label }, bold(lc))),
        ])
    });
    let widths = if narrow {
        [Constraint::Length(1), Constraint::Min(12), Constraint::Length(14), Constraint::Length(0)]
    } else {
        [Constraint::Length(1), Constraint::Min(20), Constraint::Length(20), Constraint::Length(7)]
    };
    let table = Table::new(rows, widths).column_spacing(2);
    if with_header {
        table.header(Row::new(vec!["", "TICKET", "STAGE", ""]).style(Style::default().fg(BORDER)).bottom_margin(0))
    } else {
        table
    }
}

/// Straight-line blend between two colours, `t` in 0..=1.
fn lerp((a, b): (Color, Color), t: f32) -> Color {
    let (Color::Rgb(r0, g0, b0), Color::Rgb(r1, g1, b1)) = (a, b) else { return a };
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color::Rgb(mix(r0, r1), mix(g0, g1), mix(b0, b1))
}

fn overall(app: &App, width: u16) -> Line<'static> {
    let (prs, total) = (app.prs(), app.tickets.len());
    let bar = width.saturating_sub(22).min(40) as usize;
    let filled = bar * prs / total;
    let fill = paint_color(lerp((PURPLE, GREEN), prs as f32 / total as f32), app.truecolor);
    Line::from(vec![
        Span::styled("Overall  ", Style::default().fg(MUTED)),
        Span::styled("█".repeat(filled), Style::default().fg(fill)),
        Span::styled("░".repeat(bar - filled), Style::default().fg(BORDER)),
        Span::styled(format!("  {prs}/{total} PRs"), Style::default().fg(TEXT)),
    ])
}

fn event_line(app: &App, ev: &Ev, name_width: usize) -> Line<'static> {
    let (name, color) = match ev.ticket {
        Some(n) => (format!("{} {}", n, app.tickets[n - 1].title), app.color(n)),
        None => ("harness".to_string(), MUTED),
    };
    let name: String = name.chars().take(name_width).collect();
    Line::from(vec![
        Span::styled(format!("{}  ", hms(ev.t)), Style::default().fg(MUTED)),
        Span::styled(format!("{name:<name_width$}  "), Style::default().fg(color)),
        Span::styled(ev.text.clone(), Style::default().fg(TEXT)),
    ])
}

fn recent_lines(app: &App, rows: usize, width: u16, newest_first: bool) -> Vec<Line<'static>> {
    let name_width = if width < 70 { 12 } else { 22 };
    let evs: Vec<&Ev> = app.events.iter().rev().take(rows).collect();
    let evs = if newest_first { evs } else { evs.into_iter().rev().collect() };
    evs.into_iter().map(|e| event_line(app, e, name_width)).collect()
}

fn input_line(f: &mut Frame, area: Rect) {
    let l = Line::from(vec!["› ".fg(PURPLE).bold(), "▌".fg(TEXT), "  /start-epic  /continue  /retry  /park  /address  /exit".fg(BORDER)]);
    f.render_widget(l, area);
}

fn boxed(title: &'static str) -> Block<'static> {
    Block::bordered().title(Span::styled(format!(" {title} "), Style::default().fg(MUTED))).border_style(Style::default().fg(BORDER)).padding(Padding::horizontal(1))
}

// ---------- the layout ----------

/// Header, status row, Overall, boxed Ticket table, boxed RECENT (newest first), input.
fn layout_a(f: &mut Frame, area: Rect, app: &App) {
    let n = app.tickets.len() as u16;
    let [head, top, over, _, tickets, recent, _, input] = Layout::vertical([
        Constraint::Length(header_height(area)), Constraint::Length(1), Constraint::Length(1), Constraint::Length(1),
        Constraint::Length(n + 3), Constraint::Min(4), Constraint::Length(1), Constraint::Length(1),
    ]).areas(area);
    f.render_widget(status_line(app), Rect::new(top.x + 1, top.y, top.width - 1, 1));
    f.render_widget(overall(app, over.width - 1), Rect::new(over.x + 1, over.y, over.width - 1, 1));
    header(f, head, app);
    f.render_widget(ticket_table(app, true, tickets.width - 4).block(boxed("TICKETS")), tickets);
    let inner = boxed("RECENT").inner(recent);
    f.render_widget(Paragraph::new(recent_lines(app, inner.height as usize, inner.width, true)).block(boxed("RECENT")), recent);
    input_line(f, input);
}

fn switcher(f: &mut Frame, area: Rect, app: &App) {
    let text = format!(
        "  prototype    s: {}    space: {}    q: quit  ",
        if app.small { "full size" } else { "80x24" },
        if app.running { "pause" } else { "resume" },
    );
    f.render_widget(Line::from(text).style(Style::default().fg(Color::Black).bg(Color::White)), area);
}

fn draw(f: &mut Frame, app: &App) {
    let full = f.area();
    let [screen, bar] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(full);
    let screen = if app.small {
        let outer = Rect::new(0, 0, 82.min(screen.width), 26.min(screen.height));
        let block = Block::bordered().border_style(Style::default().fg(Color::Rgb(90, 90, 90))).title(" 80x24 ".fg(Color::Rgb(90, 90, 90)));
        let inner = block.inner(outer);
        f.render_widget(block, outer);
        inner
    } else {
        screen
    };
    layout_a(f, screen, app);
    switcher(f, bar, app);
}

fn main() -> std::io::Result<()> {
    let truecolor = std::env::var("COLORTERM").map(|v| v == "truecolor" || v == "24bit").unwrap_or(false);
    let mut app = App::new(truecolor);
    let mut terminal = ratatui::init();
    let mut last = Instant::now();
    loop {
        terminal.draw(|f| draw(f, &app))?;
        if !event::poll(TICK.saturating_sub(last.elapsed()))? {
            app.tick();
            last = Instant::now();
            continue;
        }
        if let Event::Key(k) = event::read()? {
            if k.kind != KeyEventKind::Press { continue; }
            match k.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char(' ') => app.running = !app.running,
                KeyCode::Char('s') => app.small = !app.small,
                _ => {}
            }
        }
    }
    ratatui::restore();
    Ok(())
}

// ---------- logo and banner, copied from docs/design/logo-prototype ----------

type Px = Option<(u8, u8, u8)>;

struct Logo { rows: Vec<Vec<Px>>, truecolor: bool }

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

    fn width(&self) -> u16 { self.rows[0].len() as u16 }
    fn height(&self) -> u16 { self.rows.len().div_ceil(2) as u16 }

    fn render_at(&self, area: Rect, dy: usize, buf: &mut Buffer) {
        let area = area.intersection(*buf.area());
        let blank = vec![None; self.rows[0].len()];
        let rows: Vec<&Vec<Px>> = std::iter::repeat(&blank).take(dy).chain(self.rows.iter()).collect();
        for (row, pair) in rows.chunks(2).enumerate() {
            let y = area.y + row as u16;
            if y >= area.bottom() { break; }
            let (top, bottom) = (pair[0], pair.get(1));
            for x in 0..area.width.min(top.len() as u16) {
                let cell = &mut buf[(area.x + x, y)];
                let hi = top[x as usize].map(|p| paint(p, self.truecolor));
                let lo = bottom.and_then(|b| b[x as usize]).map(|p| paint(p, self.truecolor));
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

impl Widget for &Logo {
    fn render(self, area: Rect, buf: &mut Buffer) { self.render_at(area, 0, buf) }
}

fn paint_color(c: Color, truecolor: bool) -> Color {
    match c { Color::Rgb(r, g, b) => paint((r, g, b), truecolor), other => other }
}

fn paint((r, g, b): (u8, u8, u8), truecolor: bool) -> Color {
    if truecolor {
        Color::Rgb(r, g, b)
    } else {
        let q = |v: u8| ((v as u16 * 5 + 127) / 255) as u8;
        Color::Indexed(16 + 36 * q(r) + 6 * q(g) + q(b))
    }
}

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

#[test]
fn draws_at_every_size_and_the_bar_ends_green() {
    use ratatui::{backend::TestBackend, Terminal};
    let mut app = App::new(true);
    for (w, h) in [(200, 60), (120, 40), (80, 24), (60, 18), (40, 10)] {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, &app)).unwrap();
    }
    for _ in 0..80 * 30 { app.tick(); }
    assert!(app.script.is_empty());
    assert_eq!(app.prs(), app.tickets.len());
    assert_eq!(lerp((PURPLE, GREEN), 1.0), GREEN);
    assert_eq!(lerp((PURPLE, GREEN), 0.0), PURPLE);
}

#[test]
#[ignore]
fn dump() {
    use ratatui::{backend::TestBackend, Terminal};
    let mut app = App::new(true);
    for _ in 0..80 * 4 { app.tick(); }
    for (w, h) in [(120, 36), (80, 24)] {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, &app)).unwrap();
        let b = t.backend().buffer();
        println!("==== {}x{}", w, h);
        for y in 0..h { println!("{}", (0..w).map(|x| b[(x, y)].symbol()).collect::<String>()); }
    }
}
