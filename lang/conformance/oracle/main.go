// The conformance oracle: Go expr, the language our compiler and VM must
// agree with (spec §9). Pinned (go.mod) to v1.17.8 plus the fix for
// findLast/findLastIndex as operands (expr-lang/expr PR #954, issue #950:
// the backward loop left a stale bool on the stack), which is unreleased
// as of v1.17.8; our VM implements the fixed behaviour.
//
//	go run . vectors CASES OUT [sailing]   run every case, write the results
//	                                       (sailing: register the engine profile's builtins)
//	go run . probe 'expr' ...              print what expr does with each expression
//
// Results are written so no precision is lost: ints as decimal strings,
// floats as their IEEE bits in hex.
package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"reflect"
	"runtime"
	"strconv"

	engines "expr-engines"

	"github.com/expr-lang/expr"
)

type input struct {
	Name     string         `json:"name"`
	Schema   map[string]any `json:"schema"`
	Nullable bool           `json:"nullable"`
}

type testCase struct {
	Name   string                       `json:"name"`
	Source string                       `json:"source"`
	Inputs []input                      `json:"inputs"`
	Output map[string]any               `json:"output"`
	Runs   []map[string]json.RawMessage `json:"runs"`
	// Test builtins the case calls (testBuiltins), by name.
	Builtins []string `json:"builtins"`
}

// The language's own test builtins (spec §9), the same as the Rust side's in
// lang/compiler/tests/conformance.rs. Each is exact: no transcendental
// functions, so Go and the VM agree to the bit.
var testBuiltins = map[string]expr.Option{
	// dist(a, b, scale?): |a.lat - b.lat| + |a.lon - b.lon|, times scale if given.
	"dist": expr.Function("dist", func(params ...any) (any, error) {
		a, aok := params[0].(map[string]any)
		b, bok := params[1].(map[string]any)
		if !aok || !bok {
			return nil, fmt.Errorf("dist: a and b must be records, got %T and %T", params[0], params[1])
		}
		d := math.Abs(num(a["lat"])-num(b["lat"])) + math.Abs(num(a["lon"])-num(b["lon"]))
		if len(params) > 2 && params[2] != nil {
			d *= num(params[2])
		}
		return d, nil
	}),
}

func num(v any) float64 {
	switch x := v.(type) {
	case int:
		return float64(x)
	case float64:
		return x
	default:
		panic(fmt.Sprintf("not a number: %#v", v))
	}
}

type result struct {
	Ok    map[string]any `json:"ok,omitempty"`
	Error string         `json:"error,omitempty"`
}

type caseResult struct {
	Name         string   `json:"name"`
	CompileError string   `json:"compile_error,omitempty"`
	Results      []result `json:"results,omitempty"`
}

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "usage: oracle vectors CASES OUT | oracle probe EXPR...")
		os.Exit(2)
	}
	switch os.Args[1] {
	case "vectors":
		var opts []expr.Option
		if len(os.Args) > 4 && os.Args[4] == "sailing" {
			opts = engines.Builtins()
		}
		if err := vectors(os.Args[2], os.Args[3], opts); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
	case "probe":
		for _, src := range os.Args[2:] {
			fmt.Printf("%-40s => %s\n", src, probe(src))
		}
	default:
		fmt.Fprintln(os.Stderr, "unknown command", os.Args[1])
		os.Exit(2)
	}
}

func probe(src string) string {
	env := map[string]any{
		"n": nil, "i": 3, "f": 2.5,
		"r":  map[string]any{"a": 1, "b": map[string]any{"c": 2}, "o": nil},
		"rn": nil,
		"xs": []any{1, 2, 3},
	}
	p, err := expr.Compile(src, expr.Env(env), expr.Optimize(false))
	if err != nil {
		return "COMPILE ERROR: " + firstLine(err.Error())
	}
	out, err := expr.Run(p, env)
	if err != nil {
		return "RUN ERROR: " + firstLine(err.Error())
	}
	return fmt.Sprintf("%#v", out)
}

