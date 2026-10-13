// Package lint holds an engine's engine.js to the language in LANGUAGE.md:
// a strict subset of JavaScript. It works on goja's parse tree. What is not
// listed there is reported, with what to write instead.
//
// The rules that keep an engine pure and bounded:
//   - only `const` and `function` declarations at the top level;
//   - every binding is immutable: `const` only (no `let`), and nothing is
//     ever reassigned (`=`, `+=`, `++`), parameters included;
//   - no `if`: branching is `? :` (and `??`, `?.`), or a `switch` whose
//     every case ends in `return` or `throw` (no `break`, no fall-through);
//   - no loops at all, and no recursion (direct, or through other
//     functions: the call graph must have no cycle): iteration is array
//     methods and the list builtins;
//   - only functions declared in the file, the builtins and Math.* can be
//     called, and callbacks (to array methods and builtins) are written
//     inline or named, so a function value can never reach itself;
//   - nothing is changed in place: no assignment to properties or indexes,
//     no mutating array methods, no `delete`;
//   - no globals beyond Math, Error (for `throw new Error(...)`), undefined,
//     Infinity and the builtins.
//
// Every diagnostic is an error; there are no warnings.
package lint

import (
	"errors"
	"fmt"
	"sort"
	"strings"

	engines "js-engines"

	"github.com/dop251/goja/ast"
	"github.com/dop251/goja/file"
	"github.com/dop251/goja/token"
)

type Diag struct {
	File      string
	Line, Col int
	Severity  string // always "error"
	Rule      string
	Msg       string
}

func (d Diag) String() string {
	return fmt.Sprintf("%s:%d:%d: %s [%s] %s", d.File, d.Line, d.Col, d.Severity, d.Rule, d.Msg)
}

// File lints an engine's source. A syntax error is reported alone.
func File(name, src string) []Diag {
	prg, err := engines.Parse(name, src)
	if err != nil {
		var se *engines.SourceError
		if errors.As(err, &se) {
			return []Diag{{File: name, Line: se.Line, Col: se.Col, Severity: "error", Rule: "syntax", Msg: se.Msg}}
		}
		return []Diag{{File: name, Line: 1, Col: 1, Severity: "error", Rule: "syntax", Msg: err.Error()}}
	}
	l := &linter{name: name, src: src, file: prg.File, builtins: map[string]bool{}}
	for _, b := range engines.BuiltinNames() {
		l.builtins[b] = true
	}
	l.program(prg)
	sort.SliceStable(l.diags, func(i, j int) bool {
		if l.diags[i].Line != l.diags[j].Line {
			return l.diags[i].Line < l.diags[j].Line
		}
		return l.diags[i].Col < l.diags[j].Col
	})
	return l.diags
}

// ---- tables ----

var mathMembers = set("abs", "min", "max", "round", "floor", "ceil", "trunc", "sign", "sqrt", "hypot",
	"sin", "cos", "tan", "asin", "acos", "atan", "atan2", "PI")

const mathList = "abs, min, max, round, floor, ceil, trunc, sign, sqrt, hypot, sin, cos, tan, asin, acos, atan, atan2 and PI"

// Array methods that don't change the array. Strings answer to the ones
// they share (slice, at, concat, includes, indexOf).
var methods = set("map", "filter", "reduce", "find", "findIndex", "findLast", "findLastIndex",
	"some", "every", "slice", "at", "concat", "includes", "indexOf", "toSorted", "toReversed")

const methodList = "map, filter, reduce, find, findIndex, findLast, findLastIndex, some, every, slice, at, concat, includes, indexOf, toSorted and toReversed (and .length)"

// Methods whose first argument is called back.
var callbackMethods = set("map", "filter", "reduce", "find", "findIndex", "findLast", "findLastIndex", "some", "every", "toSorted")

var mutators = map[string]string{
	"push":       "`.push()` changes the array: write `[...list, item]`, or `push_last(list, item, n)` to keep only the last n",
	"pop":        "`.pop()` changes the array: read `list.at(-1)` and keep `list.slice(0, -1)`",
	"shift":      "`.shift()` changes the array: read `list[0]` and keep `list.slice(1)`",
	"unshift":    "`.unshift()` changes the array: write `[item, ...list]`",
	"splice":     "`.splice()` changes the array: build a new one with `slice`, spread and `filter`",
	"sort":       "`.sort()` changes the array: write `list.toSorted((a, b) => a - b)`",
	"reverse":    "`.reverse()` changes the array: write `list.toReversed()`",
	"fill":       "`.fill()` changes the array: write `list.map(() => value)`",
	"copyWithin": "`.copyWithin()` changes the array: build a new one with `slice` and spread",
	"forEach":    "`.forEach()` isn't available: compute the result from the list with map/filter/reduce or a list builtin (`sum`, `count`, `pairs`, ...)",
}

