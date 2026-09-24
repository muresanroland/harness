//! The layout: header, status row, Overall, the TICKETS sections, the RECENT
//! box newest first (a Question takes its place when one shows), a notice
//! line and the input line.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};
use ratatui::Frame;

use super::logo::{
    banner, lerp, quantize, BLUE, BORDER, CYAN, DARK_ORANGE, GRAY, GREEN, HOP, MUTED, ORANGE, PINK,
    PURPLE, REST, TEXT, TICKET_COLORS,
};
use super::{suffix, About, Epic, Screen};
use crate::orchestrator::judgment::plan_said;
use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::stage::Ask;
use crate::orchestrator::stage::{plural, pr_ref, Event};
use crate::orchestrator::state::{STATUS_MERGED, STATUS_PARKED, STATUS_PR_OPEN, STATUS_RUNNING};

const PLACEHOLDER: &str =
    "  /start-epic  /start-ticket  /continue  /stop-work  /retry  /park  /address  /exit";
const COMPOSING: &str = "  your prompt, Enter sends it, Esc goes back";
const SPINNER: [&str; 4] = ["|", "/", "—", "\\"];
/// An Epic's color, by its place on the tree.
const EPIC_COLORS: [Color; 6] = [PURPLE, CYAN, ORANGE, PINK, BLUE, GREEN];

