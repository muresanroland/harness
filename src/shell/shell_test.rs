use super::draw::{draw, ticket_color};
use super::logo::{
    lerp, quantize, BLUE, BORDER, CYAN, DARK_ORANGE, GREEN, MUTED, ORANGE, PINK, PURPLE, RED, TEXT,
};
use super::{About, Epic, Pending, Screen};
use crate::orchestrator::judgment::fake::Fake as TypeSafeFake;
use crate::orchestrator::judgment::Action;
use crate::orchestrator::plan_test::{at_dialog, noul};
use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::stage::{Ask, Config, Event, Orchestrator};
use crate::orchestrator::state::{
    acquire_lock, load_state, State, TicketState, STATUS_MERGED, STATUS_PARKED, STATUS_PR_OPEN,
    STATUS_RUNNING,
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
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// The two canned nudges' prompts over a result file.
fn nudges(file: &Path) -> [String; 2] {
    [Action::NudgeWriteResult, Action::NudgeProceed].map(|a| a.nudge(file).unwrap().0)
}

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
        Config::for_tests(tools, repo, Path::new("")),
        "~/harness".to_string(),
        true,
        vec![epic],
        state,
    )
}

/// The Shell over the fake world, no terminal: what the slash commands drive.
fn shell(w: &Arc<World>) -> Screen {
    Screen::new(
        Config::for_tests(w.clone(), &w.repo, &w.home),
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
        ask: None,
    }
}

/// A panel line that asks, as the Orchestrator sends a Wake or a prompt.
fn asking(ticket: &str, text: &str, ask: Ask) -> Event {
    Event {
        ask: Some(ask),
        ..event(Some(ticket), text, true)
    }
}

/// Whether the log holds `line` ('<bd id> <event>') as a whole line.
fn logged(w: &World, line: &str) -> bool {
    log(w).lines().any(|l| l.get(20..) == Some(line))
}

/// The live run's Orchestrator.
fn orchestrator(s: &Screen) -> Arc<Orchestrator> {
    s.run.as_ref().expect("no run is live").o.clone()
}

/// The pane the front Question is about.
fn asked_pane(s: &Screen) -> String {
    match &s.questions[0].about {
        About::Asked(Ask::Wake { pane, .. } | Ask::Blocked { pane }) => pane.clone(),
        _ => panic!("the front Question is not a Ticket's"),
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn type_line(s: &mut Screen, line: &str) {
    type_in(s, line);
    s.key(key(KeyCode::Enter));
}

/// Picks option `n`, as numbered, of the front Question.
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
    assert!(find(&buf, " ━━ ▾ harness-kqe").is_some());
    assert!(find(&buf, " RECENT ").is_some());
    assert!(
        row(&buf, 39).starts_with("› ▌  / for a command, @ for an Epic or Ticket"),
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
    assert!(
        find(&buf, "CLOSED").is_some(),
        "80x24 drops the status label"
    );
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

/// Types `text` without Enter.
fn type_in(s: &mut Screen, text: &str) {
    for c in text.chars() {
        s.key(key(KeyCode::Char(c)));
    }
}

/// What the open list would fill in, row by row.
fn list_keys(s: &Screen) -> Vec<&str> {
    s.list().iter().map(|row| row.0).collect()
}

/// '/' with no space yet lists the commands containing it, else those it is
/// a subsequence of; Tab fills one in, Enter too unless it is typed whole.
#[test]
fn the_slash_list_filters_the_command_table_and_fills_in() {
    let mut s = screen();
    type_in(&mut s, "/");
    assert_eq!(list_keys(&s), super::COMMANDS.map(|c| c.0));
    type_in(&mut s, "pa");
    assert_eq!(list_keys(&s), ["/park"]);
    s.key(key(KeyCode::Tab));
    assert_eq!(s.input, "/park ");
    assert!(list_keys(&s).is_empty(), "a list in the argument slot");
    // Containing wins: /questions is only a subsequence of '/st'.
    s.input.clear();
    type_in(&mut s, "/st");
    assert_eq!(
        list_keys(&s),
        ["/start-epic", "/start-ticket", "/stop-work"]
    );
    s.key(key(KeyCode::Esc));
    assert!(
        s.input.is_empty() && list_keys(&s).is_empty(),
        "Esc left it"
    );
    type_in(&mut s, "/sw");
    assert_eq!(list_keys(&s), ["/stop-work"]);
    s.key(key(KeyCode::Esc));
    // Nothing matches: no list, and Enter runs the line.
    type_line(&mut s, "/zz");
    assert_eq!(notice(&s), "unknown command: /zz");
    // Enter fills in; on a command typed whole it runs it, when it takes
    // no argument.
    type_line(&mut s, "/park");
    assert_eq!(s.input, "/park ");
    s.input.clear();
    type_line(&mut s, "/ex");
    assert_eq!(s.input, "/exit ");
    assert!(!s.quit);
    s.key(key(KeyCode::Enter));
    assert!(s.quit);
    let mut s = screen();
    type_line(&mut s, "/exit");
    assert!(s.quit);
}

/// Four open Epics for the @ list, two closed Tickets among theirs.
fn lists_screen() -> Screen {
    let epic = |id: &str, title: &str, tickets: Vec<BdIssue>| Epic {
        id: id.to_string(),
        title: title.to_string(),
        tickets,
    };
    let epics = vec![
        epic(
            "harness-0sx",
            "Wayfinder map",
            vec![
                issue("harness-0sx.4", "The screen updates", "open"),
                issue("harness-0sx.8", "Limited", "in_progress"),
                issue("harness-0sx.9", "Old", "closed"),
            ],
        ),
        epic(
            "harness-kv9",
            "Other work",
            vec![issue("harness-kv9.1", "First", "open")],
        ),
        epic(
            "harness-7nq",
            "Build",
            vec![
                issue("harness-7nq.5", "Plan review", "open"),
                issue("harness-7nq.6", "Closed review", "closed"),
            ],
        ),
        epic(
            "harness-rev",
            "Rework",
            vec![issue("harness-rev.1", "Anything", "open")],
        ),
    ];
    Screen::new(
        Config::for_tests(Fake::quiet(), Path::new(""), Path::new("")),
        "~/harness".to_string(),
        true,
        epics,
        State::default(),
    )
}

/// '@<query>' at the end lists the open Epics and Tickets: id contains it,
/// then title, then a subsequence of the id; the command before it narrows
/// the list, and Enter or Tab puts the id in place of '@<query>'.
#[test]
fn the_at_list_ranks_open_epics_and_tickets_narrowed_by_the_command() {
    let mut s = lists_screen();
    type_in(&mut s, "/start-epic @0s");
    assert_eq!(list_keys(&s), ["harness-0sx"], "a Ticket of harness-0sx");
    s.key(key(KeyCode::Enter));
    assert_eq!(s.input, "/start-epic harness-0sx ");
    assert!(s.run.is_none() && s.notice.is_none(), "{:?}", s.notice);
    s.input.clear();
    type_in(&mut s, "/start-epic @");
    assert_eq!(
        list_keys(&s),
        ["harness-0sx", "harness-kv9", "harness-7nq", "harness-rev"]
    );
    s.key(key(KeyCode::Esc));
    type_in(&mut s, "/retry @");
    let open = [
        "harness-0sx.4",
        "harness-0sx.8",
        "harness-kv9.1",
        "harness-7nq.5",
        "harness-rev.1",
    ];
    assert_eq!(list_keys(&s), open);
    for name in ["/start-ticket", "/park", "/address"] {
        s.input = format!("{name} @");
        assert_eq!(list_keys(&s), open, "{name}");
    }
    s.input.clear();
    type_in(&mut s, "@rev");
    assert_eq!(
        list_keys(&s),
        [
            "harness-rev",
            "harness-rev.1",
            "harness-7nq.5",
            "harness-kv9",
            "harness-kv9.1"
        ]
    );
    assert_eq!(
        s.list()[2],
        ("harness-7nq.5", "Ticket", "Plan review"),
        "the row"
    );
    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Tab));
    assert_eq!(s.input, "harness-7nq.5 ");
    // A space after the query closes it; no list in the argument slot.
    s.input = "/start-epic @0s --max".to_string();
    assert!(list_keys(&s).is_empty());
    s.input = "/start-epic 0s".to_string();
    assert!(list_keys(&s).is_empty());
    // '@' inside a word opens nothing.
    s.input = "/start-epic me@0s".to_string();
    assert!(list_keys(&s).is_empty());
}

/// With a list open Up and Down move its cursor, kept on its rows, and
/// leave RECENT; with none open and the line empty they scroll RECENT.
#[test]
fn up_and_down_move_an_open_lists_cursor_and_scroll_recent_when_none_is() {
    let mut s = screen();
    for n in 0..10 {
        s.push(event(None, &format!("line {n}"), true));
    }
    type_in(&mut s, "/");
    s.key(key(KeyCode::Up));
    assert_eq!(s.pick, 0);
    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Down));
    assert_eq!((s.pick, s.recent.get()), (2, 0));
    s.key(key(KeyCode::Up));
    assert_eq!((s.pick, s.recent.get()), (1, 0));
    for _ in 0..20 {
        s.key(key(KeyCode::Down));
    }
    assert_eq!(s.pick, 8, "past the last row");
    s.key(key(KeyCode::Up));
    s.key(key(KeyCode::Enter));
    assert_eq!(s.input, "/questions ");
    // Typing puts the cursor back on the top row.
    s.input.clear();
    type_in(&mut s, "/");
    s.key(key(KeyCode::Down));
    type_in(&mut s, "s");
    assert_eq!(s.pick, 0);
    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Backspace));
    assert_eq!(s.pick, 0);
    s.key(key(KeyCode::Esc));
    s.key(key(KeyCode::Up));
    assert_eq!(s.recent.get(), 1, "Up did not scroll RECENT");
    s.key(key(KeyCode::Down));
    assert_eq!(s.recent.get(), 0);
}

