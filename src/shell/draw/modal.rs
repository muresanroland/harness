//! The modal a plan Question docks in (layout C of harness-0sx.5): the live
//! Shell keeps the left 42% and the plan takes the right 58%, its markdown
//! styled; under 110 columns it folds to a box over the dimmed Shell. The
//! frame is its own, for anything else that docks.

use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Clear, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use ratatui::Frame;

use super::{bold, cut, fg, shell};
use crate::orchestrator::judgment::plan_said;
use crate::orchestrator::stage::Ask;
use crate::shell::logo::{lerp, BORDER, CYAN, GREEN, MUTED, ORANGE, PURPLE, RED, TEXT};
use crate::shell::{About, Screen};

/// Under this many columns the dock folds over the Shell.
const FOLD: u16 = 110;
/// What the Shell's colors fade toward behind the fold.
const DIM_TO: Color = Color::Rgb(8, 12, 20);
/// The ground of code, and of a diff's added and removed lines.
const CODE_BG: Color = Color::Rgb(24, 31, 48);
const ADD_BG: Color = Color::Rgb(16, 46, 28);
const DEL_BG: Color = Color::Rgb(56, 20, 26);

/// The frame: from 110 columns the Shell in the left 42% and a thick box in
/// the right 58%; under, the Shell dimmed behind a rounded box that leaves
/// it only the input line, with margins from 100x30 up. The box and its
/// border, which the caller titles and renders.
fn dock(f: &mut Frame, s: &Screen) -> (Rect, Block<'static>) {
    let area = f.area();
    let (rect, border) = if area.width >= FOLD {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)])
                .areas(area);
        shell(f, left, s);
        (right, BorderType::Thick)
    } else {
        shell(f, area, s);
        let above = Rect {
            height: area.height.saturating_sub(1),
            ..area
        };
        let buf = f.buffer_mut();
        for y in above.top()..above.bottom() {
            for x in above.left()..above.right() {
                let cell = &mut buf[(x, y)];
                cell.fg = lerp((cell.fg, DIM_TO), 0.7);
            }
        }
        let rect = if area.width < 100 || area.height < 30 {
            above
        } else {
            area.inner(Margin::new(area.width / 12, 2))
        };
        (rect, BorderType::Rounded)
    };
    f.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(border)
        .border_style(fg(PURPLE))
        .padding(Padding::horizontal(1));
    (rect, block)
}

