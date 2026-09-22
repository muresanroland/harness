use super::draw::{draw, ticket_color};
use super::logo::{lerp, quantize, CYAN, GREEN, MUTED, PURPLE};
use super::{Epic, Screen};
use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::stage::Event;
use crate::orchestrator::state::{
    State, TicketState, STATUS_MERGED, STATUS_PR_OPEN, STATUS_RUNNING,
};
use crate::orchestrator::write_file;
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;
use chrono::TimeZone;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use ratatui::Terminal;

fn issue(id: &str, title: &str, status: &str) -> BdIssue {
    BdIssue {
        id: id.to_string(),
        title: title.to_string(),
        status: status.to_string(),
        issue_type: "task".to_string(),
        parent: "harness-kqe".to_string(),
        ..Default::default()
    }
}

fn ticket(status: &str) -> TicketState {
    TicketState {
        status: status.to_string(),
        stage: "fix".to_string(),
        round: 1,
        ..Default::default()
    }
}

/// One open Epic with three Tickets, and a saved run with 2 of 7 PRs.
fn screen() -> Screen {
    let epic = Epic {
        id: "harness-kqe".to_string(),
        title: "Build: the Rust port".to_string(),
        resumable: true,
        tickets: vec![
            issue("harness-kqe.8", "Events: one plain-language line", "closed"),
            issue(
                "harness-kqe.9",
                "The Shell, idle: harness opens the screen",
                "in_progress",
            ),
            issue("harness-kqe.10", "The Shell runs the Orchestrator", "open"),
        ],
    };
    let mut state = State {
        epic: "harness-kqe".to_string(),
        ..Default::default()
    };
    for (n, status) in [
        STATUS_MERGED,
        STATUS_PR_OPEN,
        STATUS_RUNNING,
        STATUS_RUNNING,
        "",
        "",
        "",
    ]
    .into_iter()
    .enumerate()
    {
        state
            .tickets
            .insert(format!("harness-kqe.{n}"), ticket(status));
    }
    Screen::new("~/harness".to_string(), true, vec![epic], state)
}

fn render(s: &Screen, w: u16, h: u16) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| draw(f, s)).unwrap();
    t.backend().buffer().clone()
}

fn row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

/// Columns `from..to` of row `y`.
fn cols(buf: &Buffer, y: u16, from: usize, to: usize) -> String {
    row(buf, y).chars().skip(from).take(to - from).collect()
}

/// The column and row where `text` first appears.
fn find(buf: &Buffer, text: &str) -> Option<(u16, u16)> {
    (0..buf.area.height).find_map(|y| {
        let line = row(buf, y);
        line.find(text)
            .map(|i| (line[..i].chars().count() as u16, y))
    })
}

