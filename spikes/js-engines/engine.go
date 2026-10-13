// Package engines is a minimal host for engines written in a strict subset
// of JavaScript, run with goja (pure Go).
//
// An engine is one file, engine.js, declaring three functions:
//
//	function init()                  the starting state
//	function step(state, input, now) {state, wake}
//	function view(state)             what clients see
//
// plus, optionally, top-level `const`s and helper functions. Every input,
// client or host, is one object with a `tag`. The host's own are
// {tag: "Location", timestamp, fix, velocity} (a GPS update) and
// {tag: "Wake"} (the wake time the engine asked for has come). `wake` is an
// absolute time, epoch ms, or null for none.
//
// Values cross the wall as plain data: Go nil, bool, string, int, float64,
// map[string]any and []any one way; null, booleans, strings, numbers,
// objects and arrays the other. JavaScript has one number type: a whole
// number comes back to Go as an int, any other as a float64. State and
// input are deep-frozen and the code runs in strict mode, so an engine that
// tries to change them throws.
package engines

import (
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"sort"
	"time"

	"github.com/dop251/goja"
	"github.com/dop251/goja/ast"
	"github.com/dop251/goja/parser"
)

// File is the one source file of an engine.
const File = "engine.js"

// callLimit: how long one call into the engine may run.
const callLimit = 2 * time.Second

type Engine struct {
	Name  string
	State any
	wake  *int

	rt   *runtime
	step goja.Callable
	view goja.Callable
}

// SourceError is a syntax error at a place in the source (1-based line and
// column).
type SourceError struct {
	File      string
	Line, Col int
	Msg       string
}

func (e *SourceError) Error() string {
	return fmt.Sprintf("%s:%d:%d: %s", e.File, e.Line, e.Col, e.Msg)
}

// Parse parses an engine's source (the lint works on this tree).
func Parse(name, src string) (*ast.Program, error) {
	prg, err := parser.ParseFile(nil, name, src, 0)
	if err != nil {
		var list parser.ErrorList
		if errors.As(err, &list) && len(list) > 0 {
			p := list[0].Position
			return nil, &SourceError{File: name, Line: p.Line, Col: p.Column, Msg: list[0].Message}
		}
		return nil, err
	}
	return prg, nil
}

// Compile parses and compiles an engine's source, in strict mode.
func Compile(name, src string) (*goja.Program, error) {
	prg, err := Parse(name, src)
	if err != nil {
		return nil, err
	}
	p, err := goja.CompileAST(prg, true)
	if err != nil {
		var se *goja.CompilerSyntaxError
		if errors.As(err, &se) && se.File != nil {
			pos := se.File.Position(se.Offset)
			return nil, &SourceError{File: name, Line: pos.Line, Col: pos.Column, Msg: se.Message}
		}
		return nil, fmt.Errorf("%s: %w", name, err)
	}
	return p, nil
}

// Load compiles an engine's source (once) and evaluates its init.
func Load(dir string) (*Engine, error) {
	path := filepath.Join(dir, File)
	src, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	p, err := Compile(File, string(src))
	if err != nil {
		return nil, fmt.Errorf("%s/%w", dir, err)
	}
	rt := newRuntime()
	if _, err := rt.call(func() (goja.Value, error) { return rt.vm.RunProgram(p) }); err != nil {
		return nil, fmt.Errorf("%s/%s: %w", dir, File, err)
	}
	fns := map[string]goja.Callable{}
	for _, name := range []string{"init", "step", "view"} {
		f, ok := goja.AssertFunction(rt.vm.Get(name))
		if !ok {
			return nil, fmt.Errorf("%s/%s: no function %s (declare it at the top level: `function %s(...) { ... }`)", dir, File, name, name)
		}
		fns[name] = f
	}
	e := &Engine{Name: filepath.Base(dir), rt: rt, step: fns["step"], view: fns["view"]}
	out, err := rt.call(func() (goja.Value, error) { return fns["init"](goja.Undefined()) })
	if err != nil {
		return nil, e.jsErr("init", err)
	}
	state, err := rt.toGo(out, "init()", false)
	if err != nil {
		return nil, fmt.Errorf("%s init: %w", e.Name, err)
	}
	if _, ok := state.(map[string]any); !ok {
		return nil, fmt.Errorf("%s init: returned %s, not an object", e.Name, describe(state))
	}
	e.State = state
	return e, nil
}

