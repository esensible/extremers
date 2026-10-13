package lint

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	engines "js-engines"
)

// wrap puts `body` in a step that otherwise lints clean.
func wrap(top, body string) string {
	return top + `
function init() { return { n: 0, list: [] } }
function step(state, input, now) {
` + body + `
  throw new Error("unknown input " + input.tag);
}
function view(state) { return { n: state.n } }
`
}

func lintOf(t *testing.T, src string) []Diag {
	t.Helper()
	return File("engine.js", src)
}

func TestCleanEngines(t *testing.T) {
	for _, dir := range []string{
		"../../js-engines/race", "../../js-engines/tune",
		"../reference/running", "../reference/riding", "../reference/vendee", "../reference/ultra",
		"../example/steps",
	} {
		src, err := os.ReadFile(filepath.Join(dir, "engine.js"))
		if err != nil {
			t.Fatal(err)
		}
		for _, d := range lintOf(t, string(src)) {
			t.Errorf("%s: %v", dir, d)
		}
	}
}

// Everything LANGUAGE.md allows lints clean.
func TestAllowed(t *testing.T) {
	src := wrap(`
const K = 3;
const LIST = [1, 2, 3];
const helper = (x, y = 2) => x * y;
function total(xs) { return xs.reduce((a, b) => a + b, 0); }
function small(x) { return x <= 2; }
`, `
  const sums = sum(state.list) + sum(LIST, (x) => x * 2) + total(LIST) + count(LIST, small) + (mean(LIST) ?? 0)
    + (mean([], (x, i) => x + i) ?? 0) + (median(LIST, helper) ?? 0) + (first(LIST) ?? 0) + (last(LIST) ?? 0)
    + take(LIST, 2).length + pairs(LIST).map(([a, b]) => b - a).length + track_distance([{ lat: 0, lon: 0 }, { lat: 0, lon: 1 }])
    + (min_by(LIST, (x) => -x) ?? 0) + (max_by([{ a: 1 }], (o, i) => o.a + i)?.a ?? 0);
  const totalled = [[0, { a: 1 }]].map(([i, { a, b = 1 }]) => i + a + b);
  const { n, ...rest } = state;
  const [head, ...others] = LIST;
  const s = `+"`n=${n}`"+`;
  const big = Math.max(...LIST, Math.hypot(3, 4), Math.PI);
  const pos = input.fix?.altitude ?? null;
  const later = state.list.at(-1) ?? 0;
  const sorted = LIST.toSorted((a, b) => b - a).toReversed();
  const found = LIST.find(helper) ?? LIST.findLast((x) => x > 1);
  const kind = typeof input.x === "number" ? 1 : 0;
  const arr = [...LIST.slice(1), ...LIST.concat([4])].filter((x) => LIST.includes(x) && LIST.indexOf(x) >= 0);
  const ok = LIST.some((x) => x > 1) && LIST.every((x) => x > 0) && LIST.findIndex((x) => x === 2) >= 0;
  const d = distance({ lat: 0, lon: 0 }, { lat: 1, lon: 1 }) + angle_diff(1, 2) + total(arr);
  const keep = push_last(state.list, d, K);
  const mode = s < "b" ? "b" : "c";
  const tag = input.tag;
  switch (tag) {
    case "Go": {
      const next = { ...state, n: state.n + 1 + sums + totalled[0] + big + later + kind + (ok ? 1 : 0) + s.length + head + others.length + rest.list.length + found + sorted.length + (pos ?? 0), list: keep };
      return { state: next, wake: Infinity === 1 || undefined ? null : now + 1000 };
    }
    case "Wake":
      return { state, wake: null };
    case "Other":
    case "More": {
      const f = function (x) { return x + 1; };
      return { state: { ...state, n: mode === "b" ? f(1) + [1, 2].map(f)[0] + (() => 2)() : 0 }, wake: null };
    }
    case "Nested":
      switch (mode) {
        case "b":
          return { state, wake: null };
        default:
          throw new Error("nested");
      }
  }`)
	for _, d := range lintOf(t, src) {
		t.Errorf("%v", d)
	}
	// and it runs
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, engines.File), []byte(src), 0o644); err != nil {
		t.Fatal(err)
	}
	e, err := engines.Load(dir)
	if err != nil {
		t.Fatal(err)
	}
	for _, tag := range []string{"Go", "Wake", "Other"} {
		if _, err := e.Step(5, map[string]any{"tag": tag, "fix": map[string]any{"lat": 1, "lon": 2}}); err != nil {
			t.Fatalf("%s: %v", tag, err)
		}
	}
	if w, ok := e.NextWake(); ok {
		t.Fatalf("wake %d", w)
	}
}