fn event(ticket: Option<&str>, text: &str, panel: bool) -> Event {
    Event {
        time: chrono::Local
            .with_ymd_and_hms(2026, 9, 22, 12, 4, 44)
            .unwrap(),
        ticket: ticket.map(str::to_string),
        text: text.to_string(),
        panel,
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn type_line(s: &mut Screen, line: &str) {
    for c in line.chars() {
        s.key(key(KeyCode::Char(c)));
    }
    s.key(key(KeyCode::Enter));
}

#[test]
fn header_at_120x40_shows_the_logo_banner_version_and_folder() {
    let s = screen();
    let buf = render(&s, 120, 40);
    // The logo rests one cell down and is 12 half-block columns from x 1.
    assert!(
        cols(&buf, 0, 0, 14).trim().is_empty(),
        "row 0 = {:?}",
        row(&buf, 0)
    );
    let logo_row = cols(&buf, 1, 1, 13);
    assert!(
        logo_row.chars().all(|c| c == '▀' || c == '▄' || c == ' '),
        "{logo_row:?}"
    );
    assert!(logo_row.contains('▄'), "no logo cells: {logo_row:?}");
    assert!(
        cols(&buf, 1, 16, 120).contains('█'),
        "no banner: {:?}",
        row(&buf, 1)
    );
    assert_eq!(cols(&buf, 4, 16, 16 + s.version.len()), s.version);
    assert!(row(&buf, 5).contains("~/harness"), "{:?}", row(&buf, 5));
    assert!(
        row(&buf, 8).contains(
            "IDLE    1 open Epics  ·  3 Tickets  ·  saved run on harness-kqe, /continue resumes"
        ),
        "{:?}",
        row(&buf, 8)
    );
    assert!(find(&buf, " TICKETS ").is_some());
    assert!(find(&buf, " RECENT ").is_some());
    assert!(
        row(&buf, 39).starts_with("› ▌  /start-epic  /continue  /retry  /park  /address  /exit"),
        "{:?}",
        row(&buf, 39)
    );
}

#[test]
fn the_hop_stays_still_while_idle_and_moves_in_a_live_run() {
    let mut s = screen();
    for _ in 0..24 {
        s.tick();
        assert!(
            cols(&render(&s, 120, 40), 0, 0, 14).trim().is_empty(),
            "hopped at tick {}",
            s.ticks
        );
    }
    s.running = true;
    s.ticks = 20; // HOP[20] = 0: the top of the hop
    assert!(cols(&render(&s, 120, 40), 0, 1, 13).contains('▀'));
}

#[test]
fn the_fold_under_64_columns_or_18_rows_is_one_plain_line() {
    let s = screen();
    for (w, h) in [(60, 24), (120, 16)] {
        let buf = render(&s, w, h);
        assert!(
            row(&buf, 0).starts_with(&format!("HARNESS {}  ~/harness", s.version)),
            "{w}x{h}: {:?}",
            row(&buf, 0)
        );
        assert!(row(&buf, 1).contains("IDLE"), "{w}x{h}: {:?}", row(&buf, 1));
        assert!(!row(&buf, 1).contains('▀'), "{w}x{h} keeps the logo");
    }
    let buf = render(&s, 80, 24);
    assert!(
        row(&buf, 4).contains(&s.version),
        "80x24 folds the header: {:?}",
        row(&buf, 4)
    );
    assert!(find(&buf, "DONE").is_some(), "80x24 drops the status label");
    // Under 60 columns the label goes; the indicator carries the status.
    let buf = render(&s, 56, 24);
    assert!(find(&buf, "DONE").is_none());
    assert!(find(&buf, "✓  8 Events").is_some());
}

#[test]
fn the_overall_bar_blends_purple_to_green_by_the_share_of_tickets_with_a_pr() {
    let mut s = screen();
    let buf = render(&s, 120, 40);
    let line = row(&buf, 9);
    assert!(line.contains("2/7 PRs"), "{line:?}");
    assert_eq!(line.matches('█').count(), 40 * 2 / 7);
    assert_eq!(line.matches('░').count(), 40 - 40 * 2 / 7);
    // The banner has full blocks too: look on the Overall row alone.
    let fill = |buf: &Buffer| {
        buf[(
            row(buf, 9).chars().position(|c| c == '█').unwrap() as u16,
            9,
        )]
            .fg
    };
    assert_eq!(fill(&buf), lerp((PURPLE, GREEN), 2.0 / 7.0));
    assert_ne!(fill(&buf), PURPLE);

    for ts in s.state.tickets.values_mut() {
        ts.status = STATUS_MERGED.to_string();
    }
    let buf = render(&s, 120, 40);
    assert!(row(&buf, 9).contains("7/7 PRs"));
    assert_eq!(fill(&buf), GREEN);

    s.state = State::default();
    let buf = render(&s, 80, 24);
    assert!(row(&buf, 9).contains("0/0 PRs"), "{:?}", row(&buf, 9));
    assert!(!row(&buf, 9).contains('█'));
}

#[test]
fn recent_is_newest_first_with_the_ticket_column_colored_and_cut_to_22() {
    let mut s = screen();
    s.push(event(None, "started Epic harness-kqe: 3 Tickets", true));
    s.push(event(Some("harness-kqe.9"), "implement prompted", false));
    s.push(event(Some("harness-kqe.9"), "implemented", true));
    let buf = render(&s, 80, 24);
    let (_, y) = find(&buf, " RECENT ").unwrap();
    assert_eq!(
        row(&buf, y + 1).trim_matches(|c| c == '│' || c == ' '),
        "12:04:44  9 The Shell, idle: har  implemented"
    );
    assert_eq!(
        row(&buf, y + 2).trim_matches(|c| c == '│' || c == ' '),
        "12:04:44  harness                 started Epic harness-kqe: 3 Tickets"
    );
    assert!(
        find(&buf, "prompted").is_none(),
        "a log-only Event reached the panel"
    );
    let (x, y) = find(&buf, "9 The Shell, idle: har  implemented").unwrap();
    assert_eq!(buf[(x, y)].fg, CYAN);
    assert_eq!(ticket_color("harness-kqe.9"), CYAN);
    let (x, y) = find(&buf, "harness   ").unwrap();
    assert_eq!(buf[(x, y)].fg, MUTED);
    // Under 70 columns the Ticket column narrows to 12.
    let buf = render(&s, 60, 24);
    let (_, y) = find(&buf, " RECENT ").unwrap();
    assert!(
        row(&buf, y + 1).contains("12:04:44  9 The Shell,  implemented"),
        "{:?}",
        row(&buf, y + 1)
    );
}

#[test]
fn the_idle_tree_renders_from_a_fake_bd_with_the_saved_epic_resumable() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".harness/state.json"),
        r#"{"epic":"harness-kqe","tickets":{"harness-kqe.9":{"status":"running","stage":"review","round":2}}}"#,
    );
    let fake = Fake::new(|_, argv| {
        match argv.join(" ").as_str() {
        "bd list --json --brief --all" => Ok(r#"[
            {"id":"harness-kqe.10","title":"The Shell runs the Orchestrator","status":"open","issue_type":"task","parent":"harness-kqe"},
            {"id":"harness-kqe.9","title":"The Shell, idle","status":"in_progress","issue_type":"task","parent":"harness-kqe"},
            {"id":"harness-kqe.8","title":"Events","status":"closed","issue_type":"task","parent":"harness-kqe"},
            {"id":"harness-kqe","title":"Build: the Rust port","status":"open","issue_type":"epic"},
            {"id":"harness-old.1","title":"Old work","status":"closed","issue_type":"task","parent":"harness-old"},
            {"id":"harness-old","title":"Done long ago","status":"closed","issue_type":"epic"},
            {"id":"harness-7bj","title":"Wayfinder map","status":"open","issue_type":"epic"}
        ]"#.to_string()),
        other => Err(format!("unexpected {other}")),
    }
    });
    let home = repo.path().parent().unwrap().display().to_string();
    let s = Screen::open(repo.path(), &*fake, &|key| {
        if key == "HOME" {
            home.clone()
        } else {
            String::new()
        }
    });
    assert_eq!(fake.calls(), ["bd list --json --brief --all"]);
    assert!(
        s.folder.starts_with("~/harness-test-"),
        "folder = {}",
        s.folder
    );
    assert!(!s.truecolor);
    assert_eq!(
        s.epics
            .iter()
            .map(|e| (e.id.as_str(), e.resumable, e.tickets.len()))
            .collect::<Vec<_>>(),
        [("harness-kqe", true, 3), ("harness-7bj", false, 0)]
    );
    assert_eq!(
        s.epics[0]
            .tickets
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>(),
        ["harness-kqe.8", "harness-kqe.9", "harness-kqe.10"]
    );

    let buf = render(&s, 120, 40);
    let (_, y) = find(&buf, "▾  harness-kqe  Build: the Rust port").unwrap();
    assert!(
        row(&buf, y).contains("3 Tickets") && row(&buf, y).ends_with("RESUMABLE │"),
        "{:?}",
        row(&buf, y)
    );
    assert!(
        row(&buf, y + 1).contains("✓  8 Events") && row(&buf, y + 1).contains("DONE"),
        "{:?}",
        row(&buf, y + 1)
    );
    assert!(
        row(&buf, y + 2).contains("●  9 The Shell, idle")
            && row(&buf, y + 2).contains("review 2")
            && row(&buf, y + 2).contains("ACTIVE"),
        "{:?}",
        row(&buf, y + 2)
    );
    assert!(
        row(&buf, y + 3).contains("·  10 The Shell runs the Orchestrator"),
        "{:?}",
        row(&buf, y + 3)
    );
    assert!(
        row(&buf, y + 4).contains("▾  harness-7bj  Wayfinder map"),
        "{:?}",
        row(&buf, y + 4)
    );
    assert!(find(&buf, "Old work").is_none(), "a closed Epic is listed");
    // No COLORTERM: every color is folded to the 256 cube.
    assert!(buf
        .content
        .iter()
        .all(|c| !matches!(c.fg, Color::Rgb(..)) && !matches!(c.bg, Color::Rgb(..))));
    let (x, y) = find(&buf, "DONE").unwrap();
    assert_eq!(buf[(x, y)].fg, quantize(GREEN));
    assert_eq!(quantize(GREEN), Color::Indexed(156));
}

