use super::manifest::{
    add, list, parse_source, remove, update, update_all, Added, Installed, Manifest, Source, JOBS,
    NONE,
};
use crate::orchestrator::write_file;
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

const TDD: &str = "---\nname: tdd\ndescription: test first\n---\nversion one\n";

/// mattpocock/skills as a clone finds it: two skills and a README.
const TWO_SKILLS: &[(&str, &str)] = &[
    ("README.md", "the pack"),
    ("skills/engineering/tdd/SKILL.md", TDD),
    ("skills/engineering/tdd/tests.md", "good tests"),
    (
        "skills/engineering/code-review/SKILL.md",
        "---\nname: code-review\n---\n",
    ),
];

/// A remote's HEAD: its commit and its files, changed by a test to push.
type Remote = Arc<Mutex<(&'static str, Vec<(&'static str, &'static str)>)>>;

fn remote(commit: &'static str, files: &[(&'static str, &'static str)]) -> Remote {
    Arc::new(Mutex::new((commit, files.to_vec())))
}

/// git over the remote: a clone writes its files into the destination, and
/// rev-parse answers its commit.
fn git(remote: &Remote) -> Arc<Fake> {
    let remote = remote.clone();
    Fake::new(move |_, argv| {
        let (commit, files) = &*remote.lock().unwrap();
        if argv.contains(&"clone") {
            let dest = Path::new(argv.last().unwrap());
            for (path, text) in files {
                write_file(&dest.join(path), text);
            }
            Ok(String::new())
        } else if argv.contains(&"rev-parse") {
            Ok(format!("{commit}\n"))
        } else {
            Err(format!("unexpected: {}", argv.join(" ")))
        }
    })
}

fn source(repo: &str, git_ref: &str, path: &str) -> Source {
    Source {
        repo: repo.to_string(),
        git_ref: git_ref.to_string(),
        path: path.to_string(),
    }
}

#[test]
fn parse_source_takes_every_form_and_refuses_a_bare_name() {
    let gh = "https://github.com/mattpocock/skills";
    for (text, want) in [
        ("mattpocock/skills", source(gh, "", "")),
        (
            "mattpocock/skills/skills/engineering/tdd",
            source(gh, "", "skills/engineering/tdd"),
        ),
        ("https://github.com/mattpocock/skills", source(gh, "", "")),
        (
            "https://github.com/mattpocock/skills.git/",
            source(gh, "", ""),
        ),
        (
            "https://github.com/mattpocock/skills/tree/main/skills/engineering/tdd",
            source(gh, "main", "skills/engineering/tdd"),
        ),
        ("github.com/mattpocock/skills", source(gh, "", "")),
        (
            "git@github.com:mattpocock/skills.git",
            source("git@github.com:mattpocock/skills.git", "", ""),
        ),
        (
            "https://gitlab.com/someone/skills.git",
            source("https://gitlab.com/someone/skills.git", "", ""),
        ),
    ] {
        assert_eq!(parse_source(text), Ok(want), "{text}");
    }
    let err = parse_source("tdd").unwrap_err();
    assert!(
        err.contains("skills.sh") && err.contains("tdd"),
        "no hint for a bare name: {err}"
    );
    assert!(parse_source("mattpocock/skills/../../etc").is_err());
}

#[test]
fn add_refuses_a_bare_name_before_cloning() {
    let repo = TempDir::new();
    let tools = git(&remote("abc123", TWO_SKILLS));
    let err = add(repo.path(), &*tools, "tdd", None).unwrap_err();
    assert!(err.contains("skills.sh"), "{err}");
    assert!(tools.calls().is_empty(), "cloned: {:?}", tools.calls());
}

#[test]
fn add_installs_the_named_skill_of_two_and_records_its_source() {
    let repo = TempDir::new();
    let tools = git(&remote("abc123", TWO_SKILLS));
    assert_eq!(
        add(repo.path(), &*tools, "mattpocock/skills", None),
        Ok(Added::Choose(vec!["code-review".into(), "tdd".into()]))
    );
    assert!(
        !repo.path().join(".agents/skills").exists(),
        "listing the skills installed one"
    );

    assert_eq!(
        add(repo.path(), &*tools, "mattpocock/skills", Some("tdd")),
        Ok(Added::Installed("tdd".into()))
    );
    let via_link = fs::read_to_string(repo.path().join(".claude/skills/tdd/SKILL.md"))
        .expect("tdd not installed through its link");
    assert_eq!(via_link, TDD);
    assert!(repo.path().join(".agents/skills/tdd/tests.md").exists());
    assert!(!repo.path().join(".agents/skills/code-review").exists());
    assert!(!repo.path().join(".agents/skills/README.md").exists());

    let manifest = Manifest::load(repo.path()).unwrap();
    let tdd = &manifest.skills["tdd"];
    assert_eq!(
        (&*tdd.repo, &*tdd.path, &*tdd.commit, tdd.shipped),
        (
            "https://github.com/mattpocock/skills",
            "skills/engineering/tdd",
            "abc123",
            false
        )
    );

    let calls = tools.calls();
    assert!(
        calls[0].starts_with("env GIT_TERMINAL_PROMPT=0 git clone --depth 1 "),
        "not a shallow, promptless clone: {calls:?}"
    );
    let tmp = calls[0].rsplit(' ').next().unwrap();
    assert!(!Path::new(tmp).exists(), "the clone was left in {tmp}");
}

#[test]
fn add_refuses_a_source_already_installed_and_a_same_named_skill_from_another() {
    let repo = TempDir::new();
    let tools = git(&remote("abc123", TWO_SKILLS));
    add(repo.path(), &*tools, "mattpocock/skills", Some("tdd")).unwrap();
    for source in [
        "mattpocock/skills/skills/engineering/tdd",
        "https://github.com/mattpocock/skills/tree/main/skills/engineering/tdd",
    ] {
        let err = add(repo.path(), &*tools, source, None).unwrap_err();
        assert!(err.contains("already installed"), "{source}: {err}");
    }
    let err = add(repo.path(), &*tools, "someone/fork", Some("tdd")).unwrap_err();
    assert!(err.contains("remove it first"), "{err}");
    assert_eq!(
        fs::read_to_string(repo.path().join(".agents/skills/tdd/SKILL.md")).unwrap(),
        TDD
    );
    assert_eq!(
        Manifest::load(repo.path()).unwrap().skills["tdd"].repo,
        "https://github.com/mattpocock/skills"
    );
}

#[test]
fn update_refetches_the_source_and_records_the_new_commit() {
    let repo = TempDir::new();
    let remote = remote("abc123", TWO_SKILLS);
    let tools = git(&remote);
    add(repo.path(), &*tools, "mattpocock/skills", Some("tdd")).unwrap();

    const NEW: &str = "---\nname: tdd\n---\nversion two\n";
    *remote.lock().unwrap() = ("def456", vec![("skills/engineering/tdd/SKILL.md", NEW)]);
    update(repo.path(), &*tools, "tdd").unwrap();
    assert_eq!(
        fs::read_to_string(repo.path().join(".claude/skills/tdd/SKILL.md")).unwrap(),
        NEW
    );
    assert!(
        !repo.path().join(".agents/skills/tdd/tests.md").exists(),
        "a file gone upstream stayed"
    );
    assert_eq!(
        Manifest::load(repo.path()).unwrap().skills["tdd"].commit,
        "def456"
    );

    *remote.lock().unwrap() = ("0a0a0a", vec![("skills/engineering/tdd/SKILL.md", TDD)]);
    assert_eq!(update_all(repo.path(), &*tools), Ok(vec![]));
    assert_eq!(
        Manifest::load(repo.path()).unwrap().skills["tdd"].commit,
        "0a0a0a"
    );
    let err = update(repo.path(), &*tools, "code-review").unwrap_err();
    assert!(err.contains("not installed"), "{err}");
}

#[test]
fn a_failed_save_leaves_update_and_remove_undone() {
    use std::os::unix::fs::PermissionsExt;
    let repo = TempDir::new();
    let remote = remote("abc123", TWO_SKILLS);
    let tools = git(&remote);
    add(repo.path(), &*tools, "mattpocock/skills", Some("tdd")).unwrap();
    let manifest = Manifest::load(repo.path()).unwrap();
    let saved = repo.path().join(".harness/skills.json");
    fs::set_permissions(&saved, fs::Permissions::from_mode(0o444)).unwrap();

    *remote.lock().unwrap() = ("def456", vec![("skills/engineering/tdd/SKILL.md", "new")]);
    update(repo.path(), &*tools, "tdd").unwrap_err();
    remove(repo.path(), "tdd").unwrap_err();
    assert_eq!(
        fs::read_to_string(repo.path().join(".claude/skills/tdd/SKILL.md")).unwrap(),
        TDD
    );
    assert!(repo.path().join(".agents/skills/tdd/tests.md").exists());
    assert_eq!(Manifest::load(repo.path()).unwrap(), manifest);
    let mut left: Vec<_> = fs::read_dir(repo.path().join(".agents/skills"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    left.sort();
    assert_eq!(left, ["tdd"], "a copy set aside stayed");
}

#[test]
fn each_job_takes_its_default_until_a_pick_is_recorded() {
    let mut manifest = Manifest::default();
    for (job, default) in [
        ("test-first", "tdd"),
        ("self-review", "code-review"),
        ("working-mode", "ponytail"),
        ("prose", "caveman"),
        ("review", NONE),
        ("audit", "ponytail-review"),
        ("merge-conflicts", "resolving-merge-conflicts"),
    ] {
        assert_eq!(manifest.pick(job), default, "{job}");
    }
    assert_eq!(JOBS.len(), 7, "a job without a default here");
    manifest
        .picks
        .insert("test-first".into(), "test-driven-development".into());
    manifest.picks.insert("prose".into(), NONE.into());
    assert_eq!(manifest.pick("test-first"), "test-driven-development");
    assert_eq!(manifest.pick("prose"), NONE);
    // Every suggestion to install is a source add takes, the skill's own folder.
    for (job, suggestions) in JOBS {
        for (name, source) in *suggestions {
            if source.is_empty() {
                continue;
            }
            let path = parse_source(source).unwrap().path;
            assert!(path.ends_with(name), "{job}: {name} from {source}");
        }
    }
}

#[test]
fn removing_a_skill_a_job_uses_sets_that_job_to_none() {
    let repo = TempDir::new();
    let tools = git(&remote("abc123", TWO_SKILLS));
    for name in ["tdd", "code-review"] {
        add(repo.path(), &*tools, "mattpocock/skills", Some(name)).unwrap();
    }
    let mut manifest = Manifest::load(repo.path()).unwrap();
    manifest.picks.insert("test-first".into(), "tdd".into()); // self-review takes code-review by default
    manifest.save(repo.path()).unwrap();

    for name in ["tdd", "code-review"] {
        remove(repo.path(), name).unwrap();
        assert!(
            !repo.path().join(".agents/skills").join(name).exists(),
            "{name}: folder stayed"
        );
        assert!(
            fs::symlink_metadata(repo.path().join(".claude/skills").join(name)).is_err(),
            "{name}: link stayed"
        );
    }
    let manifest = Manifest::load(repo.path()).unwrap();
    assert!(manifest.skills.is_empty(), "{:?}", manifest.skills);
    assert_eq!(manifest.pick("test-first"), NONE);
    assert_eq!(manifest.pick("self-review"), NONE);
    assert_eq!(
        manifest.pick("audit"),
        "ponytail-review",
        "a job it did not do changed"
    );
    let err = remove(repo.path(), "tdd").unwrap_err();
    assert!(err.contains("not installed"), "{err}");
}

#[test]
fn a_shipped_skill_refuses_removal() {
    let repo = TempDir::new();
    write_file(
        &repo.path().join(".agents/skills/stage-fix/SKILL.md"),
        "shipped",
    );
    let mut manifest = Manifest::default();
    manifest.skills.insert(
        "stage-fix".into(),
        Installed {
            shipped: true,
            ..Installed::default()
        },
    );
    manifest.save(repo.path()).unwrap();

    let err = remove(repo.path(), "stage-fix").unwrap_err();
    assert!(err.contains("Shipped"), "{err}");
    assert!(repo
        .path()
        .join(".agents/skills/stage-fix/SKILL.md")
        .exists());
    assert_eq!(Manifest::load(repo.path()).unwrap(), manifest);
}

#[test]
fn list_finds_the_repos_the_users_and_the_plugins_skills() {
    let (repo, home, plugins) = (TempDir::new(), TempDir::new(), TempDir::new());
    for dir in [
        repo.path().join(".agents/skills/own"),
        home.path().join(".claude/skills/mine"),
        plugins.path().join("ponytail/skills/ponytail-review"),
        plugins.path().join("off/skills/hidden"),
    ] {
        write_file(&dir.join("SKILL.md"), "---\nname: x\n---\n");
    }
    write_file(&home.path().join(".claude/skills/notes.md"), "not a skill");
    let reply = serde_json::json!([
        {"id": "ponytail@ponytail", "enabled": true, "installPath": plugins.path().join("ponytail")},
        {"id": "off@market", "enabled": false, "installPath": plugins.path().join("off")},
    ])
    .to_string();
    let tools = Fake::new(move |_, argv| match argv.join(" ").as_str() {
        "claude plugin list --json" => Ok(reply.clone()),
        other => Err(format!("unexpected: {other}")),
    });

    let found = list(repo.path(), home.path(), &*tools);
    assert_eq!(
        found,
        [
            ("own".to_string(), repo.path().join(".agents/skills/own")),
            ("mine".to_string(), home.path().join(".claude/skills/mine")),
            (
                "ponytail:ponytail-review".to_string(),
                plugins.path().join("ponytail/skills/ponytail-review")
            ),
        ]
    );

    let no_claude = Fake::new(|_, _| Err("claude: not found".to_string()));
    assert_eq!(list(repo.path(), home.path(), &*no_claude).len(), 2);
}

#[test]
fn a_skill_named_none_or_a_path_is_not_taken() {
    let repo = TempDir::new();
    for text in ["---\nname: none\n---\n", "---\nname: ../escape\n---\n"] {
        let tools = git(&remote("abc123", &[("SKILL.md", text)]));
        let err = add(repo.path(), &*tools, "someone/odd", None).unwrap_err();
        assert!(err.contains("no skill"), "{text:?}: {err}");
    }
    assert!(!repo.path().join(".agents").exists());
}

#[test]
fn an_entry_whose_name_climbs_out_is_neither_removed_nor_updated() {
    let repo = TempDir::new();
    write_file(&repo.path().join("src/main.rs"), "fn main() {}");
    write_file(&repo.path().join(".agents/skills/own/SKILL.md"), "own");
    let tools = git(&remote("abc123", TWO_SKILLS));
    // A hand-edited manifest, with names that leave .agents/skills.
    for name in ["..", "../../src"] {
        let mut manifest = Manifest::default();
        manifest.skills.insert(
            name.into(),
            Installed {
                repo: "https://github.com/mattpocock/skills".into(),
                path: "skills/engineering/tdd".into(),
                ..Installed::default()
            },
        );
        manifest.save(repo.path()).unwrap();
        let err = remove(repo.path(), name).unwrap_err();
        assert!(err.contains("will not touch"), "{name}: {err}");
        let err = update(repo.path(), &*tools, name).unwrap_err();
        assert!(err.contains("will not touch"), "{name}: {err}");
        assert_eq!(update_all(repo.path(), &*tools).unwrap().len(), 1);
        assert!(repo.path().join("src/main.rs").exists(), "{name}");
        assert!(repo.path().join(".agents/skills/own").exists(), "{name}");
        assert_eq!(Manifest::load(repo.path()).unwrap(), manifest);
    }
    assert!(tools.calls().is_empty(), "cloned: {:?}", tools.calls());
}

#[test]
fn an_older_manifests_at_is_ignored() {
    let repo = TempDir::new();
    write_file(&repo.path().join("src/main.rs"), "fn main() {}");
    write_file(&repo.path().join(".agents/skills/tdd/SKILL.md"), TDD);
    write_file(
        &repo.path().join(".harness/skills.json"),
        r#"{"skills": {"tdd": {"repo": "https://github.com/mattpocock/skills", "at": "src"}}}"#,
    );
    remove(repo.path(), "tdd").unwrap();
    assert!(!repo.path().join(".agents/skills/tdd").exists());
    assert!(repo.path().join("src/main.rs").exists());
}

#[test]
fn a_skill_under_a_linked_agents_folder_is_neither_removed_nor_updated() {
    let repo = TempDir::new();
    let tools = git(&remote("abc123", TWO_SKILLS));
    add(repo.path(), &*tools, "mattpocock/skills", Some("tdd")).unwrap();
    // .agents swapped for a link out of the checkout, to a folder the
    // Harness never wrote.
    let outside = TempDir::new();
    write_file(&outside.path().join("skills/tdd/SKILL.md"), "not ours");
    fs::remove_dir_all(repo.path().join(".agents")).unwrap();
    std::os::unix::fs::symlink(outside.path(), repo.path().join(".agents")).unwrap();
    let err = remove(repo.path(), "tdd").unwrap_err();
    assert!(err.contains("will not touch"), "{err}");
    let err = update(repo.path(), &*tools, "tdd").unwrap_err();
    assert!(err.contains("will not touch"), "{err}");
    assert_eq!(
        fs::read_to_string(outside.path().join("skills/tdd/SKILL.md")).unwrap(),
        "not ours"
    );
    assert!(Manifest::load(repo.path())
        .unwrap()
        .skills
        .contains_key("tdd"));
}

#[test]
fn a_skill_under_agents_skills_linked_to_the_checkout_is_neither_removed_nor_updated() {
    let repo = TempDir::new();
    write_file(&repo.path().join("src/main.rs"), "fn main() {}");
    fs::create_dir(repo.path().join(".agents")).unwrap();
    std::os::unix::fs::symlink("..", repo.path().join(".agents/skills")).unwrap();
    let mut manifest = Manifest::default();
    manifest.skills.insert(
        "src".into(),
        Installed {
            repo: "https://github.com/someone/src".into(),
            ..Installed::default()
        },
    );
    manifest.save(repo.path()).unwrap();
    let tools = git(&remote("abc123", &[("SKILL.md", "---\nname: src\n---\n")]));
    let err = remove(repo.path(), "src").unwrap_err();
    assert!(err.contains("will not touch"), "{err}");
    let err = update(repo.path(), &*tools, "src").unwrap_err();
    assert!(err.contains("will not touch"), "{err}");
    assert_eq!(
        fs::read_to_string(repo.path().join("src/main.rs")).unwrap(),
        "fn main() {}"
    );
}

#[test]
fn a_skill_linked_from_a_linked_claude_folder_is_not_removed() {
    let repo = TempDir::new();
    let tools = git(&remote("abc123", TWO_SKILLS));
    add(repo.path(), &*tools, "mattpocock/skills", Some("tdd")).unwrap();
    // .claude swapped for a link out of the checkout, whose skills/tdd is a
    // link the Harness never made.
    let outside = TempDir::new();
    fs::create_dir(outside.path().join("skills")).unwrap();
    std::os::unix::fs::symlink("/elsewhere", outside.path().join("skills/tdd")).unwrap();
    fs::remove_dir_all(repo.path().join(".claude")).unwrap();
    std::os::unix::fs::symlink(outside.path(), repo.path().join(".claude")).unwrap();
    let err = remove(repo.path(), "tdd").unwrap_err();
    assert!(err.contains("will not touch"), "{err}");
    assert!(fs::symlink_metadata(outside.path().join("skills/tdd")).is_ok());
    assert!(repo.path().join(".agents/skills/tdd/SKILL.md").exists());
}

#[test]
fn a_source_folder_that_links_out_of_the_clone_is_not_taken() {
    let repo = TempDir::new();
    add(
        repo.path(),
        &*git(&remote("abc123", TWO_SKILLS)),
        "mattpocock/skills",
        Some("tdd"),
    )
    .unwrap();
    let outside = TempDir::new();
    write_file(&outside.path().join("SKILL.md"), TDD);
    write_file(&outside.path().join("secret"), "not the clone's");
    let out = outside.path().to_path_buf();
    let linked = Fake::new(move |_, argv| {
        if argv.contains(&"clone") {
            let engineering = Path::new(argv.last().unwrap()).join("skills/engineering");
            fs::create_dir_all(&engineering).unwrap();
            std::os::unix::fs::symlink(&out, engineering.join("tdd")).unwrap();
            Ok(String::new())
        } else {
            Ok("def456\n".to_string())
        }
    });
    let err = add(
        repo.path(),
        &*linked,
        "someone/linked/skills/engineering/tdd",
        None,
    )
    .unwrap_err();
    assert!(err.contains("no folder"), "{err}");
    let err = update(repo.path(), &*linked, "tdd").unwrap_err();
    assert!(err.contains("no longer has tdd"), "{err}");
    assert!(!repo.path().join(".agents/skills/tdd/secret").exists());
    assert_eq!(
        fs::read_to_string(repo.path().join(".agents/skills/tdd/SKILL.md")).unwrap(),
        TDD
    );
}

#[test]
fn a_skill_whose_skill_md_is_a_link_is_not_taken() {
    let repo = TempDir::new();
    add(
        repo.path(),
        &*git(&remote("abc123", TWO_SKILLS)),
        "mattpocock/skills",
        Some("tdd"),
    )
    .unwrap();
    // copy_dir copies no links, so this skill would be installed without its
    // SKILL.md.
    let linked = Fake::new(|_, argv| {
        if argv.contains(&"clone") {
            let tdd = Path::new(argv.last().unwrap()).join("skills/engineering/tdd");
            write_file(&tdd.join("real.md"), TDD);
            std::os::unix::fs::symlink("real.md", tdd.join("SKILL.md")).unwrap();
            Ok(String::new())
        } else {
            Ok("def456\n".to_string())
        }
    });
    let err = add(repo.path(), &*linked, "someone/linked", None).unwrap_err();
    assert!(err.contains("no skill"), "{err}");
    let err = update(repo.path(), &*linked, "tdd").unwrap_err();
    assert!(err.contains("no longer has tdd"), "{err}");
    assert_eq!(
        fs::read_to_string(repo.path().join(".agents/skills/tdd/SKILL.md")).unwrap(),
        TDD
    );
}

#[test]
fn add_installs_nothing_through_a_linked_agents_or_claude_folder() {
    for linked in [".agents", ".claude"] {
        let repo = TempDir::new();
        let outside = TempDir::new();
        std::os::unix::fs::symlink(outside.path(), repo.path().join(linked)).unwrap();
        let tools = git(&remote("abc123", TWO_SKILLS));
        let err = add(repo.path(), &*tools, "mattpocock/skills", Some("tdd")).unwrap_err();
        assert!(err.contains("will not install"), "{linked}: {err}");
        assert_eq!(
            fs::read_dir(outside.path()).unwrap().count(),
            0,
            "{linked}: written through"
        );
        assert!(Manifest::load(repo.path()).unwrap().skills.is_empty());
    }
}
