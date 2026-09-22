use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
    Frame,
};

const SPINNER: &[&str] = &["|", "/", "—", "\\"];
const PURPLE: Color = Color::Rgb(210, 90, 255);
const BLUE: Color = Color::Rgb(60, 180, 255);
const ORANGE: Color = Color::Rgb(255, 175, 60);
const GREEN: Color = Color::Rgb(130, 240, 120);
const TEXT: Color = Color::Rgb(232, 240, 255);
const MUTED: Color = Color::Rgb(132, 147, 173);
const BORDER: Color = Color::Rgb(35, 49, 74);

#[derive(Clone, Copy)]
enum WorkerStatus {
    Active,
    Blocked,
    Complete,
}

struct Worker<'a> {
    name: &'a str,
    task: &'a str,
    status: WorkerStatus,
    color: Color,
    phase: u64,
}

pub fn draw_combined_status(
    frame: &mut Frame,
    area: Rect,
    tick: u64,
) {
    let workers = [
        Worker {
            name: "planner",
            task: "mapping dependencies",
            status: WorkerStatus::Active,
            color: PURPLE,
            phase: 0,
        },
        Worker {
            name: "backend",
            task: "generating API",
            status: WorkerStatus::Active,
            color: BLUE,
            phase: 4,
        },
        Worker {
            name: "tests",
            task: "waiting for backend",
            status: WorkerStatus::Blocked,
            color: ORANGE,
            phase: 8,
        },
        Worker {
            name: "frontend",
            task: "dashboard complete",
            status: WorkerStatus::Complete,
            color: GREEN,
            phase: 0,
        },
    ];

    let spinner = SPINNER[tick as usize % SPINNER.len()];

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                spinner,
                Style::default()
                    .fg(PURPLE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " RUNNING",
                Style::default()
                    .fg(PURPLE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("    2 active", Style::default().fg(TEXT)),
            Span::styled("  ·  ", Style::default().fg(BORDER)),
            Span::styled("1 blocked", Style::default().fg(ORANGE)),
            Span::styled("  ·  ", Style::default().fg(BORDER)),
            Span::styled("7 complete", Style::default().fg(GREEN)),
        ]),
        Line::raw(""),
    ];

    for worker in workers {
        lines.push(worker_line(&worker, tick));
    }

    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("Overall  ", Style::default().fg(MUTED)),
        Span::styled(
            "█████████████",
            Style::default().fg(PURPLE),
        ),
        Span::styled(
            "░░░░░░",
            Style::default().fg(BORDER),
        ),
        Span::styled("  68%", Style::default().fg(TEXT)),
        Span::styled(
            format!("                                  TICK {tick:02}"),
            Style::default().fg(BORDER),
        ),
    ]));

    let panel = Paragraph::new(lines).block(
        Block::default()
            .title(" COMBINED ORCHESTRATOR STATUS ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(BORDER))
            .padding(Padding::new(2, 2, 1, 1)),
    );

    frame.render_widget(panel, area);
}

fn worker_line(worker: &Worker<'_>, tick: u64) -> Line<'static> {
    let pulsing = (tick + worker.phase) % 12 < 6;

    let (indicator, indicator_color, status_label) =
        match worker.status {
            WorkerStatus::Active => (
                if pulsing { "●" } else { "◉" },
                if pulsing { worker.color } else { BORDER },
                "ACTIVE",
            ),
            WorkerStatus::Blocked => (
                "◆",
                ORANGE,
                "BLOCKED",
            ),
            WorkerStatus::Complete => (
                "✓",
                GREEN,
                "DONE",
            ),
        };

    Line::from(vec![
        Span::styled(
            format!("{indicator}  "),
            Style::default()
                .fg(indicator_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:<10}", worker.name),
            Style::default().fg(TEXT),
        ),
        Span::styled(
            format!("{:<30}", worker.task),
            Style::default().fg(MUTED),
        ),
        Span::styled(
            status_label,
            Style::default()
                .fg(worker.color)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

// Call it from the application's draw loop:
//
// terminal.draw(|frame| {
//     let area = frame.area();
//     draw_combined_status(frame, area, tick);
// })?;
//
// tick = tick.wrapping_add(1);
