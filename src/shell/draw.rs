//! The layout, folded in from docs/design/screen-prototype: header, status
//! row, Overall, the TICKETS box, the RECENT box newest first, a notice line
//! and the input line.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Padding, Paragraph, Row, Table};
use ratatui::Frame;

use super::logo::{
    banner, lerp, quantize, BORDER, GRAY, GREEN, HOP, MUTED, ORANGE, PURPLE, REST, TEXT,
    TICKET_COLORS,
};
use super::{suffix, Screen};
use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::stage::{plural, pr_ref, Event};
use crate::orchestrator::state::{STATUS_MERGED, STATUS_PARKED, STATUS_PR_OPEN, STATUS_RUNNING};

const PLACEHOLDER: &str =
    "  /start-epic  /start-ticket  /continue  /stop-work  /retry  /park  /address  /exit";
const SPINNER: [&str; 4] = ["|", "/", "—", "\\"];

/// A Ticket's place on the tree.
#[derive(Clone, Copy, PartialEq)]
enum Status {
    Active,
    Blocked,
    Done,
    Parked,
    Queued,
}

fn fg(c: Color) -> Style {
    Style::default().fg(c)
}

fn bold(c: Color) -> Style {
    fg(c).add_modifier(Modifier::BOLD)
}

fn dot() -> Span<'static> {
    Span::styled("  ·  ", fg(BORDER))
}

/// A Ticket's color, by its child suffix.
pub(crate) fn ticket_color(id: &str) -> Color {
    let s = suffix(id);
    let n = s
        .parse::<usize>()
        .unwrap_or_else(|_| s.bytes().map(usize::from).sum::<usize>() + 1);
    TICKET_COLORS[n.wrapping_sub(1) % TICKET_COLORS.len()]
}

