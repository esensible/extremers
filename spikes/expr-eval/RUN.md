# Running the expr engine eval

This kit tests whether a model, given little context, can write engines in our dialect of Go's `expr`. A *naive agent* writes `init.expr`, `step.expr` and `view.expr` for one task. The grader then runs `check`: (a) compile, (b) dialect lint, and (c) the hidden tests.

```
LANGUAGE.md            the only language reference a naive agent gets
running/ riding/ vendee/ ultra/
    TASK.md, input.schema.json, view.schema.json
check                  ./check [--no-tests] <task> <engine dir>  (wraps cmd/check)
cmd/check/             the validator: compile, lint, then go test ./tests
lint/                  the dialect lint (works on the parse tree)
schema/                the small JSON Schema checker that views are held to
tests/                 HIDDEN: one *_test.go per task, plus the LANGUAGE.md example's test
reference/<task>/      HIDDEN: reference solutions (the hidden tests pass on them)
example/steps/         LANGUAGE.md's example engine
make-naive-bundle.sh   copies the non-hidden parts somewhere else, for naive agents
```

The harness and builtins are the spike's (`../expr-engines`, via `replace` in go.mod), used unchanged. The kit must therefore sit next to `expr-engines`.

## Grader setup (the `expr-go` container)

`expr-go` mounts only `expr-engines` (at /work), so copy both directories in, side by side. `podman cp` does not delete files, so re-copy after any change.

```
podman exec expr-go mkdir -p /spikes/expr-engines /spikes/expr-eval
podman cp /Users/sensible-1/src/extremers-wire/spikes/expr-engines/. expr-go:/spikes/expr-engines
podman cp /Users/sensible-1/src/extremers-wire/spikes/expr-eval/. expr-go:/spikes/expr-eval
```

Check the kit itself. With ENGINE_DIR unset, the tests run against `reference/`:

```
podman exec -w /spikes/expr-eval expr-go go test ./tests -count=1 -v
```

## Grading an answer

Copy the agent's engine directory in and run all three stages:

```
podman cp <answers>/<agent>/running/. expr-go:/spikes/answers/<agent>/running
podman exec -w /spikes/expr-eval expr-go ./check running /spikes/answers/<agent>/running
```

`check` exits non-zero if any stage fails, and ends with `PASS` or `FAIL`. The stages are:

- **(a) compile.** Each file is compiled with expr and the builtins (errors are reported as `file:line:col`). `init` is evaluated, and the initial view is checked against `<task>/view.schema.json`.
- **(b) dialect lint.** It reports `error`s, which fail the check, and `warning`s, which don't. The only warning is a computed index where the linter can't tell whether it indexes a list (fine) or a map (not allowed).
- **(c) hidden tests.** These run `go test ./tests -run '^Test<Task>'` with `ENGINE_DIR` set. Stage (c) is not run if (a) or (b) failed.

The hidden tests drive the engine the way a host would: they deliver any due Wake before each input. They check every view against the schema, which requires ints where it says `integer`, and they check that an unknown input fails and that an unasked Wake does not. Tolerances are in the test files.

## What each naive agent gets

Exactly these files, and nothing else from the kit:

1. `LANGUAGE.md`
2. `<task>/TASK.md`, `<task>/input.schema.json` and `<task>/view.schema.json`
3. The command `check --no-tests <task> <dir>`. It runs stages (a) and (b) only.

The agent must never see `tests/`, `reference/` or `example/`. Even the `expr-go` container holds them, so naive agents should run their checks in a container of their own that contains only the bundle. Build it once; these steps use only sh and cp on the host:

```
/Users/sensible-1/src/extremers-wire/spikes/expr-eval/make-naive-bundle.sh /Users/sensible-1/src/extremers-wire/spikes/expr-eval-naive
podman run -d --name expr-naive \
  -v /Users/sensible-1/src/extremers-wire/spikes/expr-engines:/spikes/expr-engines:ro \
  -v /Users/sensible-1/src/extremers-wire/spikes/expr-eval-naive:/spikes/kit:ro \
  -v <answers>:/answers \
  docker.io/library/golang:latest sleep infinity
podman exec expr-naive go -C /spikes/kit mod download
```

The `mod download` fetches `github.com/expr-lang/expr v1.17.8`, the only dependency, once. The bundle's `check` without `--no-tests` just says the hidden tests are missing, and fails.

Suggested brief for the agent (fill in the task and the path):

> Write an app engine for the task in TASK.md, in the language described in LANGUAGE.md. Its inputs and its view are defined by input.schema.json and view.schema.json. Put `init.expr`, `step.expr` and `view.expr` in `<answers>/<task>/`. You can check your files with `podman exec expr-naive /spikes/kit/check --no-tests <task> /answers/<task>`, which compiles them and checks the dialect but does not test behaviour. Use only the four files you were given.

Then grade the answer in `expr-go` as described above. The agent's files are already on the host under `<answers>`.
