use super::run;
use crate::orchestrator::write_file;
use crate::setup::TYPESAFE_SKILL;
use crate::skills::manifest::{Location, Manifest, JOBS};
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;
use crate::tools::Tools;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SKILL_NAMES: [&str; 5] = [
    "stage-implement",
    "stage-review",
    "stage-moderate",
    "stage-fix",
    "stage-address",
];

/// A Target repo that passes every preflight check.
pub(super) fn prepared_repo() -> TempDir {
    let repo = TempDir::new();
    for dir in [".beads", ".agents/skills/create-pr"] {
        fs::create_dir_all(repo.path().join(dir)).unwrap();
    }
    fs::write(repo.path().join(".agents/skills/create-pr/SKILL.md"), "pr").unwrap();
    repo
}

/// Every prerequisite there, and a git whose every clone holds every job's
/// default and the typesafe-ai skill at its path in the source, at commit
/// abc123.
pub(super) fn ok_tools() -> Arc<Fake> {
    Fake::new(ok)
}

/// ok_tools' answers.
fn ok(_: &Path, argv: &[&str]) -> Result<String, String> {
    if argv.contains(&"clone") {
        let dest = Path::new(argv.last().unwrap());
        let defaults = JOBS.iter().map(|(_, suggestions)| suggestions[0]);
        for (name, source) in defaults.chain([TYPESAFE_SKILL]) {
            if let Some(path) = source.splitn(3, '/').nth(2) {
                write_file(
                    &dest.join(path).join("SKILL.md"),
                    &format!("---\nname: {name}\n---\n"),
                );
            }
        }
    }
    Ok(match argv.join(" ").as_str() {
        "git remote" => "origin\n",
        "git rev-parse HEAD" => "abc123\n",
        _ => "",
    }
    .to_string())
}

/// herdr_env with HOME at home, and TYPESAFE_API_KEY at key.
fn env_home(home: &Path, key: &str) -> impl Fn(&str) -> String {
    let (home, key) = (home.display().to_string(), key.to_string());
    move |name: &str| match name {
        "HOME" => home.clone(),
        "TYPESAFE_API_KEY" => key.clone(),
        _ => herdr_env(name),
    }
}

/// init with each key its own read, as a terminal delivers them.
fn init_keys(repo: &Path, home: &Path, keys: &[&str]) -> (i32, String) {
    init_with(repo, home, ok_tools(), keys, "")
}

/// init_keys with these tools and TYPESAFE_API_KEY at typesafe_key.
fn init_with(
    repo: &Path,
    home: &Path,
    tools: Arc<dyn Tools>,
    keys: &[&str],
    typesafe_key: &str,
) -> (i32, String) {
    let mut input: Box<dyn Read> = Box::new(std::io::empty());
    for key in keys.iter().rev() {
        input = Box::new(key.as_bytes().chain(input));
    }
    let mut out = Vec::new();
    let code = run(
        &["init".to_string()],
        &mut out,
        Some(&mut input),
        repo,
        tools,
        &env_home(home, typesafe_key),
    );
    (code, String::from_utf8(out).unwrap())
}

/// Where a Location puts a skill: its folder, and its link if it has one.
fn placed(repo: &Path, home: &Path, answer: &str, name: &str) -> (PathBuf, Option<PathBuf>) {
    match answer {
        "2" => (
            repo.join(".agents/skills").join(name),
            Some(repo.join(".claude/skills").join(name)),
        ),
        "3" => (
            home.join(".agents/skills").join(name),
            Some(home.join(".claude/skills").join(name)),
        ),
        _ => (repo.join(".harness/skills").join(name), None),
    }
}

