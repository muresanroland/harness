//! /config in the dock (draw/modal.rs's frame): the Pipeline's sections down
//! the left, the picked one's page or a pick list on the right, two lines of
//! foot under a rule.

use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use super::modal::{dock, wrap_spans};
use super::{bold, cut, fg, SPINNER};
use crate::orchestrator::app::APPS;
use crate::shell::config::{distinct, Field, Pick, Settings, ROWS, SECTIONS};
use crate::shell::logo::{BORDER, CYAN, GREEN, MUTED, ORANGE, PURPLE, TEXT};
use crate::shell::Screen;

/// The ground of the row under the cursor, and of the section whose page has it.
const SEL_BG: Color = Color::Rgb(44, 36, 78);
const REST_BG: Color = Color::Rgb(28, 32, 50);

pub(super) fn config(f: &mut Frame, s: &Screen) {
    let st = s.settings.as_ref().unwrap();
    let (rect, block) = dock(f, s);
    let block = block
        .title(Span::styled(" /config ", bold(TEXT)))
        .title(Line::from(badges(s, st)).right_aligned())
        .title_bottom(Span::styled(format!(" {} ", hint(st)), fg(MUTED)));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let [main, rule, foot] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(2),
    ])
    .areas(inner);
    f.render_widget(divider(rule.width as usize), rule);
    f.render_widget(Paragraph::new(foot_lines(s, st, foot.width as usize)), foot);
    let [left, gap, right] = Layout::horizontal([
        Constraint::Length(if main.width < 80 { 22 } else { 28 }),
        Constraint::Length(2),
        Constraint::Min(0),
    ])
    .areas(main);
    let bar = vec![Line::from(Span::styled("│", fg(BORDER))); gap.height as usize];
    f.render_widget(Paragraph::new(bar), gap);
    f.render_widget(
        Paragraph::new(pipeline(s, st, left.height, left.width as usize)),
        left,
    );
    let width = right.width as usize;
    let (lines, at) = match &st.pick {
        Some(pick) => pick_lines(st, pick, width),
        None => page(st, width),
    };
    // the cursor's line in view
    let top = (at + 2).saturating_sub(right.height as usize);
    f.render_widget(Paragraph::new(lines).scroll((top as u16, 0)), right);
}

fn divider(width: usize) -> Line<'static> {
    Line::from(Span::styled("─".repeat(width), fg(BORDER)))
}

/// `text` cut and padded to `width`.
fn pad(text: &str, width: usize) -> String {
    format!("{:<width$}", cut(text, width.saturating_sub(1)))
}

/// A row filled to `width` on `bg`: the cursor's.
fn filled(mut spans: Vec<Span<'static>>, width: usize, bg: Color) -> Line<'static> {
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    spans.push(Span::raw(" ".repeat(width.saturating_sub(used))));
    Line::from(spans).style(Style::default().bg(bg))
}

/// The title's right end: the Questions waiting behind it, a live run, the
/// last save.
fn badges(s: &Screen, st: &Settings) -> Vec<Span<'static>> {
    let mut badges = Vec::new();
    if !s.questions.is_empty() {
        badges.push(Span::styled(
            format!("{} waiting", s.questions.len()),
            bold(ORANGE),
        ));
    }
    if s.running {
        badges.push(Span::styled("run live", fg(CYAN)));
    }
    if let Some(time) = &st.saved {
        badges.push(Span::styled(format!("saved {time}"), fg(GREEN)));
    }
    if badges.is_empty() {
        return Vec::new();
    }
    let mut spans = vec![Span::raw(" ")];
    for (i, badge) in badges.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", fg(MUTED)));
        }
        spans.push(badge);
    }
    spans.push(Span::raw(" "));
    spans
}

fn hint(st: &Settings) -> &'static str {
    match st {
        _ if st.probe.is_some() => "probing… · Esc drops it",
        _ if st.typing.is_some() => "Enter probes and saves · Esc cancels",
        _ if st.pick.is_some() => "↑↓ move · type to filter · Enter picks · Esc back",
        _ if st.open => "↑↓ setting · Enter changes · ← or Esc back",
        _ => "↑↓ Stage · Enter or → opens · Esc closes",
    }
}

