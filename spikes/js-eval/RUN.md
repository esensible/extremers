# Running the JavaScript engine eval

This kit tests whether a model, given little context, can write engines in our strict subset of JavaScript. A *naive agent* writes one file, `engine.js`, for one task. The grader then runs `check`: (a) compile, (b) language lint, and (c) the hidden tests.

```
LANGUAGE.md            the only language reference a naive agent gets
running/ riding/ vendee/ ultra/
    TASK.md, input.schema.json, view.schema.json
check                  ./check [--no-tests] <task> <engine dir>  (wraps cmd/check)
cmd/check/             the validator: compile, lint, then go test ./tests
lint/                  the language lint (works on goja's parse tree); lint_test.go is HIDDEN
schema/                the small JSON Schema checker that views are held to
tests/                 HIDDEN: one *_test.go per task, plus the LANGUAGE.md example's test
reference/<task>/      HIDDEN: reference solutions (the hidden tests pass on them)
example/steps/         LANGUAGE.md's example engine
make-naive-bundle.sh   copies the non-hidden parts somewhere else, for naive agents
```

The harness and builtins are the spike's (`../js-engines`, via `replace` in go.mod), used unchanged; it runs engines with goja (pure Go). The kit must therefore sit next to `js-engines`.

## Grader setup (the `expr-go` container)

`expr-go` mounts only `expr-engines` (at /work), so copy both directories in, side by side. `podman cp` does not delete files, so re-copy after any change (and remove files you deleted).

```
podman exec expr-go mkdir -p /spikes/js-engines /spikes/js-eval
podman cp /Users/sensible-1/src/extremers-wire/spikes/js-engines/. expr-go:/spikes/js-engines
podman cp /Users/sensible-1/src/extremers-wire/spikes/js-eval/. expr-go:/spikes/js-eval
```

Check the harness, then the kit itself. With ENGINE_DIR unset, the tests run against `reference/`:

```
podman exec -w /spikes/js-engines expr-go go test ./... -count=1
podman exec -w /spikes/js-eval expr-go go test ./tests ./lint -count=1 -v
```

## Grading an answer

Copy the agent's engine directory in and run all three stages:

```
podman cp <answers>/<agent>/running/. expr-go:/spikes/answers/<agent>/running
podman exec -w /spikes/js-eval expr-go ./check running /spikes/answers/<agent>/running
```

`check` exits non-zero if any stage fails, and ends with `PASS` or `FAIL`. The stages are:

- **(a) compile.** `engine.js` is parsed and compiled in strict mode (syntax errors are reported as `engine.js:line:col`). The harness then runs the top level and `init()`, and the initial view is checked against `<task>/view.schema.json`.
- **(b) language lint.** Everything outside LANGUAGE.md's subset is an error, reported as `engine.js:line:col: error [rule] message`, with what to write instead. There are no warnings.
- **(c) hidden tests.** These run `go test ./tests -run '^Test<Task>'` with `ENGINE_DIR` set. Stage (c) is not run if (a) or (b) failed.

The hidden tests drive the engine the way a host would: they deliver any due Wake before each input. They check every view against the schema, which requires a whole number where it says `integer` (the harness hands every whole JavaScript number back as a Go int, any other as a float64), and they check that an unknown input throws (`fail: ` in the error) and that an unasked Wake does not. Tolerances are in the test files.

At run time the harness also enforces what it can: strict mode; state and input deep-frozen (a mutation throws a TypeError); no `Date` or `Math.random`; a 2 s limit per call and a capped call stack; and only plain data back (a function, or `undefined`, NaN or an infinity in a view, is an error).

## What each naive agent gets

Exactly these files, and nothing else from the kit:

1. `LANGUAGE.md`
2. `<task>/TASK.md`, `<task>/input.schema.json` and `<task>/view.schema.json`
3. The command `check --no-tests <task> <dir>`. It runs stages (a) and (b) only.

The agent must never see `tests/`, `reference/`, `example/` or `lint/lint_test.go`. Even the `expr-go` container holds them, so naive agents should run their checks in a container of their own that contains only the bundle. Build it once; these steps use only sh and cp on the host:

```
/Users/sensible-1/src/extremers-wire/spikes/js-eval/make-naive-bundle.sh /Users/sensible-1/src/extremers-wire/spikes/js-eval-naive
podman run -d --name js-naive \
  -v /Users/sensible-1/src/extremers-wire/spikes/js-engines:/spikes/js-engines:ro \
  -v /Users/sensible-1/src/extremers-wire/spikes/js-eval-naive:/spikes/kit:ro \
  -v <answers>:/answers \
  docker.io/library/golang:latest sleep infinity
podman exec js-naive go -C /spikes/kit mod download
```

The `mod download` fetches goja and its dependencies once. The bundle's `check` without `--no-tests` just says the hidden tests are missing, and fails.

Suggested brief for the agent (fill in the task and the path):

> Write an app engine for the task in TASK.md, in the language described in LANGUAGE.md. Its inputs and its view are defined by input.schema.json and view.schema.json. Put `engine.js` in `<answers>/<task>/`. You can check it with `podman exec js-naive /spikes/kit/check --no-tests <task> /answers/<task>`, which compiles it and checks the language but does not test behaviour. Use only the four files you were given.

Then grade the answer in `expr-go` as described above. The agent's files are already on the host under `<answers>`.
