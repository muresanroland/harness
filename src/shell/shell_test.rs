use super::draw::{draw, ticket_color};
use super::logo::{lerp, quantize, CYAN, GREEN, MUTED, PURPLE};
use super::{Epic, Kind, Launch, Pending, Screen, NUDGE_PROCEED, NUDGE_RESULT};
use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::stage::Event;
use crate::orchestrator::state::{
    acquire_lock, load_state, lock_frees, State, TicketState, STATUS_MERGED, STATUS_PARKED,
    STATUS_PR_OPEN, STATUS_RUNNING,
};
use crate::orchestrator::world::{new_world, succeed, BdTicket, World};
use crate::orchestrator::write_file;
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;
use crate::tools::Tools;
use crate::update::{binary, FakeReleases, EVERY};
use chrono::TimeZone;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use ratatui::Terminal;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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

/// One open Epic with seven Tickets on the bd tree, four of them started in
/// the saved run, two of those with a PR: the Overall bar reads 2/7.
fn screen() -> Screen {
    screen_at(Fake::quiet(), Path::new(""))
}

fn screen_at(tools: Arc<dyn Tools>, repo: &Path) -> Screen {
    let epic = Epic {
        id: "harness-kqe".to_string(),
        title: "Build: the Rust port".to_string(),
        tickets: vec![
            issue("harness-kqe.8", "Events: one plain-language line", "closed"),
            issue(
                "harness-kqe.9",
                "The Shell, idle: harness opens the screen",
                "in_progress",
            ),
            issue("harness-kqe.10", "The Shell runs the Orchestrator", "open"),
            issue("harness-kqe.11", "Questions", "open"),
            issue("harness-kqe.12", "Judgment", "open"),
            issue("harness-kqe.13", "Plan mode", "open"),
            issue("harness-kqe.14", "Self-update", "open"),
        ],
    };
    let mut state = State {
        epic: "harness-kqe".to_string(),
        ..Default::default()
    };
    for (n, status) in [
        (8, STATUS_MERGED),
        (9, STATUS_PR_OPEN),
        (10, STATUS_RUNNING),
        (11, STATUS_RUNNING),
    ] {
        state
            .tickets
            .insert(format!("harness-kqe.{n}"), ticket(status));
    }
    Screen::new(
        Launch::for_tests(tools, repo, Path::new("")),
        "~/harness".to_string(),
        true,
        vec![epic],
        state,
    )
}

/// The Shell over the fake world, no terminal: what the slash commands drive.
fn shell(w: &Arc<World>) -> Screen {
    Screen::new(
        Launch::for_tests(w.clone(), &w.repo, &w.home),
        "~/hx".to_string(),
        true,
        Vec::new(),
        load_state(&w.repo).unwrap_or_default(),
    )
}

/// Polls the Shell until a panel line (as world::lines formats it) shows.
fn await_line(s: &mut Screen, want: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        s.poll();
        if s.events.iter().any(|e| line(e).contains(want)) {
            return;
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!(
        "the panel never showed {want:?}; it got:\n{}",
        s.events.iter().map(line).collect::<Vec<_>>().join("\n")
    );
}

fn line(e: &Event) -> String {
    match &e.ticket {
        Some(id) => format!("{id} {}", e.text),
        None => e.text.clone(),
    }
}

/// Polls the Shell until the run is over.
fn await_end(s: &mut Screen) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.run.is_some() {
        assert!(Instant::now() < deadline, "the run never ended");
        s.poll();
        thread::sleep(Duration::from_millis(1));
    }
}

fn notice(s: &Screen) -> &str {
    s.notice.as_ref().map_or("", |(t, _)| t.as_str())
}

fn log(w: &World) -> String {
    std::fs::read_to_string(w.repo.join(".harness/orchestrator.log")).unwrap_or_default()
}

fn render(s: &Screen, w: u16, h: u16) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| draw(f, s)).unwrap();
    t.backend().buffer().clone()
}

fn row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

fn rows(buf: &Buffer) -> Vec<String> {
    (0..buf.area.height).map(|y| row(buf, y)).collect()
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

/// Picks option `n` (as numbered on the form) of the front Question.
fn pick(s: &mut Screen, n: usize) {
    s.key(key(KeyCode::Char(char::from(b'0' + n as u8))));
    s.key(key(KeyCode::Enter));
}

/// The front Question's text, "" when none shows.
fn question(s: &Screen) -> &str {
    if s.showing() {
        s.questions[0].text.as_str()
    } else {
        ""
    }
}

/// Polls the Shell until its Ticket Questions number `n`.
fn await_questions(s: &mut Screen, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.questions.iter().filter(|q| q.ticket.is_some()).count() != n {
        assert!(
            Instant::now() < deadline,
            "the Questions never numbered {n}"
        );
        s.poll();
        thread::sleep(Duration::from_millis(1));
    }
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
            "IDLE    1 open Epic  ·  7 Tickets  ·  saved run on harness-kqe, /continue resumes"
        ),
        "{:?}",
        row(&buf, 8)
    );
    assert!(find(&buf, " TICKETS ").is_some());
    assert!(find(&buf, " RECENT ").is_some());
    assert!(
        row(&buf, 39).starts_with(
            "› ▌  /start-epic  /start-ticket  /continue  /stop-work  /retry  /park  /address  /exit"
        ),
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
    // 80x24 is above every fold: the logo, banner and labels all stay.
    let buf = render(&s, 80, 24);
    assert!(
        row(&buf, 4).contains(&s.version),
        "the header folds at 80x24, it must not: {:?}",
        row(&buf, 4)
    );
    assert!(find(&buf, "DONE").is_some(), "80x24 drops the status label");
    // Under 60 terminal columns the label goes; the indicator carries the status.
    assert!(
        find(&render(&s, 60, 24), "DONE").is_some(),
        "60 columns drop the label"
    );
    let buf = render(&s, 59, 24);
    assert!(find(&buf, "DONE").is_none(), "59 columns keep the label");
    assert!(find(&buf, "✓  8 Events").is_some());
}

#[test]
fn the_overall_bar_counts_the_epics_tickets_and_blends_purple_to_green_by_the_pr_share() {
    let mut s = screen();
    let buf = render(&s, 120, 40);
    let line = row(&buf, 9);
    assert!(
        line.contains("2/7 PRs"),
        "unstarted Tickets are not counted: {line:?}"
    );
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

    for n in 8..15 {
        s.state
            .tickets
            .insert(format!("harness-kqe.{n}"), ticket(STATUS_MERGED));
    }
    let buf = render(&s, 120, 40);
    assert!(row(&buf, 9).contains("7/7 PRs"));
    assert_eq!(fill(&buf), GREEN);
    // Without the Epic on the tree the started Tickets are all there is.
    s.epics.clear();
    assert!(row(&render(&s, 120, 40), 9).contains("7/7 PRs"));

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
    // Under 70 terminal columns the Ticket column narrows to 12.
    let buf = render(&s, 70, 24);
    let (_, y) = find(&buf, " RECENT ").unwrap();
    assert!(
        row(&buf, y + 1).contains("12:04:44  9 The Shell, idle: har  implemented"),
        "70 columns narrow the column: {:?}",
        row(&buf, y + 1)
    );
    let buf = render(&s, 69, 24);
    let (_, y) = find(&buf, " RECENT ").unwrap();
    assert!(
        row(&buf, y + 1).contains("12:04:44  9 The Shell,  implemented"),
        "69 columns keep the wide column: {:?}",
        row(&buf, y + 1)
    );
}