/// A Ticket's place on the tree.
#[derive(Clone, Copy, PartialEq)]
enum Status {
    Working,
    NeedsYou,
    Waiting,
    ToMerge,
    Merged,
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

/// Header, status row, Overall, the TICKETS sections, boxed RECENT (newest
/// first), the boxed QUESTION, notice, input.
pub(crate) fn draw(f: &mut Frame, s: &Screen) {
    let area = f.area();
    let tree = sections(s, area.width.saturating_sub(2) as usize);
    let head_h = header_height(area);
    // The TICKETS tree takes its rows and RECENT keeps at least four. A
    // Question takes RECENT's space, its pane tail cut first; TICKETS gives
    // up rows only when the question and its options do not fit, and on a
    // screen too short for even that the Question's bottom is cut. A taller
    // tree scrolls (Up, Down, PageUp, PageDown with the input empty).
    let free = area.height.saturating_sub(head_h + 5);
    let mut tickets_h = (tree.len() as u16).min(free.saturating_sub(4).max(3));
    let width = area.width.saturating_sub(4) as usize;
    let asked = s.showing().then(|| {
        let least = question_lines(s, width, 0).len() as u16 + 2;
        if free.saturating_sub(tickets_h) < least {
            tickets_h = free.saturating_sub(least).max(3).min(tickets_h);
        }
        let room = free.saturating_sub(tickets_h);
        let lines = question_lines(s, width, room.saturating_sub(2) as usize);
        let height = (lines.len() as u16 + 2).min(room);
        (lines, height)
    });
    let asked_h = asked.as_ref().map_or(0, |(_, h)| *h);
    let [head, top, over, _, tickets, recent, question, notice, input] = Layout::vertical([
        Constraint::Length(head_h),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(tickets_h),
        Constraint::Min(0),
        Constraint::Length(asked_h),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    header(f, head, s);
    f.render_widget(
        status_line(s, top.width.saturating_sub(1) as usize),
        inset(top),
    );
    f.render_widget(overall(s, over.width.saturating_sub(1)), inset(over));
    f.render_widget(
        Paragraph::new(scrolled(s, tree, tickets_h as usize)),
        inset(tickets),
    );
    let inner = boxed("RECENT").inner(recent);
    f.render_widget(
        Paragraph::new(recent_lines(s, inner.height as usize, area.width)).block(boxed("RECENT")),
        recent,
    );
    if let Some((lines, _)) = asked {
        let title = match s.questions.len() {
            1 => "QUESTION".to_string(),
            n => format!("QUESTION · {} waiting", plural(n - 1, "more")),
        };
        let hint = match s.questions[0].about {
            About::Continue { .. } => {
                "Space toggles resume or reset to Implement, Enter starts, Esc cancels"
            }
            About::Confirm(_) => "y or n, Enter answers, Esc cancels",
            About::Asked(Ask::Plan { .. }) => {
                "↑↓ or a number picks, PgUp PgDn scroll the plan, Enter answers, Esc hides"
            }
            About::Asked(_) => "↑↓ or a number picks, Enter answers, Esc hides",
        };
        let block = boxed(&title).title_bottom(Span::styled(format!(" {hint} "), fg(MUTED)));
        f.render_widget(Paragraph::new(lines).block(block), question);
    }
    if let Some((text, _)) = &s.notice {
        f.render_widget(
            Line::from(Span::styled(text.clone(), fg(ORANGE))),
            inset(notice),
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

/// `r` less its first column, the one-column gutter every row but the boxes keeps.
fn inset(r: Rect) -> Rect {
    Rect::new(r.x + 1, r.y, r.width.saturating_sub(1), r.height)
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
            s.shown_version().fg(GRAY),
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
        Line::from(s.shown_version().fg(GRAY)),
        Rect::new(x, area.y + 4, w, 1),
    );
    f.render_widget(
        Line::from(s.folder.as_str().fg(MUTED)),
        Rect::new(x, area.y + 5, w, 1),
    );
}

/// The status of a Ticket: the run's State first (a live snapshot or the
/// saved run), then what bd says. Needs you and waiting are a live run's;
/// live, only the run's State says a Ticket is working.
fn status(s: &Screen, t: &BdIssue) -> Status {
    match s.state.tickets.get(&t.id).map(|ts| ts.status.as_str()) {
        Some(STATUS_PARKED) => Status::Parked,
        Some(STATUS_RUNNING) if s.running && s.blocked(&t.id) => Status::NeedsYou,
        Some(STATUS_RUNNING) => Status::Working,
        Some(STATUS_PR_OPEN) => Status::ToMerge,
        Some(STATUS_MERGED) => Status::Merged,
        _ if t.status == "closed" => Status::Merged,
        _ if s.running && waits_on(s, t).is_some() => Status::Waiting,
        _ if !s.running && t.status == "in_progress" => Status::Working,
        _ => Status::Queued,
    }
}

/// The open PR a Ticket waits on: its bd blocks dependency on a Ticket whose
/// PR is open (ADR 0002).
fn waits_on<'a>(s: &'a Screen, t: &BdIssue) -> Option<&'a str> {
    t.blockers()
        .filter_map(|id| s.state.tickets.get(id))
        .find(|ts| ts.status == STATUS_PR_OPEN)
        .map(|ts| ts.pr.as_str())
}

/// Idle: every open Epic. Live: only the Epics with a Ticket in the run.
fn listed(s: &Screen) -> impl Iterator<Item = &Epic> {
    s.epics.iter().filter(|e| {
        !s.running
            || e.id == s.state.epic
            || e.tickets
                .iter()
                .any(|t| s.state.tickets.contains_key(&t.id))
    })
}

/// Live: the spinner, RUNNING (STOPPING while Ticket threads leave) and a
/// count per label over the listed Epics' Tickets, parked only when there
/// is one; wider than `width` the glyphs go, then the end is cut. Idle:
/// IDLE, the open Epics and their Tickets, and the saved run when there is one.
fn status_line(s: &Screen, width: usize) -> Line<'static> {
    if s.running {
        let all: Vec<Status> = listed(s)
            .flat_map(|e| &e.tickets)
            .map(|t| status(s, t))
            .collect();
        let (word, c) = if s.stopping() {
            (" STOPPING", ORANGE)
        } else {
            (" RUNNING", PURPLE)
        };
        // Parked before merged, so a narrow screen cuts merged, which the
        // Overall bar also carries.
        let parts = [
            (Status::Working, "●", "working", TEXT),
            (Status::NeedsYou, "◆", "needs you", ORANGE),
            (Status::Waiting, "◇", "waiting on a merge", MUTED),
            (Status::ToMerge, "○", "to merge", BLUE),
            (Status::Parked, "◌", "parked", MUTED),
            (Status::Merged, "✓", "merged", GREEN),
        ];
        let row = |glyphs: bool| {
            let mut spans = vec![
                Span::styled(SPINNER[(s.ticks / 4) as usize % SPINNER.len()], bold(c)),
                Span::styled(word, bold(c)),
            ];
            for (want, glyph, what, c) in parts {
                let n = all.iter().filter(|st| **st == want).count();
                if want == Status::Parked && n == 0 {
                    continue;
                }
                spans.push(Span::raw("  "));
                if glyphs {
                    spans.push(Span::styled(format!("{glyph} "), bold(c)));
                }
                spans.push(Span::styled(format!("{n} {what}"), fg(c)));
            }
            Line::from(waiting(s, spans))
        };
        let line = row(true);
        return if line.width() > width {
            row(false)
        } else {
            line
        };
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
    Line::from(waiting(s, spans))
}

/// The status row ends in the hidden Questions' count.
fn waiting(s: &Screen, mut spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    if s.hidden && !s.questions.is_empty() {
        spans.push(dot());
        spans.push(Span::styled(
            format!("{} waiting", plural(s.questions.len(), "question")),
            fg(ORANGE),
        ));
    }
    spans
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

/// Cut to `width` characters, the last one an ellipsis.
fn cut(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(width.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// TICKETS, `width` wide: each listed Epic a rule line in its color, the id
/// and title, the detail and its word; its Tickets hang under it, each with
/// its indicator, suffix and title, stage and label. Idle the detail counts
/// closed, in progress and open, and an Epic with every Ticket closed folds
/// to its rule; live it counts merged, and a working Ticket's dot pulses.
fn sections(s: &Screen, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (i, e) in listed(s).enumerate() {
        let c = EPIC_COLORS[i % EPIC_COLORS.len()];
        let dim = lerp((c, BORDER), 0.55);
        let all: Vec<Status> = e.tickets.iter().map(|t| status(s, t)).collect();
        let count = |want: Status| all.iter().filter(|st| **st == want).count();
        let (n, closed, open) = (all.len(), count(Status::Merged), count(Status::Queued));
        let folded = !s.running && n > 0 && closed == n && e.id != s.state.epic;
        let (word, wc) = if s.running {
            ("RUNNING", PURPLE)
        } else if e.id == s.state.epic {
            ("RESUMABLE", PURPLE)
        } else if folded {
            ("ALL CLOSED", GREEN)
        } else if open < n {
            ("IN PROGRESS", TEXT)
        } else {
            ("NOT STARTED", MUTED)
        };
        let detail = if s.running {
            format!("{closed}/{n} merged")
        } else if folded {
            format!("all {n} closed")
        } else {
            [
                (closed, "closed"),
                (n - closed - open, "in progress"),
                (open, "open"),
            ]
            .iter()
            .filter(|(k, _)| *k > 0)
            .map(|(k, what)| format!("{k} {what}"))
            .collect::<Vec<_>>()
            .join(" · ")
        };
        let detail = Span::styled(format!(" {detail}  "), fg(MUTED));
        let word = Span::styled(format!("{word:<11}"), bold(wc));
        let right = detail.width() + word.width();
        let arrow = if folded { "▸" } else { "▾" };
        let room = width.saturating_sub(right + 6);
        let left = cut(&format!("{arrow} {}  {}", e.id, e.title), room);
        let fill = width.saturating_sub(left.chars().count() + right + 4);
        lines.push(Line::from(vec![
            Span::styled("━━ ", fg(dim)),
            Span::styled(left, bold(c)),
            Span::styled(format!(" {}", "━".repeat(fill)), fg(dim)),
            detail,
            word,
        ]));
        if folded {
            continue;
        }
        for (k, (t, &st)) in e.tickets.iter().zip(&all).enumerate() {
            let color = ticket_color(&t.id);
            let (ind, ic, label, lc, text) = match st {
                Status::Working if s.running => ("●", color, "WORKING", color, TEXT),
                Status::Working => ("●", color, "IN PROGRESS", TEXT, TEXT),
                Status::NeedsYou => ("◆", ORANGE, "NEEDS YOU", ORANGE, TEXT),
                Status::Waiting => ("◇", MUTED, "WAITING", MUTED, TEXT),
                Status::ToMerge => ("○", BLUE, "TO MERGE", BLUE, TEXT),
                Status::Merged if s.running => ("✓", GREEN, "MERGED", GREEN, TEXT),
                Status::Merged => ("✓", GREEN, "CLOSED", GREEN, MUTED),
                Status::Parked => ("◌", MUTED, "PARKED", MUTED, TEXT),
                Status::Queued => ("·", BORDER, "", BORDER, MUTED),
            };
            // a live Ticket's dot pulses, each on its own phase
            let pulsing = s.running && st == Status::Working && (s.ticks / 6 + k as u64) % 12 >= 6;
            let ic = if pulsing { lerp((ic, BORDER), 0.6) } else { ic };
            let stage = match (st, s.state.tickets.get(&t.id)) {
                (Status::Waiting, _) => {
                    format!("waits on {}", pr_ref(waits_on(s, t).unwrap_or_default()))
                }
                (Status::ToMerge, Some(ts)) => pr_ref(&ts.pr),
                (Status::Merged, Some(ts)) => format!("{} merged", pr_ref(&ts.pr)),
                (_, Some(ts)) if ts.round > 0 => format!("{} {}", ts.stage, ts.round),
                (_, Some(ts)) => ts.stage.clone(),
                (_, None) => String::new(),
            };
            let stage = Span::styled(format!("{stage:>16}  "), fg(MUTED));
            let label = Span::styled(format!("{label:<11}"), bold(lc));
            let right = stage.width() + label.width();
            let room = width.saturating_sub(right + 10);
            let name = cut(&format!("{} {}", suffix(&t.id), t.title), room);
            let branch = if k + 1 == n {
                "   └─ "
            } else {
                "   ├─ "
            };
            lines.push(Line::from(vec![
                Span::styled(branch, fg(dim)),
                Span::styled(format!("{ind} "), bold(ic)),
                Span::styled(format!("{name:<room$}  "), fg(text)),
                stage,
                label,
            ]));
        }
    }
    lines
}

/// The tree from `s.scroll`, which it keeps inside the tree, as many rows
/// as `height`: a tree that fits never scrolls, a clipped one ends in
/// "… N more, PgDn".
fn scrolled(s: &Screen, tree: Vec<Line<'static>>, height: usize) -> Vec<Line<'static>> {
    let from = s.scroll.get().min(tree.len().saturating_sub(height));
    s.scroll.set(from);
    let mut lines: Vec<Line> = tree.into_iter().skip(from).collect();
    if lines.len() > height {
        let more = lines.len() - height.saturating_sub(1);
        lines.truncate(height.saturating_sub(1));
        lines.push(Line::from(Span::styled(
            format!("   … {more} more, PgDn"),
            fg(MUTED),
        )));
    }
    lines
}

/// A Question's lines: the question, a Judgment's scores, a Wake's pane
/// tail or a plan from its scroll row as far as `room` lines allow, and
/// the numbered options with the cursor on one. Everything but the tail or
/// plan is always there.
fn question_lines(s: &Screen, width: usize, room: usize) -> Vec<Line<'static>> {
    let q = &s.questions[0];
    let head = match &q.ticket {
        Some(id) => format!("{}  {}", s.name(id), q.text),
        None => q.text.clone(),
    };
    let mut lines = vec![Line::from(Span::styled(head, bold(TEXT)))];
    let judged = match &q.about {
        About::Asked(Ask::Wake {
            judged: Some(judged),
            ..
        }) => Some(judged.said()),
        About::Asked(Ask::Plan {
            judged: Some(score),
            ..
        }) => Some(plan_said(*score)),
        _ => None,
    };
    if let Some(judged) = judged {
        lines.push(Line::from(Span::styled(
            format!("judged: {judged}"),
            fg(TEXT),
        )));
    }
    lines.push(Line::default());
    let mut options = Vec::new();
    for (i, option) in s.options().iter().enumerate() {
        let (mark, style) = if i == q.cursor {
            ("›", bold(PURPLE))
        } else {
            (" ", fg(TEXT))
        };
        for (n, piece) in wrap(option, width.saturating_sub(5))
            .into_iter()
            .enumerate()
        {
            let lead = if n == 0 {
                format!("{mark} {}. ", i + 1)
            } else {
                "     ".to_string()
            };
            options.push(Line::from(vec![
                Span::styled(lead, style),
                Span::styled(piece, style),
            ]));
        }
    }
    let fit = room.saturating_sub(lines.len() + options.len());
    match &q.about {
        About::Asked(Ask::Wake { tail, .. }) => {
            let tail: Vec<&str> = tail.lines().collect();
            for line in &tail[tail.len().saturating_sub(fit)..] {
                lines.push(Line::from(Span::styled(line.to_string(), fg(MUTED))));
            }
        }
        About::Asked(Ask::Plan { plan, .. }) => {
            let rows: Vec<String> = plan.lines().flat_map(|line| wrap(line, width)).collect();
            // Scrolled by rows at this width, and kept inside the plan.
            let from = q.scroll.get().min(rows.len().saturating_sub(fit));
            q.scroll.set(from);
            lines.extend(
                rows.into_iter()
                    .skip(from)
                    .take(fit)
                    .map(|row| Line::from(Span::styled(row, fg(TEXT)))),
            );
        }
        _ => {}
    }
    lines.extend(options);
    lines
}

/// Word-wraps text to width; a word longer than the width stays whole.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for word in text.split(' ') {
        let last = lines.last_mut().unwrap();
        if !last.is_empty() && last.chars().count() + 1 + word.chars().count() > width {
            lines.push(word.to_string());
        } else {
            if !last.is_empty() {
                last.push(' ');
            }
            last.push_str(word);
        }
    }
    lines
}

/// HH:MM:SS  <suffix> <title>  <event>; the Ticket column colored per Ticket
/// and cut to `name_width`; run-level rows read harness.
fn event_line(s: &Screen, ev: &Event, name_width: usize) -> Line<'static> {
    let (name, color) = match &ev.ticket {
        Some(id) => (s.name(id), ticket_color(id)),
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
        spans.push(if s.composing { COMPOSING } else { PLACEHOLDER }.fg(DARK_ORANGE));
    }
    f.render_widget(Line::from(spans), area);
}

fn boxed(title: &str) -> Block<'static> {
    Block::bordered()
        .title(Span::styled(format!(" {title} "), fg(MUTED)))
        .border_style(fg(BORDER))
        .padding(Padding::horizontal(1))
}