#[test]
fn each_location_writes_where_it_says_and_no_answer_takes_the_checkout() {
    for (answer, location) in [
        ("", Location::Checkout),
        ("1", Location::Checkout),
        ("2", Location::Repo),
        ("3", Location::User),
    ] {
        let (repo, home) = (prepared_repo(), TempDir::new());
        let (code, out) = init_keys(repo.path(), home.path(), &[answer]);
        assert_eq!(code, 0, "{answer:?}: init exit {code}:\n{out}");
        let manifest = Manifest::load(repo.path()).unwrap();
        assert_eq!(manifest.location, Some(location), "{answer:?}");
        // A Shipped skill and a job's default, pinned by its commit.
        for name in ["stage-implement", "tdd"] {
            let (dir, link) = placed(repo.path(), home.path(), answer, name);
            let text = fs::read_to_string(dir.join("SKILL.md"))
                .unwrap_or_else(|err| panic!("{answer:?}: {name} not in {dir:?}: {err}"));
            assert!(
                text.contains(&format!("name: {name}")),
                "{answer:?}: {text}"
            );
            if let Some(link) = link {
                assert_eq!(
                    fs::read_to_string(link.join("SKILL.md")).unwrap(),
                    text,
                    "{answer:?}: {name} not linked"
                );
            }
        }
        assert_eq!(manifest.skills["tdd"].commit, "abc123", "{answer:?}");
        assert!(manifest.skills["stage-implement"].shipped, "{answer:?}");
        for elsewhere in [
            ".agents/skills/stage-implement",
            ".claude/skills/stage-implement",
        ] {
            let there = fs::symlink_metadata(repo.path().join(elsewhere)).is_ok();
            assert_eq!(there, location == Location::Repo, "{answer:?}: {elsewhere}");
        }
    }
}

pub(super) fn herdr_env(key: &str) -> String {
    match key {
        "HERDR_ENV" => "1",
        "HERDR_PANE_ID" => "w1:p1",
        "HERDR_WORKSPACE_ID" => "w1",
        _ => "",
    }
    .to_string()
}

pub(super) fn run_with(
    args: &[&str],
    repo: &Path,
    tools: Arc<dyn Tools>,
    env: &dyn Fn(&str) -> String,
) -> (i32, String) {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let mut out = Vec::new();
    let code = run(&args, &mut out, Some(&mut &b""[..]), repo, tools, env);
    (code, String::from_utf8(out).unwrap())
}

#[test]
fn init_installs_skills_with_working_symlinks() {
    let (repo, home) = (prepared_repo(), TempDir::new());
    let (code, out) = init_keys(repo.path(), home.path(), &["2"]); // in the repo
    assert_eq!(code, 0, "init exit {code}:\n{out}");
    for name in SKILL_NAMES {
        let link = repo.path().join(".claude/skills").join(name);
        let via_link = fs::read_to_string(link.join("SKILL.md"))
            .unwrap_or_else(|err| panic!("{name}: symlink does not resolve: {err}"));
        assert!(
            via_link.contains(&format!("name: {name}")),
            "{name}: SKILL.md has no matching name in frontmatter"
        );
        let target = fs::read_link(&link).unwrap();
        assert!(
            target.is_relative(),
            "{name}: symlink target {target:?} is not relative"
        );
    }
    let ignore = fs::read_to_string(repo.path().join(".gitignore")).unwrap_or_default();
    assert!(
        ignore.contains(".harness/"),
        ".gitignore lacks .harness/: {ignore:?}"
    );
}

