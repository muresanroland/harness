use super::herdr::{location, PaneInfo, TabInfo};
use super::result::{
    read_question, read_stage_result, stage_prompt, ResultRequirements, StageResult,
};
use super::write_file;
use crate::tempdir::TempDir;

#[test]
fn stage_result_acceptance() {
    let none = ResultRequirements::default();
    let findings = |n| ResultRequirements {
        review_findings: n,
        ..Default::default()
    };
    let pr = ResultRequirements {
        require_pr: true,
        ..Default::default()
    };
    let cases: [(&str, &str, ResultRequirements, &str); 16] = [
        ("done", "STATUS: done\nall good\n", none, ""),
        (
            "failed",
            "STATUS: failed\ntests red\n",
            none,
            "session reported failure",
        ),
        ("missing file", "", none, "went idle without a result"),
        (
            "status not on first line",
            "notes\nSTATUS: done\n",
            none,
            "wrote a result file whose first line is not STATUS:",
        ),
        (
            "missing prefix",
            "done\n",
            none,
            "wrote a result file whose first line is not STATUS:",
        ),
        (
            "unknown status",
            "STATUS: working\n",
            none,
            "wrote a result file whose first line is not STATUS:",
        ),
        ("crlf spacing and case", " STATUS:  DONE \r\n", none, ""),
        (
            "incomplete Verdict",
            "STATUS: done\n- [fix] a.go:1\n",
            findings(2),
            "Verdict settles 1 of the Review's 2 Findings",
        ),
        (
            "audit adds Findings",
            "STATUS: done\n- [fix] a.go:1\n- [skip] b.go:2\n",
            findings(1),
            "",
        ),
        ("empty clean Verdict", "STATUS: done\n", findings(0), ""),
        ("intermediate Fix needs no PR", "STATUS: done\n", none, ""),
        (
            "final Fix needs PR",
            "STATUS: done\n",
            pr,
            "finished without a PR link",
        ),
        (
            "final Fix with PR",
            "STATUS: done\nPR: https://example.test/pr/7\n",
            pr,
            "",
        ),
        (
            "a question is not done",
            "STATUS: question\nours or theirs?\n- ours\n",
            none,
            "asked a question",
        ),
        (
            "a plan is not done",
            "STATUS: plan\n",
            none,
            "wrote its plan",
        ),
        (
            "failed even with PR",
            "STATUS: failed\nPR: https://example.test/pr/7\n",
            pr,
            "session reported failure",
        ),
    ];
    for (name, body, want, reason) in cases {
        let dir = TempDir::new();
        let path = dir.path().join("result.md");
        if !body.is_empty() {
            write_file(&path, body);
        }
        let (result, got) = read_stage_result(&path, want);
        assert_eq!(got, reason, "{name}: reason");
        if !got.is_empty() {
            assert_eq!(
                result,
                StageResult::default(),
                "{name}: rejected result exposes content"
            );
        }
    }
}

#[test]
fn a_question_is_its_text_and_its_dash_options() {
    let dir = TempDir::new();
    let path = dir.path().join("result.md");
    assert_eq!(read_question(&path), None, "a missing file asks nothing");
    write_file(
        &path,
        "STATUS: Question\r\n\nBoth sides changed parse().\nWhich one stays?\n\n- ours: keep the new parser\n-  theirs\n",
    );
    assert_eq!(
        read_question(&path),
        Some((
            "Both sides changed parse().\nWhich one stays?".to_string(),
            vec![
                "ours: keep the new parser".to_string(),
                "theirs".to_string()
            ]
        ))
    );
    // Only the last block of "- " lines are options: a hunk in the
    // question keeps its removed lines, indentation and blank lines.
    write_file(
        &path,
        "STATUS: question\n\nBoth sides changed:\n\n-    let x = 1;\n+    let x = 2;\n- a note\n\nWhich stays?\n- ours\n- theirs\n\n",
    );
    assert_eq!(
        read_question(&path),
        Some((
            "Both sides changed:\n\n-    let x = 1;\n+    let x = 2;\n- a note\n\nWhich stays?"
                .to_string(),
            vec!["ours".to_string(), "theirs".to_string()]
        ))
    );
    write_file(&path, "STATUS: done\n- ours\n");
    assert_eq!(read_question(&path), None, "a done result asks nothing");
}