/// Header, status row, Overall, boxed TICKETS, boxed RECENT (newest first), notice, input.
pub(crate) fn draw(f: &mut Frame, s: &Screen) {
    let area = f.area();
    let rows = s.rows() as u16;
    let head_h = header_height(area);
    // The TICKETS box takes its rows; RECENT keeps at least four. A taller
    // tree scrolls (Up, Down, PageUp, PageDown with the input empty).
    let free = area.height.saturating_sub(head_h + 5);
    let tickets_h = (rows + 3).min(free.saturating_sub(4).max(3));
    let [head, top, over, _, tickets, recent, notice, input] = Layout::vertical([
        Constraint::Length(head_h),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(tickets_h),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    header(f, head, s);
    f.render_widget(
        status_line(s),
        Rect::new(top.x + 1, top.y, top.width.saturating_sub(1), 1),
    );
    f.render_widget(
        overall(s, over.width.saturating_sub(1)),
        Rect::new(over.x + 1, over.y, over.width.saturating_sub(1), 1),
    );
    f.render_widget(
        ticket_table(s, area.width, tickets_h.saturating_sub(3) as usize).block(boxed("TICKETS")),
        tickets,
    );
    let inner = boxed("RECENT").inner(recent);
    f.render_widget(
        Paragraph::new(recent_lines(s, inner.height as usize, area.width)).block(boxed("RECENT")),
        recent,
    );
    if let Some((text, _)) = &s.notice {
        f.render_widget(
            Line::from(Span::styled(text.clone(), fg(ORANGE))),
            Rect::new(notice.x + 1, notice.y, notice.width.saturating_sub(1), 1),
        );
    }
    input_line(f, input, s);
    if !s.truecolor {
        for cell in f.buffer_mut().content.iter_mut() {
            cell.fg = quantize(cell.fg);
            cell.bg = quantize(cell.bg);
        }
    }
}

/// Only a tiny terminal (under 64 columns or 18 rows) drops the logo and banner for one plain line.
fn compact(area: Rect) -> bool {
    area.width < 64 || area.height < 18
}

fn header_height(area: Rect) -> u16 {
    if compact(area) {
        1
    } else {
        8
    }
}

/// Logo left, hopping while a run is live; beside it the banner, the version in gray and the folder.
fn header(f: &mut Frame, area: Rect, s: &Screen) {
    if area.height < 8 || area.width < 64 {
        let l = Line::from(vec![
            "HARNESS ".fg(TEXT).bold(),
            s.version.as_str().fg(GRAY),
            "  ".into(),
            s.folder.as_str().fg(MUTED),
        ]);
        f.render_widget(l, Rect::new(area.x, area.y, area.width, 1));
        return;
    }
    let dy = if s.running {
        HOP[s.ticks as usize % HOP.len()]
    } else {
        REST
    };
    s.logo.render_at(
        Rect::new(area.x + 1, area.y, s.logo.width(), s.logo.height() + 1),
        dy,
        f.buffer_mut(),
    );
    let x = area.x + s.logo.width() + 4;
    let w = area.right().saturating_sub(x);
    banner(
        "THE HARNESS",
        Rect::new(x, area.y + 1, w, 3),
        s.ticks,
        f.buffer_mut(),
    );
    f.render_widget(
        Line::from(s.version.as_str().fg(GRAY)),
        Rect::new(x, area.y + 4, w, 1),
    );
    f.render_widget(
        Line::from(s.folder.as_str().fg(MUTED)),
        Rect::new(x, area.y + 5, w, 1),
    );
}

/// The status of a Ticket: the run's State first (a live snapshot or the
/// saved run), then what bd says.
fn status(s: &Screen, t: &BdIssue) -> Status {
    match s.state.tickets.get(&t.id).map(|ts| ts.status.as_str()) {
        Some(STATUS_PARKED) => Status::Parked,
        Some(STATUS_RUNNING) if s.blocked(&t.id) => Status::Blocked,
        Some(STATUS_RUNNING) => Status::Active,
        Some(STATUS_PR_OPEN | STATUS_MERGED) => Status::Done,
        _ if t.status == "closed" => Status::Done,
        _ if t.status == "in_progress" => Status::Active,
        _ => Status::Queued,
    }
}

/// Live: the spinner, RUNNING and the counts over the run's Tickets. Idle:
/// IDLE, the open Epics and their Tickets, and the saved run when there is one.
fn status_line(s: &Screen) -> Line<'static> {
    if s.running {
        let count = |want: Status| {
            s.epics
                .iter()
                .flat_map(|e| &e.tickets)
                .filter(|t| s.state.tickets.contains_key(&t.id) && status(s, t) == want)
                .count()
        };
        let mut spans = vec![
            Span::styled(
                SPINNER[(s.ticks / 4) as usize % SPINNER.len()],
                bold(PURPLE),
            ),
            Span::styled(" RUNNING", bold(PURPLE)),
            Span::styled(format!("    {} active", count(Status::Active)), fg(TEXT)),
            dot(),
            Span::styled(format!("{} blocked", count(Status::Blocked)), fg(ORANGE)),
            dot(),
            Span::styled(format!("{} complete", count(Status::Done)), fg(GREEN)),
        ];
        let parked = count(Status::Parked);
        if parked > 0 {
            spans.push(dot());
            spans.push(Span::styled(format!("{parked} parked"), fg(MUTED)));
        }
        return Line::from(spans);
    }
    let tickets: usize = s.epics.iter().map(|e| e.tickets.len()).sum();
    let mut spans = vec![
        Span::styled("○ IDLE", bold(MUTED)),
        Span::styled(
            format!("    {}", plural(s.epics.len(), "open Epic")),
            fg(TEXT),
        ),
        dot(),
        Span::styled(plural(tickets, "Ticket"), fg(TEXT)),
    ];
    if !s.state.epic.is_empty() {
        spans.push(dot());
        spans.push(Span::styled(
            format!("saved run on {}, /continue resumes", s.state.epic),
            fg(PURPLE),
        ));
    }
    Line::from(spans)
}

/// Filled cells over empty, labelled N/M PRs; purple blending to green by the
/// share of the saved Epic's Tickets (every child on the bd tree, started or
/// not) with a PR open or merged.
fn overall(s: &Screen, width: u16) -> Line<'static> {
    let total = s
        .epics
        .iter()
        .find(|e| e.id == s.state.epic)
        .map_or(s.state.tickets.len(), |e| e.tickets.len());
    let prs = s
        .state
        .tickets
        .values()
        .filter(|ts| ts.status == STATUS_PR_OPEN || ts.status == STATUS_MERGED)
        .count();
    let bar = width.saturating_sub(22).min(40) as usize;
    let filled = (bar * prs).checked_div(total).unwrap_or(0);
    let share = prs as f32 / total.max(1) as f32;
    Line::from(vec![
        Span::styled("Overall  ", fg(MUTED)),
        Span::styled("█".repeat(filled), fg(lerp((PURPLE, GREEN), share))),
        Span::styled("░".repeat(bar - filled), fg(BORDER)),
        Span::styled(format!("  {prs}/{total} PRs"), fg(TEXT)),
    ])
}

