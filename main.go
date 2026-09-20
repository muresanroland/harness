package main

import (
	"context"
	"flag"
	"fmt"
	"io"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
	"time"
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
		return start(args[1:], out, repo, run, env)
	case "status":
		return printStatus(out, repo)
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
	if lockHolder(repo) == 0 {
		fmt.Fprintln(out, "no Orchestrator is running in this repo; 'harness start <epic>' starts or resumes one")
		return 1
	}
	path := filepath.Join(repo, ".harness", "control", name)
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

// start preflights, then runs the Orchestrator detached from the launching
// shell so it survives the Main session; --foreground is that detached process.
func start(args []string, out io.Writer, repo string, run Runner, env func(string) string) int {
	flags := flag.NewFlagSet("start", flag.ContinueOnError)
	flags.SetOutput(out)
	ticket := flags.String("ticket", "", "run this one Ticket instead of an Epic")
	maxTickets := flags.Int("max", 3, "Tickets in the Pipeline at once")
	foreground := flags.Bool("foreground", false, "run the Orchestrator in this process")
	if len(args) > 0 && args[0] != "" && args[0][0] != '-' { // harness start <epic> --max 2
		args = append(args[1:], args[0])
	}
	if err := flags.Parse(args); err != nil {
		return 2
	}
	epic := flags.Arg(0)
	if (epic == "") == (*ticket == "") || *maxTickets < 1 {
		fmt.Fprint(out, usage)
		return 2
	}
	if code := reportMissing(out, preflight(repo, run, env)); code != 0 {
		return code
	}
	if err := ignoreRunDir(repo); err != nil {
		fmt.Fprintln(out, "start:", err)
		return 1
	}
	if pid := lockHolder(repo); pid != 0 {
		fmt.Fprintf(out, "an Orchestrator is already running in this repo (pid %d); 'harness stop' ends it\n", pid)
		return 1
	}
	logPath := filepath.Join(repo, ".harness", "orchestrator.log")
	if !*foreground {
		pid, err := detach(repo, logPath)
		if err != nil {
			fmt.Fprintln(out, "start:", err)
			return 1
		}
		fmt.Fprintf(out, "Orchestrator started (pid %d), log: %s\n", pid, logPath)
		return 0
	}

	release, err := acquireLock(repo)
	if err != nil {
		fmt.Fprintln(out, "start:", err)
		return 1
	}
	defer release()
	state, err := loadState(repo)
	if err != nil {
		fmt.Fprintln(out, "start:", err)
		return 1
	}
	o := &Orchestrator{
		run: run, repo: repo, state: state, max: *maxTickets,
		mainPane: env("HERDR_PANE_ID"), workspace: env("HERDR_WORKSPACE_ID"), apiKey: env("TYPESAFE_API_KEY"),
		tick: 5 * time.Second, pollPRs: 30 * time.Second,
		log: log.New(out, "", log.LstdFlags),
	}
	if *ticket != "" {
		o.RunTicket(context.Background(), *ticket)
		return 0
	}
	if err := o.Run(context.Background(), epic); err != nil {
		o.report("Orchestrator exited: %v", err)
		return 1
	}
	return 0
}

// detach reruns this command as its own session, logging to the run directory.
func detach(repo, logPath string) (int, error) {
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
	child := exec.Command(self, append(os.Args[1:], "--foreground")...)
	child.Dir, child.Stdout, child.Stderr = repo, logFile, logFile
	child.SysProcAttr = &syscall.SysProcAttr{Setsid: true}
	if err := child.Start(); err != nil {
		return 0, err
	}
	pid := child.Process.Pid
	return pid, child.Process.Release()
}
