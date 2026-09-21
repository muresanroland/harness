package orchestrator

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"harness/internal/runner/runnertest"
	"harness/internal/setup"
)

// world fakes everything behind the Runner seam: a herdr session, a bd
// workspace and gh. Sessions "work" synchronously inside 'agent prompt': the
// session hook decides what result file a Stage's session leaves behind.
type world struct {
	*runnertest.Fake
	t    *testing.T
	repo string

	mu          sync.Mutex
	nextID      int
	tabs        []string
	panes       []paneInfo
	agents      map[string]string // pane id -> herdr agent status
	rect        [2]int            // width, height in cells of every pane; zero means roomy and square
	main        []string          // lines the Main session received
	tickets     []*bdTicket
	prs         map[string]string // PR url -> gh JSON
	mainBlocked bool              // the Main session refuses prompts: agent_blocked
	mainNoAgent bool              // the launching pane is a shell: agent_not_found
	failing     map[string]error  // command prefix -> the error its next call fails with
	merged      bool              // every PR is merged as soon as gh is asked about it
	live        int               // Tickets between worktree creation and tab close
	peak        int

	// session plays one Stage session: it returns the result file's content
	// ("" writes nothing) and the agent status the session settles in.
	session func(p prompt) (result, status string)
	waitErr error // what 'agent prompt --wait' and 'agent wait' fail with
}

type bdTicket struct {
	ID        string   `json:"id"`
	Status    string   `json:"status"`
	IssueType string   `json:"issue_type"`
	deps      []string // ids that must be closed first
}

// prompt is a Stage prompt as the fake session sees it.
type prompt struct {
	pane, text, ticket, stage, file string
	round                           int
	openPR                          bool
}

var inputLine = regexp.MustCompile(`(?m)^- ([A-Za-z ]+): (.*)$`)

func parsePrompt(pane, text string) prompt {
	p := prompt{pane: pane, text: text}
	for _, m := range inputLine.FindAllStringSubmatch(text, -1) {
		switch m[1] {
		case "Ticket":
			p.ticket = m[2]
		case "Round":
			p.round, _ = strconv.Atoi(m[2])
		case "Result file":
			p.file = m[2]
			p.stage = strings.Split(strings.TrimSuffix(filepath.Base(m[2]), ".md"), "-")[0]
		case "Open PR":
			p.openPR = m[2] == "yes"
		}
	}
	return p
}

// succeed is the default session: every Stage is done, Verdicts are clean, and
// the last Fix opens a PR.
func succeed(p prompt) (string, string) {
	if p.openPR {
		return "STATUS: done\nPR: https://example.test/pr/" + p.ticket + "\n", "idle"
	}
	return "STATUS: done\n", "idle"
}

func newWorld(t *testing.T, tickets ...*bdTicket) (*world, *Orchestrator) {
	t.Helper()
	repo := t.TempDir()
	if err := setup.InstallSkills(repo, false); err != nil {
		t.Fatal(err)
	}
	w := &world{Fake: &runnertest.Fake{}, t: t, repo: repo, agents: map[string]string{}, prs: map[string]string{}, tickets: tickets, session: succeed}
	home := trustHome(t, repo) // both agents already trust this repo
	for _, ticket := range tickets {
		ticket.Status, ticket.IssueType = "open", "task"
	}
	w.Fake.Handle = w.handle
	state, _ := loadState(repo)
	o := &Orchestrator{Config: Config{
		Exec: w.Run, Repo: repo, MainPane: "main", Workspace: "w1", APIKey: "sk-test", Home: home,
		Tick: time.Millisecond, Max: 3, Log: log.New(io.Discard, "", 0),
	}, state: state}
	return w, o
}

func (w *world) id(kind string) string {
	w.nextID++
	return fmt.Sprintf("w1:%s%d", kind, w.nextID)
}

func reply(result any) (string, error) {
	raw, err := json.Marshal(map[string]any{"result": result})
	return string(raw), err
}

func flagValue(argv []string, flag string) string {
	for i, a := range argv {
		if a == flag && i+1 < len(argv) {
			return argv[i+1]
		}
	}
	return ""
}