/// The input line's placeholder draws in dark orange.
#[test]
fn the_placeholder_draws_in_dark_orange() {
    let buf = render(&screen(), 120, 40);
    assert_eq!(
        row(&buf, 39).trim_end(),
        "› ▌  / for a command, @ for an Epic or Ticket"
    );
    let (x, y) = find(&buf, "/ for a command").unwrap();
    assert_eq!(buf[(x, y)].fg, DARK_ORANGE);
    assert!(find(&buf, "/start-epic").is_none(), "{:#?}", rows(&buf));
}

/// The / list inline above the notice and input lines: up to eight rows
/// around its cursor, then the hint. The command purple, bold on the cursor
/// row; args muted; the description TEXT on the cursor row, muted otherwise.
#[test]
fn the_slash_list_renders_above_the_input_with_its_hint() {
    let mut s = screen();
    type_in(&mut s, "/");
    let buf = render(&s, 120, 40);
    let want = [
        " › /start-epic    <epic> [--max N]  run every Ticket of an open Epic",
        "   /start-ticket  <ticket>          run one Ticket",
        "   /continue                        resume the saved run",
        "   /stop-work                       stop the run, the panes stay",
        "   /retry         <ticket>          the Ticket's Stage again, in a fresh session",
        "   /park          <ticket>          take a Ticket out to wait for you",
        "   /address       <ticket>          resolve a PR's conflicts or review comments",
        "   /questions                       show the hidden Questions",
        "   ↑↓ pick · Tab or Enter fills in · Esc clears",
    ];
    let shown: Vec<String> = (29..38)
        .map(|y| row(&buf, y).trim_end().to_string())
        .collect();
    assert_eq!(shown, want, "{:#?}", rows(&buf));
    assert_eq!(row(&buf, 39).trim_end(), "› /▌");
    let at = |text: &str| {
        let (x, y) = find(&buf, text).unwrap();
        let cell = &buf[(x, y)];
        (cell.fg, cell.modifier.contains(Modifier::BOLD))
    };
    assert_eq!(at("› /start-epic"), (PURPLE, true));
    assert_eq!(at("/start-epic "), (PURPLE, true));
    assert_eq!(at("/start-ticket"), (PURPLE, false));
    assert_eq!(at("<epic>").0, MUTED);
    assert_eq!(at("run every Ticket").0, TEXT);
    assert_eq!(at("run one Ticket").0, MUTED);
    // The window follows the cursor to the last row.
    for _ in 0..8 {
        s.key(key(KeyCode::Down));
    }
    let buf = render(&s, 120, 40);
    assert!(
        row(&buf, 29).starts_with("   /start-ticket"),
        "{:#?}",
        rows(&buf)
    );
    assert!(row(&buf, 36).starts_with(" › /exit"), "{:#?}", rows(&buf));
    // 80x24 keeps TICKETS its three rows: the list shows seven and the hint.
    let buf = render(&s, 80, 24);
    assert!(row(&buf, 11).contains("harness-kqe"), "{:#?}", rows(&buf));
    assert!(row(&buf, 13).contains("6 more, PgDn"), "{:#?}", rows(&buf));
    assert!(
        row(&buf, 14).starts_with("   /continue"),
        "{:#?}",
        rows(&buf)
    );
    assert!(row(&buf, 20).starts_with(" › /exit"), "{:#?}", rows(&buf));
    assert!(row(&buf, 21).contains("↑↓ pick"), "{:#?}", rows(&buf));
    assert_eq!(row(&buf, 23).trim_end(), "› /▌");
}

/// The @ list's rows: the id in its Epic's color by place on the tree or in
/// its Ticket's color, then Epic or Ticket, then the title.
#[test]
fn the_at_list_renders_ids_in_their_epic_or_ticket_color() {
    let mut s = lists_screen();
    type_in(&mut s, "/start-ticket x @0s");
    let buf = render(&s, 120, 40);
    let want = [
        " › harness-0sx.4  Ticket  The screen updates",
        "   harness-0sx.8  Ticket  Limited",
        "   ↑↓ pick · Tab or Enter fills in · Esc clears",
    ];
    let shown: Vec<String> = (35..38)
        .map(|y| row(&buf, y).trim_end().to_string())
        .collect();
    assert_eq!(shown, want, "{:#?}", rows(&buf));
    let fg = |text: &str| {
        let (x, y) = find(&buf, text).unwrap();
        buf[(x, y)].fg
    };
    assert_eq!(fg("harness-0sx.4  Ticket"), ticket_color("harness-0sx.4"));
    assert_eq!(fg("harness-0sx.8  Ticket"), ticket_color("harness-0sx.8"));
    assert_eq!(fg("Ticket  The"), MUTED);
    s.input.clear();
    type_in(&mut s, "@0s");
    let buf = render(&s, 120, 40);
    let want = [
        " › harness-0sx    Epic    Wayfinder map",
        "   harness-0sx.4  Ticket  The screen updates",
        "   harness-0sx.8  Ticket  Limited",
        "   ↑↓ pick · Tab or Enter fills in · Esc clears",
    ];
    let shown: Vec<String> = (34..38)
        .map(|y| row(&buf, y).trim_end().to_string())
        .collect();
    assert_eq!(shown, want, "{:#?}", rows(&buf));
    let (x, y) = find(&buf, "harness-0sx    Epic").unwrap();
    assert_eq!(buf[(x, y)].fg, PURPLE, "the first Epic's color");
    let (x, y) = find(&buf, "▾ harness-kv9").unwrap();
    assert_eq!(buf[(x + 2, y)].fg, CYAN, "the tree's second");
    s.input = "/start-epic @kv".to_string();
    let buf = render(&s, 120, 40);
    let (x, y) = find(&buf, "harness-kv9  Epic").unwrap();
    assert_eq!(buf[(x, y)].fg, CYAN, "the list's Epic color is the tree's");
}

/// RECENT under its rule, newest on its last row with blank rows above; the
/// Ticket column as wide as the longest name shown, up to 34% of the width,
/// cut with … and colored per Ticket.
#[test]
fn recent_is_newest_at_the_bottom_with_the_ticket_column_as_wide_as_its_longest_name() {
    let mut s = screen();
    s.push(event(None, "started Epic harness-kqe: 3 Tickets", true));
    s.push(event(Some("harness-kqe.11"), "implement prompted", false));
    s.push(event(Some("harness-kqe.11"), "implemented", true));
    // 80x24: RECENT is rows 18 to 21, its rule and three lines, unboxed.
    let buf = render(&s, 80, 24);
    assert_eq!(row(&buf, 18), format!(" ── RECENT {} ", "─".repeat(68)));
    assert!(row(&buf, 19).trim().is_empty(), "{:#?}", rows(&buf));
    assert_eq!(
        row(&buf, 20).trim_end(),
        " 12:04:44  harness       started Epic harness-kqe: 3 Tickets"
    );
    assert_eq!(
        row(&buf, 21).trim_end(),
        " 12:04:44  11 Questions  implemented"
    );
    assert!(
        find(&buf, "prompted").is_none(),
        "a log-only Event reached the panel"
    );
    let (x, y) = find(&buf, "11 Questions  implemented").unwrap();
    assert_eq!(buf[(x, y)].fg, ticket_color("harness-kqe.11"));
    let (x, y) = find(&buf, "harness   ").unwrap();
    assert_eq!(buf[(x, y)].fg, MUTED);
    // A name longer than 34% of the width is cut there: 27 of 80 columns.
    s.push(event(Some("harness-kqe.9"), "reviewed", true));
    let buf = render(&s, 80, 24);
    assert_eq!(
        row(&buf, 21).trim_end(),
        " 12:04:44  9 The Shell, idle: harness…  reviewed"
    );
    assert_eq!(
        row(&buf, 20).trim_end(),
        " 12:04:44  11 Questions                 implemented"
    );
    let (x, y) = find(&buf, "9 The Shell, idle: harness…").unwrap();
    assert_eq!(buf[(x, y)].fg, CYAN);
    assert_eq!(ticket_color("harness-kqe.9"), CYAN);
    // 40 of 120 columns.
    assert!(
        find(
            &render(&s, 120, 40),
            " 12:04:44  9 The Shell, idle: harness opens the sc…  reviewed"
        )
        .is_some(),
        "{:#?}",
        rows(&render(&s, 120, 40))
    );
}