#[test]
fn the_idle_tree_renders_from_a_fake_bd_with_the_saved_epic_resumable() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".harness/state.json"),
        r#"{"epic":"harness-kqe","tickets":{"harness-kqe.9":{"status":"running","stage":"review","round":2},"harness-kqe.10":{"status":"parked","stage":"implement","round":0,"reason":"went idle"}}}"#,
    );
    let fake = Fake::new(|_, argv| {
        match argv.join(" ").as_str() {
        "bd list --json --brief --all" => Ok(r#"[
            {"id":"harness-kqe.10","title":"The Shell runs the Orchestrator","status":"in_progress","issue_type":"task","parent":"harness-kqe"},
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
    let mut s = Screen::open(repo.path(), fake.clone(), &|key| {
        if key == "HOME" {
            home.clone()
        } else {
            String::new()
        }
    });
    assert_eq!(
        fake.calls(),
        [
            "gh auth status",
            "git remote",
            "bd list --json --brief --all"
        ],
        "the preflight, then the bd cache"
    );
    assert_eq!(
        s.folder,
        format!("~/{}", repo.path().file_name().unwrap().to_str().unwrap())
    );
    assert!(!s.truecolor);
    assert_eq!(
        s.epics
            .iter()
            .map(|e| (e.id.as_str(), e.tickets.len()))
            .collect::<Vec<_>>(),
        [("harness-kqe", 3), ("harness-7bj", 0)]
    );
    // Nothing prepared: a run is refused with the first missing thing.
    s.command("/continue");
    s.key(key(KeyCode::Enter)); // the checklist, resumed as saved
    assert!(
        notice(&s).starts_with("refused: no bd workspace here"),
        "{:?}",
        s.notice
    );
    assert!(s.run.is_none());
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
    // A Ticket Parked in the saved run is Parked whatever bd says.
    assert!(
        row(&buf, y + 3).contains("◌  10 The Shell runs the Orchestrator")
            && row(&buf, y + 3).contains("implement")
            && row(&buf, y + 3).contains("PARKED"),
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
fn a_parked_ticket_of_the_saved_run_reads_parked() {
    let mut s = screen();
    s.state
        .tickets
        .insert("harness-kqe.8".to_string(), ticket(STATUS_PARKED));
    let buf = render(&s, 120, 40);
    let (_, y) = find(&buf, "◌  8 Events").unwrap();
    assert!(row(&buf, y).contains("PARKED"), "{:?}", row(&buf, y));
}

#[test]
fn a_tall_tree_scrolls_to_its_last_epic() {
    let mut s = screen();
    for n in 0..3 {
        s.epics.push(Epic {
            id: format!("harness-e{n}"),
            title: format!("Epic {n}"),
            tickets: (0..5)
                .map(|i| issue(&format!("harness-e{n}.{i}"), "work", "open"))
                .collect(),
        });
    }
    assert_eq!(s.rows(), 8 + 18);
    // 80x24 leaves the TICKETS box 7 rows: the header row, 3 rows and the tail.
    let buf = render(&s, 80, 24);
    assert!(find(&buf, "▾  harness-kqe  Build").is_some());
    assert!(find(&buf, "… 23 more").is_some(), "{:#?}", rows(&buf));
    assert!(find(&buf, "Epic 2").is_none());
    for _ in 0..3 {
        s.key(key(KeyCode::PageDown));
    }
    assert_eq!(s.scroll, 25, "clamped to the last row");
    for _ in 0..3 {
        s.key(key(KeyCode::Up));
    }
    assert_eq!(s.scroll, 22);
    // Epic 2 is row 20: 8 rows of harness-kqe, then two Epics of 6 rows each.
    s.key(key(KeyCode::PageUp));
    assert_eq!(s.scroll, 12);
    for _ in 0..8 {
        s.key(key(KeyCode::Down));
    }
    assert_eq!(s.scroll, 20);
    let buf = render(&s, 80, 24);
    assert!(
        find(&buf, "▾  harness-e2  Epic 2").is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert!(find(&buf, "… 3 more").is_some(), "{:#?}", rows(&buf));
    s.scroll = 25;
    assert!(
        find(&render(&s, 80, 24), "more").is_none(),
        "the last row alone needs no tail"
    );
    // Typing takes the arrows back for the input line.
    s.key(key(KeyCode::Char('/')));
    s.key(key(KeyCode::Down));
    assert_eq!(s.scroll, 25);
}

#[test]
fn a_bd_failure_is_a_notice_over_an_empty_tree() {
    let repo = TempDir::new();
    let fake = Fake::new(|_, _| Err("boom".to_string()));
    let s = Screen::open(repo.path(), fake, &|_| String::new());
    assert!(s.epics.is_empty());
    assert_eq!(s.folder, repo.path().display().to_string(), "no HOME, no ~");
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

    // A repeat types, a release does not, nor does a Char with Ctrl or Alt.
    s.key(key(KeyCode::Char('x')));
    s.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::ALT));
    s.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
    let mut repeat = key(KeyCode::Char('x'));
    repeat.kind = KeyEventKind::Repeat;
    s.key(repeat);
    let mut release = key(KeyCode::Char('r'));
    release.kind = KeyEventKind::Release;
    s.key(release);
    assert_eq!(s.input, "xx");

    s.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(!s.quit, "one Ctrl-C exits");
    assert_eq!(
        s.notice.as_ref().map(|(t, _)| t.as_str()),
        Some("press Ctrl-C again to exit")
    );
    assert!(row(&render(&s, 80, 24), 22).contains("press Ctrl-C again to exit"));
    assert!(
        row(&render(&s, 80, 24), 23).starts_with("› xx▌"),
        "typed text is not shown"
    );
    s.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(s.quit, "two Ctrl-C do not exit");

    let mut s = screen();
    type_line(&mut s, "/exit");
    assert!(s.quit);
}

#[test]
fn start_epic_runs_the_tickets_to_prs_and_a_done_epic_clears_the_saved_run() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1"), BdTicket::new("hx-2")]);
    w.lock().merged = true;
    let mut s = shell(&w);
    s.command("/start-epic hx --max 1");
    assert_eq!(notice(&s), "", "start refused");
    assert!(s.running && s.run.is_some());
    assert!(
        acquire_lock(&w.repo).is_err(),
        "the lock is not held while the run is live"
    );
    await_line(&mut s, "hx-1 PR #hx-1 opened after 1 round");
    await_line(&mut s, "hx-2 PR #hx-2 opened after 1 round");
    await_line(&mut s, "Epic done, every Ticket closed");
    await_end(&mut s);
    assert!(!s.running, "still running after the Epic is done");
    assert!(lock_frees(&w.repo), "the lock outlived the run");
    assert_eq!(s.state, State::default(), "a done Epic is still saved");
    assert_eq!(load_state(&w.repo).unwrap(), State::default());
    assert_eq!(w.lock().peak, 1, "--max 1 was not obeyed");
    assert!(
        log(&w).contains(" hx-1 PR #hx-1 opened after 1 round (https://example.test/pr/hx-1)\n")
            && log(&w).contains(" Epic done, every Ticket closed\n"),
        "log:\n{}",
        log(&w)
    );
    // The tree came from bd again: both Tickets are closed.
    let buf = render(&s, 120, 40);
    assert!(
        find(&buf, "✓  hx-1 Ticket hx-1").is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert!(find(&buf, "IDLE    1 open Epic  ·  2 Tickets").is_some());
    assert!(find(&buf, "saved run").is_none());
}

#[test]
fn stop_work_ends_scheduling_with_panes_alive_and_continue_resumes() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().merged = true;
    w.session(|_| (String::new(), "working".to_string())); // never finishes
    let mut s = shell(&w);
    s.command("/start-epic hx");
    await_line(&mut s, "hx-1 implement started: claude (pane 1-1)");

    s.command("/start-epic hx");
    assert_eq!(notice(&s), "refused: a run is live, /stop-work first");
    s.command("/continue");
    assert_eq!(notice(&s), "refused: a run is live, /stop-work first");
    s.poll();
    let buf = render(&s, 120, 40);
    assert!(
        row(&buf, 8).contains("RUNNING    1 active  ·  0 blocked  ·  0 complete"),
        "{:?}",
        row(&buf, 8)
    );
    let (_, y) = find(&buf, "hx  Epic hx").unwrap();
    assert!(
        row(&buf, y).contains("1 Tickets") && row(&buf, y).contains("RUNNING"),
        "{:?}",
        row(&buf, y)
    );
    assert!(
        row(&buf, y + 1).contains("●  hx-1 Ticket hx-1")
            && row(&buf, y + 1).contains("implement")
            && row(&buf, y + 1).contains("ACTIVE"),
        "{:?}",
        row(&buf, y + 1)
    );

    s.command("/stop-work");
    await_line(&mut s, "stopped, panes left running, /continue resumes");
    await_end(&mut s);
    assert!(!s.running);
    assert!(lock_frees(&w.repo), "the lock outlived /stop-work");
    assert_eq!(
        w.called("herdr pane close").len() + w.called("herdr tab close").len(),
        0,
        "stop closed a live pane"
    );
    let saved = load_state(&w.repo).unwrap();
    assert!(
        saved.epic == "hx"
            && saved.tickets["hx-1"].status == STATUS_RUNNING
            && saved.tickets["hx-1"].stage == "implement",
        "saved state = {saved:?}"
    );
    assert_eq!(s.state, saved, "the Shell's State is not the saved one");
    let buf = render(&s, 120, 40);
    assert!(
        row(&buf, 8)
            .contains("IDLE    1 open Epic  ·  1 Ticket  ·  saved run on hx, /continue resumes"),
        "{:?}",
        row(&buf, 8)
    );
    s.command("/stop-work");
    assert_eq!(notice(&s), "nothing is running");

    // /continue asks which saved Tickets to resume, then watches the live
    // Implement session: idle without a result, the Ticket Wakes, and retry
    // starts Implement again in a fresh session.
    w.session(succeed);
    s.command("/continue");
    assert!(!s.running, "started before the checklist was answered");
    assert!(question(&s).starts_with("continue the saved run"));
    s.key(key(KeyCode::Enter));
    assert!(s.running, "/continue did not start: {:?}", s.notice);
    let pane = saved.tickets["hx-1"].panes["implement"].clone();
    w.lock().agents.insert(pane, "idle".to_string());
    await_line(
        &mut s,
        "hx-1 stuck in implement: went idle without a result (pane 1-1)",
    );
    pick(&mut s, 3);
    await_line(&mut s, "Epic done, every Ticket closed");
    await_end(&mut s);
    assert_eq!(
        w.called("herdr agent start h-hx-1-implement").len(),
        2,
        "want a fresh Implement session on retry"
    );
    assert_eq!(w.called("bd worktree create").len(), 1);
    assert!(s.state.epic.is_empty());
}

#[test]
fn retry_and_park_reach_the_ticket_and_refusals_are_logged() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().merged = true;
    w.session(|_| (String::new(), "idle".to_string())); // no result: a Wake
    let mut s = shell(&w);
    s.command("/retry hx-1");
    assert_eq!(
        notice(&s),
        "refused: no run is live, /start-epic or /continue starts one"
    );
    // A refusal of the Shell's own is a run-level line on RECENT and in the log.
    assert!(
        s.events
            .last()
            .is_some_and(|e| e.ticket.is_none() && e.text == notice(&s))
            && log(&w).contains(" refused: no run is live, /start-epic or /continue starts one\n"),
        "{:?}\n{}",
        s.events,
        log(&w)
    );
    s.command("/start-epic hx");
    await_line(
        &mut s,
        "hx-1 stuck in implement: went idle without a result (pane 1-1)",
    );
    s.poll();
    let buf = render(&s, 120, 40);
    assert!(
        row(&buf, 8).contains("RUNNING    0 active  ·  1 blocked  ·  0 complete"),
        "{:?}",
        row(&buf, 8)
    );
    let (_, y) = find(&buf, "◆  hx-1 Ticket hx-1").unwrap();
    assert!(row(&buf, y).contains("BLOCKED"), "{:?}", row(&buf, y));

    s.command("/retry");
    assert_eq!(notice(&s), "usage: /retry <ticket>");
    s.command("/retry hx-9");
    await_line(&mut s, "hx-9 refused: not a Ticket of this run");
    // The Ticket's Question holds it: the commands are refused until answered.
    s.command("/park hx-1");
    assert_eq!(notice(&s), "refused: Ticket hx-1 has a Question waiting");
    s.command("/retry hx-1");
    assert!(
        log(&w).contains(" refused: Ticket hx-1 has a Question waiting\n"),
        "log:\n{}",
        log(&w)
    );
    pick(&mut s, 4); // park
    await_line(&mut s, "hx-1 you answered: park");
    await_line(&mut s, "hx-1 parked: implement went idle without a result");
    assert!(s.questions.is_empty());
    s.poll();
    let buf = render(&s, 120, 40);
    assert!(
        row(&buf, 8).contains("0 complete  ·  1 parked"),
        "{:?}",
        row(&buf, 8)
    );
    assert!(find(&buf, "◌  hx-1 Ticket hx-1").is_some());
    s.command("/park hx-1");
    await_line(&mut s, "hx-1 ignored: not waiting on a Wake");
    assert!(
        log(&w).contains(" hx-9 refused: not a Ticket of this run\n")
            && log(&w).contains(" hx-1 ignored: not waiting on a Wake\n"),
        "log:\n{}",
        log(&w)
    );

    w.session(succeed);
    s.command("/retry hx-1"); // unparks
    await_line(&mut s, "Epic done, every Ticket closed");
    await_end(&mut s);
    assert_eq!(w.called("herdr agent start h-hx-1-implement").len(), 2);
}

#[test]
fn start_epic_resolves_its_argument_from_the_bd_cache_and_asks_before_discarding_a_saved_run() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1"), BdTicket::new("hx-2")]);
    w.lock().merged = true;
    let mut s = shell(&w);
    s.state = State {
        epic: "old".to_string(),
        ..Default::default()
    };
    s.state.save(&w.repo).unwrap();

    s.command("/start-epic nothing-like-it");
    assert_eq!(notice(&s), "no open Epic matches \"nothing-like-it\"");
    s.command("/start-epic hx --max 0");
    assert_eq!(notice(&s), "--max wants a number of at least 1");
    s.command("/start-ticket hx-");
    assert_eq!(notice(&s), "matches: hx-1 Ticket hx-1  ·  hx-2 Ticket hx-2");
    // Tab fills in the one match, by id or title substring.
    s.input = "/start-epic EPIC".to_string();
    s.key(key(KeyCode::Tab));
    assert_eq!(s.input, "/start-epic hx ");
    s.input = "/start-ticket ticket hx-2".to_string();
    s.key(key(KeyCode::Tab));
    assert_eq!(s.input, "/start-ticket hx-2 ");
    assert!(s.run.is_none());
    s.input.clear();

    type_line(&mut s, "/start-epic Epic hx");
    assert_eq!(question(&s), "discard the saved run on old?");
    assert!(s.run.is_none(), "started before the answer");
    type_line(&mut s, "n");
    assert!(s.run.is_none() && s.questions.is_empty(), "started on no");
    assert_eq!(notice(&s), "cancelled");
    // A command typed past the form leaves it waiting; Esc cancels it; it
    // never expires.
    type_line(&mut s, "/start-epic hx");
    type_line(&mut s, "/bogus");
    assert_eq!(notice(&s), "unknown command: /bogus");
    assert!(s.questions.len() == 1 && s.run.is_none());
    s.tick();
    s.key(key(KeyCode::Esc));
    assert!(s.questions.is_empty() && s.run.is_none());
    assert_eq!(notice(&s), "cancelled");
    type_line(&mut s, "/start-epic hx");
    s.key(key(KeyCode::Enter)); // yes is the option under the cursor
    assert!(s.run.is_some(), "did not start on yes: {:?}", s.notice);
    await_line(&mut s, "Epic done, every Ticket closed");
    await_end(&mut s);
    assert!(
        w.called("herdr agent start h-hx-1-implement").len() == 1
            && w.called("herdr agent start h-hx-2-implement").len() == 1
    );

    // /start-ticket runs one Ticket's Pipeline without an Epic to schedule.
    let (w, _) = new_world(vec![BdTicket::new("hx-1"), BdTicket::new("hx-2")]);
    let mut s = shell(&w);
    s.command("/start-ticket");
    assert_eq!(notice(&s), "matches: hx-1 Ticket hx-1  ·  hx-2 Ticket hx-2");
    s.command("/start-ticket hx-2");
    assert!(s.run.is_some(), "{:?}", s.notice);
    s.command("/address hx-2");
    await_line(&mut s, "hx-2 address refused: not an Epic run");
    await_line(&mut s, "hx-2 PR #hx-2 opened after 1 round");
    await_end(&mut s);
    assert!(
        w.called("bd ready").is_empty(),
        "a single Ticket was scheduled"
    );
    assert!(w.called("herdr agent start h-hx-1-implement").is_empty());
    assert!(s.state.epic.is_empty());
    assert_eq!(s.state.tickets["hx-2"].status, STATUS_PR_OPEN);
}

#[test]
fn start_ticket_keeps_a_saved_epic_run_and_asks_over_a_different_one() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1"), BdTicket::new("hx-2")]);
    let mut s = shell(&w);
    let mut saved = State {
        epic: "hx".to_string(),
        ..Default::default()
    };
    saved
        .tickets
        .insert("hx-1".to_string(), ticket(STATUS_RUNNING));
    saved.save(&w.repo).unwrap();
    s.state = saved.clone();

    // A Ticket of the saved Epic: no question, and the saved run stays.
    s.command("/start-ticket hx-2");
    assert!(s.run.is_some(), "{:?}", s.notice);
    await_line(&mut s, "hx-2 PR #hx-2 opened after 1 round");
    await_end(&mut s);
    let after = load_state(&w.repo).unwrap();
    assert!(
        after.epic == "hx"
            && after.tickets["hx-1"] == saved.tickets["hx-1"]
            && after.tickets["hx-2"].status == STATUS_PR_OPEN,
        "a single-Ticket run touched the saved Epic run: {after:?}"
    );
    assert_eq!(s.state, after);

    // A Ticket of another Epic asks, as /start-epic does.
    s.state.epic = "old".to_string();
    s.command("/start-ticket hx-1");
    assert_eq!(question(&s), "discard the saved run on old?");
    assert!(s.run.is_none());
    s.command("y"); // typed, as well as pressed
    assert!(s.run.is_some(), "{:?}", s.notice);
    await_end(&mut s);
    assert!(s.state.epic.is_empty(), "{:?}", s.state);
}

