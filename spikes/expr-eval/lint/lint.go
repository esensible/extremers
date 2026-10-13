// Package lint flags expr features outside the engine dialect (see
// LANGUAGE.md, "Restrictions"). It works on the parse tree only, so it
// infers what it can (string, list, map, int, float) from literals, `let`
// bindings and builtins, and stays quiet where it cannot tell.
//
// Errors fail the check; warnings are printed only.
package lint

import (
	"fmt"
	"regexp"
	"sort"

	"github.com/expr-lang/expr/ast"
	"github.com/expr-lang/expr/parser"
)

type Diag struct {
	File      string
	Line, Col int
	Severity  string // "error" | "warning"
	Rule      string
	Msg       string
}

func (d Diag) String() string {
	return fmt.Sprintf("%s:%d:%d: %s [%s] %s", d.File, d.Line, d.Col, d.Severity, d.Rule, d.Msg)
}

// File lints one engine file. role is "init", "step" or "view". A parse
// error is reported as a single error diagnostic (the compile stage reports
// it in full).
func File(name, src, role string) []Diag {
	tree, err := parser.Parse(src)
	if err != nil {
		return []Diag{{File: name, Line: 1, Col: 1, Severity: "error", Rule: "parse", Msg: err.Error()}}
	}
	l := &linter{name: name, src: []rune(src)}
	l.visit(tree.Node, nil)
	if role == "step" {
		if !l.sawFail {
			l.report(tree.Node, "error", "unknown-input",
				"step never calls fail(...): an input it does not handle must end in fail(\"unknown input \", input.tag)")
		}
		l.checkWake(tree.Node, nil)
	}
	sort.SliceStable(l.diags, func(i, j int) bool {
		if l.diags[i].Line != l.diags[j].Line {
			return l.diags[i].Line < l.diags[j].Line
		}
		return l.diags[i].Col < l.diags[j].Col
	})
	return l.diags
}

type kind int

const (
	kUnknown kind = iota
	kString
	kList
	kMap
	kInt
	kFloat
	kBool
	kNil
)

type binding struct {
	kind     kind
	constInt bool
	value    ast.Node
}

type scope struct {
	name   string
	b      binding
	parent *scope
}

func (s *scope) lookup(name string) (binding, bool) {
	for ; s != nil; s = s.parent {
		if s.name == name {
			return s.b, true
		}
	}
	return binding{}, false
}

type linter struct {
	name    string
	src     []rune
	diags   []Diag
	sawFail bool
}

func (l *linter) report(n ast.Node, sev, rule, msg string) {
	line, col := 1, 1
	from := n.Location().From
	for i := 0; i < from && i < len(l.src); i++ {
		if l.src[i] == '\n' {
			line++
			col = 1
		} else {
			col++
		}
	}
	l.diags = append(l.diags, Diag{File: l.name, Line: line, Col: col, Severity: sev, Rule: rule, Msg: msg})
}

// Builtins outside the dialect, with why.
var banned = map[string]string{
	"now":        "no clock functions: use the step's `now` (epoch ms) or input timestamps",
	"date":       "no clock functions: times are int epoch ms",
	"duration":   "no clock functions: durations are int ms",
	"timezone":   "no clock functions",
	"groupBy":    "groupBy builds a map with dynamic keys",
	"keys":       "no dynamic maps: keys() is outside the dialect",
	"values":     "no dynamic maps: values() is outside the dialect",
	"toPairs":    "no dynamic maps: toPairs() is outside the dialect",
	"fromPairs":  "no dynamic maps: fromPairs() builds dynamic keys",
	"toJSON":     "no JSON: toJSON() builds a string",
	"fromJSON":   "no JSON: fromJSON() parses a string",
	"concat":     "lists grow only through push_last(list, item, N)",
	"string":     "no string building: string() is outside the dialect",
	"trim":       "no string processing",
	"trimPrefix": "no string processing",
	"trimSuffix": "no string processing",
	"upper":      "no string processing",
	"lower":      "no string processing",
	"split":      "no string processing",
	"splitAfter": "no string processing",
	"replace":    "no string processing",
	"repeat":     "no string building",
	"join":       "no string building",
	"indexOf":    "no string processing",
	"lastIndexOf": "no string processing",
	"hasPrefix":  "no string processing",
	"hasSuffix":  "no string processing",
	"toBase64":   "no string processing",
	"fromBase64": "no string processing",
	"type":       "no run-time type inspection",
}

var stringOps = map[string]bool{"matches": true, "contains": true, "startsWith": true, "endsWith": true}

// Builtins (expr's and ours) known to return a float.
var floatFuncs = map[string]bool{
	"float": true, "sqrt": true, "sin": true, "cos": true, "atan2": true, "fmod": true,
	"distance": true, "bearing": true, "angle_diff": true, "mean_heading": true,
	"ceil": true, "floor": true, "round": true, "mean": true, "median": true,
}

var listFuncs = map[string]bool{
	"push_last": true, "map": true, "filter": true, "sort": true, "sortBy": true,
	"reverse": true, "uniq": true, "take": true, "flatten": true,
}