func TestForbidden(t *testing.T) {
	for _, c := range []struct{ top, body, rule, msg string }{
		{"", "var x = 1;", "var", "use `const`"},
		{"", "while (state.n < 3) {}", "loop", "`sum(list, fn)`"},
		{"", "do {} while (false);", "loop", "`do ... while`"},
		{"", "for (let i = 0; i < 3; i++) {}", "loop", "`pairs(list)`"},
		{"", "for (const k in state) {}", "loop", "`for ... in`"},
		{"", "for (const x of state.list) {}", "loop", "`for ... of` isn't allowed"},
		{"", "for (const x of state.list) {}", "loop", "list.reduce((acc, x) => ..., start)"},
		{"", "let x; for (x of state.list) {}", "loop", "`for ... of`"},
		{"", "let x = 1;", "let", "`const x = cond ? b : a`, not `let x = a; if (cond) x = b;`"},
		{"let counter = 0;", "", "let", "belongs in the state"},
		{"", "let x = state.n; if (input.y) x = 2;", "mutation", "`const x = cond ? 2 : ...` rather than `let x = ...; if (cond) x = 2;`"},
		{"", "let x = state.n; if (input.y) x = 2;", "mutation", "chain the ternary, `const x = c1 ? a : c2 ? b : c`"},
		{"", "let x = state.n; if (input.y) x = 2;", "mutation", "helper function with a `switch` that returns the value from each case"},
		{"", "if (input.y) { return { state, wake: null }; }", "if", "`if` isn't available"},
		{"", "if (input.y) { return { state, wake: null }; }", "if", "`const x = cond ? a : b`"},
		{"", "if (input.y) { return { state, wake: null }; }", "if", "`return cond ? a : b`"},
		{"", "if (input.y) { return { state, wake: null }; }", "if", "`return c1 ? a : c2 ? b : c`"},
		{"", "if (input.y) { return { state, wake: null }; }", "if", "`switch` that returns from each case"},
		{"", "if (input.y) { state.n = 1; }", "mutation", "changes `state` in place"},
		{"", "switch (input.tag) { case \"A\": break; }", "switch", "`break` isn't available"},
		{"", "switch (input.tag) { case \"A\": { const y = 1; } case \"B\": return { state, wake: null }; }", "switch", "every case ends in `return` or `throw`"},
		{"", "switch (input.tag) { case \"A\": return { state, wake: null }; case \"B\": }", "switch", "no falling through"},
		{"", "switch (input.tag) { case \"A\": return { state, wake: null }; default: const z = 1; }", "switch", "every case ends"},
		{"", "const total = 0; for (const x of state.list) { total += x; }", "mutation", "`total += x` changes `total`"},
		{"", "const total = 0; for (const x of state.list) { total += x; }", "mutation", "`const total = sum(list, (x) => ...)`"},
		{"", "const total = 0; for (const x of state.list) { total += x; }", "mutation", "list.reduce((acc, x) => acc + ..., start)"},
		{"", "const n = 0; n++;", "mutation", "`n++` changes `n`"},
		{"", "const n = 0; n++;", "mutation", "Write `n + 1`"},
		{"", "const n = 0; n -= 2;", "mutation", "const next = n - 2"},
		{"", "const f = (a) => { a = 2; return a; };", "mutation", "`a` can't be reassigned"},
		{"", "const next = { ...state }; next.n = state.n + 1;", "mutation", "return `{ ...next, n: state.n + 1 }`"},
		{"", "state.n = 1;", "mutation", "Make a new object instead, e.g. return `{ ...state, n: 1 }`"},
		{"", "state.n += 2;", "mutation", "`{ ...state, n: state.n + 2 }`"},
		{"", "state.list[0] = 1;", "mutation", "state.list.map((x, j) => j === 0 ? 1 : x)"},
		{"", "state.n++;", "mutation", "`{ ...state, n: state.n + 1 }`"},
		{"", "state.list.push(1);", "mutation", "`[...list, item]`"},
		{"", "state.list.sort();", "mutation", "toSorted"},
		{"", "state.list.reverse();", "mutation", "toReversed"},
		{"", "state.list.splice(0, 1);", "mutation", "slice"},
		{"", "state.list.forEach((x) => x);", "mutation", "map/filter/reduce"},
		{"", "delete state.n;", "mutation", "`delete`"},
		{"", "const x = 1; x = 2;", "mutation", "`x` can't be reassigned"},
		{"", "const f = (g) => sum(state.list, g);", "call", "pass the callback"},
		{"", "const sum = 3;", "reserved-name", "`sum` is a builtin"},
		{"", "const s = state.list.join(',');", "method", "`.join()` isn't available"},
		{"", "const d = Date.now();", "global", "no clock"},
		{"", "const o = Object.keys(state);", "global", "Object isn't available"},
		{"", "const j = JSON.stringify(state);", "global", "JSON"},
		{"", "console.log(1);", "global", "console"},
		{"", "const r = Math.random();", "global", "Math.random"},
		{"", "const r = Math.log(2);", "global", "Math.log isn't available"},
		{"", "const r = sqrt(2);", "global", "Math.sqrt"},
		{"", "const r = fmod(2, 1);", "global", "x % y"},
		{"", "const r = mystery(2);", "global", "isn't declared"},
		{"", "const r = this.n;", "this", "`this`"},
		{"", "const r = new Map();", "new", "`new` is only for"},
		{"", "try { } catch (e) { }", "try", "`try`"},
		{"", "throw 'oops';", "throw", "throw new Error"},
		{"", "const r = /a/.test('a');", "regex", "regular expressions"},
		{"", "const o = { f: (x) => x };", "plain-data", "functions can't be kept in objects"},
		{"", "const o = { f(x) { return x; } };", "plain-data", "methods"},
		{"", "const o = { get x() { return 1; } };", "plain-data", "getters"},
		{"", "const f = (g) => g(1);", "call", "can't be called"},
		{"", "const f = (g) => [1].map(g);", "call", "pass the callback"},
		{"", "const distance = 3;", "reserved-name", "`distance` is a builtin"},
		{"", "const f = async () => 1;", "async", "`async`"},
		{"", "const c = class {};", "class", "classes"},
		{"", "const x = state.constructor;", "plain-data", "`.constructor`"},
		{"state = 1;", "", "top-level", "only `const` declarations"},
		{"function f(x) { return f(x - 1); }", "", "recursion", "`f` calls itself"},
		{"function a(x) { return b(x); }\nfunction b(x) { return [x].map(a); }", "", "recursion", "`a` and `b` call each other"},
		{"const g = (x) => [x].map(g);", "", "recursion", "`g` calls itself"},
		{"function g(xs) { return sum(xs, g); }", "", "recursion", "`g` calls itself"},
		{"", "const h = function me(x) { return me(x); };", "recursion", "`h` calls itself"},
		{"", "const h = [1].map(function me(x) { return me(x); });", "recursion", "`me` calls itself"},
		{"", "const h = (x) => { const k = (y) => h(y); return k(x); };", "recursion", "`h` and `k` call each other"},
	} {
		ds := lintOf(t, wrap(c.top, c.body))
		found := false
		for _, d := range ds {
			if d.Rule == c.rule && strings.Contains(d.Msg, c.msg) {
				found = true
			}
		}
		if !found {
			t.Errorf("%q %q: want [%s] %q, got %v", c.top, c.body, c.rule, c.msg, ds)
		}
	}
}