var globals = map[string]string{
	"Date":            "there is no clock: time comes from `now` and the inputs' timestamps (epoch ms)",
	"JSON":            "JSON isn't available: state, inputs and views are plain objects already",
	"Object":          "Object isn't available: copy and extend objects with spread, `{ ...obj, x: 1 }`",
	"Array":           "Array isn't available: build arrays with literals, spread, map and filter",
	"Number":          "Number isn't available: use Math.round/Math.trunc, and `x == null` for a missing value",
	"parseInt":        "parseInt isn't available: numbers arrive as numbers; use Math.trunc to drop a fraction",
	"parseFloat":      "parseFloat isn't available: numbers arrive as numbers",
	"isNaN":           "isNaN isn't available: test for a missing value with `x == null`",
	"isFinite":        "isFinite isn't available: test for a missing value with `x == null`",
	"NaN":             "NaN isn't available: use null for a missing number",
	"String":          "String isn't available: build text with a template literal, `${x}`",
	"Boolean":         "Boolean isn't available: compare explicitly (`x != null`, `x > 0`)",
	"console":         "console isn't available: an engine's only output is its view",
	"setTimeout":      "timers aren't available: return `wake` (epoch ms) and a Wake input comes then",
	"setInterval":     "timers aren't available: return `wake` (epoch ms) and a Wake input comes then",
	"Map":             "Map isn't available: use plain objects and arrays",
	"Set":             "Set isn't available: use arrays (`includes`, `filter`)",
	"WeakMap":         "WeakMap isn't available: use plain objects and arrays",
	"WeakSet":         "WeakSet isn't available: use plain objects and arrays",
	"Symbol":          "Symbol isn't available",
	"Promise":         "Promise isn't available: engines are synchronous",
	"Proxy":           "Proxy isn't available",
	"Reflect":         "Reflect isn't available",
	"eval":            "eval isn't available",
	"Function":        "Function isn't available: declare functions with `function` or `=>`",
	"globalThis":      "globalThis isn't available: there is no global state",
	"window":          "window isn't available: there is no global state",
	"self":            "self isn't available: there is no global state",
	"global":          "global isn't available: there is no global state",
	"require":         "require isn't available: everything is in engine.js",
	"arguments":       "`arguments` isn't available: name the parameters",
	"fmod":            "fmod isn't available: write `x % y` (the remainder has the sign of x)",
	"TypeError":       "only `throw new Error(message)` is available",
	"RangeError":      "only `throw new Error(message)` is available",
	"structuredClone": "structuredClone isn't available: values are never changed in place, so there is nothing to copy",
}

var forbiddenProps = set("constructor", "__proto__", "prototype")

func set(xs ...string) map[string]bool {
	m := map[string]bool{}
	for _, x := range xs {
		m[x] = true
	}
	return m
}

// ---- scopes and the call graph ----

type bkind int

const (
	bConst bkind = iota
	bLet
	bParam
	bFunc // a function declaration, or a const bound to a function literal
)

type binding struct {
	kind bkind
	fn   *fnNode
}

type scope struct {
	names  map[string]*binding
	parent *scope
}

func (s *scope) lookup(name string) *binding {
	for ; s != nil; s = s.parent {
		if b, ok := s.names[name]; ok {
			return b
		}
	}
	return nil
}

// fnNode: a named function in the call graph. Any reference to a function
// (a call, or passing it as a callback) is an edge.
type fnNode struct {
	name  string
	idx   file.Idx
	order int
	edges []*fnNode
}

type linter struct {
	name     string
	src      string
	file     *file.File
	builtins map[string]bool
	diags    []Diag

	sc       *scope
	curNode  *fnNode
	nodes    []*fnNode
	sawThrow bool
}

func (l *linter) report(at file.Idx, rule, msg string) {
	p := l.file.Position(int(at) - l.file.Base())
	l.diags = append(l.diags, Diag{File: l.name, Line: p.Line, Col: p.Column, Severity: "error", Rule: rule, Msg: msg})
}

// text: the source of a node, for messages.
func (l *linter) text(n ast.Node) string {
	from, to := int(n.Idx0())-1-l.file.Base()+1, int(n.Idx1())-1-l.file.Base()+1
	if from < 0 || to > len(l.src) || from >= to {
		return "..."
	}
	s := l.src[from:to]
	if len(s) > 40 {
		s = s[:37] + "..."
	}
	return s
}

func (l *linter) newNode(name string, at file.Idx) *fnNode {
	n := &fnNode{name: name, idx: at, order: len(l.nodes)}
	l.nodes = append(l.nodes, n)
	return n
}

func (l *linter) push() { l.sc = &scope{names: map[string]*binding{}, parent: l.sc} }
func (l *linter) pop()  { l.sc = l.sc.parent }

func (l *linter) reserved(name string) bool {
	return l.builtins[name] || name == "Math" || name == "Error" || name == "undefined" || name == "Infinity"
}

func (l *linter) declare(id *ast.Identifier, kind bkind, fn *fnNode) {
	name := id.Name.String()
	if l.reserved(name) {
		what := "a builtin"
		if !l.builtins[name] {
			what = "a global"
		}
		l.report(id.Idx, "reserved-name", fmt.Sprintf("`%s` is %s: give your variable another name", name, what))
	}
	l.sc.names[name] = &binding{kind: kind, fn: fn}
}

