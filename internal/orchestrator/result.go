package orchestrator

import (
	"fmt"
	"os"
	"regexp"
	"strings"
)

// stageResult is the accepted content of a Stage result, interpreted once for
// the Pipeline. Session liveness is a separate part of Stage completion.
type stageResult struct {
	Findings int
	Fixes    []string
	Skips    int
	PR       string
}

// resultRequirements supplies the Pipeline context needed to accept a result.
// The zero value requires only STATUS: done.
type resultRequirements struct {
	ReviewFindings int  // a Verdict must settle at least this many Findings
	RequirePR      bool // the final Fix must identify its opened PR
}

// readStageResult interprets and accepts result contents for live completion,
// resume, and late completion alike. A nonempty reason means the result is not
// accepted; the caller decides whether to start a session, Wake, or keep waiting.
func readStageResult(path string, want resultRequirements) (stageResult, string) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return stageResult{}, "went idle without a done result"
	}
	body := string(raw)
	first, _, _ := strings.Cut(body, "\n")
	value, ok := strings.CutPrefix(strings.TrimSpace(first), "STATUS:")
	if !ok {
		return stageResult{}, "went idle without a done result"
	}
	switch strings.ToLower(strings.TrimSpace(value)) {
	case "failed":
		return stageResult{}, "reported STATUS: failed"
	case "done":
	default:
		return stageResult{}, "went idle without a done result"
	}

	result := stageResult{Findings: len(findingItem.FindAllString(body, -1))}
	for _, m := range verdictItem.FindAllStringSubmatch(body, -1) {
		if strings.EqualFold(m[1], "fix") {
			result.Fixes = append(result.Fixes, strings.TrimSpace(m[0]))
		} else {
			result.Skips++
		}
	}
	if m := prLine.FindStringSubmatch(body); m != nil {
		result.PR = m[1]
	}
	// The Moderator can add audit Findings, so more settled items are valid.
	if settled := len(result.Fixes) + result.Skips; settled < want.ReviewFindings {
		return stageResult{}, fmt.Sprintf("Verdict settles %d of the Review's %d Findings", settled, want.ReviewFindings)
	}
	if want.RequirePR && result.PR == "" {
		return stageResult{}, "wrote a done result without a 'PR:' line"
	}
	return result, ""
}

var (
	verdictItem = regexp.MustCompile(`(?mi)^- \[(fix|skip)\].*`)
	findingItem = regexp.MustCompile(`(?m)^- \(`)
	prLine      = regexp.MustCompile(`(?m)^PR:\s*(\S+)`)
)

// stagePrompt is the text a Stage's session is prompted with: the Stage skill's
// body followed by this run's inputs. The text is passed whole because Codex
// does not load Claude skills and a worktree may not contain them.
func stagePrompt(skill string, inputs [][2]string) string {
	if rest, ok := strings.CutPrefix(skill, "---\n"); ok {
		if _, body, found := strings.Cut(rest, "\n---\n"); found {
			skill = body
		}
	}
	var b strings.Builder
	b.WriteString(strings.TrimSpace(skill))
	b.WriteString("\n\n## Inputs\n\n")
	for _, in := range inputs {
		fmt.Fprintf(&b, "- %s: %s\n", in[0], in[1])
	}
	return b.String()
}
