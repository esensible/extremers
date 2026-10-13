// check: the engine validator.
//
//	check [--no-tests] <task> <dir>
//
// Stages: (a) compile init/step/view with expr and the engine builtins,
// evaluate init and check its view against <task>/view.schema.json;
// (b) dialect lint; (c) the hidden tests (skipped with --no-tests). Exits
// non-zero if any stage fails. Run from the kit root, or set
// EXPR_EVAL_ROOT; a relative <dir> is taken against CHECK_PWD if set (the
// ./check wrapper sets it), else the working directory.
package main

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"expr-eval/lint"
	"expr-eval/schema"

	engines "expr-engines"

	"github.com/expr-lang/expr"
	"github.com/expr-lang/expr/file"
)

var tasks = map[string]string{"running": "Running", "riding": "Riding", "vendee": "Vendee", "ultra": "Ultra"}

func main() {
	args := os.Args[1:]
	if len(args) > 0 && args[0] == "check" {
		args = args[1:] // `check check <task> <dir>` reads oddly but is accepted
	}
	noTests := false
	var pos []string
	for _, a := range args {
		if a == "--no-tests" {
			noTests = true
		} else {
			pos = append(pos, a)
		}
	}
	if len(pos) != 2 || tasks[pos[0]] == "" {
		fmt.Fprintln(os.Stderr, "usage: check [--no-tests] <running|riding|vendee|ultra> <engine dir>")
		os.Exit(2)
	}
	task, dir := pos[0], pos[1]
	if !filepath.IsAbs(dir) {
		if base := os.Getenv("CHECK_PWD"); base != "" {
			dir = filepath.Join(base, dir)
		}
	}
	dir, _ = filepath.Abs(dir)
	root := os.Getenv("EXPR_EVAL_ROOT")
	if root == "" {
		root, _ = os.Getwd()
	}

	ok := true
	fmt.Printf("== (a) compile: %s\n", dir)
	if compile(root, task, dir) {
		fmt.Println("ok")
	} else {
		ok = false
	}

	fmt.Println("== (b) dialect lint")
	if dialect(dir) {
		fmt.Println("ok")
	} else {
		ok = false
	}

	if noTests {
		fmt.Println("== (c) hidden tests: skipped (--no-tests)")
	} else {
		fmt.Println("== (c) hidden tests")
		if !ok {
			fmt.Println("not run: fix (a) and (b) first")
		} else if !hidden(root, task, dir) {
			ok = false
		}
	}

	if ok {
		fmt.Println("PASS")
		return
	}
	fmt.Println("FAIL")
	os.Exit(1)
}

var envs = map[string]map[string]any{
	"init.expr": {},
	"step.expr": {"state": map[string]any{}, "input": map[string]any{}, "now": int(0)},
	"view.expr": {"state": map[string]any{}},
}

func compile(root, task, dir string) bool {
	ok := true
	for _, f := range []string{"init.expr", "step.expr", "view.expr"} {
		src, err := os.ReadFile(filepath.Join(dir, f))
		if err != nil {
			fmt.Printf("%s: %v\n", f, err)
			ok = false
			continue
		}
		opts := append([]expr.Option{expr.Env(envs[f])}, engines.Builtins()...)
		if _, err := expr.Compile(string(src), opts...); err != nil {
			var fe *file.Error
			if errors.As(err, &fe) {
				fmt.Printf("%s:%d:%d: %s%s\n", f, fe.Line, fe.Column+1, fe.Message, fe.Snippet)
			} else {
				fmt.Printf("%s: %v\n", f, err)
			}
			ok = false
		}
	}
	if !ok {
		return false
	}
	// The harness's own load (compiles again, evaluates init), then the
	// initial view against the task's schema.
	e, err := engines.Load(dir)
	if err != nil {
		fmt.Println(err)
		return false
	}
	if _, isMap := e.State.(map[string]any); !isMap {
		fmt.Printf("init.expr: evaluates to %T, not a map\n", e.State)
		return false
	}
	v, err := e.View()
	if err != nil {
		fmt.Println(err)
		return false
	}
	s, err := schema.Load(filepath.Join(root, task, "view.schema.json"))
	if err != nil {
		fmt.Println(err)
		return false
	}
	if err := s.Validate(v); err != nil {
		fmt.Printf("view.expr: the initial view does not match %s/view.schema.json: %v\n", task, err)
		return false
	}
	return true
}

func dialect(dir string) bool {
	ok := true
	for _, f := range []string{"init.expr", "step.expr", "view.expr"} {
		src, err := os.ReadFile(filepath.Join(dir, f))
		if err != nil {
			fmt.Printf("%s: %v\n", f, err)
			ok = false
			continue
		}
		for _, d := range lint.File(f, string(src), strings.TrimSuffix(f, ".expr")) {
			fmt.Println(d)
			if d.Severity == "error" {
				ok = false
			}
		}
	}
	return ok
}

func hidden(root, task, dir string) bool {
	if _, err := os.Stat(filepath.Join(root, "tests")); err != nil {
		fmt.Println("hidden tests are not in this copy of the kit")
		return false
	}
	cmd := exec.Command("go", "test", "./tests", "-count=1", "-v", "-run", "^Test"+tasks[task])
	cmd.Dir = root
	cmd.Env = append(os.Environ(), "ENGINE_DIR="+dir)
	cmd.Stdout, cmd.Stderr = os.Stdout, os.Stderr
	return cmd.Run() == nil
}