// declarePattern declares every name a binding target introduces.
func (l *linter) declarePattern(t ast.Expression, kind bkind, fn *fnNode) {
	switch t := t.(type) {
	case *ast.Identifier:
		l.declare(t, kind, fn)
	case *ast.ArrayPattern:
		for _, e := range t.Elements {
			if e != nil {
				l.declarePattern(e, kind, nil)
			}
		}
		if t.Rest != nil {
			l.declarePattern(t.Rest, kind, nil)
		}
	case *ast.ObjectPattern:
		for _, p := range t.Properties {
			switch p := p.(type) {
			case *ast.PropertyShort:
				l.declare(&p.Name, kind, nil)
			case *ast.PropertyKeyed:
				l.declarePattern(p.Value, kind, nil)
			}
		}
		if t.Rest != nil {
			l.declarePattern(t.Rest, kind, nil)
		}
	case *ast.AssignExpression: // a default
		l.declarePattern(t.Left, kind, fn)
	}
}

// patternDefaults walks the default values and computed keys in a pattern.
func (l *linter) patternDefaults(t ast.Expression) {
	switch t := t.(type) {
	case *ast.ArrayPattern:
		for _, e := range t.Elements {
			if e != nil {
				l.patternDefaults(e)
			}
		}
	case *ast.ObjectPattern:
		for _, p := range t.Properties {
			switch p := p.(type) {
			case *ast.PropertyShort:
				if p.Initializer != nil {
					l.expr(p.Initializer)
				}
			case *ast.PropertyKeyed:
				if p.Computed {
					l.expr(p.Key)
				}
				l.patternDefaults(p.Value)
			}
		}
	case *ast.AssignExpression:
		l.patternDefaults(t.Left)
		l.expr(t.Right)
	}
}

func isFnLiteral(e ast.Expression) bool {
	switch e.(type) {
	case *ast.ArrowFunctionLiteral, *ast.FunctionLiteral:
		return true
	}
	return false
}

// ---- the program ----

func (l *linter) program(prg *ast.Program) {
	l.push()
	top := map[string]bool{}
	for _, st := range prg.Body {
		switch s := st.(type) {
		case *ast.FunctionDeclaration:
			name := s.Function.Name.Name.String()
			l.declare(s.Function.Name, bFunc, l.newNode(name, s.Function.Name.Idx))
			top[name] = true
		case *ast.LexicalDeclaration:
			kind := bConst
			if s.Token == token.LET {
				kind = bLet
				l.report(s.Idx, "let", "`let` isn't available: every variable is a `const`. Use `const` for a constant; what changes from one input to the next belongs in the state")
			}
			for _, b := range s.List {
				var fn *fnNode
				if id, ok := b.Target.(*ast.Identifier); ok && kind == bConst && isFnLiteral(b.Initializer) {
					fn = l.newNode(id.Name.String(), id.Idx)
				}
				l.declarePattern(b.Target, kind, fn)
			}
		case *ast.VariableStatement:
			l.report(s.Var, "var", "`var` isn't allowed: use `const` (every variable is a const)")
			for _, b := range s.List {
				l.declarePattern(b.Target, bLet, nil)
			}
		case *ast.EmptyStatement:
		case *ast.ClassDeclaration:
			l.report(s.Idx0(), "class", "classes aren't available: use plain objects and functions")
		default:
			l.report(st.Idx0(), "top-level", "only `const` declarations and `function` declarations at the top level: statements go inside a function")
		}
	}
	for _, f := range []string{"init()", "step(state, input, now)", "view(state)"} {
		name := f[:strings.Index(f, "(")]
		if !top[name] {
			l.report(file.Idx(l.file.Base()), "contract", fmt.Sprintf("no `function %s { ... }` at the top level: an engine declares init, step and view", f))
		}
	}
	for _, st := range prg.Body {
		switch s := st.(type) {
		case *ast.FunctionDeclaration:
			l.fnLiteral(s.Function, l.sc.lookup(s.Function.Name.Name.String()).fn, true)
		case *ast.LexicalDeclaration:
			l.declBindings(s.List)
		case *ast.VariableStatement:
			for _, b := range s.List {
				if b.Initializer != nil {
					l.expr(b.Initializer)
				}
			}
		}
	}
	if !l.sawThrow && top["step"] {
		l.report(file.Idx(l.file.Base()), "unknown-input", "step never throws: an input it doesn't handle must end in `throw new Error(...)`")
	}
	l.cycles()
}

// declBindings walks the initializers of an already declared let/const.
func (l *linter) declBindings(list []*ast.Binding) {
	for _, b := range list {
		if id, ok := b.Target.(*ast.Identifier); ok && isFnLiteral(b.Initializer) {
			var fn *fnNode
			if bd := l.sc.lookup(id.Name.String()); bd != nil {
				fn = bd.fn
			}
			l.fnLiteral(b.Initializer, fn, false)
		} else if b.Initializer != nil {
			l.expr(b.Initializer)
		}
		l.patternDefaults(b.Target)
	}
}

