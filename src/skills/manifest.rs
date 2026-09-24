//! The Skill manifest: the skills the Harness installed for one checkout, and
//! each job's pick. Also fetching a third-party skill with git, and listing
//! the skills the user already has.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::tools::Tools;

const MANIFEST: &str = ".harness/skills.json";

/// The pick that opts a job out: its Stage skill follows its own instructions.
pub(crate) const NONE: &str = "none";

/// Each job a Delegate skill can do, with its suggestions, the default first:
/// (skill name, source). An empty source is built into the App: nothing to
/// install.
pub(crate) const JOBS: &[(&str, &[(&str, &str)])] = &[
    (
        "test-first",
        &[
            ("tdd", "mattpocock/skills/skills/engineering/tdd"),
            (
                "test-driven-development",
                "obra/superpowers/skills/test-driven-development",
            ),
            (
                "test-driven-development",
                "addyosmani/agent-skills/skills/test-driven-development",
            ),
        ],
    ),
    (
        "self-review",
        &[
            (
                "code-review",
                "mattpocock/skills/skills/engineering/code-review",
            ),
            (
                "requesting-code-review",
                "obra/superpowers/skills/requesting-code-review",
            ),
        ],
    ),
    (
        "working-mode",
        &[
            ("ponytail", "DietrichGebert/ponytail/skills/ponytail"),
            (
                "karpathy-guidelines",
                "multica-ai/andrej-karpathy-skills/skills/karpathy-guidelines",
            ),
        ],
    ),
    (
        "prose",
        &[
            ("caveman", "JuliusBrussee/caveman/skills/caveman"),
            (
                "caveman-commit",
                "JuliusBrussee/caveman/skills/caveman-commit",
            ),
        ],
    ),
    (
        "review",
        &[
            (NONE, ""),
            ("review-agent", ""), // Codex's own
            (
                "requesting-code-review",
                "obra/superpowers/skills/requesting-code-review",
            ),
        ],
    ),
    (
        "audit",
        &[(
            "ponytail-review",
            "DietrichGebert/ponytail/skills/ponytail-review",
        )],
    ),
    (
        "merge-conflicts",
        &[
            (
                "resolving-merge-conflicts",
                "mattpocock/skills/skills/engineering/resolving-merge-conflicts",
            ),
            (
                "resolve-merge-conflicts",
                "warpdotdev/common-skills/.agents/skills/resolve-merge-conflicts",
            ),
        ],
    ),
];

/// One skill the Harness installed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Installed {
    /// The clone URL; empty for a Shipped skill.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) repo: String,
    #[serde(rename = "ref", skip_serializing_if = "String::is_empty")]
    pub(crate) git_ref: String,
    /// The skill's folder in the repo.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) commit: String,
    /// Where it was put, from the checkout's root.
    pub(crate) at: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) shipped: bool,
}

/// The Skill manifest, kept in .harness/ beside init's record of the text it
/// wrote.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Manifest {
    pub(crate) skills: BTreeMap<String, Installed>,
    /// Job to its pick: a skill name or "none". A job left out takes its
    /// default.
    pub(crate) picks: BTreeMap<String, String>,
}

impl Manifest {
    /// A checkout without one has an empty manifest. A garbled one is an
    /// error, so that saving over it cannot lose what it held.
    pub(crate) fn load(repo: &Path) -> io::Result<Self> {
        match fs::read_to_string(repo.join(MANIFEST)) {
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err),
        }
    }

    /// The job's pick: the one recorded, else the job's default.
    pub(crate) fn pick(&self, job: &str) -> &str {
        match self.picks.get(job) {
            Some(pick) => pick,
            None => JOBS
                .iter()
                .find(|(name, _)| *name == job)
                .map_or(NONE, |(_, suggestions)| suggestions[0].0),
        }
    }

    pub(crate) fn save(&self, repo: &Path) -> io::Result<()> {
        fs::create_dir_all(repo.join(".harness"))?;
        fs::write(
            repo.join(MANIFEST),
            serde_json::to_string_pretty(self)? + "\n",
        )
    }
}

/// The manifest for an operation, whose errors are messages for the Shell.
fn open(repo: &Path) -> Result<Manifest, String> {
    Manifest::load(repo).map_err(|err| format!("{MANIFEST}: {err}"))
}

fn store(repo: &Path, manifest: &Manifest) -> Result<(), String> {
    manifest
        .save(repo)
        .map_err(|err| format!("{MANIFEST}: {err}"))
}