#[test]
fn continue_resumes_a_saved_single_ticket_run() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1"), BdTicket::new("hx-2")]);
    w.session(|_| (String::new(), "working".to_string()));
    let mut s = shell(&w);
    s.command("/continue");
    assert_eq!(notice(&s), "refused: no saved Ticket to continue");
    s.command("/start-ticket hx-2");
    await_line(&mut s, "hx-2 implement started: claude (pane 1-1)");
    s.command("/stop-work");
    await_end(&mut s);
    assert!(s.state.epic.is_empty() && s.state.tickets["hx-2"].status == STATUS_RUNNING);

    w.session(succeed);
    s.command("/continue");
    assert_eq!(s.options(), ["hx-2 Ticket hx-2  implement  → resume"]);
    s.key(key(KeyCode::Enter));
    assert!(s.run.is_some(), "{:?}", s.notice);
    // The live session is watched; idle without a result it Wakes.
    let pane = s.state.tickets["hx-2"].panes["implement"].clone();
    w.lock().agents.insert(pane, "idle".to_string());
    await_line(
        &mut s,
        "hx-2 stuck in implement: went idle without a result (pane 1-1)",
    );
    pick(&mut s, 3);
    await_line(&mut s, "hx-2 you answered: retry");
    await_line(
        &mut s,
        "hx-2 retrying implement with a fresh session (pane 1-1)",
    );
    await_line(&mut s, "hx-2 PR #hx-2 opened after 1 round");
    await_end(&mut s);
    assert!(w.called("herdr agent start h-hx-1-implement").is_empty());
    assert_eq!(w.called("herdr agent start h-hx-2-implement").len(), 2);
    assert_eq!(
        load_state(&w.repo).unwrap().tickets["hx-2"].status,
        STATUS_PR_OPEN
    );
}

