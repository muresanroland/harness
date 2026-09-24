//! The Epic summary as a read-only pager over the whole terminal (layout B
//! of harness-0sx.5): the title bar, the lead and the totals, a TICKETS
//! outline on the left from 100 columns, a section per Ticket then PARKED,
//! and the position line. No cursor: a PR opens by Cmd-clicking its url.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::modal::wrap_spans;
use super::{bold, cut, fg, inset, ticket_color};
use crate::orchestrator::stage::{plural, pr_ref};
use crate::shell::logo::{lerp, BORDER, CYAN, GREEN, MUTED, ORANGE, PURPLE, TEXT};
use crate::shell::summary::{Done, Summary};
use crate::shell::{suffix, Screen};

/// From this many columns the TICKETS outline shows, this wide.
const OUTLINE: u16 = 100;
const OUTLINE_WIDTH: u16 = 30;

pub(super) fn pager(f: &mut Frame, s: &Screen, sum: &Summary) {
    let area = f.area();
    let [top, lead, mid, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);
    let count = |n: fn(&Done) -> usize| sum.tickets.iter().map(n).sum::<usize>();
    let prs = count(|t| usize::from(!t.pr.is_empty()));
    let parked = count(|t| usize::from(t.parked.is_some()));
    let done = sum
        .tickets
        .iter()
        .all(|t| !t.pr.is_empty() || t.merged || t.parked.is_some());

    let word = if done { "EPIC DONE" } else { "EPIC SUMMARY" };
    let mut bar = vec![
        Span::styled(
            format!(" {word} · {} {} ", sum.epic, sum.title),
            bold(Color::Black).bg(PURPLE),
        ),
        Span::styled(
            format!("  {} · {parked} parked", plural(prs, "PR")),
            fg(GREEN),
        ),
    ];
    if !s.questions.is_empty() {
        let text = format!(" · {} waiting", plural(s.questions.len(), "question"));
        bar.push(Span::styled(text, bold(ORANGE)));
    }
    let new = s.events.iter().filter(|e| e.time >= sum.opened).count();
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
    let totals = format!(
        "{} · {} Findings fixed · {} skipped · {} left on its PR · {parked} parked",
        plural(count(|t| t.rounds), "Round"),
        count(|t| t.fixed),
        count(|t| t.skipped.len()),
        count(|t| t.left.len()),
    );
    let w = lead.width.saturating_sub(1) as usize;
    f.render_widget(
        Paragraph::new(vec![
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
    let (rows, heads) = sections(sum, body.width as usize);
    let (total, h) = (rows.len(), body.height as usize);
    s.page.set(h.saturating_sub(2).max(1));
    *s.heads.borrow_mut() = heads.iter().map(|(row, _)| *row).collect();
    let from = sum.scroll.get().min(total.saturating_sub(h));
    sum.scroll.set(from);
    let shown: Vec<Line> = rows.into_iter().skip(from).take(h).collect();
    f.render_widget(Paragraph::new(shown), body);

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

/// The body rows `width` wide, and the row each Ticket starts on: a section
/// per Ticket not Parked (its rule, its PR, its counts, each Finding skipped
/// and each left on the PR), then PARKED with each reason.
fn sections(sum: &Summary, width: usize) -> (Vec<Line<'static>>, Vec<(usize, &Done)>) {
    let (mut rows, mut heads) = (Vec::new(), Vec::new());
    let item = |rows: &mut Vec<Line<'static>>, first: &str, text: &str, c: Color| {
        let piece = vec![(text.to_string(), fg(c))];
        rows.extend(wrap_spans(piece, width, first, "      ", fg(c)));
    };
    for t in sum.tickets.iter().filter(|t| t.parked.is_none()) {
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
    let parked: Vec<&Done> = sum.tickets.iter().filter(|t| t.parked.is_some()).collect();
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
