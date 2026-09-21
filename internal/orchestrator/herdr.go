package orchestrator

import (
	"encoding/json"
	"fmt"
	"regexp"
	"strings"
)

type tabInfo struct {
	TabID string `json:"tab_id"`
}

type paneInfo struct {
	PaneID string `json:"pane_id"`
	TabID  string `json:"tab_id"`
}

// paneRect is one pane's place in its tab, in terminal cells.
type paneRect struct {
	PaneID string `json:"pane_id"`
	Rect   struct {
		Width  int `json:"width"`
		Height int `json:"height"`
	} `json:"rect"`
}

// herdrReply covers every herdr response shape the Orchestrator reads.
type herdrReply struct {
	Result struct {
		Tab      tabInfo    `json:"tab"`
		RootPane paneInfo   `json:"root_pane"`
		Pane     paneInfo   `json:"pane"`
		Tabs     []tabInfo  `json:"tabs"`
		Panes    []paneInfo `json:"panes"`
		Layout   struct {
			Panes []paneRect `json:"panes"`
		} `json:"layout"`
		Agent struct {
			Status string `json:"agent_status"`
		} `json:"agent"`
	} `json:"result"`
}

func (o *Orchestrator) herdr(args ...string) (herdrReply, error) {
	var reply herdrReply
	out, err := o.Exec(o.Repo, "herdr", args...)
	if err != nil {
		return reply, err
	}
	if err := json.Unmarshal([]byte(out), &reply); err != nil {
		return reply, fmt.Errorf("herdr %s: unreadable reply: %w", args[0], err)
	}
	return reply, nil
}

// A terminal cell is about twice as tall as it is wide, so a pane of equal
// rows and columns is a tall sliver on screen, not a square.
const cellAspect = 2

// splitTarget picks which pane a new Stage pane is split out of, and which way
// to cut it: the roomiest pane, along its longer side. Splitting the same pane
// every time halves it again and again, and four Stage panes in a tab that was
// only ever cut one way are four slivers too narrow for an agent to draw in.
func splitTarget(panes []paneRect) (pane, direction string) {
	best := 0
	for _, p := range panes {
		area := p.Rect.Width * p.Rect.Height
		if area <= best {
			continue
		}
		best, pane, direction = area, p.PaneID, "down"
		if p.Rect.Width > cellAspect*p.Rect.Height {
			direction = "right"
		}
	}
	return pane, direction
}

// location names a pane as <tab>-<pane> by its position in herdr's tab and
// pane lists, because herdr ids are opaque and never reused.
func location(tabs []tabInfo, panes []paneInfo, paneID string) string {
	tabID := ""
	for _, p := range panes {
		if p.PaneID == paneID {
			tabID = p.TabID
		}
	}
	for t, tab := range tabs {
		if tab.TabID != tabID {
			continue
		}
		n := 0
		for _, p := range panes {
			if p.TabID == tabID {
				n++
			}
			if p.PaneID == paneID {
				return fmt.Sprintf("%d-%d", t+1, n)
			}
		}
	}
	return "?"
}

func (o *Orchestrator) locate(paneID string) string {
	tabs, err := o.herdr("tab", "list", "--workspace", o.Workspace)
	if err != nil {
		return "?"
	}
	panes, err := o.herdr("pane", "list", "--workspace", o.Workspace)
	if err != nil {
		return "?"
	}
	return location(tabs.Result.Tabs, panes.Result.Panes, paneID)
}

// agentStatus is the herdr lifecycle state of the agent in a pane; ok is false
// when no agent lives there any more.
func (o *Orchestrator) agentStatus(paneID string) (status string, ok bool) {
	reply, err := o.herdr("agent", "get", paneID)
	if err != nil {
		return "", false
	}
	return reply.Result.Agent.Status, true
}

var notNameChars = regexp.MustCompile(`[^a-z0-9_-]+`)

// agentName builds a herdr agent name: [a-z][a-z0-9_-]{0,31}, unique per live
// agent. The tail of the ticket id is its most distinctive part, so that is
// what survives truncation.
func agentName(ticket, stage string) string {
	name := notNameChars.ReplaceAllString(strings.ToLower(ticket+"-"+stage), "-")
	if len(name) > 30 {
		name = name[len(name)-30:]
	}
	return "h-" + strings.TrimLeft(name, "-_")
}
