use super::{ask_typesafe_key, install_skills, typesafe_key};
use crate::sha256::sha256;
use crate::tempdir::TempDir;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const RECORD: &str = ".harness/installed-skills.json";
const STAGE_FIX: &str = ".agents/skills/stage-fix/SKILL.md";

/// A Target repo that already has a create-pr skill of its own.
fn repo_with_own_pr() -> TempDir {
    let repo = TempDir::new();
    let dir = repo.path().join(".agents/skills/create-pr");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), "the repo's own").unwrap();
    repo
}

fn install(repo: &Path, answer: &str) -> String {
    let mut out = Vec::new();
    install_skills(repo, false, &mut out, &mut answer.as_bytes(), false).unwrap();
    String::from_utf8(out).unwrap()
}

fn read(repo: &Path, path: &str) -> String {
    fs::read_to_string(repo.join(path)).unwrap_or_else(|err| panic!("{path}: {err}"))
}

#[test]
fn install_skills_asks_before_touching_the_repos_own_create_pr() {
    for (answer, own, beside) in [
        ("\r", "the repo's own", ""), // enter on the first option keeps it
        ("", "the repo's own", ""),   // so does a closed stdin
        ("nonsense\r", "the repo's own", ""), // so do keys that mean nothing here
        ("\x1b[B\x1b[A\r", "the repo's own", ""), // down, then back up
        ("\x1b[B\r", "name: create-pr", ""), // down one: replace it
        (
            "\x1b[B\x1b[B\x1b[B\r",
            "the repo's own",
            "name: harness-create-pr",
        ), // down past the end
        ("1", "the repo's own", ""),  // a digit picks its option outright
        ("2", "name: create-pr", ""),
        ("3", "the repo's own", "name: harness-create-pr"),
    ] {
        let repo = repo_with_own_pr();
        let out = install(repo.path(), answer);
        assert!(
            out.contains("already has a create-pr skill"),
            "{answer:?}: init did not ask:\n{out}"
        );
        let got = read(repo.path(), ".agents/skills/create-pr/SKILL.md");
        assert!(
            got.contains(own),
            "{answer:?}: create-pr is {got:?}, want {own:?}"
        );
        let body = fs::read_to_string(
            repo.path()
                .join(".claude/skills/harness-create-pr/SKILL.md"),
        );
        if beside.is_empty() {
            assert!(
                body.is_err(),
                "{answer:?}: installed harness-create-pr anyway"
            );
            continue;
        }
        let body = body.unwrap_or_else(|err| {
            panic!("{answer:?}: harness-create-pr not installed through its link: {err}")
        });
        assert!(
            body.contains(beside),
            "{answer:?}: harness-create-pr not installed through its link: {body:?}"
        );
        assert!(
            !body.contains("name: create-pr"),
            "{answer:?}: harness-create-pr still calls itself create-pr"
        );
    }
}

#[test]
fn install_skills_does_not_ask_when_the_repo_has_no_create_pr() {
    let repo = TempDir::new();
    let out = install(repo.path(), "");
    assert!(
        !out.contains("already has"),
        "init asked about a create-pr the repo does not have:\n{out}"
    );
    let got = read(repo.path(), ".claude/skills/create-pr/SKILL.md");
    assert!(
        got.contains("name: create-pr"),
        "shipped create-pr not installed: {got:?}"
    );
}

#[test]
fn install_skills_force_skips_the_questions_and_keeps_the_repos_own_create_pr() {
    let repo = repo_with_own_pr();
    let mut out = Vec::new();
    install_skills(repo.path(), true, &mut out, &mut "2\n".as_bytes(), false).unwrap();
    let out = String::from_utf8(out).unwrap();
    assert!(!out.contains("already"), "--force still asked:\n{out}");
    assert_eq!(
        read(repo.path(), ".agents/skills/create-pr/SKILL.md"),
        "the repo's own"
    );
    assert!(read(repo.path(), STAGE_FIX).contains("name: stage-fix"));
}

#[test]
fn install_skills_overwrite_keeps_the_repos_own_create_pr_beside_the_recorded_one() {
    let repo = repo_with_own_pr();
    install(repo.path(), "3"); // beside it, as harness-create-pr
    let beside = repo
        .path()
        .join(".agents/skills/harness-create-pr/SKILL.md");
    fs::write(&beside, "edited").unwrap();
    let out = install(repo.path(), "3"); // the gate: overwrite everything
    assert!(out.contains("already installed"), "no gate:\n{out}");
    assert!(
        !out.contains("already has"),
        "asked about create-pr again:\n{out}"
    );
    assert_eq!(
        read(repo.path(), ".agents/skills/create-pr/SKILL.md"),
        "the repo's own"
    );
    assert!(
        fs::read_to_string(&beside)
            .unwrap()
            .contains("name: harness-create-pr"),
        "overwrite left the edited harness-create-pr"
    );
}

fn record(repo: &Path) -> BTreeMap<String, String> {
    serde_json::from_str(&read(repo, RECORD)).unwrap()
}