/// The plan Question in the dock: its badges (the Judgment's score, the
/// other Questions, the lines since it opened), its text, the plan from its
/// scroll row with a scrollbar, and its options at the foot (docked a list,
/// folded one row), or the feedback being typed in that option's place.
pub(super) fn plan(f: &mut Frame, s: &Screen) {
    let q = &s.questions[0];
    let About::Asked(Ask::Plan { plan, judged, .. }) = &q.about else {
        return;
    };
    let width = f.area().width;
    let (rect, block) = dock(f, s);
    let options = s.options();
    let n = options.len();
    let hint = match (s.composing, rect.width < 90) {
        (true, _) => "Enter sends the feedback, Esc goes back".to_string(),
        (false, false) => format!(
            "↑↓ PgUp PgDn scroll · Tab heading · ←→ or 1-{n} pick · Enter answers · Esc hides"
        ),
        (false, true) => format!("↑↓ PgUp PgDn · Tab · ←→ 1-{n} · Enter · Esc"),
    };
    let title = format!(
        " PLAN · {} ",
        s.name(q.ticket.as_deref().unwrap_or_default())
    );
    let block = block
        .title(Span::styled(title, bold(TEXT)))
        .title_bottom(Span::styled(format!(" {hint} "), fg(MUTED)));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    // Under 100 columns the badges shorten.
    let short = width < 100;
    let mut badges = Vec::new();
    if let Some(score) = judged {
        let text = match short {
            true => format!("judged {score:.2}"),
            false => format!("judged: {}", plan_said(*score)),
        };
        badges.push(Span::styled(text, fg(TEXT)));
    }
    if s.questions.len() > 1 {
        let text = format!("{} more waiting", s.questions.len() - 1);
        badges.push(Span::styled(text, bold(ORANGE)));
    }
    let opened = s.opened.get().unwrap_or_else(chrono::Local::now);
    s.opened.set(Some(opened));
    let new = s.events.iter().filter(|e| e.time >= opened).count();
    if new > 0 {
        let text = match short {
            true => format!("{new} new"),
            false => format!("{new} new on RECENT"),
        };
        badges.push(Span::styled(text, fg(CYAN)));
    }
    let mut badge_line = Vec::new();
    for (i, badge) in badges.into_iter().enumerate() {
        if i > 0 {
            badge_line.push(Span::styled(" · ", fg(MUTED)));
        }
        badge_line.push(badge);
    }

    let folded = width < FOLD;
    let foot = if folded { 1 } else { n as u16 };
    let [head, body, rule, foot] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(foot),
    ])
    .areas(inner);
    let lead = cut(&q.text, inner.width as usize);
    f.render_widget(
        Paragraph::new(vec![
            Line::from(badge_line),
            Line::from(Span::styled(lead, fg(MUTED))),
        ]),
        head,
    );

    // The plan from its scroll row, kept inside; the last column is the
    // scrollbar's.
    let (rows, heads) = md(plan, body.width.saturating_sub(1) as usize);
    let (total, h) = (rows.len(), body.height as usize);
    s.page.set(h.saturating_sub(2).max(1));
    *s.heads.borrow_mut() = heads;
    let from = q.scroll.get().min(total.saturating_sub(h));
    q.scroll.set(from);
    let shown: Vec<Line> = rows.into_iter().skip(from).take(h).collect();
    f.render_widget(
        Paragraph::new(shown),
        Rect {
            width: body.width.saturating_sub(1),
            ..body
        },
    );
    if total > h {
        let mut state = ScrollbarState::new(total - h).position(from);
        let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .thumb_style(fg(MUTED))
            .track_style(fg(BORDER));
        f.render_stateful_widget(bar, body, &mut state);
    }
    f.render_widget(
        Line::from(Span::styled("─".repeat(rule.width as usize), fg(BORDER))),
        rule,
    );

    // The feedback being typed shows its tail.
    let w = foot.width as usize;
    let composed = || {
        let room = w.saturating_sub(12);
        let typed = s
            .input
            .chars()
            .skip(s.input.chars().count().saturating_sub(room));
        Line::from(vec![
            Span::styled("feedback › ", bold(PURPLE)),
            Span::styled(typed.collect::<String>(), fg(TEXT)),
            Span::styled("▌", fg(TEXT)),
        ])
    };
    let lines: Vec<Line> = if folded && s.composing {
        vec![composed()]
    } else if folded {
        // one row; under 80 columns each option's first word
        let mut spans = Vec::new();
        for (i, option) in options.iter().enumerate() {
            let option = match width < 80 {
                true => option.split(' ').next().unwrap_or_default(),
                false => option.as_str(),
            };
            let style = match i == q.cursor {
                true => bold(Color::Black).bg(PURPLE),
                false => fg(TEXT),
            };
            spans.push(Span::styled(format!(" {} {option} ", i + 1), style));
            spans.push(Span::raw(" "));
        }
        vec![Line::from(spans)]
    } else {
        options
            .iter()
            .enumerate()
            .map(|(i, option)| match (i == q.cursor, s.composing && i == 1) {
                (_, true) => composed(),
                (true, _) => Line::from(Span::styled(
                    cut(&format!("› {}. {option}", i + 1), w),
                    bold(PURPLE),
                )),
                (false, _) => Line::from(Span::styled(
                    cut(&format!("  {}. {option}", i + 1), w),
                    fg(TEXT),
                )),
            })
            .collect()
    };
    f.render_widget(Paragraph::new(lines), foot);
}