/// Where a skill comes from: a clone URL, the ref to clone (empty for the
/// default branch) and the skill's folder in the clone (empty to search it).
#[derive(Debug, PartialEq)]
pub(crate) struct Source {
    pub(crate) repo: String,
    pub(crate) git_ref: String,
    pub(crate) path: String,
}

/// Reads a source as the user gives it: owner/repo, owner/repo/path, a
/// GitHub URL (with /tree/<ref>/<path> or not), or any other git URL. A bare
/// name is refused: it needs a source, and skills.sh finds one.
pub(crate) fn parse_source(text: &str) -> Result<Source, String> {
    let text = text.trim();
    if !text.contains('/') && !text.contains(':') {
        return Err(format!("'{text}' is a skill name, not a source: find its source on skills.sh (https://skills.sh/?q={text}), then add it as owner/repo or its URL"));
    }
    let github = ["https://github.com/", "http://github.com/", "github.com/"]
        .iter()
        .find_map(|prefix| text.strip_prefix(prefix));
    let Some(rest) = github.or_else(|| (!text.contains(':')).then_some(text)) else {
        // Any other git URL, cloned as it is.
        return Ok(Source {
            repo: text.to_string(),
            git_ref: String::new(),
            path: String::new(),
        });
    };
    let parts: Vec<&str> = rest.split('/').filter(|part| !part.is_empty()).collect();
    let [owner, name, path @ ..] = parts.as_slice() else {
        return Err(format!(
            "'{text}' is not a source: give owner/repo or a git URL"
        ));
    };
    // ponytail: the ref is one segment, so a branch named with a slash reads
    // as ref plus path; take owner/repo@... if one is ever needed.
    let (git_ref, path) = match path {
        ["tree", git_ref, path @ ..] if github.is_some() => (*git_ref, path),
        _ => ("", path),
    };
    if path.contains(&"..") {
        return Err(format!("'{text}': a path cannot leave its repo"));
    }
    Ok(Source {
        repo: format!(
            "https://github.com/{owner}/{}",
            name.trim_end_matches(".git")
        ),
        git_ref: git_ref.to_string(),
        path: path.join("/"),
    })
}

/// What add did.
#[derive(Debug, PartialEq)]
pub(crate) enum Added {
    Installed(String),
    /// The source holds several skills and none was named: their names, for
    /// the caller to pick from.
    Choose(Vec<String>),
}

/// Installs one skill from a source: the one named, or the only one there.
/// The skill is copied to .agents/skills/<name>, linked from .claude/skills
/// as init does, and recorded with its source and commit. A source already
/// installed is refused, and so is a same-named skill from anywhere else.
pub(crate) fn add(
    repo: &Path,
    tools: &dyn Tools,
    source: &str,
    name: Option<&str>,
) -> Result<Added, String> {
    let source = parse_source(source)?;
    let mut manifest = open(repo)?;
    fetch(
        repo,
        tools,
        &source.repo,
        &source.git_ref,
        |clone, commit| {
            let mut skills = Vec::new();
            find(clone, &clone.join(&source.path), &mut skills);
            skills.sort();
            let (name, path) = match (name, skills.len()) {
                (Some(name), _) => match skills.iter().position(|(found, _)| found == name) {
                    Some(i) => skills.swap_remove(i),
                    None => {
                        let names: Vec<_> =
                            skills.iter().map(|(found, _)| found.as_str()).collect();
                        return Err(format!(
                            "no skill named {name} in {}: it has {}",
                            source.repo,
                            names.join(", ")
                        ));
                    }
                },
                (None, 0) => return Err(format!("no skill in {}", source.repo)),
                (None, 1) => skills.remove(0),
                (None, _) => {
                    return Ok(Added::Choose(
                        skills.into_iter().map(|(found, _)| found).collect(),
                    ))
                }
            };
            if let Some((other, _)) = manifest
                .skills
                .iter()
                .find(|(_, skill)| skill.repo == source.repo && skill.path == path)
            {
                return Err(format!(
                    "{}/{path} is already installed, as {other}: update it instead",
                    source.repo
                ));
            }
            match manifest.skills.get(&name) {
                Some(skill) if skill.shipped => {
                    return Err(format!("{name} is a Shipped skill: it cannot be replaced"))
                }
                Some(skill) => {
                    return Err(format!(
                        "a skill named {name} from {} is installed: remove it first",
                        skill.repo
                    ))
                }
                None => {}
            }
            let at = format!(".agents/skills/{name}");
            let link = Path::new(".claude/skills").join(&name);
            if [Path::new(&at), &link]
                .iter()
                .any(|place| fs::symlink_metadata(repo.join(place)).is_ok())
            {
                let link = link.display();
                return Err(format!(
                    "{at} or {link} is there already, and the Harness did not put it there"
                ));
            }
            let installed = put(repo, &clone.join(&path), &at, &link)
                .map_err(|err| format!("{at}: {err}"))
                .and_then(|()| {
                    let skill = Installed {
                        repo: source.repo.clone(),
                        git_ref: source.git_ref.clone(),
                        path,
                        commit,
                        at: at.clone(),
                        shipped: false,
                    };
                    manifest.skills.insert(name.clone(), skill);
                    store(repo, &manifest)
                });
            if installed.is_err() {
                // Nothing left half installed, which a later add would take
                // for the repo's own.
                let _ = fs::remove_file(repo.join(&link));
                let _ = fs::remove_dir_all(repo.join(&at));
            }
            installed.map(|()| Added::Installed(name))
        },
    )
}