/// Every file under the repo with its content, to prove a run touched nothing.
fn snapshot(repo: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.symlink_metadata().unwrap().is_dir() {
                walk(&path, into);
            } else {
                into.insert(
                    path.display().to_string(),
                    fs::read(&path).unwrap_or_default(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(repo, &mut files);
    files
}

#[test]
fn install_skills_records_the_hash_of_every_file_it_writes_without_a_gate() {
    let repo = TempDir::new();
    let out = install(repo.path(), "");
    assert!(
        !out.contains("already installed"),
        "a fresh repo hit the gate:\n{out}"
    );
    let record = record(repo.path());
    assert_eq!(record.len(), 6, "record: {record:?}");
    for (rel, hash) in &record {
        assert_eq!(
            sha256(&fs::read(repo.path().join(rel)).unwrap()),
            *hash,
            "{rel}: record does not match the file"
        );
    }
    assert!(record.contains_key(STAGE_FIX), "record: {record:?}");
}

#[test]
fn install_skills_refresh_rewrites_only_files_unedited_since_install() {
    let repo = TempDir::new();
    install(repo.path(), "");
    let shipped = read(repo.path(), STAGE_FIX);
    // stage-fix as an older release wrote it, still unedited: the record agrees.
    let stale = repo.path().join(STAGE_FIX);
    fs::write(&stale, "older shipped text").unwrap();
    let mut rec = record(repo.path());
    rec.insert(STAGE_FIX.to_string(), sha256(b"older shipped text"));
    fs::write(
        repo.path().join(RECORD),
        serde_json::to_string(&rec).unwrap(),
    )
    .unwrap();
    // stage-review edited in the Target repo: the record disagrees.
    let edited = repo.path().join(".agents/skills/stage-review/SKILL.md");
    fs::write(&edited, "edited in the Target repo").unwrap();

    let out = install(repo.path(), "2");
    assert!(out.contains("already installed"), "no gate:\n{out}");
    assert_eq!(
        read(repo.path(), STAGE_FIX),
        shipped,
        "refresh left the stale skill"
    );
    assert_eq!(
        record(repo.path())[STAGE_FIX],
        sha256(shipped.as_bytes()),
        "refresh did not update the record"
    );
    assert_eq!(
        fs::read_to_string(&edited).unwrap(),
        "edited in the Target repo",
        "refresh overwrote an edited skill"
    );

    install(repo.path(), "3");
    assert!(
        fs::read_to_string(&edited)
            .unwrap()
            .contains("name: stage-review"),
        "overwrite left the edited skill"
    );
}

#[test]
fn install_skills_refresh_treats_a_file_without_a_record_as_edited() {
    // Installed by the Go binary: the files are there, the record is not.
    let repo = TempDir::new();
    install(repo.path(), "");
    fs::remove_file(repo.path().join(RECORD)).unwrap();
    fs::write(repo.path().join(STAGE_FIX), "from the Go binary").unwrap();
    install(repo.path(), "2");
    assert_eq!(read(repo.path(), STAGE_FIX), "from the Go binary");
}

#[test]
fn install_skills_cancel_and_a_closed_stdin_touch_nothing() {
    for answer in ["1", "\r", "", "\x1b[B\x1b[A\r"] {
        let repo = TempDir::new();
        install(repo.path(), "");
        fs::write(repo.path().join(STAGE_FIX), "edited").unwrap();
        fs::remove_file(repo.path().join(".gitignore")).unwrap();
        let before = snapshot(repo.path());
        let out = install(repo.path(), answer);
        assert!(
            out.contains("already installed"),
            "{answer:?}: no gate:\n{out}"
        );
        assert_eq!(
            snapshot(repo.path()),
            before,
            "{answer:?}: cancel touched a file"
        );
    }
}

fn ask_key(repo: &Path, env: &str, typed: &str) -> String {
    let mut out = Vec::new();
    ask_typesafe_key(repo, env, &mut out, &mut typed.as_bytes(), false).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn ask_typesafe_key_stores_the_typed_key_read_only_to_the_user() {
    let repo = TempDir::new();
    let out = ask_key(repo.path(), "", "sk-typed\n");
    assert!(out.contains("TypeSafe API key"), "not asked:\n{out}");
    let path = repo.path().join(".harness/typesafe-key");
    assert_eq!(fs::read_to_string(&path).unwrap().trim(), "sk-typed");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        typesafe_key(repo.path(), &|_| String::new()).as_deref(),
        Some("sk-typed")
    );

    // Asked again, the stored key stands and no question is put.
    let out = ask_key(repo.path(), "", "sk-other\n");
    assert!(
        !out.contains("TypeSafe API key"),
        "asked with a key stored:\n{out}"
    );
    assert_eq!(fs::read_to_string(&path).unwrap().trim(), "sk-typed");
}

#[test]
fn ask_typesafe_key_skips_when_the_variable_is_set_or_stdin_is_silent() {
    let repo = TempDir::new();
    let out = ask_key(repo.path(), "sk-env", "sk-typed\n");
    assert!(
        !out.contains("TypeSafe API key"),
        "asked with the variable set:\n{out}"
    );
    assert!(!repo.path().join(".harness/typesafe-key").exists());

    for typed in ["", "\n", "sk-a\x03", "sk-b\x04sk-c\n", "\x1b[A\t\n"] {
        let out = ask_key(repo.path(), "", typed);
        assert!(
            out.contains("TypeSafe API key"),
            "{typed:?}: not asked:\n{out}"
        );
        assert!(
            !repo.path().join(".harness/typesafe-key").exists(),
            "{typed:?}: stored anyway"
        );
    }
    assert_eq!(typesafe_key(repo.path(), &|_| String::new()), None);
    // Control bytes and an arrow key never reach the key.
    ask_key(repo.path(), "", "\x1b[Ask-\x01d\x7f\n");
    assert_eq!(read(repo.path(), ".harness/typesafe-key").trim(), "sk-");
}

#[test]
fn typesafe_key_prefers_the_variable_over_the_file() {
    let repo = TempDir::new();
    ask_key(repo.path(), "", "sk-file\n");
    let env = |key: &str| {
        if key == "TYPESAFE_API_KEY" {
            " sk-env ".to_string()
        } else {
            String::new()
        }
    };
    assert_eq!(typesafe_key(repo.path(), &env).as_deref(), Some("sk-env"));
}