/// A Ticket thread sees stop only at its next sleep, after its current Tools
/// call: the run is stopping, and the lock held, until it has left.
#[test]
fn stop_work_keeps_the_run_and_the_lock_until_every_ticket_thread_has_left() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    let (entered_tx, entered) = channel::<()>();
    let (release_tx, release) = channel::<()>();
    let release = Mutex::new(release);
    w.session(move |p| {
        let _ = entered_tx.send(());
        let _ = release.lock().unwrap().recv_timeout(Duration::from_secs(5));
        succeed(p)
    });
    let mut s = shell(&w);
    s.command("/start-epic hx");
    entered
        .recv_timeout(Duration::from_secs(5))
        .expect("the session was never prompted");
    s.command("/stop-work");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !s.stopping() {
        assert!(Instant::now() < deadline, "the scheduler never returned");
        s.poll();
        thread::sleep(Duration::from_millis(1));
    }
    assert!(s.run.is_some() && s.running);
    assert!(
        acquire_lock(&w.repo).is_err(),
        "the lock went while a Ticket thread was live"
    );
    s.command("/continue");
    assert_eq!(notice(&s), "refused: a run is stopping");
    s.command("/start-epic hx");
    assert_eq!(notice(&s), "refused: a run is stopping");
    assert!(
        !s.events.iter().any(|e| e.text.starts_with("stopped")),
        "said stopped before the run ended"
    );
    assert!(
        row(&render(&s, 120, 40), 8).contains("STOPPING"),
        "{:?}",
        row(&render(&s, 120, 40), 8)
    );

    release_tx.send(()).unwrap();
    await_line(&mut s, "stopped, panes left running, /continue resumes");
    await_end(&mut s);
    assert!(lock_frees(&w.repo), "the lock outlived the run");
    assert!(
        log(&w).contains(" stopped, panes left running, /continue resumes\n"),
        "log:\n{}",
        log(&w)
    );
    assert_eq!(
        load_state(&w.repo).unwrap().tickets["hx-1"].status,
        STATUS_RUNNING
    );
}

