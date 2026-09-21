// Package cli is the harness command line.
package cli

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"io"
	"log"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"slices"
	"syscall"
	"time"

	"harness/internal/orchestrator"
	"harness/internal/runner"
	"harness/internal/setup"
)

const usage = `usage: harness <command>

  init [--force]              install the shipped skills and preflight the Target repo
  start <epic> [--max N]      run an Epic's Tickets through the Pipeline, here
  start --ticket <id>         run one Ticket through the Pipeline, here
    --detach                  run it in the background instead, logging to a file
  status                      print the state of the run
  retry <ticket>              restart a Ticket's current Stage with a fresh session
  park <ticket>               park a Ticket
  address <ticket>            act on a Ticket's PR review comments and conflicts
  stop                        stop scheduling and exit, leaving live panes alone
`

// Run runs one harness command inside the Target repo and returns the exit code.
func Run(args []string, out io.Writer, repo string, run runner.Runner, env func(string) string) int {
	if len(args) == 0 {
		fmt.Fprint(out, usage)
		return 2
	}
	switch args[0] {
	case "init":
		if err := setup.InstallSkills(repo, len(args) > 1 && args[1] == "--force", out, os.Stdin); err != nil {
			fmt.Fprintln(out, "init:", err)
			return 1
		}
		return setup.ReportMissing(out, setup.Preflight(repo, run, env))
	case "start":
		return start(args[1:], out, repo, run, env)
	case "status":
		return orchestrator.PrintStatus(out, repo)
	case "stop", "retry", "park", "address":
		return command(args, out, repo)
	}
	fmt.Fprint(out, usage)
	return 2
}

// command leaves a control file for the running Orchestrator, which owns the
// state file and is the only process that acts on it.
func command(args []string, out io.Writer, repo string) int {
	name := args[0]
	if name != "stop" {
		if len(args) != 2 {
			fmt.Fprintf(out, "usage: harness %s <ticket>\n", name)
			return 2
		}
		name += "-" + args[1]
	}
	if orchestrator.LockHolder(repo) == 0 {
		fmt.Fprintln(out, "no Orchestrator is running in this repo; 'harness start <epic>' starts or resumes one")
		return 1
	}
	path := orchestrator.ControlFile(repo, name)
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		fmt.Fprintln(out, err)
		return 1
	}
	if err := os.WriteFile(path, nil, 0o644); err != nil {
		fmt.Fprintln(out, err)
		return 1
	}
	fmt.Fprintln(out, "sent:", name)
	return 0
}

type startArgs struct {
	epic, ticket string
	max          int
	detach       bool
}

// parseStart accepts the Epic and the flags in any order.
func parseStart(args []string, out io.Writer) (startArgs, error) {
	var parsed startArgs
	flags := flag.NewFlagSet("start", flag.ContinueOnError)
	flags.SetOutput(out)
	flags.StringVar(&parsed.ticket, "ticket", "", "run this one Ticket instead of an Epic")
	flags.IntVar(&parsed.max, "max", 3, "Tickets in the Pipeline at once")
	flags.BoolVar(&parsed.detach, "detach", false, "run the Orchestrator in the background, logging to .harness/orchestrator.log")
	var positional []string
	for { // flag stops at the first positional argument: take it and carry on
		if err := flags.Parse(args); err != nil {
			return parsed, err
		}
		if flags.NArg() == 0 {
			break
		}
		positional, args = append(positional, flags.Arg(0)), flags.Args()[1:]
	}
	if len(positional) == 1 {
		parsed.epic = positional[0]
	}
	if len(positional) > 1 || (parsed.epic == "") == (parsed.ticket == "") || parsed.max < 1 {
		return parsed, fmt.Errorf("want one Epic or --ticket <id>, and --max of at least 1")
	}
	return parsed, nil
}

// start preflights, then runs the Orchestrator in this process, where its log
// is the pane's output and Ctrl-C ends it. --detach puts it in the background
// instead, for a Main session that wants its prompt back.
func start(args []string, out io.Writer, repo string, run runner.Runner, env func(string) string) int {
	parsed, err := parseStart(args, out)
	if err != nil {
		fmt.Fprint(out, usage)
		return 2
	}
	if code := setup.ReportMissing(out, setup.Preflight(repo, run, env)); code != 0 {
		return code
	}
	if err := setup.IgnoreRunDir(repo); err != nil {
		fmt.Fprintln(out, "start:", err)
		return 1
	}
	if pid := orchestrator.LockHolder(repo); pid != 0 {
		fmt.Fprintf(out, "an Orchestrator is already running in this repo (pid %d); 'harness stop' ends it\n", pid)
		return 1
	}
	logPath := filepath.Join(repo, ".harness", "orchestrator.log")
	if parsed.detach {
		child := append([]string{"start"}, slices.DeleteFunc(slices.Clone(args), func(a string) bool { return a == "--detach" })...)
		pid, err := detach(repo, logPath, child)
		if err != nil {
			fmt.Fprintln(out, "start:", err)
			return 1
		}
		fmt.Fprintf(out, "Orchestrator started in the background (pid %d), log: %s\n", pid, logPath)
		return 0
	}

	release, err := orchestrator.AcquireLock(repo)
	if err != nil {
		fmt.Fprintln(out, "start:", err)
		return 1
	}
	defer release()

	// Everything the run says is shown here; the same lines go to the log file
	// so a finished run can still be read back.
	events := out
	if logFile, err := os.OpenFile(logPath, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o644); err == nil {
		defer logFile.Close()
		events = io.MultiWriter(out, logFile)
	}
	ctx, stopSignals := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stopSignals()
	o, err := orchestrator.New(orchestrator.Config{
		Exec: run, Repo: repo, Max: parsed.max,
		MainPane: env("HERDR_PANE_ID"), Workspace: env("HERDR_WORKSPACE_ID"), APIKey: env("TYPESAFE_API_KEY"),
		Tick: 5 * time.Second, PollPRs: 30 * time.Second,
		Log: log.New(events, "", log.LstdFlags),
	})
	if err != nil {
		fmt.Fprintln(out, "start:", err)
		return 1
	}
	fmt.Fprintf(out, "Orchestrator running here (pid %d). Ctrl-C, or 'harness stop' from another pane, ends it.\n", os.Getpid())
	if parsed.ticket != "" {
		if o.RunTicket(ctx, parsed.ticket) {
			return 1
		}
		return 0
	}
	if err := o.Run(ctx, parsed.epic); err != nil && !errors.Is(err, context.Canceled) {
		fmt.Fprintln(out, "start:", err)
		return 1
	}
	return 0
}

// detach reruns this command as its own session, logging to the run directory.
func detach(repo, logPath string, args []string) (int, error) {
	if err := os.MkdirAll(filepath.Dir(logPath), 0o755); err != nil {
		return 0, err
	}
	logFile, err := os.OpenFile(logPath, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o644)
	if err != nil {
		return 0, err
	}
	defer logFile.Close()
	self, err := os.Executable()
	if err != nil {
		return 0, err
	}
	child := exec.Command(self, args...)
	child.Dir, child.Stdout, child.Stderr = repo, logFile, logFile
	child.SysProcAttr = &syscall.SysProcAttr{Setsid: true}
	if err := child.Start(); err != nil {
		return 0, err
	}
	pid := child.Process.Pid
	return pid, child.Process.Release()
}