// fnLiteral walks a function. node: its node in the call graph, if it is
// named; an anonymous function belongs to the function around it.
func (l *linter) fnLiteral(e ast.Expression, node *fnNode, decl bool) {
	var params *ast.ParameterList
	var body ast.Node
	var self *ast.Identifier
	switch f := e.(type) {
	case *ast.FunctionLiteral:
		if f.Async {
			l.report(f.Idx0(), "async", "`async` isn't available: engines are synchronous")
		}
		if f.Generator {
			l.report(f.Idx0(), "generator", "generators aren't available: return arrays")
		}
		params, body = f.ParameterList, f.Body
		if f.Name != nil && !decl {
			// a named function expression can call itself by its name
			self = f.Name
			if node == nil {
				node = l.newNode(f.Name.Name.String(), f.Name.Idx)
			}
		}
	case *ast.ArrowFunctionLiteral:
		if f.Async {
			l.report(f.Idx0(), "async", "`async` isn't available: engines are synchronous")
		}
		params, body = f.ParameterList, f.Body
	default:
		return
	}
	savedNode := l.curNode
	if node != nil {
		l.curNode = node
	}
	l.push()
	if self != nil {
		l.declare(self, bFunc, node)
	}
	if params != nil {
		for _, b := range params.List {
			l.declarePattern(b.Target, bParam, nil)
		}
		if params.Rest != nil {
			l.declarePattern(params.Rest, bParam, nil)
		}
		for _, b := range params.List {
			l.patternDefaults(b.Target)
			if b.Initializer != nil {
				l.expr(b.Initializer)
			}
		}
	}
	switch b := body.(type) {
	case *ast.BlockStatement:
		l.blockBody(b.List)
	case *ast.ExpressionBody:
		l.expr(b.Expression)
	}
	l.pop()
	l.curNode = savedNode
}

// predeclare: lets, consts and functions are visible in their whole block.
func (l *linter) predeclare(list []ast.Statement) {
	for _, st := range list {
		switch s := st.(type) {
		case *ast.LexicalDeclaration:
			kind := bConst
			if s.Token == token.LET {
				kind = bLet
			}
			for _, b := range s.List {
				var fn *fnNode
				if id, ok := b.Target.(*ast.Identifier); ok && kind == bConst && isFnLiteral(b.Initializer) {
					fn = l.newNode(id.Name.String(), id.Idx)
				}
				l.declarePattern(b.Target, kind, fn)
			}
		case *ast.FunctionDeclaration:
			l.declare(s.Function.Name, bFunc, l.newNode(s.Function.Name.Name.String(), s.Function.Name.Idx))
		}
	}
}

func (l *linter) blockBody(list []ast.Statement) {
	l.predeclare(list)
	for _, st := range list {
		l.stmt(st)
	}
}

const loopHelp = "There are no loops: compute from the whole list. Transform it with map/filter; " +
	"total, count or average it with `sum(list, fn)`, `count(list, pred)`, `mean(list, fn)`; " +
	"search it with find/some/every or `min_by`/`max_by`; pair each item with the one before with `pairs(list)`; " +
	"anything else with `list.reduce((acc, x) => ..., start)` (callbacks get the index as a second argument)"

const letHelp = "`let` isn't available: every variable is a `const`, set once. Write the value in one expression where you declare it: " +
	"`const x = cond ? b : a`, not `let x = a; if (cond) x = b;`. For several cases, chain the ternary, `const x = c1 ? a : c2 ? b : c`, " +
	"or call a helper function with a `switch` that returns the value from each case; " +
	"to build a value from a list, use map/filter/reduce or a list builtin (`sum`, `count`, `pairs`, ...)"

