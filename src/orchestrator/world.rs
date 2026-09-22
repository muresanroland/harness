//! The fake world behind the Tools seam: a herdr session, a bd workspace and
//! gh, the port of world_test.go. Sessions "work" synchronously inside
//! 'agent prompt': the session hook decides what result file a Stage's
//! session leaves behind.

// ponytail: the bd dependencies, gh's PR answers and the peak count are read
// by the scheduler tests (harness-kqe.4); drop this line when they land.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::json;

use super::herdr::PaneInfo;
use super::stage::{Config, Orchestrator};
use super::state::load_state;
use super::trust_test::trust_home;
use super::write_file;
use crate::setup::install_skills;
use crate::tempdir::TempDir;
use crate::tools::{RunError, Tools};

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct BdTicket {
    pub(crate) id: String,
    pub(crate) status: String,
    pub(crate) issue_type: String,
    /// Ids that must be closed first.
    #[serde(skip)]
    pub(crate) deps: Vec<String>,
}

impl BdTicket {
    pub(crate) fn new(id: &str) -> Self {
        BdTicket {
            id: id.to_string(),
            ..Default::default()
        }
    }
}

/// A Stage prompt as the fake session sees it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Prompt {
    pub(crate) pane: String,
    pub(crate) text: String,
    pub(crate) ticket: String,
    pub(crate) stage: String,
    pub(crate) file: String,
    pub(crate) round: usize,
    pub(crate) open_pr: bool,
}

/// Go's `(?m)^- ([A-Za-z ]+): (.*)$` over the prompt's input lines.
fn parse_prompt(pane: &str, text: &str) -> Prompt {
    let mut p = Prompt {
        pane: pane.to_string(),
        text: text.to_string(),
        ..Default::default()
    };
    for line in text.lines() {
        let Some((name, value)) = line.strip_prefix("- ").and_then(|l| l.split_once(": ")) else {
            continue;
        };
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphabetic() || c == ' ') {
            continue;
        }
        match name {
            "Ticket" => p.ticket = value.to_string(),
            "Round" => p.round = value.parse().unwrap_or(0),
            "Result file" => {
                p.file = value.to_string();
                let base = Path::new(value).file_name().unwrap().to_string_lossy();
                p.stage = base
                    .trim_end_matches(".md")
                    .split('-')
                    .next()
                    .unwrap()
                    .to_string();
            }
            "Open PR" => p.open_pr = value == "yes",
            _ => {}
        }
    }
    p
}

/// Plays one Stage session: returns the result file's content ("" writes
/// nothing) and the agent status the session settles in.
pub(crate) type Session = Arc<dyn Fn(&Prompt) -> (String, String) + Send + Sync>;

/// Answers a call before the world does, or None to let the world answer.
pub(crate) type Hook = Arc<dyn Fn(&Path, &[&str]) -> Option<Result<String, String>> + Send + Sync>;

/// The default session: every Stage is done, Verdicts are clean, and the last
/// Fix opens a PR.
pub(crate) fn succeed(p: &Prompt) -> (String, String) {
    if p.open_pr {
        return (
            format!("STATUS: done\nPR: https://example.test/pr/{}\n", p.ticket),
            "idle".to_string(),
        );
    }
    ("STATUS: done\n".to_string(), "idle".to_string())
}

/// A session that starts and never finishes.
pub(crate) fn working(_: &Prompt) -> (String, String) {
    (String::new(), "working".to_string())
}

/// What the world's lock guards, the port of world's mu-guarded fields.
#[derive(Default)]
pub(crate) struct Inner {
    next_id: usize,
    pub(crate) tabs: Vec<String>,
    pub(crate) panes: Vec<PaneInfo>,
    /// Pane id -> herdr agent status.
    pub(crate) agents: BTreeMap<String, String>,
    /// Width, height in cells of every pane; zero means roomy and square.
    pub(crate) rect: (usize, usize),
    /// Lines the launching pane received.
    main: Vec<String>,
    pub(crate) tickets: Vec<BdTicket>,
    /// PR url -> gh JSON.
    pub(crate) prs: BTreeMap<String, String>,
    /// The launching pane refuses prompts: agent_blocked.
    pub(crate) main_blocked: bool,
    /// The launching pane is a shell: agent_not_found.
    pub(crate) main_no_agent: bool,
    /// Command prefix -> the error its next call fails with.
    failing: BTreeMap<String, String>,
    /// Every PR is merged as soon as gh is asked about it.
    pub(crate) merged: bool,
    /// Tickets between worktree creation and tab close.
    live: usize,
    pub(crate) peak: usize,
    session: Option<Session>,
    /// What 'agent prompt' and 'agent wait' fail with.
    pub(crate) wait_err: Option<String>,
}

