//! The Epic summary as a read-only pager over the whole terminal (layout B
//! of harness-0sx.5): the title bar, the Epic's cost and time, the lead and
//! the totals, a TICKETS outline on the left from 100 columns, the cost
//! table, a section per Ticket then PARKED, and the position line. No
//! cursor: a PR opens by Cmd-clicking its url.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::modal::{scrolled, wrap_spans};
use super::{bold, cut, fg, inset, ticket_color};
use crate::orchestrator::cost::{Cost, SUPPORTED};
use crate::orchestrator::stage::{plural, pr_ref};
use crate::shell::brand::{lerp, BORDER, CYAN, GREEN, MUTED, ORANGE, PURPLE, TEXT};
use crate::shell::summary::{Summary, Ticket};
use crate::shell::{suffix, Screen};

/// From this many columns the TICKETS outline shows, this wide.
const OUTLINE: u16 = 100;
const OUTLINE_WIDTH: u16 = 30;

/// The summary over the whole terminal, its body from its scroll row; the
/// totals count the Tickets with a section, not the Parked ones.
pub(super) fn pager(f: &mut Frame, s: &Screen, summary: &Summary) {
    let area = f.area();
    let [top, lead, mid, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(4),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);
    let listed = || summary.tickets.iter().filter(|t| t.parked.is_none());
    let (prs, totals) = counts(summary);
    // as Screen::all_prs_open, which opens it by itself
    let done = listed().next().is_some() && listed().all(|t| !t.pr.is_empty() || t.merged);

    let word = if done { "EPIC DONE" } else { "EPIC SUMMARY" };
    let mut bar = vec![
        Span::styled(
            format!(" {word} · {} {} ", summary.epic, summary.title),
            bold(Color::Black).bg(PURPLE),
        ),
        Span::styled(format!("  {prs}"), fg(GREEN)),
    ];
    if !s.questions.is_empty() {
        let text = format!(" · {} waiting", plural(s.questions.len(), "question"));
        bar.push(Span::styled(text, bold(ORANGE)));
    }
    let new = s.events.iter().filter(|e| e.time >= summary.opened).count();
    if new > 0 {
        bar.push(Span::styled(format!(" · {new} new on RECENT"), fg(CYAN)));
    }
    f.render_widget(Line::from(bar), top);

    let text = match done {
        true => {
            "Every Ticket has its PR. Review and merge them; each Ticket closes as its PR merges."
        }
        false => "Not every Ticket has its PR yet.",
    };
    let w = lead.width.saturating_sub(1) as usize;
    let time = summary.time.map_or("-".to_string(), hm);
    let spent = format!(
        "Cost {} API-equivalent, at list prices, not what was billed · time {time}",
        dollars(&summary.cost)
    );
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(cut(&spent, w), bold(TEXT))),
            Line::from(Span::styled(cut(text, w), fg(TEXT))),
            Line::from(Span::styled(cut(&totals, w), fg(MUTED))),
        ]),
        inset(lead),
    );

    // The body from its scroll row, kept inside, with a column each side.
    let outline_w = if area.width >= OUTLINE {
        OUTLINE_WIDTH
    } else {
        0
    };
    let [outline, body] =
        Layout::horizontal([Constraint::Length(outline_w), Constraint::Min(0)]).areas(mid);
    let body = Rect {
        width: body.width.saturating_sub(2),
        ..inset(body)
    };
    // The cost table first, then the sections.
    let mut rows = cost_table(summary, body.width as usize);
    let (sections, heads) = sections(summary, body.width as usize);
    let heads: Vec<(usize, &Ticket)> = heads
        .into_iter()
        .map(|(row, t)| (row + rows.len(), t))
        .collect();
    rows.extend(sections);
    let (total, h) = (rows.len(), body.height as usize);
    let starts = heads.iter().map(|(row, _)| *row).collect();
    let from = scrolled(f, s, rows, starts, &summary.scroll, body);

    // The outline marks the Ticket the first row shown belongs to.
    if outline_w > 0 {
        let here = heads.iter().rposition(|(row, _)| *row <= from);
        let mut lines = vec![Line::from(Span::styled(" TICKETS", fg(MUTED)))];
        for (i, (_, t)) in heads.iter().enumerate() {
            let (mark, style) = match Some(i) == here {
                true => ("›", bold(PURPLE)),
                false => (" ", fg(ticket_color(&t.id))),
            };
            let name = format!(" {mark} {} {}", suffix(&t.id), t.title);
            let name = cut(&name, outline.width.saturating_sub(1) as usize);
            lines.push(Line::from(Span::styled(name, style)));
        }
        let edge = Block::default()
            .borders(Borders::RIGHT)
            .border_style(fg(BORDER));
        f.render_widget(Paragraph::new(lines).block(edge), outline);
    }

    let last = (from + h).min(total);
    let pct = (last * 100).checked_div(total).unwrap_or(100);
    let left = Span::styled(
        format!(" rows {}–{last} of {total} · {pct}%", from + 1),
        fg(MUTED),
    );
    let right = Span::styled(
        "↑↓ PgUp PgDn Space scroll · Tab Ticket · Esc closes ",
        fg(MUTED),
    );
    let pad = (foot.width as usize).saturating_sub(left.width() + right.width());
    f.render_widget(
        Line::from(vec![left, Span::raw(" ".repeat(pad)), right]),
        foot,
    );
}