#[test]
fn location_is_tab_and_pane_order_not_ids() {
    let tab = |id: &str| TabInfo {
        tab_id: id.to_string(),
    };
    let pane = |id: &str, tab: &str| PaneInfo {
        pane_id: id.to_string(),
        tab_id: tab.to_string(),
    };
    let tabs = [tab("wD:t1"), tab("wD:t9"), tab("wD:t4")];
    let panes = [
        pane("wD:p1", "wD:t1"),
        pane("wD:p7", "wD:t9"),
        pane("wD:p3", "wD:t1"),
        pane("wD:p12", "wD:t9"),
        pane("wD:p8", "wD:t9"),
    ];
    for (pane, want) in [
        ("wD:p1", "1-1"),
        ("wD:p3", "1-2"),
        ("wD:p7", "2-1"),
        ("wD:p8", "2-3"),
        ("wD:gone", "?"),
    ] {
        assert_eq!(location(&tabs, &panes, pane), want, "location({pane})");
    }
}

#[test]
fn stage_result_interprets_accepted_contents() {
    let verdict = "STATUS: done

## Findings

- [fix] (high) orders.go:41 — nil map write | reason: both sides agree | settled: consensus
- [skip] (low) orders.go:12 — naming | reason: style only | settled: consensus
- [FIX] (medium) api.go:7 — missing validation | reason: score 0.81 | settled: typesafe
- [skip] (medium) api.go:90 — cache | reason: TypeSafe unreachable | settled: flagged

Not a finding: - [fix] inside prose is ignored only when it does not start the line.
";
    let cases = [
        (
            "Verdict",
            verdict,
            StageResult {
                fixes: vec![
                    "- [fix] (high) orders.go:41 — nil map write | reason: both sides agree | settled: consensus".to_string(),
                    "- [FIX] (medium) api.go:7 — missing validation | reason: score 0.81 | settled: typesafe".to_string(),
                ],
                skips: vec![
                    "- [skip] (low) orders.go:12 — naming | reason: style only | settled: consensus".to_string(),
                    "- [skip] (medium) api.go:90 — cache | reason: TypeSafe unreachable | settled: flagged".to_string(),
                ],
                ..Default::default()
            },
        ),
        (
            "Review",
            "STATUS: done\n- (high) a.go:1 — x\n- (low) b.go:2 — y\nprose\n",
            StageResult {
                findings: 2,
                ..Default::default()
            },
        ),
        (
            "Fix",
            "STATUS: done\nPR: https://github.com/o/r/pull/7\n",
            StageResult {
                pr: "https://github.com/o/r/pull/7".to_string(),
                ..Default::default()
            },
        ),
        (
            "Review skipped, its App Limited",
            "STATUS: done\nUNREVIEWED: codex was limited until 3:05pm\n",
            StageResult {
                unreviewed: "codex was limited until 3:05pm".to_string(),
                ..Default::default()
            },
        ),
        (
            "Fix with the PR after a blank line",
            "STATUS: done\nPR:\n\nhttps://github.com/o/r/pull/7\n",
            StageResult {
                pr: "https://github.com/o/r/pull/7".to_string(),
                ..Default::default()
            },
        ),
    ];
    for (name, body, want) in cases {
        let dir = TempDir::new();
        let path = dir.path().join("result.md");
        write_file(&path, body);
        let (got, reason) = read_stage_result(&path, ResultRequirements::default());
        assert!(
            reason.is_empty() && got == want,
            "{name}: read_stage_result = {got:?}, {reason:?}; want {want:?}, accepted"
        );
    }
}

#[test]
fn stage_prompt_is_skill_body_plus_inputs() {
    let skill = "---\nname: stage-review\ndescription: x\n---\n\nReview the branch.\n";
    let got = stage_prompt(
        skill,
        &[("Ticket", "hx-1"), ("Result file", "/r/review-1.md")],
    );
    assert!(
        !got.contains("name: stage-review"),
        "frontmatter leaked into the prompt:\n{got}"
    );
    for want in [
        "Review the branch.",
        "- Ticket: hx-1",
        "- Result file: /r/review-1.md",
    ] {
        assert!(got.contains(want), "prompt lacks {want:?}:\n{got}");
    }
}