/// Up and Down scroll RECENT while the input is empty, the rule counting the
/// lines hidden older and newer; scrolled up, a new line leaves the view where
/// it is, and back at the bottom the view follows the newest again.
#[test]
fn up_and_down_scroll_recent_and_its_rule_counts_older_and_newer() {
    let mut s = screen();
    for n in 0..10 {
        s.push(event(None, &format!("line {n}"), true));
    }
    // 80x24 shows three lines: the rule on row 18, the lines on 19 to 21.
    let shown = |s: &Screen| {
        let buf = render(s, 80, 24);
        let rule = row(&buf, 18).trim_end_matches([' ', '─']).to_string();
        let lines: Vec<String> = (19..22)
            .map(|y| {
                row(&buf, y)
                    .trim_end()
                    .rsplit("  ")
                    .next()
                    .unwrap()
                    .to_string()
            })
            .collect();
        (rule, lines)
    };
    let view = |rule: &str, first: usize| {
        let lines = (first..first + 3).map(|n| format!("line {n}")).collect();
        (format!(" ── RECENT{rule}"), lines)
    };
    assert_eq!(shown(&s), view("  ↑ 7 older", 7));
    s.key(key(KeyCode::Up));
    assert_eq!(shown(&s), view("  ↑ 6 older · ↓ 1 newer", 6));
    s.push(event(None, "line 10", true));
    assert_eq!(
        shown(&s),
        view("  ↑ 6 older · ↓ 2 newer", 6),
        "a new line moved the view"
    );
    for _ in 0..20 {
        s.key(key(KeyCode::Up));
    }
    assert_eq!(shown(&s), view("  ↓ 8 newer", 0), "kept inside the lines");
    for _ in 0..8 {
        s.key(key(KeyCode::Down));
    }
    assert_eq!(shown(&s), view("  ↑ 8 older", 8));
    s.push(event(None, "line 11", true));
    assert_eq!(shown(&s), view("  ↑ 9 older", 9), "the bottom follows");
    assert_eq!(s.scroll.get(), 0, "Up and Down scrolled TICKETS");
    // Text on the input line keeps the arrows off RECENT.
    s.key(key(KeyCode::Up));
    s.key(key(KeyCode::Char('x')));
    s.key(key(KeyCode::Down));
    assert_eq!(shown(&s), view("  ↑ 8 older · ↓ 1 newer", 8));
    s.key(key(KeyCode::Up));
    assert_eq!(shown(&s), view("  ↑ 8 older · ↓ 1 newer", 8));
    // Fewer lines than rows: nothing to scroll, the rule counts nothing.
    let mut s = screen();
    s.push(event(None, "line 0", true));
    s.key(key(KeyCode::Up));
    assert_eq!(shown(&s).0, " ── RECENT");
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
            "claude plugin list --json",
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
    let (_, y) = find(&buf, "▾ harness-kqe  Build: the Rust port").unwrap();
    assert!(
        row(&buf, y)
            .trim_end()
            .ends_with("1 closed · 2 in progress  RESUMABLE"),
        "{:?}",
        row(&buf, y)
    );
    assert!(
        row(&buf, y + 1).contains("✓ 8 Events") && row(&buf, y + 1).contains("CLOSED"),
        "{:?}",
        row(&buf, y + 1)
    );
    assert!(
        row(&buf, y + 2).contains("● 9 The Shell, idle")
            && row(&buf, y + 2).contains("review 2")
            && row(&buf, y + 2).contains("IN PROGRESS"),
        "{:?}",
        row(&buf, y + 2)
    );
    // A Ticket Parked in the saved run is Parked whatever bd says.
    assert!(
        row(&buf, y + 3).contains("◌ 10 The Shell runs the Orchestrator")
            && row(&buf, y + 3).contains("implement")
            && row(&buf, y + 3).contains("PARKED"),
        "{:?}",
        row(&buf, y + 3)
    );
    assert!(
        row(&buf, y + 4).contains("▾ harness-7bj  Wayfinder map"),
        "{:?}",
        row(&buf, y + 4)
    );
    assert!(find(&buf, "Old work").is_none(), "a closed Epic is listed");
    // No COLORTERM: every color is folded to the 256 cube.
    assert!(buf
        .content
        .iter()
        .all(|c| !matches!(c.fg, Color::Rgb(..)) && !matches!(c.bg, Color::Rgb(..))));
    let (x, y) = find(&buf, "CLOSED").unwrap();
    assert_eq!(buf[(x, y)].fg, quantize(GREEN));
    assert_eq!(quantize(GREEN), Color::Indexed(156));
}

/// Four open Epics as bd lists them. The first is the saved run's, with Ticket
/// 5 blocked by Ticket 3; every Ticket of the second is closed; the third has
/// one closed, one in progress and one open; the fourth is not started.
const SECTIONS: &str = r#"[
    {"id":"harness-a","title":"Build: the screen","status":"open","issue_type":"epic"},
    {"id":"harness-a.1","title":"Plan floor","status":"closed","issue_type":"task","parent":"harness-a"},
    {"id":"harness-a.2","title":"Pane focus","status":"closed","issue_type":"task","parent":"harness-a"},
    {"id":"harness-a.3","title":"Status counts","status":"in_progress","issue_type":"task","parent":"harness-a"},
    {"id":"harness-a.4","title":"The / list","status":"in_progress","issue_type":"task","parent":"harness-a"},
    {"id":"harness-a.5","title":"The @ list","status":"open","issue_type":"task","parent":"harness-a",
     "dependencies":[{"depends_on_id":"harness-a","type":"parent-child"},{"depends_on_id":"harness-a.3","type":"blocks"}]},
    {"id":"harness-a.6","title":"RECENT scroll","status":"in_progress","issue_type":"task","parent":"harness-a"},
    {"id":"harness-a.7","title":"Tree","status":"in_progress","issue_type":"task","parent":"harness-a"},
    {"id":"harness-a.8","title":"Models","status":"open","issue_type":"task","parent":"harness-a"},
    {"id":"harness-b","title":"Done work","status":"open","issue_type":"epic"},
    {"id":"harness-b.1","title":"Old one","status":"closed","issue_type":"task","parent":"harness-b"},
    {"id":"harness-b.2","title":"Old two","status":"closed","issue_type":"task","parent":"harness-b"},
    {"id":"harness-c","title":"Half done","status":"open","issue_type":"epic"},
    {"id":"harness-c.1","title":"First","status":"closed","issue_type":"task","parent":"harness-c"},
    {"id":"harness-c.2","title":"Second","status":"in_progress","issue_type":"task","parent":"harness-c"},
    {"id":"harness-c.3","title":"Third","status":"open","issue_type":"task","parent":"harness-c"},
    {"id":"harness-d","title":"Not yet","status":"open","issue_type":"epic"},
    {"id":"harness-d.1","title":"Later","status":"open","issue_type":"task","parent":"harness-d"},
    {"id":"harness-d.2","title":"Much later","status":"open","issue_type":"task","parent":"harness-d"}
]"#;

/// The Shell over SECTIONS from a fake bd list, with the saved run on
/// harness-a: 1 and 2 merged, 3's PR open, 4 and 6 running, 7 parked.
/// `running`, the run is live and 4 is blocked on a question.
fn sections_screen(running: bool) -> Screen {
    let fake = Fake::new(|_, _| Ok(SECTIONS.to_string()));
    let epics = super::load_epics(Path::new(""), &*fake).unwrap();
    let mut state = State {
        epic: "harness-a".to_string(),
        ..Default::default()
    };
    for (n, status, stage, round, pr) in [
        (1, STATUS_MERGED, "fix", 1, "29"),
        (2, STATUS_MERGED, "fix", 2, "30"),
        (3, STATUS_PR_OPEN, "fix", 1, "31"),
        (4, STATUS_RUNNING, "implement", 0, ""),
        (6, STATUS_RUNNING, "review", 1, ""),
        (7, STATUS_PARKED, "fix", 2, ""),
    ] {
        let pr = match pr {
            "" => String::new(),
            n => format!("https://github.com/o/r/pull/{n}"),
        };
        state.tickets.insert(
            format!("harness-a.{n}"),
            TicketState {
                status: status.to_string(),
                stage: stage.to_string(),
                round,
                pr,
                ..Default::default()
            },
        );
    }
    let mut s = Screen::new(
        Config::for_tests(fake, Path::new(""), Path::new("")),
        "~/harness".to_string(),
        true,
        epics,
        state,
    );
    if running {
        s.running = true;
        s.push(asking(
            "harness-a.4",
            "blocked in implement (pane 1-1)",
            Ask::Blocked {
                pane: "1-1".to_string(),
            },
        ));
    }
    s
}

/// The row where `text` first appears, right-trimmed.
fn row_of(buf: &Buffer, text: &str) -> String {
    let (_, y) = find(buf, text).unwrap_or_else(|| panic!("no {text:?} in {:#?}", rows(buf)));
    row(buf, y).trim_end().to_string()
}

