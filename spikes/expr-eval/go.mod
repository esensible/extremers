module expr-eval

go 1.26.4

require (
	expr-engines v0.0.0
	github.com/expr-lang/expr v1.17.8
)

// The harness and builtins are the spike's, unchanged.
replace expr-engines => ../expr-engines
