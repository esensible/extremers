// check: the engine validator.
//
//	check [--no-tests] <task> <dir>
//
// Stages: (a) compile <dir>/engine.js (syntax errors as file:line:col), load
// it with the harness (top level, then init()) and check the initial view
// against <task>/view.schema.json; (b) language lint; (c) the hidden tests
// (skipped with --no-tests). Exits non-zero if any stage fails. Run from the
// kit root, or set JS_EVAL_ROOT; a relative <dir> is taken against
// CHECK_PWD if set (the ./check wrapper sets it), else the working
// directory.
package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"

	"js-eval/lint"
	"js-eval/schema"

	engines "js-engines"
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
	root := os.Getenv("JS_EVAL_ROOT")
	if root == "" {
		root, _ = os.Getwd()
	}

	ok := true
	fmt.Printf("== (a) compile: %s\n", filepath.Join(dir, engines.File))
	if compile(root, task, dir) {
		fmt.Println("ok")
	} else {
		ok = false
	}

	fmt.Println("== (b) language lint")
	if language(dir) {
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

func compile(root, task, dir string) bool {
	src, err := os.ReadFile(filepath.Join(dir, engines.File))
	if err != nil {
		fmt.Println(err)
		return false
	}
	if _, err := engines.Compile(engines.File, string(src)); err != nil {
		fmt.Println(err)
		return false
	}
	// The harness's own load (compiles again, runs the top level and
	// init()), then the initial view against the task's schema.
	e, err := engines.Load(dir)
	if err != nil {
		fmt.Println(err)
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
		fmt.Printf("%s: the initial view does not match %s/view.schema.json: %v\n", engines.File, task, err)
		return false
	}
	return true
}

func language(dir string) bool {
	src, err := os.ReadFile(filepath.Join(dir, engines.File))
	if err != nil {
		fmt.Println(err)
		return false
	}
	ok := true
	for _, d := range lint.File(engines.File, string(src)) {
		fmt.Println(d)
		ok = false
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