#[test]
fn idle_every_open_epic_is_a_rule_line_in_its_color_by_place_with_its_word() {
    let s = sections_screen(false);
    let buf = render(&s, 120, 40);
    for (epic, color) in [
        ("▾ harness-a", PURPLE),
        ("▸ harness-b", CYAN),
        ("▾ harness-c", ORANGE),
        ("▾ harness-d", PINK),
    ] {
        let (x, y) = find(&buf, epic).unwrap_or_else(|| panic!("{epic}: {:#?}", rows(&buf)));
        assert_eq!(buf[(x + 2, y)].fg, color, "{epic}");
        assert!(row(&buf, y).starts_with(" ━━ "), "{:?}", row(&buf, y));
        assert_eq!(buf[(x - 2, y)].fg, lerp((color, BORDER), 0.55), "{epic}");
    }
    let a = row_of(&buf, "▾ harness-a");
    assert!(
        a.contains("▾ harness-a  Build: the screen ━")
            && a.ends_with("━ 2 closed · 4 in progress · 2 open  RESUMABLE"),
        "{a:?}"
    );
    let (x, y) = find(&buf, "RESUMABLE").unwrap();
    assert_eq!(buf[(x, y)].fg, PURPLE);
    // Every Ticket closed: folded to its rule, no Ticket rows.
    assert!(
        row_of(&buf, "▸ harness-b  Done work").ends_with("━ all 2 closed  ALL CLOSED"),
        "{:#?}",
        rows(&buf)
    );
    assert!(
        find(&buf, "Old one").is_none(),
        "a closed Epic's Tickets show"
    );
    let (x, y) = find(&buf, "ALL CLOSED").unwrap();
    assert_eq!(buf[(x, y)].fg, GREEN);
    assert!(
        row_of(&buf, "▾ harness-c").ends_with("━ 1 closed · 1 in progress · 1 open  IN PROGRESS"),
        "{:#?}",
        rows(&buf)
    );
    assert!(
        row_of(&buf, "▾ harness-d").ends_with("━ 2 open  NOT STARTED"),
        "{:#?}",
        rows(&buf)
    );
    let (x, y) = find(&buf, "NOT STARTED").unwrap();
    assert_eq!(buf[(x, y)].fg, MUTED);
    // The Tickets hang under their Epic; idle they read IN PROGRESS or CLOSED.
    let (_, y) = find(&buf, "▾ harness-a").unwrap();
    assert!(row(&buf, y + 1).starts_with("    ├─ ✓ 1 Plan floor"));
    assert!(row(&buf, y + 1)
        .trim_end()
        .ends_with("PR #29 merged  CLOSED"));
    assert!(row(&buf, y + 8).starts_with("    └─ · 8 Models"));
    assert!(
        row_of(&buf, "● 4 The / list").ends_with("implement  IN PROGRESS"),
        "{:#?}",
        rows(&buf)
    );
    assert!(row_of(&buf, "✓ 1 First").ends_with("CLOSED"));
    assert!(
        row_of(&buf, "· 3 Third").ends_with("3 Third"),
        "a queued Ticket has a label"
    );
    let (x, y) = find(&buf, "● 4 The / list").unwrap();
    assert_eq!(buf[(x, y)].fg, ticket_color("harness-a.4"));
    // Idle, a Ticket blocked on an open PR is only open.
    assert!(row_of(&buf, "· 5 The @ list").ends_with("5 The @ list"));
    assert!(
        row(&buf, 8).contains("IDLE    4 open Epics  ·  15 Tickets"),
        "the idle status row changed: {:?}",
        row(&buf, 8)
    );
}

#[test]
fn live_only_the_runs_epics_are_listed_with_each_label_and_its_stage() {
    let s = sections_screen(true);
    let buf = render(&s, 120, 40);
    for other in ["harness-b", "harness-c", "harness-d"] {
        assert!(find(&buf, other).is_none(), "{other} is not in the run");
    }
    let (x, y) = find(&buf, "▾ harness-a").unwrap();
    assert_eq!(buf[(x + 2, y)].fg, PURPLE);
    assert!(
        row(&buf, y).trim_end().ends_with("━ 2/8 merged  RUNNING"),
        "{:?}",
        row(&buf, y)
    );
    let at = |y: u16, text: &str| {
        let line = row(&buf, y);
        line[..line.find(text).unwrap()].chars().count()
    };
    assert_eq!(
        at(y, "RUNNING"),
        at(y + 1, "MERGED"),
        "the labels line up under the Epic's word"
    );
    for (text, end, color) in [
        ("✓ 1 Plan floor", "PR #29 merged  MERGED", GREEN),
        ("○ 3 Status counts", "PR #31  TO MERGE", BLUE),
        ("◆ 4 The / list", "implement  NEEDS YOU", ORANGE),
        ("◇ 5 The @ list", "waits on PR #31  WAITING", MUTED),
        (
            "● 6 RECENT scroll",
            "review 1  WORKING",
            ticket_color("harness-a.6"),
        ),
        ("◌ 7 Tree", "fix 2  PARKED", MUTED),
        ("· 8 Models", "8 Models", BORDER),
    ] {
        let line = row_of(&buf, text);
        assert!(line.ends_with(end), "{text}: {line:?}");
        let (x, y) = find(&buf, text).unwrap();
        assert_eq!(buf[(x, y)].fg, color, "{text}");
    }
}

#[test]
fn the_live_status_row_counts_each_label_and_drops_its_glyphs_then_its_end_when_narrow() {
    let mut s = sections_screen(true);
    let full = "| RUNNING  ● 1 working  ◆ 1 needs you  ◇ 1 waiting on a merge  ○ 1 to merge  ◌ 1 parked  ✓ 2 merged";
    let buf = render(&s, 120, 40);
    assert!(
        row(&buf, 8).starts_with(&format!(" {full}")),
        "{:?}",
        row(&buf, 8)
    );
    for (text, color) in [
        ("1 working", TEXT),
        ("1 needs you", ORANGE),
        ("1 waiting on a merge", MUTED),
        ("1 to merge", BLUE),
        ("1 parked", MUTED),
        ("2 merged", GREEN),
    ] {
        let (x, y) = find(&buf, text).unwrap();
        assert_eq!((y, buf[(x, y)].fg), (8, color), "{text}");
    }
    // Too wide for the row: the glyphs go, then the end is cut.
    let bare =
        "| RUNNING  1 working  1 needs you  1 waiting on a merge  1 to merge  1 parked  2 merged";
    assert_eq!(row(&render(&s, 90, 40), 8).trim_end(), format!(" {bare}"));
    assert_eq!(
        row(&render(&s, 80, 40), 8).trim_end(),
        " | RUNNING  1 working  1 needs you  1 waiting on a merge  1 to merge  1 parked"
    );
    // No Ticket parked, no parked count; 7, in progress on bd but not in
    // the run, is only queued.
    s.state.tickets.remove("harness-a.7");
    let buf = render(&s, 120, 40);
    assert!(row_of(&buf, "· 7 Tree").ends_with("7 Tree"));
    let line = row(&buf, 8);
    assert!(
        line.contains(
            "● 1 working  ◆ 1 needs you  ◇ 1 waiting on a merge  ○ 1 to merge  ✓ 2 merged"
        ) && !line.contains("parked"),
        "{line:?}"
    );
}

/// MERGE TO UNBLOCK: a red box between RECENT (or the Question in its place)
/// and the input, a line per open PR a waiting Ticket depends on with every
/// Ticket waiting on it; no box when nothing waits.
#[test]
fn merge_to_unblock_lists_each_pr_a_waiting_ticket_depends_on() {
    let mut s = sections_screen(true);
    let buf = render(&s, 120, 40);
    assert!(
        row(&buf, 35).starts_with("┌ MERGE TO UNBLOCK ─"),
        "{:#?}",
        rows(&buf)
    );
    assert_eq!(
        row(&buf, 36).trim_matches(['│', ' ']),
        "merge to unblock 5: https://github.com/o/r/pull/31"
    );
    assert!(row(&buf, 37).starts_with('└'));
    assert_eq!(buf[(0, 35)].fg, RED);
    let (x, y) = find(&buf, "merge to unblock").unwrap();
    assert_eq!(buf[(x, y)].fg, RED);
    assert!(find(&buf, " QUESTION ").unwrap().1 < 35);
    // 8 waits on 3's PR and on 6's: 3's line names 5 and 8, 6's names 8;
    // 2's PR, which nothing waits on, has no line.
    let pr_open = |s: &mut Screen, n: usize| {
        let ts = s.state.tickets.get_mut(&format!("harness-a.{n}")).unwrap();
        ts.status = STATUS_PR_OPEN.to_string();
        ts.pr = format!("https://github.com/o/r/pull/{}", 28 + n);
    };
    pr_open(&mut s, 6);
    pr_open(&mut s, 2);
    *s.epics[0]
        .tickets
        .iter_mut()
        .find(|t| t.id == "harness-a.8")
        .unwrap() = serde_json::from_str(
        r#"{"id":"harness-a.8","title":"Models","status":"open","issue_type":"task","parent":"harness-a",
            "dependencies":[{"depends_on_id":"harness-a.3","type":"blocks"},{"depends_on_id":"harness-a.6","type":"blocks"}]}"#,
    )
    .unwrap();
    let buf = render(&s, 120, 40);
    assert!(row(&buf, 34).starts_with("┌ MERGE TO UNBLOCK ─"));
    assert_eq!(
        [35, 36].map(|y| row(&buf, y).trim_matches(['│', ' ']).to_string()),
        [
            "merge to unblock 5, 8: https://github.com/o/r/pull/31",
            "merge to unblock 8: https://github.com/o/r/pull/34",
        ]
    );
    assert!(find(&buf, "pull/30").is_none(), "{:#?}", rows(&buf));
    // Nothing waits: 3 and 6 merged, or no run live.
    for n in [3, 6] {
        s.state
            .tickets
            .get_mut(&format!("harness-a.{n}"))
            .unwrap()
            .status = STATUS_MERGED.to_string();
    }
    assert!(find(&render(&s, 120, 40), "MERGE TO UNBLOCK").is_none());
    let buf = render(&sections_screen(false), 120, 40);
    assert!(find(&buf, "MERGE TO UNBLOCK").is_none());
    assert!(
        find(&buf, "merge to unblock").is_none(),
        "idle, a Ticket blocked on an open PR waits on nothing"
    );
}

