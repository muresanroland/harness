//! The layout: header, status row, Overall, the TICKETS sections, RECENT
//! under its rule newest at the bottom (a Question takes its place when one
//! shows), the MERGE TO UNBLOCK box, the / or @ list, a notice line and the
//! input line. A plan Question docks the Shell beside it (draw/modal.rs).

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};
use ratatui::Frame;

use super::logo::{
    banner, lerp, quantize, BLUE, BORDER, CYAN, DARK_ORANGE, GRAY, GREEN, HOP, MUTED, ORANGE, PINK,
    PURPLE, RED, REST, TEXT, TICKET_COLORS,
};
use super::{suffix, About, Epic, Screen};
use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::stage::Ask;
use crate::orchestrator::stage::{plural, pr_ref, Event};
use crate::orchestrator::state::{STATUS_MERGED, STATUS_PARKED, STATUS_PR_OPEN, STATUS_RUNNING};

mod modal;

const PLACEHOLDER: &str = "  / for a command, @ for an Epic or Ticket";
const COMPOSING: &str = "  your prompt, Enter sends it, Esc goes back";
const SPINNER: [&str; 4] = ["|", "/", "—", "\\"];
/// An Epic's color, by its place on the tree.
const EPIC_COLORS: [Color; 6] = [PURPLE, CYAN, ORANGE, PINK, BLUE, GREEN];
/// The / or @ list shows this many rows at most.
const LIST_ROWS: usize = 8;

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

/// The Shell over the whole terminal, or docked beside a plan Question.
pub(crate) fn draw(f: &mut Frame, s: &Screen) {
    if s.modal() {
        modal::plan(f, s);
    } else {
        s.opened.set(None);
        shell(f, f.area(), s);
    }
    if !s.truecolor {
        for cell in f.buffer_mut().content.iter_mut() {
            cell.fg = quantize(cell.fg);
            cell.bg = quantize(cell.bg);
        }
    }
}