func (l *linter) stmt(st ast.Statement) {
	switch s := st.(type) {
	case nil, *ast.EmptyStatement, *ast.BadStatement:
	case *ast.BlockStatement:
		l.push()
		l.blockBody(s.List)
		l.pop()
	case *ast.LexicalDeclaration:
		if s.Token == token.LET {
			l.report(s.Idx, "let", letHelp)
		}
		l.declBindings(s.List)
	case *ast.VariableStatement:
		l.report(s.Var, "var", "`var` isn't allowed: use `const` (every variable is a const)")
		for _, b := range s.List {
			l.declarePattern(b.Target, bLet, nil)
			if b.Initializer != nil {
				l.expr(b.Initializer)
			}
		}
	case *ast.FunctionDeclaration:
		var fn *fnNode
		if b := l.sc.lookup(s.Function.Name.Name.String()); b != nil {
			fn = b.fn
		}
		l.fnLiteral(s.Function, fn, true)
	case *ast.ExpressionStatement:
		l.expr(s.Expression)
	case *ast.IfStatement:
		// One error for a whole if / else if / else chain.
		l.report(s.If, "if", ifHelp)
		l.ifChain(s)
	case *ast.SwitchStatement:
		l.expr(s.Discriminant)
		l.push()
		for _, c := range s.Body {
			l.predeclare(c.Consequent)
		}
		for i, c := range s.Body {
			if c.Test != nil {
				l.expr(c.Test)
			}
			for _, x := range c.Consequent {
				l.stmt(x)
			}
			// An empty case shares the next case's code (`case "A": case "B": return ...`).
			if len(c.Consequent) == 0 && i < len(s.Body)-1 {
				continue
			}
			if len(c.Consequent) > 0 {
				if _, isBreak := c.Consequent[len(c.Consequent)-1].(*ast.BranchStatement); isBreak {
					continue // reported as a break
				}
			}
			if !terminates(c.Consequent) {
				l.report(c.Case, "switch", caseHelp)
			}
		}
		l.pop()
	case *ast.ForOfStatement:
		// Reported, then walked as usual so that what is inside is linted too.
		l.report(s.For, "loop", "`for ... of` isn't allowed. "+loopHelp)
		l.expr(s.Source)
		l.push()
		switch into := s.Into.(type) {
		case *ast.ForDeclaration:
			l.declarePattern(into.Target, bConst, nil)
			l.patternDefaults(into.Target)
		case *ast.ForIntoVar:
			l.declarePattern(into.Binding.Target, bConst, nil)
		case *ast.ForIntoExpression:
			l.expr(into.Expression)
		}
		l.scoped(s.Body)
		l.pop()
	case *ast.ForStatement:
		l.report(s.For, "loop", "`for (;;)` loops aren't allowed. "+loopHelp)
	case *ast.WhileStatement:
		l.report(s.While, "loop", "`while` loops aren't allowed. "+loopHelp)
	case *ast.DoWhileStatement:
		l.report(s.Do, "loop", "`do ... while` loops aren't allowed. "+loopHelp)
	case *ast.ForInStatement:
		l.report(s.For, "loop", "`for ... in` isn't allowed. "+loopHelp)
	case *ast.ReturnStatement:
		if s.Argument != nil {
			l.expr(s.Argument)
		}
	case *ast.ThrowStatement:
		l.sawThrow = true
		if !l.isNewError(s.Argument) {
			l.report(s.Throw, "throw", "throw an Error: `throw new Error(message)`")
		}
		l.expr(s.Argument)
	case *ast.BranchStatement:
		if s.Token == token.CONTINUE {
			l.report(s.Idx, "loop", "`continue` isn't available: there are no loops")
		} else {
			l.report(s.Idx, "switch", "`break` isn't available: "+caseHelp)
		}
	case *ast.LabelledStatement:
		l.report(s.Idx0(), "label", "labels aren't available")
		l.stmt(s.Statement)
	case *ast.TryStatement:
		l.report(s.Try, "try", "`try`/`catch` isn't available: a step that throws fails as a whole, and the state is kept")
	case *ast.WithStatement:
		l.report(s.With, "with", "`with` isn't available")
	case *ast.DebuggerStatement:
		l.report(s.Debugger, "debugger", "`debugger` isn't available")
	case *ast.ClassDeclaration:
		l.report(s.Idx0(), "class", "classes aren't available: use plain objects and functions")
	default:
		l.report(st.Idx0(), "syntax", "this statement isn't available")
	}
}

// scoped: the body of an if, an else or a loop (linted inside, though reported).
func (l *linter) scoped(st ast.Statement) {
	l.push()
	l.stmt(st)
	l.pop()
}

const ifHelp = "`if` isn't available: branch with the ternary operator. Choose the value where it is declared, `const x = cond ? a : b`, " +
	"or return it, `return cond ? a : b`. For several cases, chain it, `return c1 ? a : c2 ? b : c`, " +
	"or use a `switch` that returns from each case"

const caseHelp = "in a `switch`, every case ends in `return` or `throw` (no `break`, no falling through to the next case). " +
	"To pick a value, return it from the case, or choose it with a ternary, `const x = cond ? a : b`"

// ifChain walks an if and its else-ifs (reported once, as the first if).
func (l *linter) ifChain(s *ast.IfStatement) {
	l.expr(s.Test)
	l.scoped(s.Consequent)
	switch alt := s.Alternate.(type) {
	case nil:
	case *ast.IfStatement:
		l.ifChain(alt)
	default:
		l.scoped(alt)
	}
}

// terminates: the statements end in a return or a throw (or in a switch
// with a default, whose every case does).
func terminates(list []ast.Statement) bool {
	if len(list) == 0 {
		return false
	}
	switch s := list[len(list)-1].(type) {
	case *ast.ReturnStatement, *ast.ThrowStatement:
		return true
	case *ast.BlockStatement:
		return terminates(s.List)
	case *ast.SwitchStatement:
		return s.Default >= 0
	}
	return false
}

func (l *linter) isGlobal(e ast.Expression, name string) bool {
	id, ok := e.(*ast.Identifier)
	return ok && id.Name.String() == name && l.sc.lookup(name) == nil
}

func (l *linter) isNewError(e ast.Expression) bool {
	n, ok := e.(*ast.NewExpression)
	return ok && l.isGlobal(n.Callee, "Error")
}

func unwrap(e ast.Expression) ast.Expression {
	for {
		switch x := e.(type) {
		case *ast.Optional:
			e = x.Expression
		case *ast.OptionalChain:
			e = x.Expression
		default:
			return e
		}
	}
}

// ---- expressions ----