func vectors(casesPath, outPath string, builtins []expr.Option) error {
	if runtime.GOARCH != "arm64" {
		return fmt.Errorf("the oracle runs on arm64 only (int() of an out-of-range float is platform-defined); this is %s", runtime.GOARCH)
	}
	raw, err := os.ReadFile(casesPath)
	if err != nil {
		return err
	}
	var file struct {
		Defs  map[string]any `json:"defs"`
		Cases []testCase     `json:"cases"`
	}
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	if err := dec.Decode(&file); err != nil {
		return err
	}
	for i := range file.Cases {
		c := &file.Cases[i]
		for j := range c.Inputs {
			c.Inputs[j].Schema = expand(c.Inputs[j].Schema, file.Defs).(map[string]any)
		}
		c.Output = expand(c.Output, file.Defs).(map[string]any)
	}
	var out []caseResult
	for _, c := range file.Cases {
		out = append(out, runCase(c, builtins))
	}
	b, err := json.MarshalIndent(map[string]any{"oracle": "github.com/expr-lang/expr v1.17.9-0.20260412144015-3a46b1970b37 (v1.17.8 + PR #954), Optimize(false), " + runtime.GOARCH, "cases": out}, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(outPath, append(b, '\n'), 0o644)
}

func runCase(c testCase, builtins []expr.Option) caseResult {
	r := caseResult{Name: c.Name}
	// The environment is a struct built at run time, one field per input,
	// named by an `expr` tag. A plain input's field has its schema's Go type
	// (int, float64, bool, string); a nullable input's field is `any`, as a
	// host would declare it. (A map env can't do this: a nil example value
	// gives expr's "unknown" type, which rejects arithmetic at compile time.)
	var fields []reflect.StructField
	for i, in := range c.Inputs {
		t := reflect.TypeOf(zero(in.Schema))
		if in.Nullable {
			t = reflect.TypeOf((*any)(nil)).Elem()
		}
		fields = append(fields, reflect.StructField{
			Name: fmt.Sprintf("F%d", i),
			Type: t,
			Tag:  reflect.StructTag(fmt.Sprintf(`expr:"%s"`, in.Name)),
		})
	}
	envType := reflect.StructOf(fields)
	opts := append([]expr.Option{expr.Env(reflect.New(envType).Elem().Interface()), expr.Optimize(false)}, builtins...)
	for _, name := range c.Builtins {
		b, ok := testBuiltins[name]
		if !ok {
			r.CompileError = "unknown test builtin " + name
			return r
		}
		opts = append(opts, b)
	}
	p, err := expr.Compile(c.Source, opts...)
	if err != nil {
		r.CompileError = firstLine(err.Error())
		return r
	}
	for _, run := range c.Runs {
		env := reflect.New(envType).Elem()
		ok := true
		for i, in := range c.Inputs {
			v, err := value(in, run[in.Name])
			if err != nil {
				r.Results = append(r.Results, result{Error: "bad input: " + err.Error()})
				ok = false
				break
			}
			if v != nil {
				env.Field(i).Set(reflect.ValueOf(v))
			}
		}
		if !ok {
			continue
		}
		v, err := expr.Run(p, env.Interface())
		if err != nil {
			r.Results = append(r.Results, result{Error: firstLine(err.Error())})
		} else {
			r.Results = append(r.Results, result{Ok: encode(v)})
		}
	}
	return r
}

func zero(schema map[string]any) any {
	switch kind(schema) {
	case "integer":
		return 0
	case "number":
		return 0.0
	case "boolean":
		return false
	case "object":
		return map[string]any{}
	case "array":
		return []any{}
	default:
		return ""
	}
}

func kind(schema map[string]any) string {
	// A record of variants (spec §3.2); in Go, a map with one variant's key.
	if _, ok := schema["oneOf"]; ok {
		return "object"
	}
	if _, ok := schema["enum"]; ok {
		return "string"
	}
	if _, ok := schema["const"]; ok {
		return "string"
	}
	t, _ := schema["type"].(string)
	return t
}

// value builds an input from JSON according to its schema: an integer
// becomes int, a number float64.
func value(in input, raw json.RawMessage) (any, error) {
	if len(raw) == 0 || string(raw) == "null" {
		if !in.Nullable {
			return nil, fmt.Errorf("%s is required", in.Name)
		}
		return nil, nil
	}
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	var v any
	if err := dec.Decode(&v); err != nil {
		return nil, err
	}
	return typed(in.Name, in.Schema, v)
}

// typed converts a decoded JSON value by its schema: an integer becomes int,
// a number float64, an object a map with its fields converted (an absent
// optional field is left out, never set to nil).
func typed(path string, schema map[string]any, v any) (any, error) {
	switch kind(schema) {
	case "integer":
		n, ok := v.(json.Number)
		if !ok {
			return nil, fmt.Errorf("%s: want an integer", path)
		}
		i, err := strconv.ParseInt(string(n), 10, 64)
		return int(i), err
	case "number":
		n, ok := v.(json.Number)
		if !ok {
			return nil, fmt.Errorf("%s: want a number", path)
		}
		return n.Float64()
	case "object":
		o, ok := v.(map[string]any)
		if !ok {
			return nil, fmt.Errorf("%s: want an object", path)
		}
		props := properties(schema)
		out := map[string]any{}
		for k, fv := range o {
			fs, ok := props[k].(map[string]any)
			if !ok {
				return nil, fmt.Errorf("%s.%s: not in the schema", path, k)
			}
			if fv == nil {
				continue
			}
			c, err := typed(path+"."+k, fs, fv)
			if err != nil {
				return nil, err
			}
			out[k] = c
		}
		return out, nil
	case "array":
		a, ok := v.([]any)
		if !ok {
			return nil, fmt.Errorf("%s: want a list", path)
		}
		items, _ := schema["items"].(map[string]any)
		out := make([]any, len(a))
		for i, it := range a {
			c, err := typed(fmt.Sprintf("%s[%d]", path, i), items, it)
			if err != nil {
				return nil, err
			}
			out[i] = c
		}
		return out, nil
	default:
		return v, nil
	}
}

// properties gives an object schema's fields: its `properties`, and each
// `oneOf` alternative's one property (a variant, absent unless it's the one).
func properties(schema map[string]any) map[string]any {
	out := map[string]any{}
	if p, ok := schema["properties"].(map[string]any); ok {
		for k, v := range p {
			out[k] = v
		}
	}
	if alts, ok := schema["oneOf"].([]any); ok {
		for _, a := range alts {
			if am, ok := a.(map[string]any); ok {
				if p, ok := am["properties"].(map[string]any); ok {
					for k, v := range p {
						out[k] = v
					}
				}
			}
		}
	}
	return out
}

func encode(v any) map[string]any {
	switch x := v.(type) {
	case nil:
		return map[string]any{"kind": "nil"}
	case bool:
		return map[string]any{"kind": "bool", "bool": x}
	case int:
		return map[string]any{"kind": "int", "int": strconv.FormatInt(int64(x), 10)}
	case float64:
		return map[string]any{"kind": "float", "bits": fmt.Sprintf("%016x", math.Float64bits(x))}
	case string:
		return map[string]any{"kind": "string", "string": x}
	case map[string]any:
		// A nil-valued field counts as absent (spec §9).
		fields := map[string]any{}
		for k, fv := range x {
			if fv != nil {
				fields[k] = encode(fv)
			}
		}
		return map[string]any{"kind": "record", "fields": fields}
	default:
		return map[string]any{"kind": fmt.Sprintf("unsupported %T", v)}
	}
}

// expand replaces each {"$name": true} schema with defs[name], recursively.
func expand(v any, defs map[string]any) any {
	switch x := v.(type) {
	case map[string]any:
		if len(x) == 1 {
			for k := range x {
				if len(k) > 1 && k[0] == '$' {
					if d, ok := defs[k[1:]]; ok {
						return expand(d, defs)
					}
				}
			}
		}
		out := map[string]any{}
		for k, e := range x {
			out[k] = expand(e, defs)
		}
		return out
	case []any:
		out := make([]any, len(x))
		for i, e := range x {
			out[i] = expand(e, defs)
		}
		return out
	default:
		return v
	}
}

func firstLine(s string) string {
	for i, c := range s {
		if c == '\n' {
			return s[:i]
		}
	}
	return s
}