/// Fetches an installed skill's source again, replaces its folder with the
/// skill at the recorded path, and records the new commit.
pub(crate) fn update(repo: &Path, tools: &dyn Tools, name: &str) -> Result<(), String> {
    let mut manifest = open(repo)?;
    let skill = third_party(&manifest, name)?.clone();
    fetch(repo, tools, &skill.repo, &skill.git_ref, |clone, commit| {
        let from = clone.join(&skill.path);
        let text = fs::read_to_string(from.join("SKILL.md")).unwrap_or_default();
        if skill_name(&text).as_deref() != Some(name) {
            return Err(format!(
                "{} no longer has {name} at {}",
                skill.repo, skill.path
            ));
        }
        // The new copy goes beside the old one first, so that a copy that
        // fails leaves the installed skill whole.
        let at = repo.join(&skill.at);
        let fresh = at.with_file_name(format!(".{name}.new"));
        let _ = fs::remove_dir_all(&fresh);
        let swapped = copy_dir(&from, &fresh).and_then(|()| {
            if at.exists() {
                fs::remove_dir_all(&at)?;
            }
            fs::rename(&fresh, &at)
        });
        if let Err(err) = swapped {
            let _ = fs::remove_dir_all(&fresh);
            return Err(format!("{}: {err}", skill.at));
        }
        if let Some(installed) = manifest.skills.get_mut(name) {
            installed.commit = commit;
        }
        store(repo, &manifest)
    })
}

/// Updates every installed skill but the Shipped ones, and returns those
/// that failed, each with why.
pub(crate) fn update_all(repo: &Path, tools: &dyn Tools) -> Result<Vec<(String, String)>, String> {
    let names: Vec<String> = open(repo)?
        .skills
        .into_iter()
        .filter(|(_, skill)| !skill.shipped)
        .map(|(name, _)| name)
        .collect();
    Ok(names
        .into_iter()
        .filter_map(|name| update(repo, tools, &name).err().map(|err| (name, err)))
        .collect())
}

/// Removes a skill the Harness fetched: its folder, its link and its entry.
/// A job it did, picked or by default, is set to none.
pub(crate) fn remove(repo: &Path, name: &str) -> Result<(), String> {
    let mut manifest = open(repo)?;
    let at = third_party(&manifest, name)?.at.clone();
    let link = repo.join(".claude/skills").join(name);
    if fs::symlink_metadata(&link).is_ok_and(|meta| meta.file_type().is_symlink()) {
        fs::remove_file(&link).map_err(|err| format!("{}: {err}", link.display()))?;
    }
    if repo.join(&at).exists() {
        fs::remove_dir_all(repo.join(&at)).map_err(|err| format!("{at}: {err}"))?;
    }
    for (job, _) in JOBS {
        if manifest.pick(job) == name {
            manifest.picks.insert(job.to_string(), NONE.to_string());
        }
    }
    manifest.skills.remove(name);
    store(repo, &manifest)
}

/// An installed skill the Harness fetched from a source, which update and
/// remove may touch.
fn third_party<'a>(manifest: &'a Manifest, name: &str) -> Result<&'a Installed, String> {
    match manifest.skills.get(name) {
        None => Err(format!("{name} is not installed by the Harness")),
        Some(skill) if skill.shipped => Err(format!(
            "{name} is a Shipped skill: it cannot be removed, and harness init updates it"
        )),
        Some(skill) => Ok(skill),
    }
}

