// Package engines is a minimal host for engines written as expr expressions.
//
// An engine is a directory of three expressions:
//
//	init.expr  the starting state
//	step.expr  (state, input, now) -> {state, wake}
//	view.expr  (state) -> what clients see
//
// Every input, client or host, is one map with a `tag`. The host's own are
// {tag: "Location", timestamp, fix, velocity} (a GPS update) and
// {tag: "Wake"} (the wake time the engine asked for has come). `wake` is an
// absolute time, epoch ms, or nil for none.
package engines

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"github.com/expr-lang/expr"
	"github.com/expr-lang/expr/vm"
)

type Engine struct {
	Name  string
	step  *vm.Program
	view  *vm.Program
	State any
	wake  *int
}

// Load compiles an engine's expressions (once) and evaluates its init.
func Load(dir string) (*Engine, error) {
	compile := func(file string, env map[string]any) (*vm.Program, error) {
		src, err := os.ReadFile(filepath.Join(dir, file))
		if err != nil {
			return nil, err
		}
		opts := append([]expr.Option{expr.Env(env)}, Builtins()...)
		p, err := expr.Compile(string(src), opts...)
		if err != nil {
			return nil, fmt.Errorf("%s/%s: %w", dir, file, err)
		}
		return p, nil
	}
	initP, err := compile("init.expr", map[string]any{})
	if err != nil {
		return nil, err
	}
	// state and input are maps of anything: checked at run time.
	step, err := compile("step.expr", map[string]any{"state": map[string]any{}, "input": map[string]any{}, "now": int(0)})
	if err != nil {
		return nil, err
	}
	view, err := compile("view.expr", map[string]any{"state": map[string]any{}})
	if err != nil {
		return nil, err
	}
	state, err := expr.Run(initP, map[string]any{})
	if err != nil {
		return nil, fmt.Errorf("%s/init.expr: %w", dir, err)
	}
	return &Engine{Name: filepath.Base(dir), step: step, view: view, State: state}, nil
}

// Step feeds one input at `now`, and says whether the view changed (the
// host diffs views; engines no longer report it).
func (e *Engine) Step(now int, input map[string]any) (changed bool, err error) {
	before, err := e.viewKey()
	if err != nil {
		return false, err
	}
	out, err := expr.Run(e.step, map[string]any{"state": e.State, "input": input, "now": now})
	if err != nil {
		// includes fail(msg): the state is kept
		return false, fmt.Errorf("%s step %v: %w", e.Name, input["tag"], err)
	}
	res, ok := out.(map[string]any)
	if !ok {
		return false, fmt.Errorf("%s step: returned %T, not {state, wake}", e.Name, out)
	}
	state, ok := res["state"]
	if !ok {
		return false, fmt.Errorf("%s step: no state in %v", e.Name, res)
	}
	var wake *int
	switch w := res["wake"].(type) {
	case nil:
	case int:
		wake = &w
	default:
		return false, fmt.Errorf("%s step: wake is %T (%v), not a time", e.Name, w, w)
	}
	e.State, e.wake = state, wake
	after, err := e.viewKey()
	if err != nil {
		return false, err
	}
	return before != after, nil
}

// Wake delivers a Wake input at `now` whether or not one is due, as a host
// would when a wake raced with an input that cancelled it.
func (e *Engine) Wake(now int) (bool, error) {
	return e.Step(now, map[string]any{"tag": "Wake"})
}

// AdvanceTo is the host's clock reaching `now`: while the engine's wake time
// has come, it is delivered, with `now` the time it was due at (as on_timer).
// Says whether any delivery changed the view, and how many were delivered.
func (e *Engine) AdvanceTo(now int) (changed bool, wakes int, err error) {
	for e.wake != nil && *e.wake <= now {
		if wakes == 1000 {
			return changed, wakes, fmt.Errorf("%s: wake loop: still due at %d", e.Name, *e.wake)
		}
		c, err := e.Wake(*e.wake)
		if err != nil {
			return changed, wakes, err
		}
		changed = changed || c
		wakes++
	}
	return changed, wakes, nil
}

// NextWake is the engine's requested wake time, if any.
func (e *Engine) NextWake() (int, bool) {
	if e.wake == nil {
		return 0, false
	}
	return *e.wake, true
}

// View evaluates the view expression on the current state.
func (e *Engine) View() (map[string]any, error) {
	out, err := expr.Run(e.view, map[string]any{"state": e.State})
	if err != nil {
		return nil, fmt.Errorf("%s view: %w", e.Name, err)
	}
	v, ok := out.(map[string]any)
	if !ok {
		return nil, fmt.Errorf("%s view: returned %T, not a map", e.Name, out)
	}
	return v, nil
}

// viewKey is the view as canonical JSON, for diffing (distinct floats
// compare unequal).
func (e *Engine) viewKey() (string, error) {
	v, err := e.View()
	if err != nil {
		return "", err
	}
	b, err := json.Marshal(v)
	return string(b), err
}