func (w *world) handle(dir string, argv []string) (string, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	cmd := strings.Join(argv, " ")
	for prefix, err := range w.failing {
		if strings.HasPrefix(cmd, prefix) {
			delete(w.failing, prefix)
			return "", err
		}
	}
	switch {
	case strings.HasPrefix(cmd, "herdr tab list"):
		tabs := []tabInfo{}
		for _, id := range w.tabs {
			tabs = append(tabs, tabInfo{TabID: id})
		}
		return reply(map[string]any{"tabs": tabs})
	case strings.HasPrefix(cmd, "herdr pane list"):
		return reply(map[string]any{"panes": w.panes})
	case strings.HasPrefix(cmd, "herdr pane layout"):
		return reply(map[string]any{"layout": map[string]any{"panes": w.layout(flagValue(argv, "--pane"))}})
	case strings.HasPrefix(cmd, "herdr tab create"):
		tab, pane := w.id("t"), w.id("p")
		w.tabs = append(w.tabs, tab)
		w.panes = append(w.panes, paneInfo{PaneID: pane, TabID: tab})
		return reply(map[string]any{"tab": tabInfo{TabID: tab}, "root_pane": paneInfo{PaneID: pane, TabID: tab}})
	case strings.HasPrefix(cmd, "herdr pane split"):
		for _, p := range w.panes {
			if p.PaneID == argv[3] {
				pane := paneInfo{PaneID: w.id("p"), TabID: p.TabID}
				w.panes = append(w.panes, pane)
				return reply(map[string]any{"pane": pane})
			}
		}
		return "", errors.New(`{"error":{"code":"pane_not_found"}}`)
	case strings.HasPrefix(cmd, "herdr pane close"):
		w.closePane(argv[3])
		return reply(map[string]any{"type": "ok"})
	case strings.HasPrefix(cmd, "herdr tab close"):
		for _, p := range append([]paneInfo{}, w.panes...) {
			if p.TabID == argv[3] {
				w.closePane(p.PaneID)
			}
		}
		w.live--
		return reply(map[string]any{"type": "ok"})
	case strings.HasPrefix(cmd, "herdr agent start"):
		w.agents[flagValue(argv, "--pane")] = "idle"
		return reply(map[string]any{})
	case strings.HasPrefix(cmd, "herdr agent prompt"):
		if argv[3] == "main" && w.mainBlocked {
			return "", errors.New(`{"error":{"code":"agent_blocked"}}`)
		}
		if argv[3] == "main" && w.mainNoAgent {
			return "", errors.New(`{"error":{"code":"agent_not_found","message":"agent target main not found"}}`)
		}
		if argv[3] == "main" {
			w.main = append(w.main, argv[4])
			return reply(map[string]any{})
		}
		p := parsePrompt(argv[3], argv[4])
		w.mu.Unlock()
		result, status := w.session(p)
		w.mu.Lock()
		if result != "" {
			os.WriteFile(p.file, []byte(result), 0o644)
		}
		if _, alive := w.agents[p.pane]; alive {
			w.agents[p.pane] = status
		}
		return "{}", w.waitErr
	case strings.HasPrefix(cmd, "herdr agent wait"):
		return "{}", w.waitErr
	case strings.HasPrefix(cmd, "herdr agent get"):
		status, alive := w.agents[argv[3]]
		if !alive {
			return "", errors.New(`{"error":{"code":"agent_not_found"}}`)
		}
		return reply(map[string]any{"agent": map[string]string{"agent_status": status}})

	case strings.HasPrefix(cmd, "bd worktree create"):
		w.live++
		w.peak = max(w.peak, w.live)
		return "", os.MkdirAll(argv[3], 0o755)
	case strings.HasPrefix(cmd, "bd worktree remove"):
		return "", os.RemoveAll(argv[3])
	case strings.HasPrefix(cmd, "bd close"):
		w.find(argv[2]).Status = "closed"
	case strings.HasPrefix(cmd, "bd update"):
		w.find(argv[2]).Status = "in_progress"
	case strings.HasPrefix(cmd, "bd list"):
		raw, err := json.Marshal(w.tickets)
		return string(raw), err
	case strings.HasPrefix(cmd, "bd ready"):
		ready := []*bdTicket{}
		for _, ticket := range w.tickets {
			blocked := ticket.Status != "open"
			for _, dep := range ticket.deps {
				blocked = blocked || w.find(dep).Status != "closed"
			}
			if !blocked {
				ready = append(ready, ticket)
			}
		}
		raw, err := json.Marshal(ready)
		return string(raw), err

	case strings.HasPrefix(cmd, "gh pr view"):
		if pr, ok := w.prs[argv[3]]; ok {
			return pr, nil
		}
		if w.merged {
			return `{"state":"MERGED","mergeable":"UNKNOWN"}`, nil
		}
		return `{"state":"OPEN","mergeable":"MERGEABLE"}`, nil
	case cmd == "git remote":
		return "origin\n", nil
	}
	return "", nil
}

// layout answers 'herdr pane layout' for the tab holding pane: every pane's
// rect, a roomy default for the tests that do not care about geometry.
func (w *world) layout(pane string) []map[string]any {
	tab := ""
	for _, p := range w.panes {
		if p.PaneID == pane {
			tab = p.TabID
		}
	}
	var out []map[string]any
	for _, p := range w.panes {
		if p.TabID != tab {
			continue
		}
		size := w.rect
		if size == [2]int{} {
			size = [2]int{200, 100}
		}
		out = append(out, map[string]any{"pane_id": p.PaneID, "rect": map[string]int{"width": size[0], "height": size[1]}})
	}
	return out
}

func (w *world) closePane(id string) {
	delete(w.agents, id)
	for i, p := range w.panes {
		if p.PaneID != id {
			continue
		}
		w.panes = append(w.panes[:i], w.panes[i+1:]...)
		for _, other := range w.panes {
			if other.TabID == p.TabID {
				return
			}
		}
		for t, tab := range w.tabs { // herdr closes a tab with its last pane
			if tab == p.TabID {
				w.tabs = append(w.tabs[:t], w.tabs[t+1:]...)
			}
		}
		return
	}
}

func (w *world) find(id string) *bdTicket {
	for _, ticket := range w.tickets {
		if ticket.ID == id {
			return ticket
		}
	}
	w.t.Errorf("bd was asked about unknown ticket %q", id)
	return &bdTicket{}
}

func (w *world) mainLines() []string {
	w.mu.Lock()
	defer w.mu.Unlock()
	return append([]string{}, w.main...)
}

// awaitLine waits for the Main session to receive a line containing want.
func (w *world) awaitLine(want string) string {
	w.t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		for _, line := range w.mainLines() {
			if strings.Contains(line, want) {
				return line
			}
		}
		time.Sleep(time.Millisecond)
	}
	w.t.Fatalf("Main session never received %q; it got:\n%s", want, strings.Join(w.mainLines(), "\n"))
	return ""
}

func (w *world) control(name string) {
	w.t.Helper()
	dir := filepath.Join(w.repo, ".harness", "control")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		w.t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, name), nil, 0o644); err != nil {
		w.t.Fatal(err)
	}
}

func writeFile(t *testing.T, path, body string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
}

// failOnce makes the next command starting with prefix fail.
func (w *world) failOnce(prefix string, err error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.failing == nil {
		w.failing = map[string]error{}
	}
	w.failing[prefix] = err
}