/// The title bar's 'N PRs · N parked' and the totals line; the totals
/// count the Tickets with a section, not the Parked ones.
fn counts(summary: &Summary) -> (String, String) {
    let listed = || summary.tickets.iter().filter(|t| t.parked.is_none());
    let count = |n: fn(&Ticket) -> usize| listed().map(n).sum::<usize>();
    let parked = summary.tickets.len() - listed().count();
    let prs = plural(count(|t| usize::from(!t.pr.is_empty())), "PR");
    let totals = format!(
        "{} · {} Findings fixed · {} skipped · {} left on its PR · {parked} parked",
        plural(count(|t| t.rounds), "Round"),
        count(|t| t.fixed),
        count(|t| t.skipped.len()),
        count(|t| t.left.len()),
    );
    (format!("{prs} · {parked} parked"), totals)
}

/// The summary as plain text 72 columns wide, the close reason of a done
/// Epic: its counts, its totals, then the pager's body rows.
pub(crate) fn plain(summary: &Summary) -> String {
    let (prs, totals) = counts(summary);
    let (rows, _) = sections(summary, 72);
    let body = rows.iter().map(|row| {
        let text: String = row.spans.iter().map(|s| s.content.as_ref()).collect();
        text.trim_end().to_string()
    });
    [prs, totals, String::new()]
        .into_iter()
        .chain(body)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

/// The cost table `width` wide: a row per Ticket with its Apps, tokens,
/// API-equivalent cost and time, a row per App whose cost is not read yet,
/// then the totals, the time the Epic's on the wall clock.
fn cost_table(summary: &Summary, width: usize) -> Vec<Line<'static>> {
    let head = ["Ticket", "Apps", "tokens", "cost", "time"].map(String::from);
    let row = |id: &str, cost: &Cost, time: String| {
        let apps = cost.apps.iter().cloned().collect::<Vec<_>>().join(", ");
        [
            id.to_string(),
            apps,
            tokens(cost.tokens),
            dollars(cost),
            time,
        ]
    };
    let mut cells = vec![(head, bold(MUTED))];
    for t in &summary.tickets {
        let time = t.time.map_or("-".to_string(), |span| match span.pr {
            true => hm(span.length()),
            false => format!("{}, no PR", hm(span.length())),
        });
        cells.push((row(suffix(&t.id), &t.cost, time), fg(ticket_color(&t.id))));
    }
    let mut total = row(
        "total",
        &summary.cost,
        summary.time.map_or("-".to_string(), hm),
    );
    total[1].clear();
    let at = cells.len();
    cells.push((total, bold(TEXT)));
    let w: [usize; 5] = std::array::from_fn(|i| {
        cells
            .iter()
            .map(|(c, _)| c[i].chars().count())
            .max()
            .unwrap_or(0)
    });
    let mut rows: Vec<Line<'static>> = cells
        .into_iter()
        .map(|(c, style)| {
            let text = format!(
                "{:<a$}  {:<b$}  {:>t$}  {:>d$}  {}",
                c[0],
                c[1],
                c[2],
                c[3],
                c[4],
                a = w[0],
                b = w[1],
                t = w[2],
                d = w[3]
            );
            Line::from(Span::styled(cut(text.trim_end(), width), style))
        })
        .collect();
    let unsupported = summary
        .cost
        .apps
        .iter()
        .filter(|app| !SUPPORTED.contains(&app.as_str()));
    let notes = unsupported.map(|app| {
        let text = cut(&format!("{app}: app not supported yet"), width);
        Line::from(Span::styled(text, fg(ORANGE)))
    });
    rows.splice(at..at, notes.collect::<Vec<_>>());
    rows.push(Line::default());
    rows
}