#[test]
fn a_parked_ticket_of_the_saved_run_reads_parked() {
    let mut s = screen();
    s.state
        .tickets
        .insert("harness-kqe.8".to_string(), ticket(STATUS_PARKED));
    let buf = render(&s, 120, 40);
    let (_, y) = find(&buf, "◌ 8 Events").unwrap();
    assert!(row(&buf, y).contains("PARKED"), "{:?}", row(&buf, y));
}

#[test]
fn a_tall_tree_scrolls_to_its_last_epic_and_one_that_fits_never_scrolls() {
    let mut s = screen();
    s.key(key(KeyCode::PageDown));
    render(&s, 120, 40);
    assert_eq!(s.scroll.get(), 0, "8 rows fit at 120x40 and scrolled");
    for n in 0..3 {
        s.epics.push(Epic {
            id: format!("harness-e{n}"),
            title: format!("Epic {n}"),
            tickets: (0..5)
                .map(|i| issue(&format!("harness-e{n}.{i}"), "work", "open"))
                .collect(),
        });
    }
    // 80x24 leaves TICKETS 7 of its 26 rows: six rows and the tail.
    let buf = render(&s, 80, 24);
    assert!(find(&buf, "▾ harness-kqe  Build").is_some());
    assert!(find(&buf, "… 20 more, PgDn").is_some(), "{:#?}", rows(&buf));
    assert!(find(&buf, "Epic 2").is_none());
    for _ in 0..3 {
        s.key(key(KeyCode::PageDown));
    }
    let buf = render(&s, 80, 24);
    assert_eq!(s.scroll.get(), 19, "kept inside the tree: its last 7 rows");
    assert!(
        find(&buf, "▾ harness-e2  Epic 2").is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert!(find(&buf, "more").is_none(), "the last rows need no tail");
    // Up and Down are RECENT's.
    s.key(key(KeyCode::Up));
    s.key(key(KeyCode::Down));
    assert_eq!(s.scroll.get(), 19);
    s.key(key(KeyCode::PageUp));
    let buf = render(&s, 80, 24);
    assert_eq!(s.scroll.get(), 9);
    assert!(find(&buf, "… 11 more, PgDn").is_some(), "{:#?}", rows(&buf));
    // Typing takes the keys back for the input line.
    s.key(key(KeyCode::Char('/')));
    s.key(key(KeyCode::PageUp));
    assert_eq!(s.scroll.get(), 9);
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
    assert!(acquire_lock(&w.repo).is_ok(), "the lock outlived the run");
    assert_eq!(s.state, State::default(), "a done Epic is still saved");
    assert_eq!(load_state(&w.repo).unwrap(), State::default());
    assert_eq!(w.lock().peak, 1, "--max 1 was not obeyed");
    assert!(
        log(&w).contains(" hx-1 PR #hx-1 opened after 1 round (https://example.test/pr/hx-1)\n")
            && log(&w).contains(" Epic done, every Ticket closed\n"),
        "log:\n{}",
        log(&w)
    );
    // The tree came from bd again: both Tickets are closed, the Epic folded.
    let buf = render(&s, 120, 40);
    assert!(
        row_of(&buf, "▸ hx  Epic hx").ends_with("all 2 closed  ALL CLOSED"),
        "{:#?}",
        rows(&buf)
    );
    assert!(find(&buf, "✓ hx-1").is_none(), "{:#?}", rows(&buf));
    assert!(find(&buf, "IDLE    1 open Epic  ·  2 Tickets").is_some());
    assert!(find(&buf, "saved run").is_none());
}

/// .harness/config.json is read when a run starts: one no Pipeline Stage can
/// start on refuses the run, naming the file.
#[test]
fn an_unreadable_config_or_a_stage_off_claude_refuses_the_run() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    let file = w.repo.join(".harness/config.json");
    let mut s = shell(&w);
    write_file(&file, "{ not json");
    s.command("/start-epic hx");
    assert!(
        notice(&s).starts_with(&format!("{}: ", file.display())),
        "{}",
        notice(&s)
    );
    assert!(s.run.is_none());

    write_file(&file, r#"{"implement": {"app": "codex"}}"#);
    s.command("/start-epic hx");
    assert_eq!(notice(&s), "implement runs on claude only");
    assert!(s.run.is_none() && w.called("bd worktree create").is_empty());

    write_file(&file, r#"{"fix": {"app": "codex"}}"#);
    s.command("/start-epic hx");
    assert_eq!(notice(&s), "fix runs on claude only");
    assert!(s.run.is_none());

    // Address runs on demand: its row does not hold up the Pipeline.
    w.lock().merged = true;
    write_file(&file, r#"{"address": {"app": "codex"}}"#);
    s.command("/start-epic hx");
    assert!(s.run.is_some(), "{:?}", s.notice);
    await_line(&mut s, "Epic done, every Ticket closed");
    await_end(&mut s);
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
        row(&buf, 8).contains(
            "RUNNING  ● 1 working  ◆ 0 needs you  ◇ 0 waiting on a merge  ○ 0 to merge  ✓ 0 merged"
        ),
        "{:?}",
        row(&buf, 8)
    );
    let (_, y) = find(&buf, "hx  Epic hx").unwrap();
    assert!(
        row(&buf, y).trim_end().ends_with("0/1 merged  RUNNING"),
        "{:?}",
        row(&buf, y)
    );
    assert!(
        row(&buf, y + 1).contains("● hx-1 Ticket hx-1")
            && row(&buf, y + 1).trim_end().ends_with("implement  WORKING"),
        "{:?}",
        row(&buf, y + 1)
    );

    s.command("/stop-work");
    await_line(&mut s, "stopped, panes left running, /continue resumes");
    await_end(&mut s);
    assert!(!s.running);
    assert!(
        acquire_lock(&w.repo).is_ok(),
        "the lock outlived /stop-work"
    );
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
        row(&buf, 8).contains("RUNNING  ● 0 working  ◆ 1 needs you"),
        "{:?}",
        row(&buf, 8)
    );
    let (_, y) = find(&buf, "◆ hx-1 Ticket hx-1").unwrap();
    assert!(row(&buf, y).contains("NEEDS YOU"), "{:?}", row(&buf, y));

    s.command("/retry");
    assert_eq!(notice(&s), "usage: /retry <ticket>");
    s.command("/retry hx-9");
    await_line(&mut s, "hx-9 refused: not a Ticket of this run");
    // The Ticket's Question holds it: the commands are refused until answered.
    s.command("/park @hx-1"); // a leading @ is stripped
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
        row(&buf, 8).contains("○ 0 to merge  ◌ 1 parked  ✓ 0 merged"),
        "{:?}",
        row(&buf, 8)
    );
    assert!(find(&buf, "◌ hx-1 Ticket hx-1").is_some());
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
    s.notice = None;
    s.command("/start-ticket @hx-");
    assert_eq!(notice(&s), "matches: hx-1 Ticket hx-1  ·  hx-2 Ticket hx-2");
    // No list opens by itself in the argument slot: Tab there does nothing.
    s.input = "/start-epic EPIC".to_string();
    s.key(key(KeyCode::Tab));
    assert_eq!(s.input, "/start-epic EPIC");
    assert!(s.run.is_none());
    s.input.clear();

    type_line(&mut s, "/start-epic Epic hx");
    assert_eq!(question(&s), "discard the saved run on old?");
    assert!(s.run.is_none(), "started before the answer");
    type_line(&mut s, "n");
    assert!(s.run.is_none() && s.questions.is_empty(), "started on no");
    assert_eq!(notice(&s), "cancelled");
    // A command typed past the Question leaves it waiting; Esc cancels it; it
    // never expires.
    type_line(&mut s, "/start-epic hx");
    type_line(&mut s, "/bogus");
    assert_eq!(notice(&s), "unknown command: /bogus");
    assert!(s.questions.len() == 1 && s.run.is_none());
    s.tick();
    s.key(key(KeyCode::Esc));
    assert!(s.questions.is_empty() && s.run.is_none());
    assert_eq!(notice(&s), "cancelled");
    // Enter alone is no, and a repeated command does not stack a second
    // confirmation.
    type_line(&mut s, "/start-epic hx");
    type_line(&mut s, "/start-epic hx");
    assert_eq!(s.questions.len(), 1, "the confirmation stacked");
    s.key(key(KeyCode::Enter));
    assert!(
        s.run.is_none() && s.questions.is_empty(),
        "Enter discarded the saved run"
    );
    assert_eq!(load_state(&w.repo).unwrap().epic, "old");
    type_line(&mut s, "/start-epic hx");
    s.key(key(KeyCode::Char('y')));
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
    assert!(acquire_lock(&w.repo).is_ok(), "the lock outlived the run");
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
    s.cfg.exe = w.repo.join("harness");
    std::fs::write(&s.cfg.exe, b"old").unwrap();
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
    let fake = Fake::quiet();
    let mut s = screen_at(fake.clone(), repo.path());
    let file = Path::new("/r/.harness/runs/harness-kqe.11/fix-1.md");
    let wake = || Ask::Wake {
        pane: "w1:p7".to_string(),
        tail: "Ran the tests: 12 passed.\n> Should I also update the docs?\n".to_string(),
        file: file.to_path_buf(),
        // its waits spent
        actions: Action::ALL[..4].to_vec(),
        judged: None,
    };
    s.push(asking(
        "harness-kqe.11",
        "stuck in fix 1: went idle without a result (pane 2-1)",
        wake(),
    ));
    assert!(
        fake.calls().is_empty(),
        "the screen thread ran {:?}",
        fake.calls()
    );
    // The Question takes RECENT's space, RECENT keeping what it leaves; the
    // tree stays whole, and the tail shows in what the question and its
    // options leave.
    let buf = render(&s, 120, 40);
    assert!(find(&buf, "14 Self-update").is_some(), "{:#?}", rows(&buf));
    let (_, y) = find(&buf, " QUESTION ").unwrap();
    assert_eq!(find(&buf, " RECENT ").map(|(_, r)| y - r), Some(3));
    let body: Vec<String> = (y + 1..39)
        .map(|y| {
            row(&buf, y)
                .trim_matches(|c| c == '│' || c == ' ')
                .to_string()
        })
        .collect();
    assert_eq!(
        body[..4],
        [
            "11 Questions  stuck in fix 1: went idle without a result (pane 2-1)",
            "",
            "Ran the tests: 12 passed.",
            "> Should I also update the docs?",
        ],
        "{:#?}",
        rows(&buf)
    );
    assert!(body[4].starts_with("› 1. nudge: "), "{body:#?}");
    let text = body.join(" ");
    let [first, second] = nudges(file);
    for option in [
        format!("› 1. nudge: {first}"),
        format!("2. nudge: {second}"),
        "3. retry with a fresh session".to_string(),
        "4. park".to_string(),
        "5. open the pane".to_string(),
        "6. a prompt of your own".to_string(),
    ] {
        assert!(
            text.contains(&option),
            "{option:?} is not shown in full:\n{body:#?}"
        );
    }
    let (x, y1) = find(&buf, "› 1. nudge").unwrap();
    assert_eq!(buf[(x, y1)].fg, PURPLE);
    let (x, y2) = find(&buf, "2. nudge").unwrap();
    assert!(y2 > y1 && buf[(x, y2)].fg != PURPLE);
    let (_, hint) = find(&buf, "↑↓ or a number picks, Enter answers, Esc hides").unwrap();
    assert!(
        row(&buf, hint).starts_with('└'),
        "the hint is not on the border"
    );
    s.running = true;
    assert!(
        find(&render(&s, 120, 40), "◆ 11 Questions").is_some(),
        "a live Ticket with a Question waiting does not need you"
    );
    s.running = false;
    assert!(
        std::fs::read_to_string(repo.path().join(".harness/orchestrator.log"))
            .unwrap()
            .lines()
            .any(|l| l.get(20..) == Some("harness-kqe.11 asking you: stuck in fix 1"))
    );

    // A number or the arrows move the cursor.
    s.key(key(KeyCode::Char('4')));
    assert_eq!(s.questions[0].cursor, 3);
    s.key(key(KeyCode::Up));
    s.key(key(KeyCode::Up));
    s.key(key(KeyCode::Down));
    assert_eq!(s.questions[0].cursor, 2);
    assert!(find(&render(&s, 120, 40), "› 3. retry with a fresh session").is_some());
    s.key(key(KeyCode::Char('9')));
    assert_eq!(
        s.questions[0].cursor, 2,
        "a number past the options moved the cursor"
    );

    // Esc hides the Question; the status row counts it; Esc on an empty
    // input line or /questions brings it back.
    s.key(key(KeyCode::Esc));
    assert!(s.hidden);
    let buf = render(&s, 120, 40);
    assert!(find(&buf, " QUESTION ").is_none());
    assert!(
        row(&buf, 37).contains("11 Questions  asking you: stuck in fix 1"),
        "RECENT's last row, above the notice: {:#?}",
        rows(&buf)
    );
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

    // One at a time, oldest first, the rest counted; a Ticket asking again
    // replaces its Question, and any other line of the Ticket closes it.
    s.push(asking(
        "harness-kqe.10",
        "waiting at a prompt in fix 1 (pane 3-1)",
        Ask::Blocked {
            pane: "w1:p9".to_string(),
        },
    ));
    assert!(s.hidden, "a new Question unhid the others");
    s.hidden = false;
    s.push(asking(
        "harness-kqe.11",
        "stuck in fix 1: timed out after 1h (pane 2-1)",
        wake(),
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
        body[..5],
        [
            "10 The Shell runs the Orchestrator  waiting at a prompt in fix 1 (pane 3-1)",
            "",
            "› 1. open the pane",
            "2. park",
            "3. I answered it",
        ]
    );
    assert!(body[5].starts_with("└ ↑↓ or a number picks"), "{body:#?}");
    s.push(event(Some("harness-kqe.10"), "carrying on", true));
    assert_eq!(
        question(&s),
        "stuck in fix 1: timed out after 1h (pane 2-1)"
    );
    // A short screen keeps every option: the tail goes first, then tree rows.
    let buf = render(&s, 120, 32);
    assert!(
        find(&buf, "6. a prompt of your own").is_some(),
        "{:#?}",
        rows(&buf)
    );
    assert!(find(&buf, "Ran the tests").is_none(), "{:#?}", rows(&buf));
    assert!(find(&buf, "14 Self-update").is_none(), "{:#?}", rows(&buf));
    assert!(find(&buf, " ━━ ▾ harness-kqe").is_some());
    assert!(row(&buf, 31).starts_with("› ▌"), "{:#?}", rows(&buf));
    s.push(event(Some("harness-kqe.11"), "fix 1 done", true));
    assert!(
        s.questions.is_empty(),
        "the Ticket moved on and its Question stayed"
    );
}

#[test]
fn a_confirmation_and_the_continue_checklist_render_as_questions() {
    let mut s = screen();
    s.confirm("stop the run and exit?", Pending::Exit);
    s.confirm("stop the run and exit?", Pending::Exit);
    assert_eq!(s.questions.len(), 1, "a repeated confirmation stacked");
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
        body[..4],
        ["stop the run and exit?", "", "1. yes", "› 2. no"]
    );
    assert!(
        body[4].starts_with("└ y or n, Enter answers, Esc cancels"),
        "{body:#?}"
    );
    s.key(key(KeyCode::Enter));
    assert!(s.questions.is_empty() && !s.quit, "Enter alone exited");
    assert_eq!(notice(&s), "cancelled");
    s.confirm("stop the run and exit?", Pending::Exit);
    s.key(key(KeyCode::Esc));
    assert!(s.questions.is_empty() && !s.quit);

    s.state.tickets.insert(
        "harness-kqe.9".to_string(),
        TicketState {
            reason: "went idle".to_string(),
            ..ticket(STATUS_PARKED)
        },
    );
    s.command("/continue");
    s.command("/continue");
    assert_eq!(s.questions.len(), 1, "a repeated /continue stacked");
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
            "continue the saved run: each Ticket resumes at its Stage, or is reset to Implement",
            "",
            "› 1. 9 The Shell, idle: harness opens the screen  fix 1  parked: went idle  → resume",
            "2. 10 The Shell runs the Orchestrator  fix 1  → resume",
            "3. 11 Questions  fix 1  → resume",
        ]
    );
    assert!(find(
        &buf,
        "Space toggles resume or reset to Implement, Enter starts, Esc cancels"
    )
    .is_some());
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

