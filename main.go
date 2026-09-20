package main

import (
	"fmt"
	"io"
	"os"
)

const usage = `usage: harness <command>

  init [--force]              install the Stage skills and preflight the Target repo
  start <epic> [--max N]      run an Epic's Tickets through the Pipeline
  start --ticket <id>         run one Ticket through the Pipeline
  status                      print the state of the run
  retry <ticket>              restart a Ticket's current Stage with a fresh session
  park <ticket>               park a Ticket
  address <ticket>            act on a Ticket's PR review comments and conflicts
  stop                        stop scheduling and exit, leaving live panes alone
`

func main() {
	repo, err := os.Getwd()
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	os.Exit(cli(os.Args[1:], os.Stdout, repo, execRunner, os.Getenv))
}

// cli runs one harness command inside the Target repo and returns the exit code.
func cli(args []string, out io.Writer, repo string, run Runner, env func(string) string) int {
	if len(args) == 0 {
		fmt.Fprint(out, usage)
		return 2
	}
	switch args[0] {
	case "init":
		if err := installSkills(repo, len(args) > 1 && args[1] == "--force"); err != nil {
			fmt.Fprintln(out, "init:", err)
			return 1
		}
		return reportMissing(out, preflight(repo, run, env))
	case "start":
		if code := reportMissing(out, preflight(repo, run, env)); code != 0 {
			return code
		}
		fmt.Fprintln(out, "not implemented")
		return 1
	case "status", "stop", "retry", "park", "address":
		fmt.Fprintln(out, "not implemented")
		return 1
	}
	fmt.Fprint(out, usage)
	return 2
}