// Step feeds one input at `now`, and says whether the view changed (the
// host diffs views; engines do not report it). On any error the state is
// kept.
func (e *Engine) Step(now int, input map[string]any) (changed bool, err error) {
	before, err := e.viewKey(e.State)
	if err != nil {
		return false, err
	}
	what := fmt.Sprintf("%s step %v", e.Name, input["tag"])
	st, err := e.rt.toJS(e.State)
	if err != nil {
		return false, fmt.Errorf("%s: state: %w", what, err)
	}
	in, err := e.rt.toJS(input)
	if err != nil {
		return false, fmt.Errorf("%s: input: %w", what, err)
	}
	out, err := e.rt.call(func() (goja.Value, error) { return e.step(goja.Undefined(), st, in, e.rt.vm.ToValue(int64(now))) })
	if err != nil {
		// includes throw new Error(msg): the state is kept
		return false, e.jsErr(what, err)
	}
	res, ok := out.(*goja.Object)
	if !ok || res.ClassName() != "Object" {
		return false, fmt.Errorf("%s: returned %s, not {state, wake}", what, out)
	}
	sv := res.Get("state")
	if sv == nil || goja.IsUndefined(sv) {
		return false, fmt.Errorf("%s: returned no state (want {state, wake})", what)
	}
	state, err := e.rt.toGo(sv, "state", false)
	if err != nil {
		return false, fmt.Errorf("%s: %w", what, err)
	}
	if _, ok := state.(map[string]any); !ok {
		return false, fmt.Errorf("%s: state is %s, not an object", what, describe(state))
	}
	var wake *int
	if wv := res.Get("wake"); wv != nil && !goja.IsUndefined(wv) && !goja.IsNull(wv) {
		w, err := e.rt.toGo(wv, "wake", false)
		if err != nil {
			return false, fmt.Errorf("%s: %w", what, err)
		}
		n, ok := w.(int)
		if !ok {
			return false, fmt.Errorf("%s: wake is %s, not a whole number of epoch ms (or null)", what, describe(w))
		}
		wake = &n
	}
	after, err := e.viewKey(state)
	if err != nil {
		return false, err
	}
	e.State, e.wake = state, wake
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

// View evaluates view(state) on the current state.
func (e *Engine) View() (map[string]any, error) { return e.viewOf(e.State) }

func (e *Engine) viewOf(state any) (map[string]any, error) {
	st, err := e.rt.toJS(state)
	if err != nil {
		return nil, fmt.Errorf("%s view: state: %w", e.Name, err)
	}
	out, err := e.rt.call(func() (goja.Value, error) { return e.view(goja.Undefined(), st) })
	if err != nil {
		return nil, e.jsErr(e.Name+" view", err)
	}
	v, err := e.rt.toGo(out, "view", true)
	if err != nil {
		return nil, fmt.Errorf("%s view: %w", e.Name, err)
	}
	m, ok := v.(map[string]any)
	if !ok {
		return nil, fmt.Errorf("%s view: returned %s, not an object", e.Name, describe(v))
	}
	return m, nil
}

// viewKey is the view as canonical JSON, for diffing (distinct floats
// compare unequal).
func (e *Engine) viewKey(state any) (string, error) {
	v, err := e.viewOf(state)
	if err != nil {
		return "", err
	}
	b, err := json.Marshal(v)
	return string(b), err
}

// jsErr: an error from a call into the engine. A thrown Error (or string)
// is the engine failing on purpose: "fail: <message>". Anything else (a
// TypeError, reading a field of undefined, a timeout) is reported as is.
func (e *Engine) jsErr(what string, err error) error {
	var ex *goja.Exception
	if errors.As(err, &ex) {
		if msg, ok := failMessage(ex.Value()); ok {
			return fmt.Errorf("%s: fail: %s", what, msg)
		}
		return fmt.Errorf("%s: %s", what, ex.Error())
	}
	return fmt.Errorf("%s: %w", what, err)
}

func failMessage(v goja.Value) (string, bool) {
	if o, ok := v.(*goja.Object); ok {
		if o.ClassName() == "Error" && o.Get("name") != nil && o.Get("name").String() == "Error" {
			if m := o.Get("message"); m != nil && !goja.IsUndefined(m) {
				return m.String(), true
			}
			return "", true
		}
		return "", false
	}
	if s, ok := v.Export().(string); ok {
		return s, true
	}
	return "", false
}

// ---- the runtime ----

type runtime struct {
	vm     *goja.Runtime
	freeze goja.Callable
}

// newRuntime: a goja runtime with the builtins, and without the clock and
// randomness.
func newRuntime() *runtime {
	vm := goja.New()
	vm.SetMaxCallStackSize(256)
	r := &runtime{vm: vm}
	freeze, ok := goja.AssertFunction(vm.Get("Object").ToObject(vm).Get("freeze"))
	if !ok {
		panic("no Object.freeze")
	}
	r.freeze = freeze
	g := vm.GlobalObject()
	for _, name := range []string{"Date", "eval", "Function", "WeakRef", "FinalizationRegistry", "Proxy", "Reflect"} {
		g.Delete(name)
	}
	vm.Get("Math").ToObject(vm).Delete("random")
	for _, name := range BuiltinNames() {
		r.install(name, builtins[name])
	}
	return r
}

// call runs f with the time limit.
func (r *runtime) call(f func() (goja.Value, error)) (goja.Value, error) {
	timer := time.AfterFunc(callLimit, func() { r.vm.Interrupt(fmt.Sprintf("took longer than %v", callLimit)) })
	defer func() {
		timer.Stop()
		r.vm.ClearInterrupt()
	}()
	return f()
}

func (r *runtime) install(name string, fn func([]any) (any, error)) {
	r.vm.Set(name, func(call goja.FunctionCall) goja.Value {
		args := make([]any, len(call.Arguments))
		for i, a := range call.Arguments {
			if goja.IsUndefined(a) {
				continue // nil
			}
			if f, ok := goja.AssertFunction(a); ok {
				args[i] = r.callback(name, f)
				continue
			}
			v, err := r.toGo(a, fmt.Sprintf("%s: argument %d", name, i+1), false)
			if err != nil {
				panic(r.vm.NewTypeError(err.Error()))
			}
			args[i] = v
		}
		out, err := fn(args)
		if err != nil {
			panic(r.vm.NewTypeError(err.Error()))
		}
		v, err := r.toJS(out)
		if err != nil {
			panic(r.vm.NewTypeError(fmt.Sprintf("%s: %v", name, err)))
		}
		return v
	})
}

// callback: a function an engine passed to a builtin, for the builtin to
// call. Its arguments go in as plain data and its result comes back as
// plain data. A throw (or the time limit) inside it is re-raised as it is,
// so it goes on through the builtin to the engine's caller unchanged.
func (r *runtime) callback(builtin string, f goja.Callable) Fn {
	return func(args ...any) (any, error) {
		js := make([]goja.Value, len(args))
		for i, a := range args {
			v, err := r.toJS(a)
			if err != nil {
				return nil, err
			}
			js[i] = v
		}
		out, err := f(goja.Undefined(), js...)
		if err != nil {
			panic(err)
		}
		return r.toGo(out, builtin+": the function's result", false)
	}
}

// toJS builds a deep-frozen JavaScript value from plain Go data.
func (r *runtime) toJS(v any) (goja.Value, error) {
	switch x := v.(type) {
	case nil:
		return goja.Null(), nil
	case bool, string:
		return r.vm.ToValue(x), nil
	case int:
		return r.intToJS(int64(x))
	case int64:
		return r.intToJS(x)
	case float64:
		return r.vm.ToValue(x), nil
	case float32:
		return r.vm.ToValue(float64(x)), nil
	case map[string]any:
		o := r.vm.NewObject()
		ks := make([]string, 0, len(x))
		for k := range x {
			ks = append(ks, k)
		}
		sort.Strings(ks)
		for _, k := range ks {
			jv, err := r.toJS(x[k])
			if err != nil {
				return nil, fmt.Errorf("%s: %w", k, err)
			}
			if err := o.Set(k, jv); err != nil {
				return nil, err
			}
		}
		return r.frozen(o)
	case []any:
		vals := make([]any, len(x))
		for i, e := range x {
			jv, err := r.toJS(e)
			if err != nil {
				return nil, fmt.Errorf("[%d]: %w", i, err)
			}
			vals[i] = jv
		}
		return r.frozen(r.vm.NewArray(vals...))
	}
	return nil, fmt.Errorf("%T is not plain data", v)
}

// maxSafe: JavaScript numbers hold whole numbers exactly up to 2^53 - 1.
const maxSafe = 1<<53 - 1

func (r *runtime) intToJS(x int64) (goja.Value, error) {
	if x > maxSafe || x < -maxSafe {
		return nil, fmt.Errorf("%d is too large for a JavaScript number to hold exactly", x)
	}
	return r.vm.ToValue(x), nil
}

func (r *runtime) frozen(o *goja.Object) (goja.Value, error) {
	if _, err := r.freeze(goja.Undefined(), o); err != nil {
		return nil, err
	}
	return o, nil
}

// toGo reads a JavaScript value back as plain Go data. A whole number is an
// int, any other number a float64. In a view (view true) undefined, NaN and
// the infinities are errors; in state an undefined field is left out, as
// JSON would, and an undefined list item is null.
func (r *runtime) toGo(v goja.Value, path string, view bool) (any, error) {
	return r.toGoDepth(v, path, view, 0)
}

func (r *runtime) toGoDepth(v goja.Value, path string, view bool, depth int) (any, error) {
	if depth > 64 {
		return nil, fmt.Errorf("%s: nested too deeply", path)
	}
	if v == nil || goja.IsUndefined(v) {
		return nil, fmt.Errorf("%s is undefined (use null for none)", path)
	}
	if goja.IsNull(v) {
		return nil, nil
	}
	if o, ok := v.(*goja.Object); ok {
		if _, isFn := goja.AssertFunction(o); isFn {
			return nil, fmt.Errorf("%s is a function: only plain data (numbers, strings, booleans, null, arrays and objects) can be kept or shown", path)
		}
		switch o.ClassName() {
		case "Array":
			n := int(o.Get("length").ToInteger())
			out := make([]any, n)
			for i := 0; i < n; i++ {
				ev := o.Get(fmt.Sprint(i))
				p := fmt.Sprintf("%s[%d]", path, i)
				if !view && (ev == nil || goja.IsUndefined(ev)) {
					continue // null
				}
				x, err := r.toGoDepth(ev, p, view, depth+1)
				if err != nil {
					return nil, err
				}
				out[i] = x
			}
			return out, nil
		case "Object":
			out := map[string]any{}
			for _, k := range o.Keys() {
				fv := o.Get(k)
				if !view && (fv == nil || goja.IsUndefined(fv)) {
					continue
				}
				x, err := r.toGoDepth(fv, path+"."+k, view, depth+1)
				if err != nil {
					return nil, err
				}
				out[k] = x
			}
			return out, nil
		}
		return nil, fmt.Errorf("%s is a %s, not plain data", path, o.ClassName())
	}
	switch x := v.Export().(type) {
	case bool, string:
		return x, nil
	case int64:
		return int(x), nil
	case float64:
		if math.IsNaN(x) || math.IsInf(x, 0) {
			if view {
				return nil, fmt.Errorf("%s is %v: a view holds finite numbers only", path, x)
			}
			return x, nil
		}
		if x == math.Trunc(x) && math.Abs(x) < 1<<63 {
			return int(x), nil // -0 too
		}
		return x, nil
	}
	return nil, fmt.Errorf("%s is %s, not plain data", path, v.String())
}

func describe(v any) string {
	switch v.(type) {
	case nil:
		return "null"
	case []any:
		return "an array"
	case map[string]any:
		return "an object"
	case string:
		return fmt.Sprintf("the string %q", v)
	case Fn:
		return "a function"
	}
	return fmt.Sprintf("%v", v)
}
