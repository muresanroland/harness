package main

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

// herdrReply covers every herdr response shape the Orchestrator reads.
type herdrReply struct {
	Result struct {
		Tab      tabInfo    `json:"tab"`
		RootPane paneInfo   `json:"root_pane"`
		Pane     paneInfo   `json:"pane"`
		Tabs     []tabInfo  `json:"tabs"`
		Panes    []paneInfo `json:"panes"`
		Agent    struct {
			Status string `json:"agent_status"`
		} `json:"agent"`
	} `json:"result"`
}

func (o *Orchestrator) herdr(args ...string) (herdrReply, error) {
	var reply herdrReply
	out, err := o.run(o.repo, "herdr", args...)
	if err != nil {
		return reply, err
	}
	if err := json.Unmarshal([]byte(out), &reply); err != nil {
		return reply, fmt.Errorf("herdr %s: unreadable reply: %w", args[0], err)
	}
	return reply, nil
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
	tabs, err := o.herdr("tab", "list", "--workspace", o.workspace)
	if err != nil {
		return "?"
	}
	panes, err := o.herdr("pane", "list", "--workspace", o.workspace)
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