#[test]
fn exit_during_a_run_asks_and_ctrl_c_twice_stops_the_run() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    w.session(|_| (String::new(), "working".to_string()));
    let mut s = shell(&w);
    s.command("/start-epic hx");
    await_line(&mut s, "hx-1 implement started");
    type_line(&mut s, "/exit");
    assert_eq!(question(&s), "stop the run and exit?");
    assert!(!s.quit);
    type_line(&mut s, "n");
    assert!(!s.quit && s.running && s.questions.is_empty());
    type_line(&mut s, "/exit");
    type_line(&mut s, "y");
    assert!(s.quit, "yes did not exit");
    await_line(&mut s, "stopped, panes left running, /continue resumes");
    await_end(&mut s);
    assert_eq!(w.called("herdr pane close").len(), 0, "exit closed a pane");

    let mut s = shell(&w);
    s.command("/continue");
    s.key(key(KeyCode::Enter));
    assert!(s.running, "{:?}", s.notice); // the live session is watched
    let closed = w.called("herdr pane close").len();
    s.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    s.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(s.quit);
    await_line(&mut s, "stopped, panes left running, /continue resumes");
    await_end(&mut s);
    assert_eq!(
        load_state(&w.repo).unwrap().tickets["hx-1"].status,
        STATUS_RUNNING
    );
    assert_eq!(
        w.called("herdr pane close").len(),
        closed,
        "Ctrl-C closed a pane"
    );
}

/// A scratch exe in the Target repo stands in for the running binary; the
/// Screen is a release build of v1.0.0 over it.
fn release_shell(w: &Arc<World>) -> (Screen, PathBuf) {
    let mut s = shell(w);
    s.version = "v1.0.0".to_string();
    s.exe = w.repo.join("harness");
    std::fs::write(&s.exe, b"old").unwrap();
    (s, w.repo.join("harness"))
}

/// The pending download beside the scratch exe.
fn temp_file(w: &World) -> PathBuf {
    w.repo.join(format!("harness.new.{}", std::process::id()))
}

fn await_update(s: &mut Screen) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.update.is_none() && !s.quit {
        assert!(Instant::now() < deadline, "the update never arrived");
        s.poll();
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn an_update_waits_while_the_lock_is_held_and_installs_at_stop_work() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().merged = true;
    w.session(|_| (String::new(), "working".to_string()));
    let (mut s, exe) = release_shell(&w);
    s.command("/start-epic hx");
    await_line(&mut s, "hx-1 implement started: claude (pane 1-1)");

    // Checks every millisecond: once a release is handed over the thread stops,
    // so no second download rewrites the pending file.
    let releases = Arc::new(FakeReleases::new("v1.1.0", &binary(b"new")));
    s.check_updates(releases.clone(), Duration::from_millis(1));
    await_update(&mut s);
    await_line(&mut s, "v1.1.0 downloaded, installs when the run stops");
    thread::sleep(Duration::from_millis(30));
    s.poll();
    assert_eq!(releases.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(std::fs::read(temp_file(&w)).unwrap(), binary(b"new"));
    assert_eq!(
        std::fs::read(&exe).unwrap(),
        b"old",
        "swapped while the lock was held"
    );
    assert_eq!(s.shown_version(), "v1.0.0 → v1.1.0 at stop");
    let buf = render(&s, 120, 40);
    assert_eq!(cols(&buf, 4, 16, 16 + 24), "v1.0.0 → v1.1.0 at stop ");
    assert_eq!(
        buf[(16, 4)].fg,
        buf[(30, 4)].fg,
        "the pending text is not the version's gray"
    );
    assert!(
        row(&render(&s, 60, 16), 0).starts_with("HARNESS v1.0.0 → v1.1.0 at stop  ~/hx"),
        "{:?}",
        row(&render(&s, 60, 16), 0)
    );
    assert!(s.run.is_some() && !s.quit);

    s.command("/stop-work");
    await_line(&mut s, "stopped, panes left running, /continue resumes");
    await_end(&mut s);
    assert_eq!(
        std::fs::read(&exe).unwrap(),
        binary(b"new"),
        "not installed at /stop-work"
    );
    assert!(
        s.update.is_none() && !s.reexec && !s.quit,
        "a deferred swap re-execs"
    );
    assert_eq!(s.shown_version(), "v1.0.0");
    let log = log(&w);
    assert!(
        !log.contains("updating to") && !log.contains("update check failed"),
        "a deferred install said something:\n{log}"
    );
    assert!(!temp_file(&w).exists());
}

#[test]
fn an_idle_shell_installs_an_update_at_once_and_reexecs() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    let (mut s, exe) = release_shell(&w);
    s.check_updates(
        Arc::new(FakeReleases::new("v1.1.0", &binary(b"new"))),
        EVERY,
    );
    await_update(&mut s);
    assert_eq!(std::fs::read(&exe).unwrap(), binary(b"new"));
    assert!(s.quit && s.reexec, "an idle swap did not re-exec");
    assert!(
        log(&w).contains(" updating to v1.1.0\n"),
        "log:\n{}",
        log(&w)
    );
    assert!(!log(&w).contains("downloaded"));
    assert_eq!(s.shown_version(), "v1.0.0");

    // A failed check is one log line, the temp file gone, nothing in the header.
    let (mut s, exe) = release_shell(&w);
    let mut releases = FakeReleases::new("v1.1.0", &binary(b"new binary"));
    releases.truncated = true;
    s.check_updates(Arc::new(releases), EVERY);
    await_line(&mut s, "update check failed: truncated body: 7 of 14 bytes");
    assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    assert!(!s.quit && !s.reexec && s.update.is_none());
    assert_eq!(s.shown_version(), "v1.0.0");
    assert!(!temp_file(&w).exists());

    // A dev build never asks.
    let (mut s, _) = release_shell(&w);
    s.version = "v1.0.0-dev".to_string();
    let releases = Arc::new(FakeReleases::new("v9.0.0", &binary(b"new")));
    s.check_updates(releases.clone(), EVERY);
    thread::sleep(Duration::from_millis(20));
    s.poll();
    assert_eq!(releases.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(s.update.is_none() && !s.quit);
}

#[test]
fn exit_with_a_run_live_installs_the_update_as_its_last_act() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    w.session(|_| (String::new(), "working".to_string()));
    let (mut s, exe) = release_shell(&w);
    s.command("/start-epic hx");
    await_line(&mut s, "hx-1 implement started");
    s.check_updates(
        Arc::new(FakeReleases::new("v1.1.0", &binary(b"new"))),
        EVERY,
    );
    await_update(&mut s);
    type_line(&mut s, "/exit");
    type_line(&mut s, "y");
    assert!(s.quit && s.run.is_some(), "the run went before close");
    assert_eq!(
        std::fs::read(&exe).unwrap(),
        b"old",
        "swapped before the last act"
    );
    // The install takes the repo lock, so it only lands once the run and its
    // lock are gone.
    s.close();
    assert!(s.run.is_none());
    assert_eq!(std::fs::read(&exe).unwrap(), binary(b"new"));
    assert!(!s.reexec);
    assert!(!log(&w).contains("updating to"), "log:\n{}", log(&w));
}

