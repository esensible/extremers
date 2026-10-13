package evaltests

// LANGUAGE.md's example engine, kept honest: it lints clean and does what
// the document says.

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	engines "expr-engines"

	"expr-eval/lint"
)

func TestExampleSteps(t *testing.T) {
	dir := filepath.Join("..", "example", "steps")
	for _, f := range []string{"init.expr", "step.expr", "view.expr"} {
		src, err := os.ReadFile(filepath.Join(dir, f))
		if err != nil {
			t.Fatal(err)
		}
		for _, d := range lint.File(f, string(src), strings.TrimSuffix(f, ".expr")) {
			t.Errorf("lint: %v", d)
		}
	}
	e, err := engines.Load(dir)
	if err != nil {
		t.Fatal(err)
	}
	for _, at := range []int{1000, 1500, 2000} {
		if _, err := e.Step(at, map[string]any{"tag": "Step"}); err != nil {
			t.Fatal(err)
		}
	}
	if w, ok := e.NextWake(); !ok || w != 12000 {
		t.Fatalf("wake: got %d %v, want 12000", w, ok)
	}
	v, _ := e.View()
	if v["count"] != 3 || v["idle"] != false || v["cadence_spm"] != 120.0 {
		t.Fatalf("view: %v", v)
	}
	if _, _, err := e.AdvanceTo(12000); err != nil {
		t.Fatal(err)
	}
	v, _ = e.View()
	if v["idle"] != true {
		t.Fatalf("after the wake: %v", v)
	}
	if _, ok := e.NextWake(); ok {
		t.Fatalf("idle: want no wake")
	}
	if _, err := e.Step(13000, map[string]any{"tag": "Bogus"}); err == nil {
		t.Fatalf("Bogus must fail")
	}
}