// An if / else if / else chain is one error; a case ending in break is one
// error (the break), not two.
func TestOneErrorEach(t *testing.T) {
	for _, body := range []string{
		"if (input.a) { return { state, wake: null }; } else if (input.b) { return { state, wake: null }; } else if (input.c) { return { state, wake: null }; } else { return { state, wake: null }; }",
		"switch (input.tag) { case \"A\": const y = 1; break; default: return { state, wake: null }; }",
	} {
		ds := lintOf(t, wrap("", body))
		if len(ds) != 1 {
			t.Errorf("%q: want one error, got %v", body, ds)
		}
	}
	// a nested if is its own if
	ds := lintOf(t, wrap("", "if (input.a) { if (input.b) { return { state, wake: null }; } }"))
	if len(ds) != 2 {
		t.Errorf("nested if: want two errors, got %v", ds)
	}
}

func TestContract(t *testing.T) {
	ds := lintOf(t, "function init() { return {} }\nconst view = (s) => s;\n")
	want := []string{"function step(state, input, now)", "function view(state)"}
	for _, w := range want {
		found := false
		for _, d := range ds {
			if strings.Contains(d.Msg, w) {
				found = true
			}
		}
		if !found {
			t.Errorf("want %q in %v", w, ds)
		}
	}
	ds = lintOf(t, "function init() { return {} }\nfunction step(s, i, n) { return { state: s, wake: null } }\nfunction view(s) { return s }\n")
	if len(ds) != 1 || ds[0].Rule != "unknown-input" {
		t.Errorf("want only unknown-input, got %v", ds)
	}
}

func TestSyntaxPosition(t *testing.T) {
	ds := lintOf(t, "function init() {\n  return { a: 1 \n}\n")
	if len(ds) != 1 || ds[0].Rule != "syntax" || ds[0].Line < 2 {
		t.Errorf("want one syntax error past line 1, got %v", ds)
	}
}

func TestPositions(t *testing.T) {
	ds := lintOf(t, wrap("", "  state.n = 1;"))
	if len(ds) != 1 || ds[0].Line != 4 || ds[0].Col != 3 {
		t.Errorf("want 4:3, got %v", ds)
	}
}