func (l *linter) expr(e ast.Expression) {
	switch x := e.(type) {
	case nil, *ast.NumberLiteral, *ast.StringLiteral, *ast.BooleanLiteral, *ast.NullLiteral, *ast.BadExpression:
	case *ast.Identifier:
		l.ref(x)
	case *ast.TemplateLiteral:
		if x.Tag != nil {
			l.report(x.Idx0(), "template", "tagged templates aren't available: write a plain template literal")
		}
		for _, ex := range x.Expressions {
			l.expr(ex)
		}
	case *ast.RegExpLiteral:
		l.report(x.Idx, "regex", "regular expressions aren't available")
	case *ast.ArrayLiteral:
		for _, v := range x.Value {
			if isFnLiteral(v) {
				l.report(v.Idx0(), "plain-data", "functions can't be kept in arrays: arrays and objects hold plain data")
			}
			l.expr(v)
		}
	case *ast.SpreadElement:
		l.expr(x.Expression)
	case *ast.ObjectLiteral:
		for _, p := range x.Value {
			switch p := p.(type) {
			case *ast.PropertyShort:
				l.ref(&p.Name)
			case *ast.PropertyKeyed:
				switch p.Kind {
				case ast.PropertyKindGet, ast.PropertyKindSet:
					l.report(p.Idx0(), "plain-data", "getters and setters aren't available: objects hold plain data")
				case ast.PropertyKindMethod:
					l.report(p.Idx0(), "plain-data", "methods aren't available: objects hold plain data; declare a function instead")
				default:
					if isFnLiteral(p.Value) {
						l.report(p.Value.Idx0(), "plain-data", "functions can't be kept in objects: objects hold plain data; declare a function and call it (use `switch` to choose)")
					}
				}
				if p.Computed {
					l.expr(p.Key)
				} else if s, ok := p.Key.(*ast.StringLiteral); ok && forbiddenProps[s.Value.String()] {
					l.report(p.Key.Idx0(), "plain-data", fmt.Sprintf("`%s` isn't available as a field name", s.Value))
				}
				if fl, ok := p.Value.(*ast.FunctionLiteral); ok {
					l.fnLiteral(fl, nil, false)
				} else {
					l.expr(p.Value)
				}
			case *ast.SpreadElement:
				l.expr(p.Expression)
			}
		}
	case *ast.FunctionLiteral, *ast.ArrowFunctionLiteral:
		l.fnLiteral(x, nil, false)
	case *ast.ClassLiteral:
		l.report(x.Idx0(), "class", "classes aren't available: use plain objects and functions")
	case *ast.ThisExpression:
		l.report(x.Idx, "this", "`this` isn't available: pass values as parameters")
	case *ast.SuperExpression:
		l.report(x.Idx, "class", "`super` isn't available")
	case *ast.MetaProperty:
		l.report(x.Idx, "syntax", "`new.target` and `import.meta` aren't available")
	case *ast.PrivateDotExpression:
		l.report(x.Idx0(), "class", "private fields aren't available")
	case *ast.YieldExpression:
		l.report(x.Yield, "generator", "generators aren't available")
	case *ast.AwaitExpression:
		l.report(x.Await, "async", "`await` isn't available: engines are synchronous")
	case *ast.NewExpression:
		if !l.isGlobal(x.Callee, "Error") {
			l.report(x.New, "new", "`new` is only for `throw new Error(message)`: build values with object and array literals")
		}
		for _, a := range x.ArgumentList {
			l.expr(a)
		}
	case *ast.CallExpression:
		l.call(x)
	case *ast.DotExpression:
		l.dot(x)
	case *ast.BracketExpression:
		l.expr(x.Left)
		l.expr(x.Member)
		if s, ok := x.Member.(*ast.StringLiteral); ok && forbiddenProps[s.Value.String()] {
			l.report(x.Member.Idx0(), "plain-data", fmt.Sprintf("`%s` isn't available", s.Value))
		}
	case *ast.Optional:
		l.expr(x.Expression)
	case *ast.OptionalChain:
		l.expr(x.Expression)
	case *ast.AssignExpression:
		a := assign{text: l.text(x), value: l.text(x.Right)}
		if x.Operator != token.ASSIGN {
			a.op = x.Operator.String()
		}
		l.target(x.Left, a)
		l.expr(x.Right)
	case *ast.UnaryExpression:
		switch x.Operator {
		case token.INCREMENT, token.DECREMENT:
			op := "+"
			if x.Operator == token.DECREMENT {
				op = "-"
			}
			l.target(x.Operand, assign{text: l.text(x), op: op, value: "1", step: true})
			return
		case token.DELETE:
			l.report(x.Idx, "mutation", "`delete` changes the object: build a new one without the field, `{ ...obj, x: null }` or `const { x, ...rest } = obj`")
		case token.VOID:
			l.report(x.Idx, "syntax", "`void` isn't available: use `undefined` or null")
		}
		l.expr(x.Operand)
	case *ast.BinaryExpression:
		if x.Operator == token.INSTANCEOF {
			l.report(x.Idx0(), "syntax", "`instanceof` isn't available: there are no classes; test a field, such as `x.tag === \"...\"`")
		}
		l.expr(x.Left)
		l.expr(x.Right)
	case *ast.ConditionalExpression:
		l.expr(x.Test)
		l.expr(x.Consequent)
		l.expr(x.Alternate)
	case *ast.SequenceExpression:
		for _, s := range x.Sequence {
			l.expr(s)
		}
	case *ast.ArrayPattern, *ast.ObjectPattern:
		l.target(x, assign{text: l.text(x), value: "value"})
	default:
		l.report(e.Idx0(), "syntax", "this expression isn't available")
	}
}