/// Every answer to a Wake goes to the Orchestrator for the Wake's session:
/// a canned nudge and a prompt of your own are sent there by herdr agent
/// prompt and re-arm the hold, and a nudged session is offered no canned
/// nudge again; open the pane keeps the Question, park parks; each answer
/// logs its two lines.
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
    let file = w.repo.join(".harness/runs/hx-1/implement.md");
    let pane = asked_pane(&s);
    assert_eq!(pane, s.state.tickets["hx-1"].panes["implement"]);
    // What the session was prompted with, the Stage prompt left out.
    let prompts = |w: &World| {
        w.called(&format!("herdr agent prompt {pane} "))
            .iter()
            .filter(|c| !c.contains("- Result file: "))
            .map(|c| c.splitn(5, ' ').nth(4).unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let [first, _] = nudges(&file);

    pick(&mut s, 1);
    assert!(s.questions.is_empty(), "the answered Question stayed");
    await_line(&mut s, "hx-1 nudged: write the result file");
    assert_eq!(prompts(&w), [first]);
    // The hold is re-armed: the nudged session, idle without a result, Wakes again.
    await_questions(&mut s, 1);
    assert_eq!(
        s.options(),
        [
            "retry with a fresh session",
            "park",
            "wait ten minutes",
            "open the pane",
            "a prompt of your own"
        ],
        "a nudged session was offered a nudge"
    );

    pick(&mut s, 5);
    assert!(s.composing && s.questions.len() == 1);
    assert!(row(&render(&s, 120, 40), 39).contains("your prompt, Enter sends it"));
    type_line(&mut s, "read the failing test first");
    assert!(!s.composing && s.input.is_empty());
    await_line(&mut s, "hx-1 nudged with your prompt");
    assert_eq!(
        prompts(&w).last().map(String::as_str),
        Some("read the failing test first")
    );
    await_questions(&mut s, 1);

    pick(&mut s, 4); // open the pane
    assert_eq!(
        w.called("herdr agent focus"),
        [format!("herdr agent focus {pane}")]
    );
    assert_eq!(s.questions.len(), 1, "open the pane answered the Question");
    assert_eq!(
        w.called("herdr agent start h-hx-1-implement").len(),
        1,
        "a nudge started a fresh session"
    );

    pick(&mut s, 2);
    await_line(&mut s, "hx-1 parked: implement went idle without a result");
    for line in [
        "hx-1 asking you: stuck in implement",
        "hx-1 you answered: nudge",
        "hx-1 nudged: write the result file",
        "hx-1 you answered: your prompt",
        "hx-1 nudged with your prompt",
        "hx-1 you answered: park",
        "hx-1 parked: implement went idle without a result",
    ] {
        assert!(
            logged(&w, line),
            "the log lacks the line {line:?}:\n{}",
            log(&w)
        );
    }
    assert_eq!(log(&w).matches(" asking you: ").count(), 3);
    assert!(!log(&w).contains("open"), "open the pane logged something");
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
    await_line(
        &mut s,
        "hx-1 retrying implement with a fresh session (pane 1-1)",
    );
    await_line(&mut s, "Epic done, every Ticket closed");
    await_end(&mut s);
    assert!(logged(&w, "hx-1 you answered: retry"), "log:\n{}", log(&w));
    assert!(
        logged(
            &w,
            "hx-1 retrying implement with a fresh session (pane 1-1)"
        ),
        "log:\n{}",
        log(&w)
    );
    assert_eq!(w.called("herdr agent start h-hx-1-implement").len(), 2);
    assert!(s.questions.is_empty());
}

/// Below the floor the Wake's Question shows the Judgment's scores and the
/// one nudge it picked, then the unspent actions: after a retry, no retry.
#[test]
fn a_wake_question_below_the_floor_shows_the_scores_and_its_one_nudge() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    w.session(|_| (String::new(), "idle".to_string()));
    let mut s = shell(&w);
    s.cfg.typesafe = TypeSafeFake::new(|_| {
        Ok(serde_json::json!({ "answers": { "action": {
            "choice": "park",
            "confidence": 0.25,
            "probabilities": { "park": 0.5, "nudge_proceed": 0.3, "retry": 0.1, "nudge_write_result": 0.06, "wait": 0.04 },
        } } }))
    });
    s.command("/start-epic hx");
    await_line(&mut s, "hx-1 asking you: stuck in implement");
    let [_, proceed] = nudges(&w.repo.join(".harness/runs/hx-1/implement.md"));
    let rest = ["open the pane", "a prompt of your own"].map(str::to_string);
    let options = |actions: &[&str]| -> Vec<String> {
        std::iter::once(format!("nudge: {proceed}"))
            .chain(actions.iter().map(|a| a.to_string()))
            .chain(rest.clone())
            .collect()
    };
    assert_eq!(
        s.options(),
        options(&["retry with a fresh session", "park", "wait ten minutes"])
    );
    let buf = render(&s, 120, 40);
    assert!(
        find(
            &buf,
            "judged: park 0.50, nudge to carry on 0.30, retry 0.10, nudge to write the result 0.06, wait 0.04"
        )
        .is_some(),
        "{:#?}",
        rows(&buf)
    );

    pick(&mut s, 2);
    await_line(
        &mut s,
        "hx-1 retrying implement with a fresh session (pane 1-1)",
    );
    // The fresh session's Wake: the retry re-armed the nudge, and is spent.
    await_questions(&mut s, 1);
    assert_eq!(s.options(), options(&["park", "wait ten minutes"]));
    pick(&mut s, 1);
    await_line(&mut s, "hx-1 nudged: carry on, the Ticket is the spec");
    s.command("/stop-work");
    await_end(&mut s);
}