#[test]
fn an_update_waits_on_another_processs_lock_and_retries_from_tick() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    let (mut s, exe) = release_shell(&w);
    let other = acquire_lock(&w.repo).unwrap(); // a run in another process
    s.check_updates(
        Arc::new(FakeReleases::new("v1.1.0", &binary(b"new"))),
        EVERY,
    );
    await_update(&mut s);
    assert_eq!(
        std::fs::read(&exe).unwrap(),
        b"old",
        "swapped under a held lock"
    );
    assert!(!s.quit && s.update.is_some() && temp_file(&w).exists());
    assert!(!log(&w).contains("update"), "log:\n{}", log(&w));

    drop(other);
    s.tick();
    assert_eq!(
        std::fs::read(&exe).unwrap(),
        b"old",
        "retried within the minute"
    );
    // A process spawned on a parallel test thread may hold the lock's
    // descriptor a moment after the drop: retry until it frees.
    let deadline = Instant::now() + Duration::from_secs(1);
    while !s.quit {
        assert!(Instant::now() < deadline, "the lock never freed");
        s.retry = Instant::now();
        s.tick();
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(std::fs::read(&exe).unwrap(), binary(b"new"));
    assert!(s.quit && s.reexec && s.update.is_none());
    assert!(
        log(&w).contains(" updating to v1.1.0\n"),
        "log:\n{}",
        log(&w)
    );
}