/// The left: PIPELINE, each section with its summary joined by │ where
/// there is room, a rule, then the lines whose pages come later.
fn pipeline(s: &Screen, st: &Settings, height: u16, width: usize) -> Vec<Line<'static>> {
    let row = |name: &str, summary: String, selected: bool| {
        let name_style = match (selected, st.open) {
            (true, false) => bold(PURPLE),
            (true, true) => bold(TEXT),
            (false, _) => fg(TEXT),
        };
        let spans = vec![
            Span::styled(if selected { "▸ " } else { "  " }, fg(PURPLE)),
            Span::styled(pad(name, 10), name_style),
            Span::styled(cut(&summary, width.saturating_sub(12)), fg(MUTED)),
        ];
        match (selected, st.open) {
            (true, false) => filled(spans, width, SEL_BG),
            (true, true) => filled(spans, width, REST_BG),
            (false, _) => Line::from(spans),
        }
    };
    let mut lines = vec![Line::from(Span::styled("PIPELINE", bold(MUTED)))];
    for (i, (_, short, _)) in SECTIONS.iter().enumerate() {
        if i > 0 && height >= 17 {
            lines.push(Line::from(Span::styled("  │", fg(BORDER))));
        }
        lines.push(row(short, st.summary(i), i == st.section));
    }
    lines.push(divider(width));
    let typesafe = if s.cfg.api_key.is_empty() {
        "off"
    } else {
        "on"
    };
    lines.push(row(
        "Apps",
        format!("{} of {} installed", st.installed, APPS.len()),
        false,
    ));
    lines.push(row("Skills", format!("{} installed", st.skills), false));
    lines.push(row("TypeSafe", typesafe.to_string(), false));
    lines
}

/// A setting's label on its section's page.
fn label(row: usize, field: Field) -> String {
    match ROWS[row].lead {
        "" => field.name().to_string(),
        lead => format!("{lead} {}", field.name()),
    }
}

/// A setting's value: the App; the model and its family, or none; the
/// effort, or that the App has none.
fn value(st: &Settings, row: usize, field: Field) -> Vec<Span<'static>> {
    let v = st.value(row, field);
    let muted = |text: String| Span::styled(text, fg(MUTED));
    let shown = match v.as_str() {
        "default" | "none" => muted(v.clone()),
        _ => Span::styled(v.clone(), bold(TEXT)),
    };
    let app = st.app(row);
    match field {
        Field::Model if v == "none" => {
            vec![shown, muted("  no fallback".into())]
        }
        Field::Model => match app {
            Some(app) => vec![shown, muted(format!("  {}", app.family))],
            None => vec![shown],
        },
        Field::Effort => match app {
            Some(app) if app.effort.is_empty() => {
                vec![muted(format!("— {} has no effort flag", app.name))]
            }
            _ => vec![shown],
        },
        Field::App => vec![shown],
    }
}

/// The section's page: its title and Apps, its description, its rows'
/// settings a blank line apart; and the cursor's line.
fn page(st: &Settings, width: usize) -> (Vec<Line<'static>>, usize) {
    let (title, _, about) = SECTIONS[st.section];
    let items = Settings::items(st.section);
    let apps = distinct(items.iter().map(|&(row, _)| st.value(row, Field::App)));
    let mut lines = vec![Line::from(vec![
        Span::styled(title, bold(TEXT)),
        Span::styled(format!("  {}", apps.join(", ")), fg(MUTED)),
    ])];
    lines.extend(wrap_spans(
        vec![(about.to_string(), fg(MUTED))],
        width,
        "",
        "",
        fg(MUTED),
    ));
    let mut at = 0;
    for (i, &(row, field)) in items.iter().enumerate() {
        if field == Field::App {
            lines.push(Line::default());
        }
        let selected = st.open && i == st.setting;
        let mut spans = vec![
            Span::styled(if selected { "▸ " } else { "  " }, fg(PURPLE)),
            Span::styled(
                pad(&label(row, field), 22),
                if selected { bold(TEXT) } else { fg(TEXT) },
            ),
        ];
        spans.extend(value(st, row, field));
        if selected {
            at = lines.len();
            lines.push(filled(spans, width, SEL_BG));
        } else {
            lines.push(Line::from(spans));
        }
    }
    (lines, at)
}