/// The Shell drawn into `area`: header, status row, Overall, the TICKETS
/// sections, RECENT (newest at the bottom), the boxed QUESTION (a plan
/// docks in the modal instead), the red MERGE TO UNBLOCK box, the / or @
/// list, notice, input. The row from which the list, a notice and the input
/// line show, for the fold to leave.
fn shell(f: &mut Frame, area: Rect, s: &Screen) -> u16 {
    let tree = sections(s, area.width.saturating_sub(2) as usize);
    let head_h = header_height(area);
    let unblock = unblock_lines(s);
    let unblock_h = match unblock.len() {
        0 => 0,
        n => n as u16 + 2,
    };
    // MERGE TO UNBLOCK takes its rows first, then the / or @ list, leaving
    // TICKETS its three. The TICKETS tree takes its rows and RECENT keeps at
    // least four, its rule and three lines. A Question takes RECENT's space,
    // its pane tail cut first; TICKETS gives up rows only when the question
    // and its options do not fit, and on a screen too short for even that
    // the Question's bottom is cut. A taller tree scrolls (PageUp, PageDown
    // with the input empty).
    let free = area.height.saturating_sub(head_h + 5 + unblock_h);
    let list = list_lines(
        s,
        area.width.saturating_sub(2) as usize,
        free.saturating_sub(3),
    );
    let list_h = list.len() as u16;
    let free = free - list_h;
    let mut tickets_h = (tree.len() as u16).min(free.saturating_sub(4).max(3));
    let width = area.width.saturating_sub(4) as usize;
    let asked = (s.showing() && !s.modal()).then(|| {
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
    let [head, top, over, _, tickets, recent, question, merge, lists, notice, input] =
        Layout::vertical([
            Constraint::Length(head_h),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(tickets_h),
            Constraint::Min(0),
            Constraint::Length(asked_h),
            Constraint::Length(unblock_h),
            Constraint::Length(list_h),
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
    f.render_widget(
        Paragraph::new(recent_lines(s, recent.height as usize, area.width)),
        inset(recent),
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
            About::Asked(_) => "↑↓ or a number picks, Enter answers, Esc hides",
        };
        let block = boxed(&title).title_bottom(Span::styled(format!(" {hint} "), fg(MUTED)));
        f.render_widget(Paragraph::new(lines).block(block), question);
    }
    if !unblock.is_empty() {
        f.render_widget(
            Paragraph::new(unblock).block(boxed("MERGE TO UNBLOCK").border_style(fg(RED))),
            merge,
        );
    }
    f.render_widget(Paragraph::new(list), inset(lists));
    if let Some((text, _)) = &s.notice {
        f.render_widget(
            Line::from(Span::styled(text.clone(), fg(ORANGE))),
            inset(notice),
        );
    }
    input_line(f, input, s);
    match (&s.notice, lists.height) {
        (None, 0) => input.y,
        _ => lists.y,
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
        _ if s.running && waits_on(s, t).next().is_some() => Status::Waiting,
        _ if !s.running && t.status == "in_progress" => Status::Working,
        _ => Status::Queued,
    }
}

/// The open PRs a Ticket waits on: its bd blocks dependencies on Tickets
/// whose PR is open (ADR 0002).
fn waits_on<'a>(s: &'a Screen, t: &'a BdIssue) -> impl Iterator<Item = &'a str> {
    t.blockers()
        .filter_map(|id| s.state.tickets.get(id))
        .filter(|ts| ts.status == STATUS_PR_OPEN)
        .map(|ts| ts.pr.as_str())
}

/// MERGE TO UNBLOCK's lines: each open PR a waiting Ticket depends on, and
/// every waiting Ticket's suffix, 'merge to unblock 5, 11: <url>'.
fn unblock_lines(s: &Screen) -> Vec<Line<'static>> {
    let mut prs: Vec<(&str, Vec<&str>)> = Vec::new();
    for t in listed(s).flat_map(|e| &e.tickets) {
        if status(s, t) != Status::Waiting {
            continue;
        }
        for pr in waits_on(s, t) {
            match prs.iter_mut().find(|(p, _)| *p == pr) {
                Some((_, waiting)) => waiting.push(suffix(&t.id)),
                None => prs.push((pr, vec![suffix(&t.id)])),
            }
        }
    }
    prs.into_iter()
        .map(|(pr, waiting)| {
            Line::from(vec![
                Span::styled(
                    format!("merge to unblock {}: ", waiting.join(", ")),
                    bold(RED),
                ),
                Span::styled(pr.to_string(), fg(RED)),
            ])
        })
        .collect()
}

/// An Epic's color by its place on the tree; off the tree, the first.
fn epic_color(s: &Screen, id: &str) -> Color {
    let i = listed(s).position(|e| e.id == id).unwrap_or(0);
    EPIC_COLORS[i % EPIC_COLORS.len()]
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
    for e in listed(s) {
        let c = epic_color(s, &e.id);
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
                    format!(
                        "waits on {}",
                        pr_ref(waits_on(s, t).next().unwrap_or_default())
                    )
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
/// tail as far as `room` lines allow, and the numbered options with the
/// cursor on one. Everything but the tail is always there.
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
        let first = format!("{mark} {}. ", i + 1);
        options.extend(modal::wrap_spans(
            vec![(option.clone(), style)],
            width,
            &first,
            "     ",
            style,
        ));
    }
    let fit = room.saturating_sub(lines.len() + options.len());
    if let About::Asked(Ask::Wake { tail, .. }) = &q.about {
        let tail: Vec<&str> = tail.lines().collect();
        for line in &tail[tail.len().saturating_sub(fit)..] {
            lines.push(Line::from(Span::styled(line.to_string(), fg(MUTED))));
        }
    }
    lines.extend(options);
    lines
}

/// RECENT, `height` rows: the rule, counting the lines hidden older and
/// newer, then `HH:MM:SS  <suffix> <title>  <event>` with the newest on the
/// last row, `s.recent` rows up from it, which it keeps inside the lines.
/// The Ticket column is as wide as the longest name shown, up to 34% of
/// `width`, cut with … and colored per Ticket; run-level rows read harness.
fn recent_lines(s: &Screen, height: usize, width: u16) -> Vec<Line<'static>> {
    let rows = height.saturating_sub(1);
    let back = s.recent.get().min(s.events.len().saturating_sub(rows));
    s.recent.set(back);
    let end = s.events.len() - back;
    let start = end.saturating_sub(rows);
    let shown: Vec<(&Event, String, Color)> = s.events[start..end]
        .iter()
        .map(|e| match &e.ticket {
            Some(id) => (e, s.name(id), ticket_color(id)),
            None => (e, "harness".to_string(), MUTED),
        })
        .collect();
    let name_width = shown
        .iter()
        .map(|(_, name, _)| name.chars().count())
        .max()
        .unwrap_or(0)
        .min(width as usize * 34 / 100);
    let hint = match (start, back) {
        (0, 0) => String::new(),
        (o, 0) => format!(" ↑ {o} older "),
        (0, n) => format!(" ↓ {n} newer "),
        (o, n) => format!(" ↑ {o} older · ↓ {n} newer "),
    };
    let label = format!("── RECENT {hint}");
    let fill = (width as usize).saturating_sub(label.chars().count() + 2);
    let mut lines = vec![Line::from(vec![
        Span::styled(label, fg(MUTED)),
        Span::styled("─".repeat(fill), fg(BORDER)),
    ])];
    lines.resize(height.saturating_sub(shown.len()), Line::default());
    for (e, name, c) in shown {
        lines.push(Line::from(vec![
            Span::styled(format!("{}  ", e.time.format("%H:%M:%S")), fg(MUTED)),
            Span::styled(format!("{:<name_width$}  ", cut(&name, name_width)), fg(c)),
            Span::styled(e.text.clone(), fg(TEXT)),
        ]));
    }
    lines
}

/// The / or @ list, `width` wide and at most `height` rows: a window of up
/// to LIST_ROWS rows around the cursor, marked › there, then the keys' hint;
/// nothing when no list is open or it has no room. A row's key is purple
/// for a command, else in its Epic's or Ticket's color, bold on the cursor;
/// its middle column muted; its text TEXT on the cursor, muted otherwise.
fn list_lines(s: &Screen, width: usize, height: u16) -> Vec<Line<'static>> {
    let rows = s.list();
    let shown = LIST_ROWS.min(height.saturating_sub(1) as usize);
    if rows.is_empty() || shown == 0 {
        return Vec::new();
    }
    let kw = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(0);
    let mw = rows.iter().map(|r| r.1.chars().count()).max().unwrap_or(0);
    let room = width.saturating_sub(kw + mw + 6);
    let pick = s.pick.min(rows.len() - 1);
    let start = (pick + 1).saturating_sub(shown);
    let mut lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(start)
        .take(shown)
        .map(|(n, &(key, mid, text))| {
            let on = n == pick;
            let c = match mid {
                "Epic" => epic_color(s, key),
                "Ticket" => ticket_color(key),
                _ => PURPLE,
            };
            Line::from(vec![
                Span::styled(if on { "› " } else { "  " }, bold(PURPLE)),
                Span::styled(format!("{key:<kw$}  "), if on { bold(c) } else { fg(c) }),
                Span::styled(format!("{mid:<mw$}  "), fg(MUTED)),
                Span::styled(cut(text, room), fg(if on { TEXT } else { MUTED })),
            ])
        })
        .collect();
    lines.push(Line::from(Span::styled(
        "  ↑↓ pick · Tab or Enter fills in · Esc clears",
        fg(BORDER),
    )));
    lines
}

/// The input line; feedback typed for the plan shows in the modal instead.
fn input_line(f: &mut Frame, area: Rect, s: &Screen) {
    if s.modal() && s.composing {
        return f.render_widget(Line::from("› ".fg(PURPLE).bold()), area);
    }
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
