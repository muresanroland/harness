// Package runnertest is the test double for the Runner seam.
package runnertest

import (
	"strings"
	"sync"
)

// Fake records every call and answers it from Handle; a nil Handle returns ""
// with no error.
type Fake struct {
	mu     sync.Mutex
	calls  []string
	Handle func(dir string, argv []string) (string, error)
}

// Run is the runner.Runner to hand to the code under test.
func (f *Fake) Run(dir, name string, args ...string) (string, error) {
	argv := append([]string{name}, args...)
	f.mu.Lock()
	f.calls = append(f.calls, strings.Join(argv, " "))
	f.mu.Unlock()
	if f.Handle == nil {
		return "", nil
	}
	return f.Handle(dir, argv)
}

// Calls returns every call so far, in order, as space-joined command lines.
func (f *Fake) Calls() []string {
	f.mu.Lock()
	defer f.mu.Unlock()
	return append([]string{}, f.calls...)
}

// Called returns the calls that start with prefix.
func (f *Fake) Called(prefix string) []string {
	var out []string
	for _, c := range f.Calls() {
		if strings.HasPrefix(c, prefix) {
			out = append(out, c)
		}
	}
	return out
}