#[test]
fn a_wake_question_renders_the_pane_tail_and_its_options_and_hides_on_esc() {
    let repo = TempDir::new();
    let fake = Fake::new(|_, argv| {
        Ok(match argv {
            ["herdr", "agent", "read", "h-harness-kqe-11-fix", ..] => {
                "Ran the tests: 12 passed.\n> Should I also update the docs?\n".to_string()
            }
            _ => String::new(),
        })
    });
    let mut s = screen_at(fake.clone(), repo.path());
    s.push(event(
        Some("harness-kqe.11"),
        "stuck in fix 1: went idle without a result (pane 2-1)",
        true,
    ));
    assert_eq!(
        fake.calls(),
        ["herdr agent read h-harness-kqe-11-fix --source recent-unwrapped --lines 120"]
    );
    // 50 rows: TICKETS keeps its 11, RECENT its 4, the form gets the rest
    // and shows two lines of the tail.
    let buf = render(&s, 120, 50);
    let (_, y) = find(&buf, " QUESTION ").unwrap();
    let body: Vec<String> = (y + 1..y + 8)
        .map(|y| {
            row(&buf, y)
                .trim_matches(|c| c == '│' || c == ' ')
                .to_string()
        })
        .collect();
    assert_eq!(
        body[..5],
        [
            "11 Questions  stuck in fix 1: went idle without a result (pane 2-1)",
            "",
            "Ran the tests: 12 passed.",
            "> Should I also update the docs?",
            "",
        ],
        "{:#?}",
        rows(&buf)
    );
    let file = format!(
        "{}/.harness/runs/harness-kqe.11/fix-1.md",
        repo.path().display()
    );
    assert!(
        body[5].starts_with("› 1. nudge: The Orchestrator is waiting for your result file")
            && body[5..7].join(" ").contains(&file),
        "{body:#?}"
    );
    let (x, y1) = find(&buf, "› 1. nudge").unwrap();
    assert_eq!(buf[(x, y1)].fg, PURPLE);
    for option in [
        "  2. nudge: Nobody is watching this pane and no one will answer.",
        "  3. retry with a fresh session",
        "  4. park",
        "  5. open the pane",
        "  6. a prompt of your own",
        "↑↓ or a number picks, Enter answers, Esc hides",
    ] {
        let (x, y2) =
            find(&buf, option).unwrap_or_else(|| panic!("{option:?} missing:\n{:#?}", rows(&buf)));
        assert!(y2 > y1 && buf[(x + 2, y2)].fg != PURPLE, "{option:?}");
    }
    assert!(
        find(&buf, "'STATUS: done' as its first line.").is_some(),
        "the prompt is cut"
    );
    assert!(
        find(&buf, "◆  11 Questions").is_some(),
        "a Ticket with a Question waiting is blocked"
    );
    let (_, y) = find(&buf, " RECENT ").unwrap();
    assert!(
        row(&buf, y + 1).contains("11 Questions            asking you: stuck in fix 1"),
        "{:?}",
        row(&buf, y + 1)
    );
    assert!(
        std::fs::read_to_string(repo.path().join(".harness/orchestrator.log"))
            .unwrap()
            .contains(" harness-kqe.11 asking you: stuck in fix 1\n")
    );

    // A number or the arrows move the cursor.
    s.key(key(KeyCode::Char('4')));
    assert_eq!(s.questions[0].cursor, 3);
    s.key(key(KeyCode::Up));
    s.key(key(KeyCode::Up));
    s.key(key(KeyCode::Down));
    assert_eq!(s.questions[0].cursor, 2);
    assert!(find(&render(&s, 120, 50), "› 3. retry with a fresh session").is_some());
    s.key(key(KeyCode::Char('9')));
    assert_eq!(
        s.questions[0].cursor, 2,
        "a number past the options moved the cursor"
    );

    // Esc hides the form; the status row counts it; Esc on an empty input
    // line or /questions brings it back.
    s.key(key(KeyCode::Esc));
    assert!(s.hidden);
    let buf = render(&s, 120, 40);
    assert!(find(&buf, " QUESTION ").is_none());
    assert!(
        row(&buf, 8).contains("/continue resumes  ·  1 question waiting"),
        "{:?}",
        row(&buf, 8)
    );
    s.key(key(KeyCode::Esc));
    assert!(s.showing());
    s.key(key(KeyCode::Esc));
    type_line(&mut s, "/questions");
    assert!(s.showing());
    s.key(key(KeyCode::Esc));
    type_line(&mut s, "/retry harness-kqe.11");
    assert_eq!(
        notice(&s),
        "refused: no run is live, /start-epic or /continue starts one"
    );

    // One form at a time, oldest first, the rest counted; a new Wake for a
    // Ticket replaces its Question, and a session carrying on closes it.
    s.push(event(
        Some("harness-kqe.10"),
        "waiting at a prompt in fix 1 (pane 3-1)",
        true,
    ));
    assert!(s.hidden, "a new Question unhid the form");
    s.hidden = false;
    s.push(event(
        Some("harness-kqe.11"),
        "stuck in fix 1: timed out after 1h (pane 2-1)",
        true,
    ));
    assert_eq!(
        s.questions.len(),
        2,
        "the new Wake did not replace the old Question"
    );
    assert_eq!(question(&s), "waiting at a prompt in fix 1 (pane 3-1)");
    let buf = render(&s, 120, 40);
    let (_, y) = find(&buf, " QUESTION · 1 more waiting ").unwrap();
    let body: Vec<String> = (y + 1..y + 7)
        .map(|y| {
            row(&buf, y)
                .trim_matches(|c| c == '│' || c == ' ')
                .to_string()
        })
        .collect();
    assert_eq!(
        body,
        [
            "10 The Shell runs the Orchestrator  waiting at a prompt in fix 1 (pane 3-1)",
            "",
            "› 1. open the pane",
            "2. park",
            "3. I answered it",
            "↑↓ or a number picks, Enter answers, Esc hides",
        ]
    );
    s.push(event(Some("harness-kqe.10"), "carrying on", true));
    assert_eq!(s.questions.len(), 1);
    assert_eq!(
        question(&s),
        "stuck in fix 1: timed out after 1h (pane 2-1)"
    );
    // A short screen keeps every option: the tail goes first, then RECENT
    // whole, then tree rows (the 17-row form here needs one).
    let buf = render(&s, 120, 40);
    assert!(find(&buf, " QUESTION ").is_some());
    assert!(
        find(&buf, "6. a prompt of your own").is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert!(find(&buf, "Esc hides").is_some(), "{:#?}", rows(&buf));
    assert!(find(&buf, "12 passed").is_none());
    assert!(find(&buf, " RECENT ").is_none(), "{:#?}", rows(&buf));
    assert!(find(&buf, "… 2 more").is_some(), "{:#?}", rows(&buf));
    assert!(find(&buf, "12 Judgment").is_some(), "{:#?}", rows(&buf));
    // Shorter still, the tree keeps its header alone and the hint is cut.
    let buf = render(&s, 120, 32);
    assert!(
        find(&buf, "6. a prompt of your own").is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert!(find(&buf, "14 Self-update").is_none(), "{:#?}", rows(&buf));
    assert!(find(&buf, " TICKETS ").is_some());
    assert!(row(&buf, 31).starts_with("› ▌"), "{:#?}", rows(&buf));
}

#[test]
fn a_confirmation_and_the_continue_checklist_render_as_forms() {
    let mut s = screen();
    s.confirm("stop the run and exit?", Pending::Exit);
    let buf = render(&s, 120, 40);
    let (_, y) = find(&buf, " QUESTION ").unwrap();
    let body: Vec<String> = (y + 1..y + 6)
        .map(|y| {
            row(&buf, y)
                .trim_matches(|c| c == '│' || c == ' ')
                .to_string()
        })
        .collect();
    assert_eq!(
        body,
        [
            "stop the run and exit?",
            "",
            "› 1. yes",
            "2. no",
            "y or n, Enter answers, Esc cancels",
        ]
    );
    s.key(key(KeyCode::Esc));
    assert!(s.questions.is_empty() && !s.quit);
    assert_eq!(notice(&s), "cancelled");

    s.state.tickets.insert(
        "harness-kqe.9".to_string(),
        TicketState {
            reason: "went idle".to_string(),
            ..ticket(STATUS_PARKED)
        },
    );
    s.command("/continue");
    let buf = render(&s, 120, 40);
    let (_, y) = find(&buf, " QUESTION ").unwrap();
    let body: Vec<String> = (y + 1..y + 7)
        .map(|y| {
            row(&buf, y)
                .trim_matches(|c| c == '│' || c == ' ')
                .to_string()
        })
        .collect();
    assert_eq!(
        body,
        [
            "continue the saved run: each Ticket resumes at its Stage, or is reset to Implement",
            "",
            "› 1. 9 The Shell, idle: harness opens the screen  fix 1  parked: went idle  → resume",
            "2. 10 The Shell runs the Orchestrator  fix 1  → resume",
            "3. 11 Questions  fix 1  → resume",
            "Space toggles resume or reset to Implement, Enter starts, Esc cancels",
        ]
    );
    s.key(key(KeyCode::Char('2')));
    s.key(key(KeyCode::Char(' ')));
    assert!(find(
        &render(&s, 120, 40),
        "› 2. 10 The Shell runs the Orchestrator  fix 1  → reset to Implement"
    )
    .is_some());
    s.key(key(KeyCode::Char(' ')));
    assert_eq!(
        s.options()[1],
        "10 The Shell runs the Orchestrator  fix 1  → resume"
    );
    s.key(key(KeyCode::Esc));
    assert!(s.questions.is_empty() && s.run.is_none());
}

/// Every answer to a Wake: the two canned nudges and a prompt of your own go
/// to the session by herdr agent prompt and re-arm the hold, open the pane
/// keeps the Question, park parks; each answer logs its two lines.
#[test]
fn a_wake_question_nudges_opens_the_pane_and_parks() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().merged = true;
    w.session(|_| (String::new(), "idle".to_string())); // no result: a Wake, again and again
    let mut s = shell(&w);
    s.command("/start-epic hx");
    await_line(&mut s, "hx-1 asking you: stuck in implement");
    assert_eq!(
        question(&s),
        "stuck in implement: went idle without a result (pane 1-1)"
    );
    let file = format!("{}/.harness/runs/hx-1/implement.md", w.repo.display());
    let pane = s.state.tickets["hx-1"].panes["implement"].clone();
    // What the Shell prompted the session with, the Stage prompt left out.
    let prompts = |w: &World| {
        w.called(&format!("herdr agent prompt {pane} "))
            .iter()
            .filter(|c| !c.contains("- Result file: "))
            .map(|c| c.splitn(5, ' ').nth(4).unwrap().to_string())
            .collect::<Vec<_>>()
    };

    pick(&mut s, 1);
    assert_eq!(prompts(&w), [NUDGE_RESULT.replace("{result_file}", &file)]);
    await_line(&mut s, "hx-1 you answered: nudge");
    await_line(&mut s, "hx-1 nudged: write the result file");
    assert!(s.questions.is_empty(), "the answered Question stayed");
    // The hold is re-armed: the nudged session, idle without a result, Wakes again.
    await_questions(&mut s, 1);

    pick(&mut s, 6);
    assert!(s.composing && s.questions.len() == 1);
    assert!(row(&render(&s, 120, 40), 39).contains("your prompt, Enter sends it"));
    type_line(&mut s, "read the failing test first");
    assert!(!s.composing && s.input.is_empty());
    assert_eq!(
        prompts(&w).last().map(String::as_str),
        Some("read the failing test first")
    );
    await_line(&mut s, "hx-1 you answered: your prompt");
    await_line(&mut s, "hx-1 nudged with your prompt");
    await_questions(&mut s, 1);

    pick(&mut s, 5); // open the pane
    assert_eq!(
        w.called("herdr pane focus"),
        [format!("herdr pane focus {pane}")]
    );
    assert_eq!(s.questions.len(), 1, "open the pane answered the Question");
    pick(&mut s, 2);
    assert_eq!(
        prompts(&w).last(),
        Some(&NUDGE_PROCEED.replace("{result_file}", &file))
    );
    await_line(&mut s, "hx-1 nudged: carry on, the Ticket is the spec");
    await_questions(&mut s, 1);
    assert_eq!(
        w.called("herdr agent start h-hx-1-implement").len(),
        1,
        "a nudge started a fresh session"
    );

    pick(&mut s, 4);
    await_line(&mut s, "hx-1 you answered: park");
    await_line(&mut s, "hx-1 parked: implement went idle without a result");
    let log = log(&w);
    for line in [
        " hx-1 asking you: stuck in implement\n",
        " hx-1 you answered: nudge\n",
        " hx-1 nudged: write the result file\n",
        " hx-1 you answered: your prompt\n",
        " hx-1 nudged with your prompt\n",
        " hx-1 nudged: carry on, the Ticket is the spec\n",
        " hx-1 you answered: park\n",
        " hx-1 parked: implement went idle without a result\n",
    ] {
        assert!(log.contains(line), "log lacks {line:?}:\n{log}");
    }
    assert_eq!(log.matches("asking you").count(), 4);
    assert!(!log.contains("open"), "open the pane logged something");
    s.command("/stop-work");
    await_end(&mut s);
}

#[test]
fn a_wake_question_retries_with_a_fresh_session() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().merged = true;
    let once = std::sync::atomic::AtomicBool::new(false);
    w.session(move |p| {
        if !once.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return (String::new(), "idle".to_string());
        }
        succeed(p)
    });
    let mut s = shell(&w);
    s.command("/start-epic hx");
    await_line(&mut s, "hx-1 asking you: stuck in implement");
    pick(&mut s, 3);
    await_line(&mut s, "hx-1 you answered: retry");
    await_line(
        &mut s,
        "hx-1 retrying implement with a fresh session (pane 1-1)",
    );
    await_line(&mut s, "Epic done, every Ticket closed");
    await_end(&mut s);
    assert_eq!(w.called("herdr agent start h-hx-1-implement").len(), 2);
    assert!(s.questions.is_empty());
}