#[test]
fn init_keeps_edited_skill_unless_forced() {
    let repo = prepared_repo();
    run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    let skill = repo.path().join(".harness/skills/stage-fix/SKILL.md");
    let shipped = fs::read_to_string(&skill).unwrap();
    fs::write(&skill, "edited in the Target repo").unwrap();
    let ignore_before = fs::read_to_string(repo.path().join(".gitignore")).unwrap();

    let (code, _) = run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "second init exit {code}");
    assert_eq!(
        fs::read_to_string(&skill).unwrap(),
        "edited in the Target repo",
        "rerun overwrote an edited skill"
    );
    assert_eq!(
        fs::read_to_string(repo.path().join(".gitignore")).unwrap(),
        ignore_before,
        "rerun changed .gitignore"
    );

    run_with(&["init", "--force"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(
        fs::read_to_string(&skill).unwrap(),
        shipped,
        "--force did not restore the shipped skill"
    );
}

#[test]
fn preflight_names_each_missing_prerequisite() {
    let repo = TempDir::new(); // no bd workspace, no create-pr skill
    let tools: Arc<dyn Tools> = Fake::new(|_, argv| {
        if argv.join(" ") == "gh auth status" {
            return Err("not logged in".to_string());
        }
        Ok(String::new()) // git remote prints nothing
    });
    let no_env = |_: &str| String::new();

    let (code, out) = run_with(&["init"], repo.path(), tools.clone(), &no_env);
    assert_ne!(code, 0, "init: exit 0 with nothing prepared");
    for want in [
        "bd workspace",
        "gh is not authenticated",
        "git remote",
        "HERDR_ENV",
    ] {
        assert!(out.contains(want), "init: output lacks {want:?}:\n{out}");
    }
}

#[test]
fn init_installs_create_pr_and_keeps_the_repos_own() {
    let fresh = prepared_repo();
    fs::remove_dir_all(fresh.path().join(".agents/skills/create-pr")).unwrap();
    let (code, _) = run_with(&["init"], fresh.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "init exit {code}");
    let got = fs::read_to_string(fresh.path().join(".harness/skills/create-pr/SKILL.md"));
    assert!(
        got.as_ref()
            .is_ok_and(|got| got.contains("name: create-pr")),
        "shipped create-pr not installed: {got:?}"
    );

    // Nothing on stdin to answer with, so the repo's own create-pr stands.
    let own = prepared_repo();
    let (_, out) = run_with(&["init"], own.path(), ok_tools(), &herdr_env);
    let got = fs::read_to_string(own.path().join(".agents/skills/create-pr/SKILL.md")).unwrap();
    assert_eq!(
        got, "pr",
        "init replaced the repo's own create-pr unasked: {got:?}"
    );
    assert!(
        out.contains("already has a create-pr skill"),
        "init did not ask:\n{out}"
    );
}

#[test]
fn init_asks_for_typesafe_and_preflight_warns_when_off() {
    let repo = prepared_repo();
    let (code, out) = run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "TypeSafe off failed preflight:\n{out}");
    assert!(
        out.contains("preflight: TypeSafe is off: every Wake will be a Question"),
        "no warning with TypeSafe off:\n{out}"
    );
    assert!(out.contains("Use TypeSafe"), "init did not ask:\n{out}");

    let with_key = |key: &str| {
        let key = key.to_string();
        move |name: &str| {
            if name == "TYPESAFE_API_KEY" {
                key.clone()
            } else {
                herdr_env(name)
            }
        }
    };
    let (_, out) = run_with(&["init"], repo.path(), ok_tools(), &with_key("sk-env"));
    assert!(
        !out.contains("Use TypeSafe"),
        "asked with the variable set:\n{out}"
    );
    assert!(
        !out.contains("preflight: TypeSafe") && !out.contains("no TypeSafe key"),
        "warned with the variable set:\n{out}"
    );

    // Typed on init's stdin after the answers to where and to the gate (each
    // its own keystroke, as a terminal delivers them), yes and the key are
    // kept in the repo.
    let args = ["init".to_string()];
    let mut out = Vec::new();
    let mut keys = b"\r"
        .chain(&b"2"[..])
        .chain(&b"\r"[..])
        .chain(&b"sk-typed\n"[..]);
    let code = run(
        &args,
        &mut out,
        Some(&mut keys),
        repo.path(),
        ok_tools(),
        &herdr_env,
    );
    let out = String::from_utf8(out).unwrap();
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        fs::read_to_string(repo.path().join(".harness/typesafe-key"))
            .unwrap()
            .trim(),
        "sk-typed"
    );
    assert!(
        !out.contains("preflight: TypeSafe"),
        "warned with the key kept:\n{out}"
    );
}