func (l *linter) visit(n ast.Node, sc *scope) kind {
	switch n := n.(type) {
	case nil:
		return kUnknown
	case *ast.NilNode:
		return kNil
	case *ast.IntegerNode:
		return kInt
	case *ast.FloatNode:
		return kFloat
	case *ast.BoolNode:
		return kBool
	case *ast.StringNode:
		return kString
	case *ast.BytesNode:
		l.report(n, "error", "strings", "byte strings are outside the dialect")
		return kUnknown
	case *ast.ConstantNode:
		return kUnknown
	case *ast.IdentifierNode:
		if n.Value == "$env" {
			l.report(n, "error", "dynamic-key", "$env is outside the dialect")
		}
		if b, ok := sc.lookup(n.Value); ok {
			return b.kind
		}
		if n.Value == "now" {
			return kInt
		}
		return kUnknown
	case *ast.PointerNode:
		if n.Name == "index" {
			return kInt
		}
		return kUnknown
	case *ast.UnaryNode:
		k := l.visit(n.Node, sc)
		if n.Operator == "!" || n.Operator == "not" {
			return kBool
		}
		return k
	case *ast.BinaryNode:
		lk := l.visit(n.Left, sc)
		rk := l.visit(n.Right, sc)
		switch n.Operator {
		case "+":
			if lk == kString || rk == kString {
				l.report(n, "error", "string-building", "no string concatenation (strings are literals compared with == and !=; fail() takes several arguments)")
				return kString
			}
			if lk == kList || rk == kList {
				l.report(n, "error", "list-growth", "`+` on lists grows without bound: use push_last(list, item, N)")
				return kList
			}
			return arith(lk, rk)
		case "-", "*", "%":
			return arith(lk, rk)
		case "/", "**", "^":
			return kFloat
		case "??":
			if lk == kUnknown || lk == kNil {
				return rk
			}
			return lk
		case "..":
			return kList
		}
		if stringOps[n.Operator] {
			l.report(n, "error", "strings", fmt.Sprintf("no string processing: `%s`", n.Operator))
		}
		return kBool
	case *ast.ChainNode:
		return l.visit(n.Node, sc)
	case *ast.MemberNode:
		ok := l.visit(n.Node, sc)
		switch p := n.Property.(type) {
		case *ast.StringNode:
			if p.Value == "tag" {
				return kString // input.tag and other union tags
			}
		case *ast.IntegerNode:
			// m.field, m["field"], list[0]
		case *ast.UnaryNode:
			if _, isInt := p.Node.(*ast.IntegerNode); isInt && p.Operator == "-" {
				break // list[-1]
			}
			l.checkKey(n, ok, l.visit(n.Property, sc))
		default:
			l.checkKey(n, ok, l.visit(n.Property, sc))
		}
		return kUnknown
	case *ast.SliceNode:
		k := l.visit(n.Node, sc)
		l.visit(n.From, sc)
		l.visit(n.To, sc)
		return k
	case *ast.CallNode:
		name := ""
		if id, ok := n.Callee.(*ast.IdentifierNode); ok {
			name = id.Value
		} else {
			l.visit(n.Callee, sc)
		}
		return l.call(n, name, n.Arguments, sc)
	case *ast.BuiltinNode:
		return l.call(n, n.Name, n.Arguments, sc)
	case *ast.PredicateNode:
		return l.visit(n.Node, sc)
	case *ast.ConditionalNode:
		l.visit(n.Cond, sc)
		a := l.visit(n.Exp1, sc)
		b := l.visit(n.Exp2, sc)
		return join(a, b)
	case *ast.VariableDeclaratorNode:
		k := l.visit(n.Value, sc)
		inner := &scope{name: n.Name, b: binding{kind: k, constInt: l.constInt(n.Value, sc), value: n.Value}, parent: sc}
		return l.visit(n.Expr, inner)
	case *ast.SequenceNode:
		k := kUnknown
		for _, x := range n.Nodes {
			k = l.visit(x, sc)
		}
		return k
	case *ast.ArrayNode:
		for _, x := range n.Nodes {
			l.visit(x, sc)
		}
		return kList
	case *ast.MapNode:
		for _, p := range n.Pairs {
			pair := p.(*ast.PairNode)
			key, literal := pair.Key.(*ast.StringNode)
			if !literal {
				l.report(pair.Key, "error", "dynamic-key", "map literal keys must be plain names or string literals")
				l.visit(pair.Key, sc)
			}
			vk := l.visit(pair.Value, sc)
			if literal && vk == kFloat && timeLike.MatchString(key.Value) {
				l.report(pair.Value, "error", "float-time", fmt.Sprintf("`%s` looks like a time or ms duration but is a float: times are int epoch ms (wrap in int(...); `/` always gives a float)", key.Value))
			}
		}
		return kMap
	case *ast.PairNode:
		l.visit(n.Key, sc)
		return l.visit(n.Value, sc)
	}
	return kUnknown
}