/// A blocked session's Question: "I answered it" closes it, the pane moving
/// on closes it by itself with "carrying on", and park takes the Ticket out
/// at its Stage.
#[test]
fn a_blocked_question_closes_itself_when_the_session_carries_on() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().merged = true;
    w.session(|_| ("STATUS: done\n".to_string(), "blocked".to_string()));
    let mut s = shell(&w);
    s.command("/start-epic hx");
    await_line(
        &mut s,
        "hx-1 asking you: waiting at a prompt in implement (pane 1-1)",
    );
    assert!(matches!(s.questions[0].kind, Kind::Blocked));
    pick(&mut s, 3);
    await_line(&mut s, "hx-1 you answered: I answered it");
    assert!(s.questions.is_empty());
    let unblock = |w: &World| {
        for status in w.lock().agents.values_mut() {
            *status = "idle".to_string();
        }
    };
    unblock(&w);
    await_line(&mut s, "hx-1 carrying on");
    // Review blocks next; nobody answers, and the session moves on by itself.
    await_line(
        &mut s,
        "hx-1 asking you: waiting at a prompt in review 1 (pane 1-2)",
    );
    assert_eq!(s.questions.len(), 1);
    unblock(&w);
    await_questions(&mut s, 0);
    assert_eq!(
        log(&w).matches(" hx-1 carrying on\n").count(),
        2,
        "log:\n{}",
        log(&w)
    );
    // The Debate blocks: park.
    await_line(
        &mut s,
        "hx-1 asking you: waiting at a prompt in debate 1 (pane 1-3)",
    );
    s.command("/retry hx-1");
    assert_eq!(notice(&s), "refused: Ticket hx-1 has a Question waiting");
    pick(&mut s, 2);
    await_line(&mut s, "hx-1 you answered: park");
    await_line(&mut s, "hx-1 parked: by you at debate 1");
    s.command("/stop-work");
    await_end(&mut s);
    assert_eq!(
        load_state(&w.repo).unwrap().tickets["hx-1"].status,
        STATUS_PARKED
    );
}

/// A reset row of the /continue checklist runs Implement over: its panes
/// close, its run directory goes, and a fresh Implement session starts.
#[test]
fn the_continue_checklist_resets_a_ticket_to_implement() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().merged = true;
    w.session(|p| {
        if p.stage == "implement" {
            succeed(p)
        } else {
            (String::new(), "working".to_string())
        }
    });
    let mut s = shell(&w);
    s.command("/start-epic hx");
    await_line(&mut s, "hx-1 review 1 started: codex (pane 1-2)");
    s.command("/stop-work");
    await_end(&mut s);
    assert!(w.repo.join(".harness/runs/hx-1/implement.md").exists());

    w.session(succeed);
    s.command("/continue");
    assert_eq!(s.options(), ["hx-1 Ticket hx-1  review 1  → resume"]);
    s.key(key(KeyCode::Char(' ')));
    assert_eq!(
        s.options(),
        ["hx-1 Ticket hx-1  review 1  → reset to Implement"]
    );
    let closed = w.called("herdr pane close").len();
    s.key(key(KeyCode::Enter));
    assert!(s.running, "{:?}", s.notice);
    assert_eq!(
        w.called("herdr pane close").len(),
        closed + 2,
        "the old panes stay open"
    );
    assert!(!w.repo.join(".harness/runs/hx-1/implement.md").exists());
    await_line(&mut s, "Epic done, every Ticket closed");
    await_end(&mut s);
    assert_eq!(w.called("herdr agent start h-hx-1-implement").len(), 2);
    assert_eq!(w.called("herdr agent start h-hx-1-review").len(), 2);
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