/// Of the waiting lines, only a trust dialog blocks a Ticket on the user; a
/// Judgment's wait does not.
#[test]
fn only_a_trust_dialog_waiting_line_blocks_a_ticket() {
    let mut s = screen();
    s.push(event(
        Some("harness-kqe.10"),
        "waiting: still working (pane 2-1)",
        true,
    ));
    assert!(!s.blocked("harness-kqe.10"), "a wait blocked the Ticket");
    s.push(event(
        Some("harness-kqe.10"),
        "waiting: claude does not trust /r yet, open it there once and accept (pane 2-1)",
        true,
    ));
    assert!(s.blocked("harness-kqe.10"));
}

/// A blocked session's Question: "I answered it" closes it, the pane moving
/// on closes it by itself with "carrying on", and park takes the Ticket out
/// at its Stage.
#[test]
fn a_blocked_question_closes_itself_when_the_session_carries_on() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().merged = true;
    w.session(|_| ("STATUS: done\n".to_string(), "blocked".to_string()));
    let unblock = |w: &World| {
        for status in w.lock().agents.values_mut() {
            *status = "idle".to_string();
        }
    };
    let mut s = shell(&w);
    s.command("/start-epic hx");
    await_line(
        &mut s,
        "hx-1 asking you: waiting at a prompt in implement (pane 1-1)",
    );
    assert!(matches!(
        s.questions[0].about,
        About::Asked(Ask::Blocked { .. })
    ));
    pick(&mut s, 3);
    assert!(s.questions.is_empty());
    unblock(&w);
    await_line(&mut s, "hx-1 carrying on");
    assert!(
        logged(&w, "hx-1 you answered: I answered it"),
        "log:\n{}",
        log(&w)
    );
    assert!(logged(&w, "hx-1 carrying on"), "log:\n{}", log(&w));
    // Review blocks next; nobody answers, and the session moves on by itself.
    await_line(
        &mut s,
        "hx-1 asking you: waiting at a prompt in review 1 (pane 1-2)",
    );
    assert_eq!(s.questions.len(), 1);
    unblock(&w);
    await_questions(&mut s, 0);
    // The Debate blocks: park.
    await_line(
        &mut s,
        "hx-1 asking you: waiting at a prompt in debate 1 (pane 1-3)",
    );
    s.command("/retry hx-1");
    assert_eq!(notice(&s), "refused: Ticket hx-1 has a Question waiting");
    pick(&mut s, 2);
    await_line(&mut s, "hx-1 parked: by you at debate 1");
    assert!(logged(&w, "hx-1 you answered: park"), "log:\n{}", log(&w));
    assert!(
        logged(&w, "hx-1 parked: by you at debate 1"),
        "log:\n{}",
        log(&w)
    );
    assert_eq!(log(&w).matches(" hx-1 carrying on\n").count(), 2);
    s.command("/stop-work");
    await_end(&mut s);
    assert_eq!(
        load_state(&w.repo).unwrap().tickets["hx-1"].status,
        STATUS_PARKED
    );
}

/// A Ticket that moves on while its Question waits (the user fixed it in
/// the pane) closes the Question, and /retry is taken again; an answer that
/// arrives after its session has moved on is dropped, not applied to the
/// next Stage.
#[test]
fn a_question_closes_when_its_ticket_moves_on_and_a_late_answer_is_dropped() {
    for late in [false, true] {
        let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
        w.session(|p| match p.stage.as_str() {
            "review" => (String::new(), "working".to_string()),
            _ => (String::new(), "idle".to_string()),
        });
        let mut s = shell(&w);
        s.command("/start-epic hx");
        await_line(&mut s, "hx-1 asking you: stuck in implement");
        write_file(
            &w.repo.join(".harness/runs/hx-1/implement.md"),
            "STATUS: done\n",
        );
        let o = orchestrator(&s);
        if !late {
            await_line(&mut s, "hx-1 review 1 started");
            assert!(s.questions.is_empty(), "the Question outlived its Wake");
            assert!(o.commands().is_empty() && o.answers.lock().unwrap().is_empty());
            s.command("/retry hx-1");
            assert_eq!(notice(&s), "", "/retry was refused");
            assert_eq!(o.commands(), ["retry-hx-1"]);
        } else {
            // The Shell has not seen the Ticket move on yet: park is answered.
            let deadline = Instant::now() + Duration::from_secs(5);
            while !logged(&w, "hx-1 review 1 started: codex (pane 1-2)") {
                assert!(Instant::now() < deadline, "Review never started");
                thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(
                question(&s),
                "stuck in implement: went idle without a result (pane 1-1)"
            );
            pick(&mut s, 4);
            let deadline = Instant::now() + Duration::from_secs(5);
            while !logged(&w, "hx-1 dropped your park: that session has moved on") {
                assert!(
                    Instant::now() < deadline,
                    "the late park was not dropped:\n{}",
                    log(&w)
                );
                thread::sleep(Duration::from_millis(1));
            }
            s.poll();
            assert_eq!(
                s.state.tickets["hx-1"].status, STATUS_RUNNING,
                "a late park parked Review"
            );
            assert!(o.answers.lock().unwrap().is_empty());
        }
        s.command("/stop-work");
        await_end(&mut s);
    }
}

/// /park on a running Ticket, no Wake: it leaves at its Stage at once, its
/// pane left alone.
#[test]
fn park_takes_a_running_ticket_out_at_its_stage() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    w.session(|_| (String::new(), "working".to_string()));
    let mut s = shell(&w);
    s.command("/start-ticket hx-1");
    await_line(&mut s, "hx-1 implement started: claude (pane 1-1)");
    s.command("/park hx-1");
    await_line(&mut s, "hx-1 parked: by you at implement");
    await_end(&mut s);
    let ts = s.state.tickets["hx-1"].clone();
    assert_eq!(
        (ts.status.as_str(), ts.reason.as_str()),
        (STATUS_PARKED, "by you at implement")
    );
    assert!(
        w.called("herdr pane close").is_empty(),
        "park closed the pane"
    );
    assert_eq!(w.lock().agents[&ts.panes["implement"]], "working");
    assert!(!log(&w).contains("stuck in"), "park waited for a Wake");

    // /continue takes a saved run of Parked Tickets alone: resume unparks
    // the Ticket at its Stage, where the result it wrote meanwhile is taken.
    write_file(
        &w.repo.join(".harness/runs/hx-1/implement.md"),
        "STATUS: done\n",
    );
    w.session(succeed);
    s.command("/continue");
    assert_eq!(
        s.options(),
        ["hx-1 Ticket hx-1  implement  parked: by you at implement  → resume"]
    );
    s.key(key(KeyCode::Enter));
    assert!(s.running, "{:?}", s.notice);
    await_line(&mut s, "hx-1 PR #hx-1 opened after 1 round");
    await_end(&mut s);
    assert_eq!(w.called("herdr agent start h-hx-1-implement").len(), 1);
}