/// One row per Epic, then one per Ticket: indicator (● pulsing while live,
/// ◆ blocked, ✓ done, ◌ parked, · queued), suffix and title, the Stage in
/// muted text, and ACTIVE / BLOCKED / DONE / PARKED in bold; the Epic of the
/// run reads RUNNING or RESUMABLE. Under 60 terminal columns the label goes
/// and the indicator carries the status. From `s.scroll`, `visible` rows; a
/// clipped tree ends in "… N more".
fn ticket_table(s: &Screen, width: u16, visible: usize) -> Table<'static> {
    let narrow = width < 60;
    let label = |text: &'static str, c: Color| {
        Cell::from(Span::styled(if narrow { "" } else { text }, bold(c)))
    };
    let mut rows = Vec::new();
    for epic in &s.epics {
        rows.push(Row::new(vec![
            Cell::from(Span::styled("▾", bold(MUTED))),
            Cell::from(Span::styled(
                format!("{}  {}", epic.id, epic.title),
                bold(TEXT),
            )),
            Cell::from(Span::styled(
                format!("{} Tickets", epic.tickets.len()),
                fg(MUTED),
            )),
            match (epic.id == s.state.epic, s.running) {
                (false, _) => label("", BORDER),
                (true, true) => label("RUNNING", PURPLE),
                (true, false) => label("RESUMABLE", PURPLE),
            },
        ]));
        for (n, t) in epic.tickets.iter().enumerate() {
            let color = ticket_color(&t.id);
            let saved = s.state.tickets.get(&t.id);
            let (ind, ic, text, status) = match status(s, t) {
                Status::Parked => ("◌", MUTED, TEXT, label("PARKED", MUTED)),
                Status::Blocked => ("◆", ORANGE, TEXT, label("BLOCKED", ORANGE)),
                Status::Done => ("✓", GREEN, TEXT, label("DONE", GREEN)),
                Status::Active => ("●", color, TEXT, label("ACTIVE", color)),
                Status::Queued => ("·", BORDER, MUTED, label("", BORDER)),
            };
            // a live Ticket's dot pulses, each on its own phase
            let pulsing = s.running && ind == "●" && (s.ticks / 6 + n as u64) % 12 >= 6;
            let ic = if pulsing { lerp((ic, BORDER), 0.6) } else { ic };
            let stage = saved.map_or(String::new(), |ts| match ts.status.as_str() {
                STATUS_PR_OPEN => pr_ref(&ts.pr),
                STATUS_MERGED => "merged".to_string(),
                _ if ts.round > 0 => format!("{} {}", ts.stage, ts.round),
                _ => ts.stage.clone(),
            });
            rows.push(Row::new(vec![
                Cell::from(Span::styled(ind, bold(ic))),
                Cell::from(Span::styled(
                    format!("{} {}", suffix(&t.id), t.title),
                    fg(text),
                )),
                Cell::from(Span::styled(stage, fg(MUTED))),
                status,
            ]));
        }
    }
    let mut rows: Vec<Row> = rows.into_iter().skip(s.scroll).collect();
    if rows.len() > visible {
        let more = rows.len() - visible.saturating_sub(1);
        rows.truncate(visible.saturating_sub(1));
        rows.push(Row::new(vec![
            Cell::from(""),
            Cell::from(Span::styled(format!("… {more} more"), fg(MUTED))),
        ]));
    }
    let widths = if narrow {
        [
            Constraint::Length(1),
            Constraint::Min(12),
            Constraint::Length(14),
            Constraint::Length(0),
        ]
    } else {
        [
            Constraint::Length(1),
            Constraint::Min(20),
            Constraint::Length(20),
            Constraint::Length(9),
        ]
    };
    Table::new(rows, widths)
        .column_spacing(2)
        .header(Row::new(vec!["", "TICKET", "STAGE", ""]).style(fg(BORDER)))
}

/// HH:MM:SS  <suffix> <title>  <event>; the Ticket column colored per Ticket
/// and cut to `name_width`; run-level rows read harness.
fn event_line(s: &Screen, ev: &Event, name_width: usize) -> Line<'static> {
    let (name, color) = match &ev.ticket {
        Some(id) => (
            match s.title(id) {
                Some(title) => format!("{} {title}", suffix(id)),
                None => id.clone(),
            },
            ticket_color(id),
        ),
        None => ("harness".to_string(), MUTED),
    };
    let name: String = name.chars().take(name_width).collect();
    Line::from(vec![
        Span::styled(format!("{}  ", ev.time.format("%H:%M:%S")), fg(MUTED)),
        Span::styled(format!("{name:<name_width$}  "), fg(color)),
        Span::styled(ev.text.clone(), fg(TEXT)),
    ])
}

/// Newest first; the Ticket column narrows to 12 under 70 terminal columns.
fn recent_lines(s: &Screen, rows: usize, width: u16) -> Vec<Line<'static>> {
    let name_width = if width < 70 { 12 } else { 22 };
    s.events
        .iter()
        .rev()
        .take(rows)
        .map(|e| event_line(s, e, name_width))
        .collect()
}

fn input_line(f: &mut Frame, area: Rect, s: &Screen) {
    let mut spans = vec![
        "› ".fg(PURPLE).bold(),
        s.input.as_str().fg(TEXT),
        "▌".fg(TEXT),
    ];
    if s.input.is_empty() {
        spans.push(PLACEHOLDER.fg(BORDER));
    }
    f.render_widget(Line::from(spans), area);
}

fn boxed(title: &'static str) -> Block<'static> {
    Block::bordered()
        .title(Span::styled(format!(" {title} "), fg(MUTED)))
        .border_style(fg(BORDER))
        .padding(Padding::horizontal(1))
}