/// A pick list: its title and filter, its entries with the current one
/// marked; and the cursor's line.
fn pick_lines(st: &Settings, pick: &Pick, width: usize) -> (Vec<Line<'static>>, usize) {
    let row = &ROWS[pick.row];
    let on = match (pick.field, st.pick_app(pick)) {
        (Field::App, _) | (_, None) => String::new(),
        (_, Some(app)) if pick.app.is_some() => format!(" · {} (new App)", app.name),
        (_, Some(app)) => format!(" · {}", app.name),
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!("{} {}{on}", row.name, pick.field.name()),
            bold(CYAN),
        ),
        Span::styled("   filter › ", fg(MUTED)),
        Span::styled(pick.filter.clone(), fg(TEXT)),
        Span::styled("▏", fg(PURPLE)),
    ])];
    let name_w = if pick.field == Field::App { 12 } else { 18 };
    let detail_w = width.saturating_sub(2 + name_w + 10).min(44);
    let (mut at, mut n) = (0, 0);
    for e in st.entries(pick) {
        if e.picks.is_none() {
            lines.push(Line::from(Span::styled(
                format!("  {}", e.name),
                bold(MUTED),
            )));
            continue;
        }
        let selected = n == pick.cursor;
        n += 1;
        let spans = vec![
            Span::styled(if selected { "▸ " } else { "  " }, fg(PURPLE)),
            Span::styled(
                pad(&e.name, name_w),
                if selected { bold(TEXT) } else { fg(TEXT) },
            ),
            Span::styled(pad(&e.detail, detail_w), fg(MUTED)),
            Span::styled(if e.current { "✓ current" } else { "" }, fg(GREEN)),
        ];
        if selected {
            at = lines.len();
            lines.push(filled(spans, width, SEL_BG));
        } else {
            lines.push(Line::from(spans));
        }
    }
    if n == 0 {
        lines.push(Line::from(Span::styled("  nothing matches", fg(MUTED))));
    }
    (lines, at)
}

/// The foot's two lines: a probe going, the id being typed, the last
/// note, else the note on the setting under the cursor.
fn foot_lines(s: &Screen, st: &Settings, width: usize) -> Vec<Line<'static>> {
    if let Some(probe) = &st.probe {
        let spin = SPINNER[(s.ticks / 2) as usize % SPINNER.len()];
        let text = format!(
            "probing {} on {} with a one-line prompt…",
            probe.model, probe.app
        );
        return vec![Line::from(vec![
            Span::styled(format!("{spin} "), bold(ORANGE)),
            Span::styled(cut(&text, width.saturating_sub(2)), fg(ORANGE)),
        ])];
    }
    if let Some((pick, text)) = &st.typing {
        let app = st.pick_app(pick).map_or("", |a| a.name);
        let help = match &st.note {
            Some((note, _)) => note.clone(),
            None => format!("A {app} model id: probed with a one-line prompt before it saves."),
        };
        return vec![
            Line::from(vec![
                Span::styled(format!("{} model id › ", ROWS[pick.row].name), bold(PURPLE)),
                Span::styled(text.clone(), fg(TEXT)),
                Span::styled("▏", fg(PURPLE)),
            ]),
            Line::from(Span::styled(cut(&help, width), fg(MUTED))),
        ];
    }
    let (text, color) = match (&st.note, &st.pick) {
        (Some((note, color)), _) => (note.clone(), *color),
        (None, Some(pick)) => (Settings::note_of(pick.row, pick.field), MUTED),
        (None, None) if st.open => {
            let (row, field) = Settings::items(st.section)[st.setting];
            (Settings::note_of(row, field), MUTED)
        }
        (None, None) => (
            "↑↓ picks a Stage; Enter opens it. Every change saves at once to .harness/config.json."
                .to_string(),
            MUTED,
        ),
    };
    let mut lines = wrap_spans(vec![(text, fg(color))], width, "", "", fg(color));
    lines.truncate(2);
    lines
}