#[test]
fn rerunning_init_with_another_answer_moves_the_installed_skills() {
    let (repo, home) = (prepared_repo(), TempDir::new());
    fs::remove_dir_all(repo.path().join(".agents/skills/create-pr")).unwrap(); // the shipped one, then
    init_keys(repo.path(), home.path(), &["1"]);
    let mut was = "1";
    for (answer, location) in [("2", Location::Repo), ("3", Location::User)] {
        // Asked where, the gate after it takes no answer: cancel.
        let (code, out) = init_keys(repo.path(), home.path(), &[answer]);
        assert_eq!(code, 0, "{answer:?}: init exit {code}:\n{out}");
        assert_eq!(
            Manifest::load(repo.path()).unwrap().location,
            Some(location),
            "{answer:?}"
        );
        for name in ["stage-implement", "create-pr", "tdd"] {
            let (dir, link) = placed(repo.path(), home.path(), answer, name);
            let via = link.unwrap_or(dir.clone());
            assert!(
                fs::read_to_string(via.join("SKILL.md")).is_ok_and(|text| text.contains(name)),
                "{answer:?}: {name} not moved to {via:?}:\n{out}"
            );
            let (old, old_link) = placed(repo.path(), home.path(), was, name);
            assert!(!old.exists(), "{answer:?}: {name} left at {old:?}");
            if let Some(old_link) = old_link {
                assert!(
                    fs::symlink_metadata(&old_link).is_err(),
                    "{answer:?}: {old_link:?} stayed"
                );
            }
        }
        was = answer;
    }
}

#[test]
fn at_user_level_a_skill_of_yours_is_not_overwritten() {
    let (repo, home) = (prepared_repo(), TempDir::new());
    let mine = home.path().join(".agents/skills/tdd/SKILL.md");
    write_file(&mine, "---\nname: tdd\n---\nmine\n");
    let (code, out) = init_keys(repo.path(), home.path(), &["3"]);
    assert_eq!(code, 0, "init exit {code}:\n{out}");
    assert_eq!(
        fs::read_to_string(&mine).unwrap(),
        "---\nname: tdd\n---\nmine\n"
    );
    assert!(out.contains("keeping your tdd"), "{out}");
    // Linked for Claude, which reads ~/.claude/skills only.
    assert_eq!(
        fs::read_to_string(home.path().join(".claude/skills/tdd/SKILL.md")).unwrap(),
        "---\nname: tdd\n---\nmine\n"
    );
    let manifest = Manifest::load(repo.path()).unwrap();
    assert!(
        !manifest.skills.contains_key("tdd"),
        "{:?}",
        manifest.skills
    );
    assert!(manifest.skills.contains_key("code-review"));

    // Moved there later, yours stays too, and so does everything else: a
    // skill left behind would leave the manifest naming yours.
    let (repo, home) = (prepared_repo(), TempDir::new());
    init_keys(repo.path(), home.path(), &["1"]);
    let mine = home.path().join(".agents/skills/tdd/SKILL.md");
    write_file(&mine, "---\nname: tdd\n---\nmine\n");
    let (_, out) = init_keys(repo.path(), home.path(), &["3"]);
    assert!(out.contains("is there already"), "{out}");
    assert_eq!(
        fs::read_to_string(&mine).unwrap(),
        "---\nname: tdd\n---\nmine\n"
    );
    assert_eq!(
        Manifest::load(repo.path()).unwrap().location,
        Some(Location::Checkout)
    );
    for name in ["tdd", "stage-implement"] {
        assert!(
            repo.path().join(".harness/skills").join(name).exists(),
            "{name} moved"
        );
    }
}

/// A Target repo that passes every preflight check and has no skill of its
/// own: init asks where, then the docs/agents setup, then TypeSafe.
fn bare_repo() -> TempDir {
    let repo = prepared_repo();
    fs::remove_dir_all(repo.path().join(".agents")).unwrap();
    repo
}