pub(crate) struct World {
    repo_dir: TempDir,
    home_dir: TempDir,
    pub(crate) repo: PathBuf,
    calls: Mutex<Vec<String>>,
    inner: Mutex<Inner>,
    hook: Mutex<Option<Hook>>,
}

/// The fake world and an Orchestrator over it; both agents already trust the
/// repo.
pub(crate) fn new_world(tickets: Vec<BdTicket>) -> (Arc<World>, Orchestrator) {
    let repo_dir = TempDir::new();
    let repo = repo_dir.path().to_path_buf();
    install_skills(
        &repo,
        false,
        &mut std::io::sink(),
        Some(&mut std::io::empty()),
    )
    .unwrap();
    let home_dir = trust_home(&repo);
    let home = home_dir.path().to_path_buf();
    let tickets = tickets
        .into_iter()
        .map(|t| BdTicket {
            status: "open".to_string(),
            issue_type: "task".to_string(),
            ..t
        })
        .collect();
    let w = Arc::new(World {
        repo_dir,
        home_dir,
        repo: repo.clone(),
        calls: Mutex::new(Vec::new()),
        inner: Mutex::new(Inner {
            tickets,
            ..Default::default()
        }),
        hook: Mutex::new(None),
    });
    let state = load_state(&repo).unwrap_or_default();
    let o = Orchestrator::with_state(Config::for_tests(w.clone(), &repo, &home), state);
    (w, o)
}

fn reply(result: serde_json::Value) -> Result<String, String> {
    Ok(json!({ "result": result }).to_string())
}

fn flag_value<'a>(argv: &[&'a str], flag: &str) -> &'a str {
    argv.iter()
        .position(|a| *a == flag)
        .and_then(|i| argv.get(i + 1))
        .copied()
        .unwrap_or("")
}