/// Shallow-clones url at git_ref (the default branch when empty) into a
/// scratch directory, hands it and its commit to f, and removes it whatever
/// f returns. No terminal prompt: a mistyped GitHub repo asks for a password,
/// which would hang the Shell.
fn fetch<T>(
    repo: &Path,
    tools: &dyn Tools,
    url: &str,
    git_ref: &str,
    f: impl FnOnce(&Path, String) -> Result<T, String>,
) -> Result<T, String> {
    static N: AtomicU64 = AtomicU64::new(0);
    let clone = std::env::temp_dir().join(format!(
        "harness-skill-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&clone);
    let dest = clone.to_string_lossy();
    let mut argv = vec![
        "env",
        "GIT_TERMINAL_PROMPT=0",
        "git",
        "clone",
        "--depth",
        "1",
        "--quiet",
    ];
    if !git_ref.is_empty() {
        argv.extend(["--branch", git_ref]);
    }
    argv.extend(["--", url, &dest]);
    let result = tools
        .run(repo, &argv)
        .and_then(|_| tools.run(&clone, &["git", "rev-parse", "HEAD"]))
        .map_err(|err| err.to_string())
        .and_then(|commit| f(&clone, commit.trim().to_string()));
    let _ = fs::remove_dir_all(&clone);
    result
}

/// Every skill under dir, as (name, folder from the clone's root).
fn find(clone: &Path, dir: &Path, into: &mut Vec<(String, String)>) {
    let skill = fs::read_to_string(dir.join("SKILL.md")).unwrap_or_default();
    if let Some(name) = skill_name(&skill) {
        let path = dir.strip_prefix(clone).unwrap_or(dir);
        into.push((name, path.to_string_lossy().into_owned()));
    }
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) && entry.file_name() != ".git" {
            find(clone, &entry.path(), into);
        }
    }
}

/// The name in a SKILL.md's frontmatter, when it is safe as a folder name (it
/// comes from someone else's repo) and cannot be read as the pick none.
fn skill_name(skill: &str) -> Option<String> {
    let mut lines = skill.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let name = lines
        .take_while(|line| line.trim() != "---")
        .find_map(|line| line.strip_prefix("name:"))?
        .trim()
        .trim_matches(['"', '\'']);
    let safe = !name.is_empty()
        && !name.starts_with('.')
        && name != NONE
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    safe.then(|| name.to_string())
}

/// Copies the skill's folder to at and links it from link.
fn put(repo: &Path, from: &Path, at: &str, link: &Path) -> io::Result<()> {
    copy_dir(from, &repo.join(at))?;
    fs::create_dir_all(repo.join(".claude/skills"))?;
    symlink(Path::new("../..").join(at), repo.join(link))
}

/// Copies files and folders only, so that a link in the clone cannot pull in
/// anything from outside it.
fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let dest = to.join(entry.file_name());
        if kind.is_dir() && entry.file_name() != ".git" {
            copy_dir(&entry.path(), &dest)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

/// A Claude Code plugin, as claude plugin list --json gives it.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Plugin {
    id: String,
    enabled: bool,
    install_path: PathBuf,
}

/// Every skill the user has, each with where it is: the repo's (the
/// Harness's installs included), the user's, and the enabled Claude Code
/// plugins', named plugin:skill. A skill is named by its folder, as the
/// agents name it. A skill linked from .claude/skills to .agents/skills is
/// listed at both places: Claude reads the one, codex the other. Without
/// claude there are no plugin skills.
// ponytail: a plugin's skills are read from its skills/ folder only, not a
// skills list in its plugin.json; read that when a plugin needs it.
pub(crate) fn list(repo: &Path, home: &Path, tools: &dyn Tools) -> Vec<(String, PathBuf)> {
    let plugins: Vec<Plugin> = tools
        .run(repo, &["claude", "plugin", "list", "--json"])
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    let dirs = [
        repo.join(".agents/skills"),
        repo.join(".claude/skills"),
        home.join(".claude/skills"),
        home.join(".agents/skills"),
    ]
    .map(|dir| (String::new(), dir));
    let plugin_dirs = plugins
        .into_iter()
        .filter(|plugin| plugin.enabled)
        .map(|plugin| {
            let name = plugin.id.split('@').next().unwrap_or_default();
            (format!("{name}:"), plugin.install_path.join("skills"))
        });
    let mut found = Vec::new();
    for (prefix, dir) in dirs.into_iter().chain(plugin_dirs) {
        let mut skills: Vec<PathBuf> = fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.join("SKILL.md").is_file())
            .collect();
        skills.sort();
        for path in skills {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            found.push((format!("{prefix}{name}"), path));
        }
    }
    found
}