// Field names that hold a time (epoch ms) or a duration in ms.
// (`wake` itself is checked by checkWake.)
var timeLike = regexp.MustCompile(`(?i)(^|_)(ts|timestamp|at|since|until|deadline|cutoff|eta|start|started|start_time|end_time)$|_ms$`)

func arith(a, b kind) kind {
	if a == kFloat || b == kFloat {
		return kFloat
	}
	if a == kInt && b == kInt {
		return kInt
	}
	return kUnknown
}

func join(a, b kind) kind {
	switch {
	case a == b:
		return a
	case a == kNil:
		return b
	case b == kNil:
		return a
	case a == kFloat || b == kFloat:
		return kFloat // a float on either branch may become a time
	}
	return kUnknown
}

// checkKey: m[k] with a computed k. A number indexes a list (fine); a
// string, or anything indexing a known map, is a dynamic map key.
func (l *linter) checkKey(n ast.Node, obj, key kind) {
	switch {
	case key == kString || obj == kMap:
		l.report(n, "error", "dynamic-key", "no dynamic map keys: use m.field (or m[\"field\"]); only lists take a computed index")
	case key == kInt || obj == kList:
	default:
		l.report(n, "warning", "dynamic-key", "computed index: fine on a list, outside the dialect on a map")
	}
}

func (l *linter) call(n ast.Node, name string, args []ast.Node, sc *scope) kind {
	if why, ok := banned[name]; ok {
		l.report(n, "error", "banned", fmt.Sprintf("%s(): %s", name, why))
	}
	kinds := make([]kind, len(args))
	for i, a := range args {
		kinds[i] = l.visit(a, sc)
	}
	switch name {
	case "fail":
		l.sawFail = true
		return kUnknown
	case "push_last":
		if len(args) != 3 {
			l.report(n, "error", "list-growth", "push_last(list, item, N) takes 3 arguments")
		} else if !l.constInt(args[2], sc) {
			l.report(args[2], "error", "list-growth", "push_last's cap N must be a constant integer (a literal, or a let bound to one)")
		}
		return kList
	case "get":
		if len(args) == 2 {
			_, str := args[1].(*ast.StringNode)
			_, num := args[1].(*ast.IntegerNode)
			if !str && !num {
				l.checkKey(n, kinds[0], kinds[1])
			}
		}
		return kUnknown
	case "flatten":
		l.report(n, "warning", "list-growth", "flatten() can grow a list: keep lists in state capped with push_last")
		return kList
	case "int", "len", "count", "findIndex", "findLastIndex":
		return kInt
	case "abs":
		if len(kinds) == 1 {
			return kinds[0]
		}
	case "max", "min":
		k := kUnknown
		for i, x := range kinds {
			if i == 0 {
				k = x
			} else {
				k = arith(k, x)
			}
		}
		return k
	case "all", "any", "none", "one":
		return kBool
	}
	if floatFuncs[name] {
		return kFloat
	}
	if listFuncs[name] {
		return kList
	}
	return kUnknown
}

// constInt: an integer known when the file is written.
func (l *linter) constInt(n ast.Node, sc *scope) bool {
	switch n := n.(type) {
	case *ast.IntegerNode:
		return true
	case *ast.IdentifierNode:
		b, ok := sc.lookup(n.Value)
		return ok && b.constInt
	case *ast.UnaryNode:
		return n.Operator == "-" && l.constInt(n.Node, sc)
	case *ast.BinaryNode:
		switch n.Operator {
		case "+", "-", "*", "%":
			return l.constInt(n.Left, sc) && l.constInt(n.Right, sc)
		}
	}
	return false
}

// checkWake finds the step's result map(s) and checks `wake` is not a float.
func (l *linter) checkWake(n ast.Node, sc *scope) {
	switch n := n.(type) {
	case *ast.VariableDeclaratorNode:
		k := l.quietKind(n.Value, sc)
		l.checkWake(n.Expr, &scope{name: n.Name, b: binding{kind: k, constInt: l.constInt(n.Value, sc), value: n.Value}, parent: sc})
	case *ast.SequenceNode:
		if len(n.Nodes) > 0 {
			l.checkWake(n.Nodes[len(n.Nodes)-1], sc)
		}
	case *ast.ConditionalNode:
		l.checkWake(n.Exp1, sc)
		l.checkWake(n.Exp2, sc)
	case *ast.MapNode:
		for _, p := range n.Pairs {
			pair := p.(*ast.PairNode)
			if key, ok := pair.Key.(*ast.StringNode); ok && key.Value == "wake" {
				if l.quietKind(pair.Value, sc) == kFloat {
					l.report(pair.Value, "error", "float-time", "wake is a float: it must be an int epoch ms (or nil)")
				}
			}
		}
	}
}

// quietKind: a kind without reporting (the main pass has reported).
func (l *linter) quietKind(n ast.Node, sc *scope) kind {
	saved, savedFail := l.diags, l.sawFail
	k := l.visit(n, sc)
	l.diags, l.sawFail = saved, savedFail
	return k
}