impl World {
    pub(crate) fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap()
    }

    /// Replaces the session hook.
    pub(crate) fn session(
        &self,
        session: impl Fn(&Prompt) -> (String, String) + Send + Sync + 'static,
    ) {
        self.lock().session = Some(Arc::new(session));
    }

    /// Wraps the world's handling: the port of tests that wrap Fake.Handle.
    pub(crate) fn hook(
        &self,
        hook: impl Fn(&Path, &[&str]) -> Option<Result<String, String>> + Send + Sync + 'static,
    ) {
        *self.hook.lock().unwrap() = Some(Arc::new(hook));
    }

    /// Every call so far, in order, as space-joined command lines.
    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    /// The calls that start with prefix.
    pub(crate) fn called(&self, prefix: &str) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter(|c| c.starts_with(prefix))
            .collect()
    }

    fn handle(&self, argv: &[&str]) -> Result<String, String> {
        let mut w = self.lock();
        let cmd = argv.join(" ");
        if let Some(prefix) = w
            .failing
            .keys()
            .find(|p| cmd.starts_with(p.as_str()))
            .cloned()
        {
            return Err(w.failing.remove(&prefix).unwrap());
        }
        if cmd.starts_with("herdr tab list") {
            let tabs: Vec<_> = w.tabs.iter().map(|id| json!({ "tab_id": id })).collect();
            return reply(json!({ "tabs": tabs }));
        }
        if cmd.starts_with("herdr pane list") {
            return reply(json!({ "panes": w.panes }));
        }
        if cmd.starts_with("herdr pane layout") {
            let layout = w.layout(flag_value(argv, "--pane"));
            return reply(json!({ "layout": { "panes": layout } }));
        }
        if cmd.starts_with("herdr tab create") {
            let (tab, pane) = (w.id("t"), w.id("p"));
            w.tabs.push(tab.clone());
            let root = PaneInfo {
                pane_id: pane,
                tab_id: tab.clone(),
            };
            w.panes.push(root.clone());
            return reply(json!({ "tab": { "tab_id": tab }, "root_pane": root }));
        }
        if cmd.starts_with("herdr pane split") {
            let Some(tab_id) = w
                .panes
                .iter()
                .find(|p| p.pane_id == argv[3])
                .map(|p| p.tab_id.clone())
            else {
                return Err(r#"{"error":{"code":"pane_not_found"}}"#.to_string());
            };
            let pane = PaneInfo {
                pane_id: w.id("p"),
                tab_id,
            };
            w.panes.push(pane.clone());
            return reply(json!({ "pane": pane }));
        }
        if cmd.starts_with("herdr pane close") {
            w.close_pane(argv[3]);
            return reply(json!({ "type": "ok" }));
        }
        if cmd.starts_with("herdr tab close") {
            for p in w.panes.clone() {
                if p.tab_id == argv[3] {
                    w.close_pane(&p.pane_id);
                }
            }
            w.live -= 1;
            return reply(json!({ "type": "ok" }));
        }
        if cmd.starts_with("herdr agent start") {
            let pane = flag_value(argv, "--pane").to_string();
            w.agents.insert(pane, "idle".to_string());
            return reply(json!({}));
        }
        if cmd.starts_with("herdr agent prompt") {
            if argv[3] == "main" {
                if w.main_blocked {
                    return Err(r#"{"error":{"code":"agent_blocked"}}"#.to_string());
                }
                if w.main_no_agent {
                    return Err(r#"{"error":{"code":"agent_not_found","message":"agent target main not found"}}"#.to_string());
                }
                w.main.push(argv[4].to_string());
                return reply(json!({}));
            }
            let p = parse_prompt(argv[3], argv[4]);
            let session = w.session.clone();
            drop(w);
            let (result, status) = match session {
                Some(session) => session(&p),
                None => succeed(&p),
            };
            let mut w = self.lock();
            if !result.is_empty() {
                let _ = fs::write(&p.file, result);
            }
            if let Some(alive) = w.agents.get_mut(&p.pane) {
                *alive = status;
            }
            return w.wait_err.clone().map_or(Ok("{}".to_string()), Err);
        }
        if cmd.starts_with("herdr agent wait") {
            return w.wait_err.clone().map_or(Ok("{}".to_string()), Err);
        }
        if cmd.starts_with("herdr agent get") {
            return match w.agents.get(argv[3]) {
                None => Err(r#"{"error":{"code":"agent_not_found"}}"#.to_string()),
                Some(status) => reply(json!({ "agent": { "agent_status": status } })),
            };
        }

        if cmd.starts_with("bd worktree create") {
            w.live += 1;
            w.peak = w.peak.max(w.live);
            return fs::create_dir_all(argv[3])
                .map(|_| String::new())
                .map_err(|e| e.to_string());
        }
        if cmd.starts_with("bd worktree remove") {
            let _ = fs::remove_dir_all(argv[3]);
            return Ok(String::new());
        }
        if cmd.starts_with("bd close") {
            w.find(argv[2]).status = "closed".to_string();
            return Ok(String::new());
        }
        if cmd.starts_with("bd update") {
            w.find(argv[2]).status = "in_progress".to_string();
            return Ok(String::new());
        }
        if cmd.starts_with("bd list") {
            return Ok(serde_json::to_string(&w.tickets).unwrap());
        }
        if cmd.starts_with("bd ready") {
            let ready: Vec<&BdTicket> = w
                .tickets
                .iter()
                .filter(|t| {
                    t.status == "open"
                        && t.deps.iter().all(|dep| {
                            w.tickets
                                .iter()
                                .any(|d| d.id == *dep && d.status == "closed")
                        })
                })
                .collect();
            return Ok(serde_json::to_string(&ready).unwrap());
        }

        if cmd.starts_with("gh pr view") {
            if let Some(pr) = w.prs.get(argv[3]) {
                return Ok(pr.clone());
            }
            if w.merged {
                return Ok(r#"{"state":"MERGED","mergeable":"UNKNOWN"}"#.to_string());
            }
            return Ok(r#"{"state":"OPEN","mergeable":"MERGEABLE"}"#.to_string());
        }
        if cmd == "git remote" {
            return Ok("origin\n".to_string());
        }
        Ok(String::new())
    }

    pub(crate) fn main_lines(&self) -> Vec<String> {
        self.lock().main.clone()
    }

    /// Waits for the launching pane to receive a line containing want.
    pub(crate) fn await_line(&self, want: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(line) = self.main_lines().into_iter().find(|l| l.contains(want)) {
                return line;
            }
            thread::sleep(Duration::from_millis(1));
        }
        panic!(
            "the launching pane never received {want:?}; it got:\n{}",
            self.main_lines().join("\n")
        );
    }

    pub(crate) fn control(&self, name: &str) {
        write_file(&self.repo.join(".harness").join("control").join(name), "");
    }

    /// Makes the next command starting with prefix fail.
    pub(crate) fn fail_once(&self, prefix: &str, err: &str) {
        self.lock()
            .failing
            .insert(prefix.to_string(), err.to_string());
    }
}

impl Inner {
    fn id(&mut self, kind: &str) -> String {
        self.next_id += 1;
        format!("w1:{kind}{}", self.next_id)
    }

    /// Answers 'herdr pane layout' for the tab holding pane: every pane's
    /// rect, a roomy default for the tests that do not care about geometry.
    fn layout(&self, pane: &str) -> Vec<serde_json::Value> {
        let tab = self
            .panes
            .iter()
            .rfind(|p| p.pane_id == pane)
            .map(|p| p.tab_id.clone())
            .unwrap_or_default();
        let (width, height) = if self.rect == (0, 0) {
            (200, 100)
        } else {
            self.rect
        };
        self.panes
            .iter()
            .filter(|p| p.tab_id == tab)
            .map(|p| json!({ "pane_id": p.pane_id, "rect": { "width": width, "height": height } }))
            .collect()
    }

    fn close_pane(&mut self, id: &str) {
        self.agents.remove(id);
        let Some(i) = self.panes.iter().position(|p| p.pane_id == id) else {
            return;
        };
        let closed = self.panes.remove(i);
        if self.panes.iter().any(|p| p.tab_id == closed.tab_id) {
            return;
        }
        // herdr closes a tab with its last pane
        self.tabs.retain(|t| *t != closed.tab_id);
    }

    fn find(&mut self, id: &str) -> &mut BdTicket {
        match self.tickets.iter_mut().find(|t| t.id == id) {
            Some(t) => t,
            None => panic!("bd was asked about unknown ticket {id:?}"),
        }
    }
}

impl Tools for World {
    fn run(&self, dir: &Path, argv: &[&str]) -> Result<String, RunError> {
        let command = argv.join(" ");
        self.calls.lock().unwrap().push(command.clone());
        let hook = self.hook.lock().unwrap().clone();
        let answer = hook
            .and_then(|hook| hook(dir, argv))
            .unwrap_or_else(|| self.handle(argv));
        answer.map_err(|stderr| RunError {
            command,
            status: "exit status 1".to_string(),
            stderr,
        })
    }
}

/// A Ticket running on its own thread, the port of `go o.runTicket(...)`
/// with the test's cancel: dropping it stops the run and joins the thread.
pub(crate) struct Running {
    o: Arc<Orchestrator>,
    handle: Option<JoinHandle<()>>,
}

/// runTicket in the background.
pub(crate) fn spawn_ticket(o: Arc<Orchestrator>, ticket: &str) -> Running {
    let (run, ticket) = (o.clone(), ticket.to_string());
    Running {
        o,
        handle: Some(thread::spawn(move || run.run_ticket(&ticket))),
    }
}

/// RunTicket in the background: stale control files drained first.
pub(crate) fn spawn_single(o: Arc<Orchestrator>, ticket: &str) -> Running {
    let (run, ticket) = (o.clone(), ticket.to_string());
    Running {
        o,
        handle: Some(thread::spawn(move || {
            run.run_single(&ticket);
        })),
    }
}

impl Running {
    pub(crate) fn finished(&self) -> bool {
        self.handle.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Waits up to `within` for the run to end.
    pub(crate) fn finished_within(&self, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while !self.finished() {
            if Instant::now() > deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(1));
        }
        true
    }

    /// The port of `<-finished` under the Go tests' 5s context.
    pub(crate) fn wait(&mut self) {
        assert!(
            self.finished_within(Duration::from_secs(5)),
            "the run never finished"
        );
        self.handle.take().unwrap().join().unwrap();
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.o.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
