package orchestrator

import (
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func TestStageResultAcceptance(t *testing.T) {
	cases := []struct {
		name, body string
		want       resultRequirements
		reason     string
	}{
		{"done", "STATUS: done\nall good\n", resultRequirements{}, ""},
		{"failed", "STATUS: failed\ntests red\n", resultRequirements{}, "reported STATUS: failed"},
		{"missing file", "", resultRequirements{}, "went idle without a done result"},
		{"status not on first line", "notes\nSTATUS: done\n", resultRequirements{}, "went idle without a done result"},
		{"missing prefix", "done\n", resultRequirements{}, "went idle without a done result"},
		{"unknown status", "STATUS: working\n", resultRequirements{}, "went idle without a done result"},
		{"crlf spacing and case", " STATUS:  DONE \r\n", resultRequirements{}, ""},
		{"incomplete Verdict", "STATUS: done\n- [fix] a.go:1\n", resultRequirements{ReviewFindings: 2}, "Verdict settles 1 of the Review's 2 Findings"},
		{"audit adds Findings", "STATUS: done\n- [fix] a.go:1\n- [skip] b.go:2\n", resultRequirements{ReviewFindings: 1}, ""},
		{"empty clean Verdict", "STATUS: done\n", resultRequirements{ReviewFindings: 0}, ""},
		{"intermediate Fix needs no PR", "STATUS: done\n", resultRequirements{}, ""},
		{"final Fix needs PR", "STATUS: done\n", resultRequirements{RequirePR: true}, "wrote a done result without a 'PR:' line"},
		{"final Fix with PR", "STATUS: done\nPR: https://example.test/pr/7\n", resultRequirements{RequirePR: true}, ""},
		{"failed even with PR", "STATUS: failed\nPR: https://example.test/pr/7\n", resultRequirements{RequirePR: true}, "reported STATUS: failed"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "result.md")
			if c.body != "" {
				writeFile(t, path, c.body)
			}
			result, reason := readStageResult(path, c.want)
			if reason != c.reason {
				t.Errorf("reason = %q, want %q", reason, c.reason)
			}
			if reason != "" && !reflect.DeepEqual(result, stageResult{}) {
				t.Errorf("rejected result exposes content: %+v", result)
			}
		})
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

func TestStageResultInterpretsAcceptedContents(t *testing.T) {
	verdict := `STATUS: done

## Findings

- [fix] (high) orders.go:41 — nil map write | reason: both sides agree | settled: consensus
- [skip] (low) orders.go:12 — naming | reason: style only | settled: consensus
- [FIX] (medium) api.go:7 — missing validation | reason: score 0.81 | settled: typesafe
- [skip] (medium) api.go:90 — cache | reason: TypeSafe unreachable | settled: flagged

Not a finding: - [fix] inside prose is ignored only when it does not start the line.
`
	cases := []struct {
		name, body string
		want       stageResult
	}{
		{"Verdict", verdict, stageResult{Fixes: []string{
			"- [fix] (high) orders.go:41 — nil map write | reason: both sides agree | settled: consensus",
			"- [FIX] (medium) api.go:7 — missing validation | reason: score 0.81 | settled: typesafe",
		}, Skips: 2}},
		{"Review", "STATUS: done\n- (high) a.go:1 — x\n- (low) b.go:2 — y\nprose\n", stageResult{Findings: 2}},
		{"Fix", "STATUS: done\nPR: https://github.com/o/r/pull/7\n", stageResult{PR: "https://github.com/o/r/pull/7"}},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), "result.md")
			writeFile(t, path, c.body)
			got, reason := readStageResult(path, resultRequirements{})
			if reason != "" || !reflect.DeepEqual(got, c.want) {
				t.Errorf("readStageResult = %+v, %q; want %+v, accepted", got, reason, c.want)
			}
		})
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