#[test]
fn a_bd_failure_is_a_notice_over_an_empty_tree() {
    let repo = TempDir::new();
    let fake = Fake::new(|_, _| Err("boom".to_string()));
    let s = Screen::open(repo.path(), &*fake, &|_| String::new());
    assert!(s.epics.is_empty());
    let buf = render(&s, 80, 24);
    assert!(
        row(&buf, 22).contains("bd list failed: bd list --json --brief --all: exit status 1: boom"),
        "{:?}",
        row(&buf, 22)
    );
    assert!(
        row(&buf, 8).contains("IDLE    0 open Epics  ·  0 Tickets"),
        "{:?}",
        row(&buf, 8)
    );
}

#[test]
fn exit_command_ctrl_c_twice_and_an_unknown_command() {
    let mut s = screen();
    type_line(&mut s, "/bogus now");
    assert!(!s.quit);
    assert_eq!(
        s.notice.as_ref().map(|(t, _)| t.as_str()),
        Some("unknown command: /bogus now")
    );
    assert!(row(&render(&s, 80, 24), 22).contains("unknown command: /bogus now"));
    assert!(s.input.is_empty());

    s.key(key(KeyCode::Char('x')));
    s.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(!s.quit, "one Ctrl-C exits");
    assert_eq!(
        s.notice.as_ref().map(|(t, _)| t.as_str()),
        Some("press Ctrl-C again to exit")
    );
    assert!(row(&render(&s, 80, 24), 22).contains("press Ctrl-C again to exit"));
    assert!(
        row(&render(&s, 80, 24), 23).starts_with("› x▌"),
        "typed text is not shown"
    );
    s.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(s.quit, "two Ctrl-C do not exit");

    let mut s = screen();
    type_line(&mut s, "/exit");
    assert!(s.quit);
}

#[test]
#[ignore]
fn dump() {
    let mut s = screen();
    s.push(event(None, "started Epic harness-kqe: 3 Tickets", true));
    s.push(event(Some("harness-kqe.9"), "implemented", true));
    for (w, h) in [(120, 40), (80, 24), (60, 16)] {
        let buf = render(&s, w, h);
        println!("==== {w}x{h}");
        for y in 0..h {
            println!("{}", row(&buf, y));
        }
    }
}