// ref: a name read. A function referenced from a function is an edge in
// the call graph.
func (l *linter) ref(id *ast.Identifier) {
	name := id.Name.String()
	if b := l.sc.lookup(name); b != nil {
		if b.fn != nil && l.curNode != nil {
			l.curNode.edges = append(l.curNode.edges, b.fn)
		}
		return
	}
	switch {
	case l.builtins[name], name == "undefined", name == "Infinity":
		return
	case name == "Math":
		l.report(id.Idx, "global", "use Math's members directly: Math."+strings.ReplaceAll(mathList, ", ", ", Math."))
	case name == "Error":
		l.report(id.Idx, "global", "Error is only for `throw new Error(message)`")
	case globals[name] != "":
		l.report(id.Idx, "global", globals[name])
	case mathMembers[name]:
		l.report(id.Idx, "global", fmt.Sprintf("`%s` isn't declared: write `Math.%s`", name, name))
	default:
		l.report(id.Idx, "global", fmt.Sprintf("`%s` isn't declared. The only globals are Math, the builtins (%s), undefined and Infinity", name, strings.Join(engines.BuiltinNames(), ", ")))
	}
}

func (l *linter) dot(x *ast.DotExpression) {
	prop := x.Identifier.Name.String()
	if l.isGlobal(x.Left, "Math") {
		switch {
		case mathMembers[prop]:
		case prop == "random":
			l.report(x.Identifier.Idx, "global", "Math.random isn't available: an engine must give the same result for the same inputs")
		default:
			l.report(x.Identifier.Idx, "global", fmt.Sprintf("Math.%s isn't available: Math has %s", prop, mathList))
		}
		return
	}
	l.expr(x.Left)
	if forbiddenProps[prop] {
		l.report(x.Identifier.Idx, "plain-data", fmt.Sprintf("`.%s` isn't available", prop))
	}
}

const callHelp = "only functions declared in this file (`function f() {}` or `const f = (...) => ...`), the builtins and Math.* can be called"

func (l *linter) call(x *ast.CallExpression) {
	method := ""
	cbArg := -1 // which argument is called back
	switch c := unwrap(x.Callee).(type) {
	case *ast.Identifier:
		name := c.Name.String()
		b := l.sc.lookup(name)
		switch {
		case b != nil && b.fn != nil:
			l.ref(c)
		case b != nil:
			l.report(c.Idx, "call", fmt.Sprintf("`%s` can't be called: %s", name, callHelp))
		case name == "Error":
			l.report(c.Idx, "throw", "throw an Error: `throw new Error(message)`")
		default:
			l.ref(c) // a builtin, or reported
			if i, ok := engines.BuiltinCallback(name); ok && l.builtins[name] {
				cbArg = i
			}
		}
	case *ast.DotExpression:
		if l.isGlobal(c.Left, "Math") {
			l.dot(c)
			break
		}
		method = c.Identifier.Name.String()
		l.expr(c.Left)
		switch {
		case mutators[method] != "":
			l.report(c.Identifier.Idx, "mutation", mutators[method])
		case !methods[method]:
			l.report(c.Identifier.Idx, "method", fmt.Sprintf("`.%s()` isn't available. The array methods are %s", method, methodList))
		}
	case *ast.ArrowFunctionLiteral, *ast.FunctionLiteral:
		l.fnLiteral(c, nil, false)
	default:
		l.report(x.Callee.Idx0(), "call", callHelp)
		l.expr(c)
	}
	if callbackMethods[method] {
		cbArg = 0
	}
	for i, a := range x.ArgumentList {
		if i == cbArg {
			l.callback(a)
		} else {
			l.expr(a)
		}
	}
}

// callback: an array method's or a builtin's function, written inline or
// named.
func (l *linter) callback(a ast.Expression) {
	switch c := a.(type) {
	case *ast.ArrowFunctionLiteral, *ast.FunctionLiteral:
		l.fnLiteral(c, nil, false)
		return
	case *ast.Identifier:
		name := c.Name.String()
		if b := l.sc.lookup(name); (b != nil && b.fn != nil) || (b == nil && l.builtins[name]) {
			l.ref(c)
			return
		}
	}
	l.report(a.Idx0(), "call", "pass the callback as an arrow function, `(x) => ...`, or by the name of a function declared in this file")
	l.expr(a)
}

// assign: how something is assigned to, for the messages. text: the whole
// assignment; op: "" for `=`, else the operator of `+=` etc. (or of `++`,
// `--` when step); value: the right-hand side.
type assign struct {
	text, op, value string
	step            bool
}

// newValue: the value an assignment would give `old`.
func (a assign) newValue(old string) string {
	if a.op == "" {
		return a.value
	}
	return old + " " + a.op + " " + a.value
}