/// A reset row of the /continue checklist runs Implement over, once the
/// lock is held: its panes close, its run directory moves aside as evidence,
/// and a fresh Implement session starts.
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
    let runs = w.repo.join(".harness/runs");
    let state = std::fs::read_to_string(w.repo.join(".harness/state.json")).unwrap();
    let closed = w.called("herdr pane close").len();

    // Another process's run holds the lock: nothing is closed, moved or saved.
    let other = acquire_lock(&w.repo).unwrap();
    s.command("/continue");
    s.key(key(KeyCode::Char(' ')));
    s.key(key(KeyCode::Enter));
    assert!(s.run.is_none());
    assert!(
        notice(&s).starts_with("a run is live in this repo"),
        "{:?}",
        s.notice
    );
    assert_eq!(w.called("herdr pane close").len(), closed);
    assert!(runs.join("hx-1/implement.md").exists() && !runs.join("hx-1.reset-1").exists());
    assert_eq!(
        std::fs::read_to_string(w.repo.join(".harness/state.json")).unwrap(),
        state
    );
    drop(other);
    assert!(acquire_lock(&w.repo).is_ok());

    w.session(succeed);
    s.command("/continue");
    assert_eq!(s.options(), ["hx-1 Ticket hx-1  review 1  → resume"]);
    s.key(key(KeyCode::Char(' ')));
    assert_eq!(
        s.options(),
        ["hx-1 Ticket hx-1  review 1  → reset to Implement"]
    );
    s.key(key(KeyCode::Enter));
    assert!(s.running, "{:?}", s.notice);
    assert_eq!(
        w.called("herdr pane close").len(),
        closed + 2,
        "the old panes stay open"
    );
    assert!(
        runs.join("hx-1.reset-1/implement.md").exists(),
        "the evidence went"
    );
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

/// A plan Question: the judged line, then the Question, the Judgment's
/// answer and score above the plan, which PageDown and PageUp scroll by
/// rows, while the feedback is typed too. Feedback goes back to the session
/// and its revised plan asks again; approve approves it; each answer logs
/// two lines.
#[test]
fn a_plan_question_scrolls_its_plan_by_rows_and_sends_feedback_then_approval() {
    let (w, _) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().merged = true;
    let run = w.repo.join(".harness/runs/hx-1");
    let words = vec!["word"; 60].join(" "); // three rows at 116 columns
    let steps: String = (1..=40).map(|n| format!("- step {n}\n")).collect();
    let plan = format!("{words}\n{steps}");
    let revised = format!("{plan}- step 41\n");
    w.session(move |p| match (p.stage.as_str(), p.approved) {
        ("implement", false) => at_dialog(&run, &plan),
        ("", _) if p.text == "cover y too" => at_dialog(&run, &revised),
        _ => succeed(p),
    });
    let mut s = shell(&w);
    s.cfg.typesafe = TypeSafeFake::new(|_| Ok(noul(0.3)));
    s.command("/start-epic hx");
    await_line(
        &mut s,
        "hx-1 asking you: plan ready in implement (pane 1-1)",
    );
    assert_eq!(
        s.options(),
        ["approve", "feedback of your own", "park", "open the pane"]
    );
    let body = |s: &Screen| -> Vec<String> {
        let buf = render(s, 120, 40);
        let (_, y) = find(&buf, " QUESTION ").unwrap();
        (y + 1..39)
            .map(|y| {
                row(&buf, y)
                    .trim_matches(|c| c == '│' || c == ' ')
                    .to_string()
            })
            .collect()
    };
    let shown = body(&s);
    assert_eq!(
        shown[..3],
        [
            "hx-1 Ticket hx-1  plan ready in implement (pane 1-1)",
            "judged: plan strays from the Ticket 0.70",
            "",
        ],
        "{shown:#?}"
    );
    assert!(shown[3].starts_with("word word") && shown[5].starts_with("word"));
    assert_eq!(shown[6], "- step 1", "{shown:#?}");
    assert!(shown
        .iter()
        .any(|l| l.contains("PgUp PgDn scroll the plan")));
    // The last plan row sits just above the options.
    let last_row = |s: &Screen| {
        let shown = body(s);
        let at = shown
            .iter()
            .position(|l| l.starts_with("› 1. approve"))
            .unwrap();
        shown[at - 1].clone()
    };
    s.key(key(KeyCode::PageDown));
    assert_eq!(
        body(&s)[3],
        "- step 8",
        "PageDown is ten rows, not ten lines"
    );
    for _ in 0..6 {
        s.key(key(KeyCode::PageDown));
        body(&s);
    }
    assert_eq!(last_row(&s), "- step 40", "PageDown ran past the plan");
    let bottom = body(&s)[3].clone();
    s.key(key(KeyCode::PageUp));
    let up = body(&s)[3].clone();
    assert_ne!(up, bottom, "PageUp after the end did not move");

    pick(&mut s, 2);
    assert!(s.composing);
    s.key(key(KeyCode::PageUp));
    assert_ne!(
        body(&s)[3],
        up,
        "PageUp does not scroll while typing feedback"
    );
    type_line(&mut s, "cover y too");
    await_line(&mut s, "hx-1 plan sent back with your feedback");
    await_questions(&mut s, 1);
    assert_eq!(
        body(&s)[6],
        "- step 1",
        "the revised plan kept the old scroll"
    );
    pick(&mut s, 1);
    await_line(&mut s, "hx-1 plan approved");
    await_line(&mut s, "Epic done, every Ticket closed");
    await_end(&mut s);
    let log = log(&w);
    let lines: Vec<&str> = log.lines().filter_map(|l| l.get(20..)).collect();
    let at = |line: &str| {
        lines
            .iter()
            .position(|l| *l == line)
            .unwrap_or_else(|| panic!("the log lacks {line:?}:\n{log}"))
    };
    let order = [
        "hx-1 plan ready in implement (pane 1-1)",
        "hx-1 judged: plan strays from the Ticket 0.70",
        "hx-1 asking you: plan ready in implement (pane 1-1)",
        "hx-1 you answered: feedback",
        "hx-1 plan sent back with your feedback",
    ]
    .map(at);
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{log}");
    assert!(at("hx-1 you answered: approve") < at("hx-1 plan approved"));
    assert_eq!(log.matches(" asking you: plan ready").count(), 2);
    assert_eq!(log.matches(" plan ready in implement").count(), 4, "{log}");
}

/// A plan Question whose feedback was not sent offers to resend it; a
/// plan failure offers open the pane, park, retry and resend.
#[test]
fn kept_feedback_can_be_resent_and_a_failed_plan_step_offers_retry() {
    let repo = TempDir::new();
    let fake = Fake::quiet();
    let mut s = screen_at(fake.clone(), repo.path());
    let feedback = Some("cover y".to_string());
    s.push(Event {
        panel: false, // a Question with no line of its own
        ..asking(
            "harness-kqe.11",
            "plan ready in implement (pane 2-1)",
            Ask::Plan {
                pane: "w1:p7".to_string(),
                plan: "- x\n".to_string(),
                judged: None,
                feedback: feedback.clone(),
            },
        )
    });
    assert_eq!(question(&s), "plan ready in implement (pane 2-1)");
    assert_eq!(
        s.events.iter().map(line).collect::<Vec<_>>(),
        ["harness-kqe.11 asking you: plan ready in implement (pane 2-1)"]
    );
    assert_eq!(
        s.options(),
        [
            "approve",
            "feedback of your own",
            "resend your feedback: cover y",
            "park",
            "open the pane"
        ]
    );
    pick(&mut s, 3);
    assert!(s.questions.is_empty());

    s.push(asking(
        "harness-kqe.11",
        "stuck in implement: left plan mode before your feedback (pane 2-1)",
        Ask::PlanFailed {
            pane: "w1:p7".to_string(),
            feedback,
        },
    ));
    assert_eq!(
        s.options(),
        [
            "open the pane",
            "park",
            "retry with a fresh session",
            "resend your feedback: cover y"
        ]
    );
    pick(&mut s, 1);
    assert_eq!(fake.calls(), ["herdr agent focus w1:p7"]);
    assert_eq!(s.questions.len(), 1, "open the pane answered the Question");
    let log = std::fs::read_to_string(repo.path().join(".harness/orchestrator.log")).unwrap();
    for line in [
        "harness-kqe.11 you answered: feedback",
        "harness-kqe.11 asking you: stuck in implement",
    ] {
        assert!(
            log.lines().any(|l| l.get(20..) == Some(line)),
            "{line:?}:\n{log}"
        );
    }
}
