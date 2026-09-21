package main

import (
	"fmt"
	"os"

	"harness/internal/cli"
	"harness/internal/runner"
)

func main() {
	repo, err := os.Getwd()
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	os.Exit(cli.Run(os.Args[1:], os.Stdout, repo, runner.Exec, os.Getenv))
}
