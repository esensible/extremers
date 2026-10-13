module js-eval

go 1.26.4

require (
	github.com/dop251/goja v0.0.0-20261009133625-a4bedf5f6000
	js-engines v0.0.0
)

require (
	github.com/dlclark/regexp2/v2 v2.8.1 // indirect
	github.com/go-sourcemap/sourcemap v2.1.3+incompatible // indirect
	github.com/google/pprof v0.0.0-20230207041349-798e818bf904 // indirect
	golang.org/x/text v0.3.8 // indirect
)

// The harness and builtins are the spike's, unchanged.
replace js-engines => ../js-engines
