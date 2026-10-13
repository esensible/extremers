module oracle

go 1.26.4

require (
	expr-engines v0.0.0
	github.com/expr-lang/expr v1.17.9-0.20260412144015-3a46b1970b37
)

// The engine profile's builtins, as Go implementations (spikes/expr-engines):
// registered with -builtins sailing, for the profile's conformance cases.
replace expr-engines => ../../../spikes/expr-engines
