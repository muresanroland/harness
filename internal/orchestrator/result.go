package orchestrator

import (
	"fmt"
	"os"
	"regexp"
	"strings"
)

// resultStatus returns the STATUS on the first line of a Stage's result file
// ("done", "failed", or "" when the file or the line is missing) and the body.
func resultStatus(path string) (status, body string) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return "", ""
	}
	body = string(raw)
	first, _, _ := strings.Cut(body, "\n")
	value, ok := strings.CutPrefix(strings.TrimSpace(first), "STATUS:")
	if !ok {
		return "", body
	}
	return strings.ToLower(strings.TrimSpace(value)), body
}

var (
	verdictItem = regexp.MustCompile(`(?mi)^- \[(fix|skip)\]`)
	findingItem = regexp.MustCompile(`(?m)^- \(`)
	prLine      = regexp.MustCompile(`(?m)^PR:\s*(\S+)`)
)

// verdictCounts counts a Verdict's "- [fix]" and "- [skip]" items.
func verdictCounts(verdict string) (fix, skip int) {
	for _, m := range verdictItem.FindAllStringSubmatch(verdict, -1) {
		if strings.EqualFold(m[1], "fix") {
			fix++
		} else {
			skip++
		}
	}
	return fix, skip
}

// fixItems are a Verdict's "- [fix]" lines, all the Fix Stage is given.
func fixItems(verdict string) []string {
	var items []string
	for _, line := range strings.Split(verdict, "\n") {
		if m := verdictItem.FindStringSubmatch(line); m != nil && strings.EqualFold(m[1], "fix") {
			items = append(items, strings.TrimSpace(line))
		}
	}
	return items
}

// findingCount counts a Review's "- (severity) location — problem" items.
func findingCount(review string) int {
	return len(findingItem.FindAllString(review, -1))
}

func prURL(fixResult string) string {
	if m := prLine.FindStringSubmatch(fixResult); m != nil {
		return m[1]
	}
	return ""
}

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