// target: something assigned to. Nothing can be: every binding is a const,
// and objects and arrays are never changed in place.
func (l *linter) target(e ast.Expression, a assign) {
	switch t := e.(type) {
	case *ast.Identifier:
		name := t.Name.String()
		b := l.sc.lookup(name)
		switch {
		case b == nil:
			if l.reserved(name) {
				l.report(t.Idx, "mutation", fmt.Sprintf("`%s` can't be changed", name))
			} else {
				l.ref(t)
			}
		case b.kind == bFunc:
			l.report(t.Idx, "mutation", fmt.Sprintf("`%s` is a function: it can't be reassigned", name))
		case a.step:
			l.report(t.Idx, "mutation", fmt.Sprintf("`%s` changes `%s`: every variable is a `const`, set once. Write `%s` where the new value is needed (`const next = %s`); to count items in a list, `count(list, pred)`",
				a.text, name, a.newValue(name), a.newValue(name)))
		case a.op != "":
			l.report(t.Idx, "mutation", fmt.Sprintf("`%s` changes `%s`: every variable is a `const`, set once. To total a list, write `const %s = sum(list, (x) => ...)` (or `count`, `mean`), or fold it with `list.reduce((acc, x) => acc %s ..., start)`; otherwise name the new value, `const next = %s`",
				a.text, name, name, a.op, a.newValue(name)))
		default:
			l.report(t.Idx, "mutation", fmt.Sprintf("`%s` can't be reassigned: every variable is a `const`, set once. Choose its value where it is declared: `const %s = cond ? %s : ...` rather than `let %s = ...; if (cond) %s = %s;`. For several cases, chain the ternary, `const %s = c1 ? a : c2 ? b : c`, or use a helper function with a `switch` that returns the value from each case",
				name, name, a.value, name, name, a.value, name))
		}
	case *ast.DotExpression:
		obj, field := l.text(t.Left), t.Identifier.Name.String()
		l.report(t.Idx0(), "mutation", fmt.Sprintf("`%s` changes `%s` in place: objects and arrays are never changed. Make a new object instead, e.g. return `{ ...%s, %s: %s }`",
			a.text, obj, obj, field, a.newValue(l.text(t))))
		l.expr(t.Left)
	case *ast.BracketExpression:
		obj, key, v := l.text(t.Left), l.text(t.Member), a.newValue(l.text(t))
		build := fmt.Sprintf("`%s.map((x, j) => j === %s ? %s : x)` for an array, `{ ...%s, [%s]: %s }` for an object", obj, key, v, obj, key, v)
		switch t.Member.(type) {
		case *ast.NumberLiteral:
			build = fmt.Sprintf("`%s.map((x, j) => j === %s ? %s : x)`", obj, key, v)
		case *ast.StringLiteral:
			build = fmt.Sprintf("`{ ...%s, %s: %s }`", obj, key, v)
		}
		l.report(t.Idx0(), "mutation", fmt.Sprintf("`%s` changes `%s` in place: objects and arrays are never changed. Make a new one instead: %s", a.text, obj, build))
		l.expr(t.Left)
		l.expr(t.Member)
	case *ast.ArrayPattern:
		for _, el := range t.Elements {
			if el != nil {
				l.target(el, a)
			}
		}
		if t.Rest != nil {
			l.target(t.Rest, a)
		}
	case *ast.ObjectPattern:
		for _, p := range t.Properties {
			switch p := p.(type) {
			case *ast.PropertyShort:
				l.target(&p.Name, a)
				if p.Initializer != nil {
					l.expr(p.Initializer)
				}
			case *ast.PropertyKeyed:
				if p.Computed {
					l.expr(p.Key)
				}
				l.target(p.Value, a)
			}
		}
		if t.Rest != nil {
			l.target(t.Rest, a)
		}
	case *ast.AssignExpression: // a default in a pattern
		l.target(t.Left, a)
		l.expr(t.Right)
	default:
		l.expr(e)
	}
}

// cycles reports each group of functions that reach themselves.
func (l *linter) cycles() {
	index := map[*fnNode]int{}
	low := map[*fnNode]int{}
	on := map[*fnNode]bool{}
	var stack []*fnNode
	n := 0
	var strong func(v *fnNode)
	strong = func(v *fnNode) {
		index[v], low[v] = n, n
		n++
		stack = append(stack, v)
		on[v] = true
		for _, w := range v.edges {
			if _, seen := index[w]; !seen {
				strong(w)
				low[v] = min(low[v], low[w])
			} else if on[w] {
				low[v] = min(low[v], index[w])
			}
		}
		if low[v] != index[v] {
			return
		}
		var group []*fnNode
		for {
			w := stack[len(stack)-1]
			stack = stack[:len(stack)-1]
			on[w] = false
			group = append(group, w)
			if w == v {
				break
			}
		}
		self := false
		for _, w := range v.edges {
			if w == v {
				self = true
			}
		}
		if len(group) == 1 && !self {
			return
		}
		sort.Slice(group, func(i, j int) bool { return group[i].order < group[j].order })
		first := group[0]
		if len(group) == 1 {
			l.report(first.idx, "recursion", fmt.Sprintf("`%s` calls itself: recursion isn't allowed. Work over an array with map/filter/reduce or a list builtin (`sum`, `count`, `pairs`, ...)", first.name))
			return
		}
		names := make([]string, len(group))
		for i, g := range group {
			names[i] = "`" + g.name + "`"
		}
		l.report(first.idx, "recursion", fmt.Sprintf("%s call each other in a cycle: recursion isn't allowed. Work over an array with map/filter/reduce or a list builtin (`sum`, `count`, `pairs`, ...)", strings.Join(names, " and ")))
	}
	for _, v := range l.nodes {
		if _, seen := index[v]; !seen {
			strong(v)
		}
	}
}