/// The plan's markdown as rows `width` wide, and the rows its headings
/// start on: # purple and underlined, ## cyan, ### bold; - and * bullets
/// (• and, nested, ◦) and numbered items hanging under their text; >
/// quotes muted italic; `code` orange on a tint and **bold** inline; fenced
/// code on a tinted ground, a diff's + lines green, - lines red, @@ cyan.
fn md(text: &str, width: usize) -> (Vec<Line<'static>>, Vec<usize>) {
    let width = width.max(12);
    let (mut rows, mut heads) = (Vec::new(), Vec::new());
    let mut fence = false;
    for line in text.lines() {
        let line = line.trim_end();
        let body = line.trim_start();
        if body.starts_with("```") {
            fence = !fence;
            let label = if fence {
                body.trim_start_matches('`')
            } else {
                ""
            };
            rows.push(Line::from(Span::styled(
                format!(" {label:<w$}", w = width - 1),
                fg(MUTED).bg(CODE_BG),
            )));
            continue;
        }
        if fence {
            let (c, bg) = match line {
                _ if line.starts_with("@@") => (CYAN, CODE_BG),
                _ if line.starts_with('+') => (GREEN, ADD_BG),
                _ if line.starts_with('-') => (RED, DEL_BG),
                _ => (TEXT, CODE_BG),
            };
            let chars: Vec<char> = line.chars().collect();
            let pieces = chars.chunks(width - 1).map(String::from_iter);
            for piece in pieces.chain(chars.is_empty().then(String::new)) {
                rows.push(Line::from(Span::styled(
                    format!(" {piece:<w$}", w = width - 1),
                    fg(c).bg(bg),
                )));
            }
            continue;
        }
        if body.is_empty() {
            rows.push(Line::default());
            continue;
        }
        let level = body.chars().take_while(|c| *c == '#').count();
        if (1..=3).contains(&level) && body[level..].starts_with(' ') {
            heads.push(rows.len());
            let style = [
                bold(PURPLE).add_modifier(Modifier::UNDERLINED),
                bold(CYAN),
                bold(TEXT),
            ][level - 1];
            let text = inline(&body[level + 1..], style);
            rows.extend(wrap_spans(text, width, "", "", style));
            continue;
        }
        let pad = " ".repeat(line.len() - body.len());
        let numbered = body
            .split_once(". ")
            .filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        let (first, hang, text, base, lead) =
            if let Some(item) = body.strip_prefix("- ").or(body.strip_prefix("* ")) {
                let bullet = if pad.is_empty() { "• " } else { "◦ " };
                let hang = format!("{pad}  ");
                (format!("{pad}{bullet}"), hang, item, fg(TEXT), fg(MUTED))
            } else if let Some((n, item)) = numbered {
                let first = format!("{pad}{n}. ");
                let hang = " ".repeat(first.chars().count());
                (first, hang, item, fg(TEXT), fg(MUTED))
            } else if let Some(quote) = body.strip_prefix("> ") {
                let italic = fg(MUTED).add_modifier(Modifier::ITALIC);
                (
                    "│ ".to_string(),
                    "│ ".to_string(),
                    quote,
                    italic,
                    fg(ORANGE),
                )
            } else {
                (pad.clone(), pad, body, fg(TEXT), fg(TEXT))
            };
        rows.extend(wrap_spans(inline(text, base), width, &first, &hang, lead));
    }
    (rows, heads)
}

/// A line's `code` and **bold** as styled pieces over `base`.
fn inline(text: &str, base: Style) -> Vec<(String, Style)> {
    let style = |code: bool, strong: bool| match (code, strong) {
        (true, _) => fg(ORANGE).bg(CODE_BG),
        (false, true) => base.add_modifier(Modifier::BOLD),
        (false, false) => base,
    };
    let mut pieces = Vec::new();
    let (mut piece, mut code, mut strong) = (String::new(), false, false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '`' || (c == '*' && !code && chars.next_if_eq(&'*').is_some()) {
            pieces.push((std::mem::take(&mut piece), style(code, strong)));
            if c == '`' {
                code = !code;
            } else {
                strong = !strong;
            }
        } else {
            piece.push(c);
        }
    }
    pieces.push((piece, style(code, strong)));
    pieces.retain(|(text, _)| !text.is_empty());
    pieces
}

/// Word-wraps styled pieces to `width`, `first` leading the first row and
/// `hang` the rest, both in `lead`; a word longer than the width stays whole.
fn wrap_spans(
    pieces: Vec<(String, Style)>,
    width: usize,
    first: &str,
    hang: &str,
    lead: Style,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut row = vec![Span::styled(first.to_string(), lead)];
    let (mut used, mut fresh) = (first.chars().count(), true);
    for (text, style) in pieces {
        for word in text.split_inclusive(' ') {
            if !fresh && used + word.trim_end().chars().count() > width {
                let next = vec![Span::styled(hang.to_string(), lead)];
                lines.push(Line::from(std::mem::replace(&mut row, next)));
                used = hang.chars().count();
                fresh = true;
            }
            let word = if fresh { word.trim_start() } else { word };
            if word.is_empty() {
                continue;
            }
            used += word.chars().count();
            row.push(Span::styled(word.to_string(), style));
            fresh = false;
        }
    }
    lines.push(Line::from(row));
    lines
}
