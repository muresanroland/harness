package main

import (
	"bytes"
	"strings"
	"testing"
)

func TestNoArgsPrintsUsage(t *testing.T) {
	var out bytes.Buffer
	code := cli(nil, &out, t.TempDir(), nil, nil)
	if code == 0 {
		t.Errorf("exit code = 0, want non-zero")
	}
	for _, sub := range []string{"start", "init", "status", "stop", "retry", "park", "address"} {
		if !strings.Contains(out.String(), sub) {
			t.Errorf("usage does not mention %q:\n%s", sub, out.String())
		}
	}
}

func TestUnknownCommandPrintsUsage(t *testing.T) {
	var out bytes.Buffer
	if code := cli([]string{"bogus"}, &out, t.TempDir(), nil, nil); code == 0 {
		t.Errorf("exit code = 0, want non-zero")
	}
	if !strings.Contains(out.String(), "usage:") {
		t.Errorf("no usage in output:\n%s", out.String())
	}
}
