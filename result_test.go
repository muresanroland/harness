package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestCompletionRuleReadsFirstLineOfResultFile(t *testing.T) {
	dir := t.TempDir()
	write := func(name, body string) string {
		path := filepath.Join(dir, name)
		os.WriteFile(path, []byte(body), 0o644)
		return path
	}
	cases := []struct {
		name, path, want string
	}{
		{"done", write("done.md", "STATUS: done\nall good\n"), "done"},
		{"failed", write("failed.md", "STATUS: failed\ntests red\n"), "failed"},
		{"missing file", filepath.Join(dir, "nope.md"), ""},
		{"status not on first line", write("late.md", "notes\nSTATUS: done\n"), ""},
		{"crlf and spacing", write("crlf.md", "STATUS:  done \r\n"), "done"},
	}
	for _, c := range cases {
		if got, _ := resultStatus(c.path); got != c.want {
			t.Errorf("%s: status = %q, want %q", c.name, got, c.want)
		}
	}
}

func TestLocationIsTabAndPaneOrderNotIDs(t *testing.T) {
	tabs := []tabInfo{{TabID: "wD:t1"}, {TabID: "wD:t9"}, {TabID: "wD:t4"}}
	panes := []paneInfo{
		{PaneID: "wD:p1", TabID: "wD:t1"},
		{PaneID: "wD:p7", TabID: "wD:t9"},
		{PaneID: "wD:p3", TabID: "wD:t1"},
		{PaneID: "wD:p12", TabID: "wD:t9"},
		{PaneID: "wD:p8", TabID: "wD:t9"},
	}
	for pane, want := range map[string]string{"wD:p1": "1-1", "wD:p3": "1-2", "wD:p7": "2-1", "wD:p8": "2-3", "wD:gone": "?"} {
		if got := location(tabs, panes, pane); got != want {
			t.Errorf("location(%s) = %q, want %q", pane, got, want)
		}
	}
}

func TestVerdictCountsFixAndSkipItems(t *testing.T) {
	verdict := `STATUS: done

## Findings

- [fix] (high) orders.go:41 — nil map write | reason: both sides agree | settled: consensus
- [skip] (low) orders.go:12 — naming | reason: style only | settled: consensus
- [FIX] (medium) api.go:7 — missing validation | reason: score 0.81 | settled: typesafe
- [skip] (medium) api.go:90 — cache | reason: TypeSafe unreachable | settled: flagged

Not a finding: - [fix] inside prose is ignored only when it does not start the line.
`
	fix, skip := verdictCounts(verdict)
	if fix != 2 || skip != 2 {
		t.Errorf("verdictCounts = %d fix, %d skip; want 2, 2", fix, skip)
	}
	if n := findingCount("STATUS: done\n- (high) a.go:1 — x\n- (low) b.go:2 — y\nprose\n"); n != 2 {
		t.Errorf("findingCount = %d, want 2", n)
	}
	if got := prURL("STATUS: done\nPR: https://github.com/o/r/pull/7\n"); got != "https://github.com/o/r/pull/7" {
		t.Errorf("prURL = %q", got)
	}
}

func TestStagePromptIsSkillBodyPlusInputs(t *testing.T) {
	skill := "---\nname: stage-review\ndescription: x\n---\n\nReview the branch.\n"
	got := stagePrompt(skill, [][2]string{{"Ticket", "hx-1"}, {"Result file", "/r/review-1.md"}})
	if strings.Contains(got, "name: stage-review") {
		t.Errorf("frontmatter leaked into the prompt:\n%s", got)
	}
	for _, want := range []string{"Review the branch.", "- Ticket: hx-1", "- Result file: /r/review-1.md"} {
		if !strings.Contains(got, want) {
			t.Errorf("prompt lacks %q:\n%s", want, got)
		}
	}
}