/// "$12.34"; "$12.34 + unpriced" with tokens on a model with no price, and
/// "unpriced" with only those.
fn dollars(cost: &Cost) -> String {
    match (cost.unpriced, cost.dollars > 0.0) {
        (true, false) => "unpriced".to_string(),
        (true, true) => format!("${:.2} + unpriced", cost.dollars),
        (false, _) => format!("${:.2}", cost.dollars),
    }
}

/// "950", "48k", "8.3M".
fn tokens(n: u64) -> String {
    match n {
        1_000_000.. => format!("{:.1}M", n as f64 / 1e6),
        1_000.. => format!("{}k", n / 1_000),
        _ => n.to_string(),
    }
}

/// "1h 32m"; minutes alone under an hour.
fn hm(d: chrono::TimeDelta) -> String {
    let m = d.num_minutes();
    match m / 60 {
        0 => format!("{m}m"),
        h => format!("{h}h {}m", m % 60),
    }
}

/// The body rows `width` wide, and the row each Ticket starts on: a section
/// per Ticket not Parked (its rule, its PR, its counts, each Finding skipped
/// and each left on the PR), then PARKED with each reason.
fn sections(summary: &Summary, width: usize) -> (Vec<Line<'static>>, Vec<(usize, &Ticket)>) {
    let (mut rows, mut heads) = (Vec::new(), Vec::new());
    let item = |rows: &mut Vec<Line<'static>>, first: &str, text: &str, c: Color| {
        let piece = vec![(text.to_string(), fg(c))];
        rows.extend(wrap_spans(piece, width, first, "      ", fg(c)));
    };
    for t in summary.tickets.iter().filter(|t| t.parked.is_none()) {
        heads.push((rows.len(), t));
        let c = ticket_color(&t.id);
        let (word, wc) = match (t.merged, t.pr.is_empty()) {
            (true, _) => ("merged", GREEN),
            (false, false) => ("to merge", ORANGE),
            (false, true) => ("no PR yet", MUTED),
        };
        let used = word.chars().count() + 2;
        let name = cut(
            &format!("{} {}", suffix(&t.id), t.title),
            width.saturating_sub(used + 3),
        );
        let fill = width.saturating_sub(name.chars().count() + used);
        rows.push(Line::from(vec![
            Span::styled(name, bold(c)),
            Span::styled(
                format!(" {} ", "─".repeat(fill)),
                fg(lerp((c, BORDER), 0.55)),
            ),
            Span::styled(word, bold(wc)),
        ]));
        if !t.pr.is_empty() {
            rows.push(Line::from(vec![
                Span::styled(format!("  {}  ", pr_ref(&t.pr)), bold(TEXT)),
                Span::styled(t.pr.clone(), fg(CYAN).add_modifier(Modifier::UNDERLINED)),
            ]));
        }
        let counts = format!(
            "  {} · {} fixed · {} skipped · {} left",
            plural(t.rounds, "Round"),
            t.fixed,
            t.skipped.len(),
            t.left.len()
        );
        rows.push(Line::from(Span::styled(counts, fg(MUTED))));
        for finding in &t.skipped {
            item(&mut rows, "    skipped: ", finding, MUTED);
        }
        for finding in &t.left {
            item(&mut rows, "    left on the PR: ", finding, ORANGE);
        }
        rows.push(Line::default());
    }
    let parked: Vec<&Ticket> = summary
        .tickets
        .iter()
        .filter(|t| t.parked.is_some())
        .collect();
    if !parked.is_empty() {
        rows.push(Line::from(Span::styled("PARKED", bold(MUTED))));
    }
    for t in parked {
        heads.push((rows.len(), t));
        let name = format!("  {} {}", suffix(&t.id), t.title);
        rows.push(Line::from(Span::styled(name, bold(ticket_color(&t.id)))));
        let reason = t.parked.as_deref().unwrap_or_default();
        item(&mut rows, "    parked: ", reason, MUTED);
    }
    (rows, heads)
}
