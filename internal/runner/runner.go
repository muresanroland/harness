// Package runner is the one seam to every external tool.
package runner

import (
	"bytes"
	"fmt"
	"os/exec"
	"strings"
)

// Runner is the one seam to every external tool (herdr, bd, gh, git). It runs
// name with args in dir and returns stdout; a failure carries stderr.
type Runner func(dir, name string, args ...string) (string, error)

// Exec is the real Runner.
func Exec(dir, name string, args ...string) (string, error) {
	cmd := exec.Command(name, args...)
	cmd.Dir = dir
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	if err := cmd.Run(); err != nil {
		return stdout.String(), fmt.Errorf("%s %s: %w: %s", name, strings.Join(args, " "), err, strings.TrimSpace(stderr.String()))
	}
	return stdout.String(), nil
}