#[test]
fn typesafe_is_its_own_opt_in_kept_in_config_json() {
    // (answers after where and the docs/agents setup, TYPESAFE_API_KEY, on)
    for (keys, env_key, on) in [
        (&["\n", "sk-typed\n"][..], "", true),
        (&["y\n", "\n"][..], "", false), // an empty key is no
        (&["n\n"][..], "", false),
        (&[][..], "sk-env", true), // not asked
        (&[][..], "", false),      // non-interactive
    ] {
        let (repo, home) = (bare_repo(), TempDir::new());
        let keys: Vec<&str> = ["1", "\n"].iter().chain(keys).copied().collect();
        let (code, out) = init_with(repo.path(), home.path(), ok_tools(), &keys, env_key);
        let case = format!("{keys:?} {env_key:?}");
        assert_eq!(code, 0, "{case}: init exit {code}:\n{out}");
        assert_eq!(
            out.contains("Use TypeSafe"),
            env_key.is_empty(),
            "{case}: asked or not:\n{out}"
        );
        let config: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(repo.path().join(".harness/config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(config["typesafe"], on, "{case}:\n{out}");
        let manifest = Manifest::load(repo.path()).unwrap();
        assert_eq!(
            manifest.skills.contains_key("typesafe-ai"),
            on,
            "{case}:\n{out}"
        );
        assert_eq!(
            out.contains("preflight: TypeSafe is off"),
            !on,
            "{case}:\n{out}"
        );
    }
    // The typed key is kept where it always was.
    let (repo, home) = (bare_repo(), TempDir::new());
    init_keys(repo.path(), home.path(), &["1", "\n", "\n", "sk-typed\n"]);
    assert_eq!(
        fs::read_to_string(repo.path().join(".harness/typesafe-key"))
            .unwrap()
            .trim(),
        "sk-typed"
    );
    // Nobody answering later keeps the choice made; Ctrl-C is no.
    let (_, out) = init_keys(repo.path(), home.path(), &["\r", "2"]);
    assert!(out.contains("init: TypeSafe on"), "{out}");
    let (_, out) = init_keys(repo.path(), home.path(), &["\r", "2", "\x03"]);
    assert!(out.contains("init: TypeSafe off"), "{out}");
}

#[test]
fn with_no_beads_yes_runs_bd_init_and_non_interactive_skips() {
    let (repo, home) = (bare_repo(), TempDir::new());
    fs::remove_dir_all(repo.path().join(".beads")).unwrap();
    let tools = ok_tools();
    let (_, out) = init_with(repo.path(), home.path(), tools.clone(), &["1", "\n"], "");
    assert!(out.contains("Run bd init now? [Y/n]"), "{out}");
    assert!(
        tools
            .calls()
            .contains(&"bd init --non-interactive".to_string()),
        "{out}"
    );

    let repo = bare_repo();
    fs::remove_dir_all(repo.path().join(".beads")).unwrap();
    let tools = ok_tools();
    let (code, out) = run_with(&["init"], repo.path(), tools.clone(), &herdr_env);
    assert!(
        !tools.calls().iter().any(|c| c.starts_with("bd init")),
        "{out}"
    );
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("preflight: no bd workspace here"), "{out}");
}

#[test]
fn docs_agents_writes_only_what_is_missing_and_the_block_into_claude_md_else_agents_md() {
    let model = |name: &str| fs::read_to_string(Path::new("docs/agents").join(name)).unwrap();
    let repo = prepared_repo();
    write_file(&repo.path().join("docs/agents/domain.md"), "ours");
    write_file(&repo.path().join("CLAUDE.md"), "# Ours\n");
    let (code, out) = run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert_eq!(code, 0, "{out}");
    assert_eq!(read(repo.path(), "docs/agents/domain.md"), "ours");
    for name in ["issue-tracker.md", "triage-labels.md"] {
        assert_eq!(
            read(repo.path(), &format!("docs/agents/{name}")),
            model(name)
        );
    }
    let claude = read(repo.path(), "CLAUDE.md");
    assert!(
        claude.starts_with("# Ours\n\n## Agent skills\n")
            && claude.contains("docs/agents/domain.md"),
        "{claude}"
    );
    assert!(!repo.path().join("AGENTS.md").exists());
    // Ctrl-C is no: nothing written.
    let repo_c = bare_repo();
    init_keys(repo_c.path(), TempDir::new().path(), &["1", "\x03"]);
    assert!(!repo_c.path().join("docs").exists());
    assert!(!repo_c.path().join("AGENTS.md").exists());
    // Everything there: asked nothing.
    let (_, out) = init_keys(repo.path(), TempDir::new().path(), &["\r", "2"]);
    assert!(!out.contains("docs/agents setup"), "{out}");

    // No CLAUDE.md: the block goes into AGENTS.md.
    let repo = prepared_repo();
    write_file(&repo.path().join("AGENTS.md"), "# Agents");
    run_with(&["init"], repo.path(), ok_tools(), &herdr_env);
    assert!(
        read(repo.path(), "AGENTS.md").starts_with("# Agents\n\n## Agent skills\n"),
        "{}",
        read(repo.path(), "AGENTS.md")
    );
    assert_eq!(
        read(repo.path(), "docs/agents/domain.md"),
        model("domain.md")
    );
    assert!(!repo.path().join("CLAUDE.md").exists());
}

fn read(repo: &Path, path: &str) -> String {
    fs::read_to_string(repo.join(path)).unwrap_or_else(|err| panic!("{path}: {err}"))
}

#[test]
fn an_outdated_app_is_offered_once_and_yes_installs_it() {
    const STATUS: &str = "pi: not installed (/h/.pi/agent/extensions/herdr-agent-state.ts)
claude: current (v10) (/h/.claude/hooks/herdr-agent-state.sh)
codex: outdated (v7) (/h/.codex/herdr-agent-state.sh)
";
    let herdr = |codex_on_path: bool| {
        Fake::new(move |dir, argv| match argv.join(" ").as_str() {
            "herdr integration status" => Ok(STATUS.to_string()),
            "which codex" if !codex_on_path => Err("codex not found".to_string()),
            _ => ok(dir, argv),
        })
    };
    let installs = |tools: &Fake| -> Vec<String> {
        let calls = tools.calls();
        calls
            .into_iter()
            .filter(|c| c.starts_with("herdr integration install"))
            .collect()
    };
    // Where, the docs/agents setup, TypeSafe no, then the integrations: yes.
    let (repo, home) = (bare_repo(), TempDir::new());
    let tools = herdr(true);
    let (_, out) = init_with(
        repo.path(),
        home.path(),
        tools.clone(),
        &["1", "\n", "n\n", "\n"],
        "",
    );
    assert_eq!(
        out.matches("Install herdr's integration").count(),
        1,
        "{out}"
    );
    assert!(
        out.contains("  codex: outdated (v7) (/h/.codex/herdr-agent-state.sh)\r\n"),
        "{out}"
    );
    assert!(
        !out.contains("/h/.pi/") && !out.contains("/h/.claude/"),
        "{out}"
    );
    assert_eq!(installs(&tools), ["herdr integration install codex"]);

    // Nobody answering skips it and says what that costs.
    let tools = herdr(true);
    let (_, out) = run_with(&["init"], bare_repo().path(), tools.clone(), &herdr_env);
    assert!(out.contains("/continue starts them fresh"), "{out}");
    assert_eq!(installs(&tools), Vec::<String>::new());

    // An App not on PATH is not offered.
    let tools = herdr(false);
    let (_, out) = run_with(&["init"], bare_repo().path(), tools.clone(), &herdr_env);
    assert!(!out.contains("Install herdr's integration"), "{out}");
}
