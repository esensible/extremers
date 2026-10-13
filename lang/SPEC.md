# Expression language: specification (draft 4)

This document is the reference for every part of the language:
- **Compiler and checker** (`lang/compiler`): run off-device, natively and as WASM.
- **Executor** (`lang/vm`): `no_std`, no `alloc`.
- **Bytecode format** (`lang/bytecode`): shared by the compiler and the executor.
- **Go oracle and conformance corpus** (`lang/conformance`).

The language is **generic**. A program is one expression over named, immutable, typed inputs, and it yields one typed output. What the inputs mean, when a host evaluates the program, and which builtins a host provides all belong to the host. Each use of the language is a *profile*; the extremers engine profile is `engine/PROFILE.md`.

Anything still open is marked **(provisional)**.

**Changes since draft 3:**
- **Nil narrowing** (§3.3): after `p != nil`, `p` isn't nil, in the checker's eyes. It only changes which programs are accepted, never what one does.
- **Variants** (`oneOf`, §3.2): a record may have variants, exactly one of which is present, in inputs and outputs alike. They replace the flattened `oneOf` inputs of draft 3, which were never built.

**Changes since draft 2:**
- Everything application-specific has moved to the profile: the engine contract, gps, the sailing builtins, the smoke run's inputs, budgets, and when the host evaluates.
- Inputs are declared generically, and builtins are declared by the host.

**Probes.** Every statement about expr's behaviour was checked against expr v1.17.8 in the `expr-go` container (linux/arm64). Surprising facts are marked **(probe)**.

---

## 1. Programs and interfaces

### 1.1 The interface

A program is compiled against an **interface**, which the host or profile supplies:

```json
{
  "inputs": [
    {"name": "prev", "schema": {"$ref": "prev.schema.json"}, "nullable": true},
    {"name": "now",  "schema": {"type": "integer", "format": "int64"}},
    {"name": "xs",   "schema": {"type": "array", "maxItems": 10000, "items": {"$ref": "#/$defs/Sample"}}}
  ],
  "output": {"$ref": "prev.schema.json"},
  "builtins": [ … ]
}
```

- **`inputs`**: an ordered list.
  - Each input has a name, a schema (§3.2), and an optional `nullable` flag.
  - The wire subset has no null, so a whole input that may be missing is declared `nullable`. It is then typed `T?`.
  - An input's index in the list is its slot in the bytecode.
- **`output`**: a schema. The output is never `nil`.
- **`builtins`**: the host's builtin declarations (§2.4).

The **interface hash** is the first 8 bytes of SHA-256 over the interface's canonical JSON. It is written into the bytecode (§7.1).

### 1.2 Evaluation

The host provides every input, then evaluates the program **once**. Evaluation either produces the output, or **fails** with a run-time error code and the source span where it happened (§5.2). Programs have no side effects.

**The host guarantees:**
- **Inputs don't change during an evaluation.** Every read of an input returns the same bytes, from the first instruction to the last.
- **Inputs conform to their schemas:** ranges, enum values, string lengths and list lengths. The checker relies on this: an input schema's ranges and bounds are facts. A host that decodes an input from outside must validate it first.

**The output** always fits the output schema, at every position. The checker proves it (§3.4), and the VM re-checks each store as a safety net.

**Memory.** Inputs are owned by the host and read through an accessor (§8), so a host may keep a large input in flash. Inputs are **not** charged to the program. The program is charged for:
- its output buffer;
- its arena;
- its stack and locals;
- its code, if the host loads the code into RAM.

The caller passes the limits (§4.4).

---

## 2. The language

The language is **a subset of expr v1.17.8**, with one upstream fix applied: `findLast` and `findLastIndex` give their item or index wherever they're used. In v1.17.8 itself, used as an operand (`1 + findLast(xs, p)`, `{r: findLast(…)}`), they leave a stale bool on expr's stack and the expression goes wrong ([expr-lang/expr#950](https://github.com/expr-lang/expr/issues/950), fixed by [#954](https://github.com/expr-lang/expr/pull/954), merged 2026-04-12, unreleased as of v1.17.8). Every program the language accepts parses, and means the same thing in expr, apart from the differences in §2.6–2.7.

### 2.1 Grammar

The compiler accepts expr's syntax for:

- **Literals:**
  - ints: `42`, `1_000`, `0x10`;
  - floats: `2.5`, `1e3` (`1e400` is a compile error);
  - strings `"…"`, `true`, `false`, `nil`;
  - lists `[a, b]`;
  - records `{a: 1, "b": 2}`.

  Trailing commas are allowed in lists, records and argument lists (probe).
- **Names:**
  - the inputs;
  - `let x = v; rest`.

  Rebinding a `let` name is a compile error. So is binding the name of a builtin, whether it's an expr builtin or a host builtin (probe: `cannot redeclare builtin max`, `cannot redeclare function f`). So is binding an input's name: expr gives `cannot redeclare i` when `i` is in the environment (conformance oracle; an earlier probe that suggested otherwise used an environment without the input).
- **Access:**
  - `a.f` and `a?.f`;
  - `a["f"]`, with a literal key only;
  - `xs[i]` and `xs?.[i]`;
  - slices `xs[a:b]`, `xs[a:]`, `xs[:b]`.
- **Conditionals:** `c ? a : b`, and `if c { a } else { b }`. `else` is required (probe).
- **Operators:** `+ - * / % **`, `== != < <= > >=`, `&& || !`, `and or not`, `??`, `in`, `not in`, `..`, and the pipe `|`. Comparisons chain: `0 < x < 5` is `0 < x && x < 5` (probe).
- **Calls:** builtins only (§2.3, §2.4).
- **Predicates:** the expression argument of a list builtin.
  - `#` is the item, and `.f` is `#.f`.
  - `#index` exists only in `map` and `reduce`. It is the item's position in the list being iterated (probe: `map(xs[1:], #index)` is `[0, 1]`).
  - `#acc` exists only in `reduce`.
  - A record literal in a predicate needs parentheses, `map(xs, ({v: #}))`, unless it's the last expression of a braced block.
- **Comments:** `//` and `/* */`.

**Precedence, loosest first** (probe):

1. `|`
2. `?:`, `if`: right-nesting
3. `||`, `or`
4. `&&`, `and`
5. `== != < <= > >= in not in`: these chain
6. `..`
7. `+ -`
8. `* / %`
9. unary `- ! not`
10. `**`: right-associative
11. **`??`**
12. postfix `. ?. [] [:] ()`

**`??` binds tighter than every non-postfix operator.** For example:
- `1 + s.n ?? 5` is `1 + (s.n ?? 5)`;
- `!s.n ?? true` is `!(s.n ?? true)`;
- `now - prev?.start ?? now` is `now - (prev?.start ?? now)`.

A `??` expression can't be the left operand of another binary operator without parentheses. `s.n ?? 1 + 1` is a compile error with expr's message (probe).

Also, `-2 ** 2 == -4` and `2 ** 3 ** 2 == 512`.

### 2.2 Numbers, booleans, nil and strings

**Integers**
- `int` is i64.
- `+ - *`, unary `-` and `abs` wrap: `MaxInt + 1 == MinInt`, and `abs(MinInt) == MinInt`.
- `%` takes two ints and has the sign of its left operand. `MinInt % -1 == 0` (probe). `% 0` is a run-time error (`R-DIV0`), and is never folded into a compile error (§2.6).

**Floats**
- `float` is f64. An int mixed with a float gives a float.
- `/` and `**` always give floats.
- `x / 0` is `±Inf`, and `0 / 0` is `NaN`. Both are ordinary values.

**Conversions**
- `int(f)` truncates toward zero, saturates out of range, and gives 0 for NaN. That is arm64's behaviour; Go leaves it undefined.
- `float(i)` converts an int to a float.
- `round` rounds half away from zero. `round`, `floor` and `ceil` return floats, so write `int(round(x))` for an int.
- `abs` keeps its argument's type.

**Equality**
- `==` compares ints and floats numerically. Lists compare element by element, also numerically: `[1] == [1.0]`.
- Equality on records is **not supported** (§2.6).
- `nil == 0` is false.
- Strings support only `==`, `!=` and `in`.

**Nil**
- An absent record field reads as `nil`.
- `a.f` on `nil` is a run-time error. `a?.f` gives `nil` and short-circuits the rest of the chain.
- On `nil` these fail with `R-NIL`: `+ - * / % < > -x !x`, `len`, `int`, `map`, `filter`, `sum`, `reverse`, `concat`.
- On `nil` these don't fail: `first(nil)` and `last(nil)` give `nil`, and `x in nil` gives `false`.
- `??`, `?.` and a test (`p != nil ? … : …`) are the tools for nil. At run time the language adds no nil rules of its own: narrowing (§3.3) is a typing rule, and a narrowed program does exactly what expr does.

### 2.3 Builtins from expr

`n` is the length of the list argument. Arguments are evaluated left to right.

**Aggregates**

| Builtin | Result | Notes (probe) |
|---|---|---|
| `len(xs)` | int | Lists only. |
| `sum(xs)` | int if every item is an int, otherwise float | Accumulates left to right from int 0, and stays int until the first float. `sum([]) == 0`. |
| `mean(xs)`, `median(xs)` | float | Both give `0.0` on an empty list. `median` of an even count is the mean of the two middle values. |
| `min(xs)`, `max(xs)`, `min(a, b, …)`, `max(a, b, …)` | the winning item | `nil` on an empty list. The comparison is strict, so the first winner stays: `min(-0.0, 0.0) == -0.0`. A NaN in first place sticks; later NaNs are skipped. **Restricted:** exactly one `list<number>`, or two or more numbers (§2.6). The static type is the unification of the arguments. |
| `count(xs, p)` | int | `count(xs)` counts the `true`s. |
| `any`, `all`, `none`, `one` `(xs, p)` | bool | On an empty list, `any` is false and `all` and `none` are true. |

`mean`, `median`, `max` and `min` take no predicate: `max(xs, .v)` is a parse error in expr. Write `max(map(xs, .v))`.

**Finding things**

| Builtin | Result | Notes (probe) |
|---|---|---|
| `first(xs)`, `last(xs)` | item or `nil` | |
| `find`, `findLast` `(xs, p)` | item or `nil` | |
| `findIndex`, `findLastIndex` `(xs, p)` | int, or **`nil`** if nothing matches | Not `-1`. |
| `xs[i]` | item | A negative `i` counts from the end. Out of range is `R-INDEX`. `xs?.[i]` guards a nil list only. |
| `get(xs, i)` | item or `nil` | A safe index. Lists only, with an int `i`. A negative `i` counts from the end. |

**Building lists**

| Builtin | Result | Notes (probe) |
|---|---|---|
| `xs[a:b]` | list | Indices are clamped, so a slice never fails. |
| `map(xs, e)`, `filter(xs, p)` | list | |
| `reduce(xs, e[, init])` | the accumulator | With no `init`, an empty list is `R-ARG`. |
| `take(xs, k)` | the first `min(k, n)` items | A negative `k` is `R-ARG`. |
| `reverse(xs)` | list | |
| `uniq(xs)` | first occurrences, in order | Compares with `==`, so `1 == 1.0` and a NaN never matches anything. Not on records. |
| `sortBy(xs, e[, dir])`, `sort(xs[, dir])` | list | `dir` must be the literal `"asc"` or `"desc"`. Keys are numbers. A `nil` key is `R-ARG`. **The sort is stable.** NaN keys sort first. |
| `concat(a, b, …)` | list | Takes at least one argument. A `nil` argument is `R-NIL`. |
| `flatten(xs)` | list | |
| `a..b` | ints, inclusive | Empty when `b < a`. |
| `x in xs` | bool | Compares with `==`. Not on records. |

**Math:** `abs`, `int`, `float`, `round`, `floor`, `ceil`.

### 2.4 Host builtins

A host adds builtins, as `expr.Function` does in Go. Each one has two halves:
- a **declaration** in the interface's `builtins`, which the compiler type-checks calls against;
- an **implementation** that the host registers with the VM (§8).

The language crate itself ships only expr's builtins (§2.3). An application's builtins live with the application; the extremers engine profile's are in `engine/builtins`.

**A declaration.** Parameters and the result are typed by schemas in the same subset as inputs and outputs (§3.2):

```json
{"name": "courses_cross",
 "params": [{"name": "pos1", "schema": {"$ref": "position"}}, {"name": "heading1", "schema": {"type": "number"}},
            {"name": "pos2", "schema": {"$ref": "position"}}, {"name": "heading2", "schema": {"type": "number"}},
            {"name": "range", "schema": {"type": "number"}, "optional": true}],
 "result": {"$ref": "position"}, "nullable": true}
```

- `name`: an identifier that isn't one of expr's builtins or an input's name.
- `params`: each has a `name` and a `schema`, and may be `"optional": true`. Only trailing parameters can be optional; leaving one out passes nil.
- `result`: a schema. `"nullable": true` makes the result `T?`.
- `description`: free text, ignored.
- (`$ref` here stands for a schema the host's tooling substitutes; the compiler sees the schema itself.)

**Calls** are checked against the declaration:
- the argument count, between the required and the total number of parameters;
- each argument against its parameter. An int passes where a float is wanted; nil only where the parameter is nullable; a **record argument is structural**: it needs the parameter's fields, and may have more (a gps fix with `alt` passes as a `{lat, lon}` position).
- A builtin's name can't be bound with `let`, and is an error used without a call.

**The calling convention** is the leaf layout of §7.3, the same as inputs and outputs:
- Each argument is laid out as its *parameter's* type: a record argument contributes only the parameter's fields, in field-name order; a left-out optional parameter contributes nils.
- The result is the result type's leaves. An optional record result is a presence leaf (true, or nil when absent, when its fields are nil too) and then its fields.
- Leaves are VM values: a numeric leaf may be an int or a float, and the implementation accepts either.
- Results stay on the stack as values; nothing is built in an arena.

**Example.** `courses_cross(p1, h1, p2, h2[, range])` receives the 7 leaves `p1.lat, p1.lon, h1, p2.lat, p2.lon, h2, range` (`range` nil when left out) and writes the 3 leaves `present, lat, lon`.

**Linking.** Each declaration has a **signature**: its name and the leaf shapes of its parameters and result, e.g. `distance({lat:f,lon:f},{lat:f,lon:f})f` and `courses_cross({lat:f,lon:f},f,{lat:f,lon:f},f,?f)?{lat:f,lon:f}` (`f` float, `i` int, `b` bool, `s` string, `{…}` a record's fields in name order, `?` nullable; ranges, enums and lengths aren't part of it). The bytecode lists the signatures of the builtins the program calls (§7.1), and the VM links each to the host builtin with exactly that signature at load. A missing or mismatched builtin fails the load, never an evaluation.

**Purity and failure.** Builtins must be pure and deterministic: the same arguments always give the same result. An implementation may fail (e.g. a negative range), which fails the evaluation with `R-ARG`.

**Not yet:**
- Builtins that take lists (e.g. a weighted mean of headings) come with lists.
- A per-builtin cost for the work estimate (§4.5) comes with the work estimate.

### 2.5 Excluded

| Excluded | Why, and what to write instead |
|---|---|
| `now()`, `date`, `duration`, `timezone` | Programs are pure. Time comes in as an input. |
| String building: `+` on strings, `string()`, `len(s)`, `split`, `upper`, `contains`, `startsWith`, `endsWith`, `matches`, … | **Programs output numbers, and whatever displays them formats them.** Strings are literals or schema values, compared with `== != in`. |
| Dynamic keys: `m[k]` with a non-literal `k`, `$env`, `keys`, `values`, `toPairs`, `fromPairs`, `groupBy`, `get(record, k)` | Record shapes are static (§3). |
| `toJSON`, `fromJSON`, `type`, `len(record)`, bitwise and regex operators, `x in record` | Their size or type isn't static. |
| A record literal that repeats a key | expr is inconsistent here (probe). |

**The bounded-list idiom.** `concat(prev?.recent ?? [], [x])[-5:]` has a static bound of 5.

The trap: once the list is capped, `len(prev.recent)` stops growing. Keep a separate count if you need one. The help for `E-OUTPUT-BOUND` says so.

### 2.6 Where we differ from expr

The checker is statically typed, so it rejects some programs that expr would run. The conformance corpus contains only programs that both sides accept.

| Program | expr (probe) | Us |
|---|---|---|
| a float index or slice bound, `1..f` | runs | `E-TYPE` ("`/` gives a float; use `int(…)`") |
| `b && x` with `b` nil, or a `bool?` condition | `nil`, or a run-time error | `E-TYPE` ("write `… ?? false`") |
| `c ? "x" : 2` | allowed | `E-TYPE` |
| `c ? 1 : 2.5` | the int `1` | the float `1.0`; equal numerically, and the type can't be observed |
| `==`, `in`, `uniq` on records | deep equality, int ≠ float | `E-UNSUPPORTED` |
| `min`/`max`/`mean`/`median` on nested lists, or lists mixed with scalars | flattens them | `E-TYPE` |
| `sum([m, 1, 2.5])` near `MaxInt` | the int accumulation wraps first | the list unifies to floats first; such programs are kept out of the corpus |
| a non-literal sort direction | checked at run time | `E-TYPE` |
| `1 % 0` with constants | a compile error under expr's optimizer | `R-DIV0` at run time; the oracle runs unoptimised (§9) |
| `{a: 1, a: 2}` | inconsistent | `E-DUP-KEY` |
| errors in items that nothing consumes | raised, because expr is eager | not raised (§2.7) |

### 2.7 Lazy evaluation

Lists are **lazy**: an item is computed only if a sink consumes it.
- `take(map(xs, 1 % #), 1)`, `first(map(…))`, `len(map(…))` and `map(…)[:1]` raise no error from items they never reach. expr runs `map` eagerly, so in expr they fail (probe).
- Dropping a sort (H2) likewise skips evaluating its keys.

This is a deliberate difference from expr. Two rules go with it:
- **Fusion never runs code that expr would not run.** Aggregates are fused into one loop only when they are evaluated under the same condition (§6.2).
- **Errors are the only observable effect,** so the evaluation order can't otherwise be seen.

---

## 3. Types

### 3.1 Types

| Type | Notes |
|---|---|
| `int[lo..hi]` | i64 with a known range; the full range when nothing is known |
| `float` | f64 |
| `bool` | |
| `string[≤m]` | `m` is the maximum length in bytes |
| `enum{"a", "b", …}` | a string known to be one of these values; a string literal has type `enum{"lit"}` |
| `nil` | |
| `{f1: T1, …}` | a record with a fixed set of fields; optional fields are `T?` |
| `{f1: T1, …; one of v1, v2, …}` | a record with **variants**: each `vi` is a field of type `{…}?`, and exactly one of them is present (§3.2) |
| `list<T>[≤n]` | |
| `T?` | `T` or `nil` |

`[]` has type `list<never>[≤0]` and unifies with any list type. `{}` is a record with no fields.

A record type carries its variants only while every value of it is known to have exactly one present: a schema's type does, and so does a unification that keeps the rule (§3.3). A record literal's type has no variants of its own; it is checked against the variants where it's used (§3.4).

### 3.2 Schemas: the wire subset, plus bounds

Every input schema and the output schema must first be valid under `wire/SPEC.md` §2. The language then adds the requirements below. Each one is an `E-SCHEMA` error that names the path.

| Wire kind | Language type | Language requirement |
|---|---|---|
| `integer` + `format` | `int[lo..hi]`: the width intersected with `minimum`/`maximum` | `uint64` is rejected, because it can exceed i64. |
| `number` (`double`/`float`) | `float` | A `format: float` value is held as f64 and rounded to binary32 when it's encoded. |
| `boolean` | `bool` | |
| `string` | `string[≤4·maxLength]` | `maxLength` is required. `contentEncoding` (bytes) is rejected. |
| `enum` / `const` | `enum{…}` | |
| `object` | a record; fields not in `required` are `T?` | Optional means *absent*. There are no null values (wire §2.4). |
| `array` | `list<T>[≤maxItems]` | `maxItems` is required. Items can't be optional. |
| `oneOf` | a record with variants (below) | Inputs and the output. Not inside list items, nor in a builtin's parameters or result, yet. |

**Size limits**
- A list holds at most 65,535 items.
- A string holds at most 65,535 bytes, so `maxLength` is at most 16,383.
- Layout sizes are computed in u64 and must fit in u32.

**Variants (`oneOf`)**

An object schema may carry `oneOf`. Each alternative names one variant: it has exactly one property, the variant's name, which it lists in `required`, and the variant's schema is an object schema:

```json
{"type": "object",
 "properties": {"line": {"type": "object", "properties": {"a": {"type": "integer"}}}},
 "required": ["line"],
 "oneOf": [
   {"properties": {"idle": {"type": "object"}}, "required": ["idle"]},
   {"properties": {"countdown": {"type": "object", "properties": {"gun": {"type": "integer"}}, "required": ["gun"]}}, "required": ["countdown"]},
   {"properties": {"racing": {"type": "object", "properties": {"gun": {"type": "integer"}}, "required": ["gun"]}}, "required": ["racing"]}
 ]}
```

- The type is the record `{line: {a: int?}, idle: {}?, countdown: {gun: int}?, racing: {gun: int}?; one of countdown, idle, racing}`: each variant is an optional record field, and **exactly one of them is present**. Ordinary `properties` and `required` beside `oneOf` are plain fields.
- A schema of only `{"oneOf": [...]}` (a field's schema, say) is a record with variants and no plain fields. A `type`, if given, must be `"object"`.
- A variant's own record may have variants, at any depth.
- `E-SCHEMA` for: an alternative that isn't a record (`{"type": "integer"}`), or has no property, or more than one (an untagged alternative, or one tagged by a property as in `wire/SPEC.md` §2.2: the message says so); a variant whose schema isn't an object schema; a variant not in its alternative's `required`; a name used for two variants, or for a variant and a plain property; more than 256 alternatives; `oneOf` inside list items or in a builtin's declaration ("not yet").
- In JSON (and in Go), a value with variants is an object with exactly one of the variants' keys.

> **As built:** the wire encodes this form directly, as a record with variants (`wire/SPEC.md` §2.2, "Records with variants"), with no tag field added.

### 3.3 Typing rules

- **Inputs** have their schema type, which is `T?` when the input is declared `nullable`.
- **Literals** have their own type. A list literal's element type is the unification of its items.
- **Unification**, used for branches, list items and `??`:

  | Operands | Result |
  |---|---|
  | `T` with `T` | `T` |
  | `nil` with `T` | `T?` |
  | `int` with `float` | `float` (the int is converted) |
  | `enum A` with `enum B` | `enum A∪B` |
  | `enum` with `string[≤m]` | `string[≤max]` |
  | two lists | element-wise, with the larger bound |
  | two records | field by field; a field only one side has becomes optional, since an absent field reads as nil (as in expr). The result keeps one side's variants if the other side is sure to keep the same rule: it has the same variants, or (by its type) exactly one of them as a field that can't be nil and none of the others, as `{idle: {}}` has |
  | anything else | `E-TYPE` |

  `a ?? b` has type `unify(nonnil(a), b)`.
- **Members.** An unknown field of a known record is `E-NAME`, which suggests the nearest field. `.` on `nil` fails at run time, as in expr. A variant reads like any optional field: `v.countdown` is `{gun: int}?`, and `v.countdown?.gun` is `int?`.
- **Operands.**
  - Conditions and `&& || !` need `bool`. A `bool?` is `E-TYPE`.
  - Indices and slice bounds need `int`.
  - Arithmetic on a `T?` is allowed, and fails at run time on `nil`.
- **Builtin calls** are checked against §2.3 or the declaration (§2.4).

**Narrowing.** A test against `nil` tells the checker a value isn't nil where the test has passed:

- **Paths.** A test is about a *path*: an input, a `let` name, `#` or `#acc`, followed by any number of field reads, `.f` or `?.f`. `.` and `?.` name the same path (`s?.last` and `s.last` are one path).
- **What a condition shows.** For a condition `c`, the checker works out the paths that aren't nil when `c` is true, and when it's false:

  | `c` | true: not nil | false: not nil |
  |---|---|---|
  | `p != nil`, `nil != p`, and for a record `p != nil` | `p` and each prefix of `p` | — |
  | `p == nil`, `nil == p` | — | `p` and each prefix of `p` |
  | `!a` | `a`'s false | `a`'s true |
  | `a && b` | `a`'s true ∪ `b`'s true | `a`'s false ∩ `b`'s false |
  | `a \|\| b` | `a`'s true ∩ `b`'s true | `a`'s false ∪ `b`'s false |
  | a boolean `let` name | its value's | its value's |
  | `let x = v; body` | `body`'s | `body`'s |
  | anything else | — | — |

  A prefix is narrowed too because a path that isn't nil was reached without failing or short-circuiting: if `s?.last` isn't nil, nor is `s`.
- **Where it applies.**
  - `c ? a : b` (and `if`): `a` is checked with `c`'s true paths, `b` with its false paths.
  - `l && r`: `r` is checked with `l`'s true paths. `l || r`: `r` is checked with `l`'s false paths.
  - Nowhere else: the paths are known inside that branch or right side only, and not after the conditional ends.
- **The narrowed type.** A read of a narrowed path, or of a longer path through it, has the nil taken out: `T?` becomes `T`, and a `?.` on a narrowed prefix doesn't make the rest nullable. A narrowed path can go where nil isn't allowed: an output position, a builtin's argument, a variant (§3.4).
- **Why it's sound.** Inputs and names never change during an evaluation (§1.2), so a path that wasn't nil when tested isn't nil when read.
- **Code.** Narrowing never changes what a program computes. A read through a narrowed optional record skips the nil check (`.` on nil would fail, but it can't be nil), which saves instructions; layouts don't change, so neither do the memory figures' rules (§7.3).
- **Not narrowed (as built):** a value inside a list after `filter` (`map(filter(xs, .a != nil), .a.b)`), an expression that isn't a path (`(p ?? q).f`, a call's result), a comparison of a test (`(p != nil) == true`), and variants by elimination (that `v.a` and `v.b` are nil doesn't narrow `v.c`).

For example, with `s` a nullable `{count: int, last: {lat, lon}?}`:

```
s?.last != nil ? dist(s.last, p) : 0.0          // s.last is {lat, lon} in the first branch
let ok = n != nil && m != nil; ok ? {a: n, b: m} : {a: 0, b: 0}   // n and m are ints
n == nil || n < 0 ? 0 : n                       // n is an int on the right of || and in the else branch
```

### 3.4 The output

The program's value must fit the output schema **recursively**, at every position:

| Rule | Error |
|---|---|
| It has the right shape. A record has its required fields, may leave optional fields absent, and has no unknown fields. | `E-FIELD` |
| An `int` position takes no `float`. A non-nullable position takes no `T?`. | `E-TYPE` |
| An int's static range lies inside the position's range. An enum's set lies inside the position's enum. | `E-RANGE` |
| A string's bound is at most the position's. Every list bound, at every depth, is at most the position's `maxItems`. | `E-OUTPUT-BOUND` |
| A position with variants gets a value with exactly one of them present (below). | `E-VARIANT` |

**Exactly one variant.** The checker follows the value at a position with variants back to the expressions that make it, and accepts it when every one of them is:
- a **record literal** with exactly one of the variants as a field, whose value can't be nil (narrowed values count), and none of the others (or the others `nil`): `{idle: {}}`, `{racing: {gun: x}}` with `x` not nil;
- a value whose **type** has these variants (or some of them, and none of the others as fields): an input or a field of one passed on, `s.phase`, or a narrowed path;
- `c ? a : b` with `a` and `b` accepted; `a ?? b` with `a`'s non-nil value and `b` accepted; `let x = v; body` with `body` accepted; a `let` name whose value is accepted; a field of a value it can follow (a record literal's field, a name's value, each branch);
- `nil`, where the position is optional.

Anything else is `E-VARIANT`, naming what is wrong: two variants set, none, a variant that may be nil, or a value whose type doesn't show the rule (e.g. one taken out of a list of records). The rest of the value is checked as any record: `{countdown: {}}` is `E-FIELD` for the missing `gun`, and `{}` stands for a record whose fields are all optional.

So with `state` a nullable `{phase: …; one of idle, countdown, racing}`, `state ?? {phase: {idle: {}}}` is accepted, and so is passing that on as `s` in a branch.

The VM re-checks every int, enum, string and list store into the output (`R-RANGE`), and that each variant group in the output has exactly one variant present (`R-RANGE`). That check is a safety net against compiler bugs, never a substitute for the static one.

---

## 4. Bounds and holds

**The principle.** Every list has a static length bound. A list either:
- **flows**: each item is produced and consumed at once, costing no RAM; or
- is **held**: it sits in an arena buffer.

The charges below are the **most the compiler may use**. Accepting or rejecting a program always uses them, so the decision never depends on whether an optimisation happened.

### 4.1 Integer ranges

Ranges are computed **exactly in i128**. If a result could leave i64 (so it could wrap at run time), its range becomes the full range.

| Expression | Range |
|---|---|
| literal `k` | `[k..k]` |
| a schema int (an input, or a field of one) | the schema's range |
| `len(xs)`, `count(xs, p)` | `[0..bound(xs)]` |
| `#index` in a loop over `xs` | `[0..bound(xs)−1]` |
| `findIndex`, `findLastIndex` | `[0..bound(xs)−1]?` |
| `a + b`, `a − b`, `a * b`, `-a` | exact |
| `min(a, b)`, `max(a, b)` | `[min(lo)..min(hi)]`, `[max(lo)..max(hi)]` |
| `c ? a : b`, `a ?? b` | the union |
| a `reduce` accumulator | widened (H8) |
| anything else (`int(f)`, `%`, `abs`, an `int` builtin result) | the full range |

### 4.2 Length bounds

| Expression | Bound |
|---|---|
| a list input, or a list inside an input | `maxItems` |
| a list literal | its item count |
| `a..b` | `max(0, hi(b) − lo(a) + 1)` |
| `map`, `filter`, `reverse`, `sortBy`, `sort` or `uniq` of `xs` | `n` |
| `take(xs, k)` | `min(n, max(0, hi(k)))` |
| `xs[a:]` | constant `a < 0`: `min(n, −a)`; constant `a ≥ 0`: `max(0, n − a)` |
| `xs[:b]` | constant `b ≥ 0`: `min(n, b)`; constant `b < 0`: `max(0, n + b)` |
| `xs[a:b]`, constants `0 ≤ a ≤ b` | `min(n, b − a)` |
| any other slice | `n` |
| `concat(a, b, …)` | the sum of the bounds |
| `flatten(xs)` | the product of the bounds at every level |
| `c ? a : b`, `a ?? b` | the larger of the two |

A bound only matters for holds, for output positions and for work. A flowing list doesn't need a useful bound: `sum(lo..hi)` is fine.

### 4.3 What is held, and the charge

`size(T)` is the size of T's layout (§7.3).

**H1. A list written into the output.**
- Charge: none. The list is built in place in the output layout, and its bound must be at most `maxItems` (§3.4).
- A `sortBy`, `sort` or `uniq` written straight into the output is done in place in that list.

**H2. `sortBy` and `sort`.**
- Charge: `n × (8 + size(T))`. The buffer holds `(key, item)` pairs.
- The consumer can lower the charge. These rewrites are **mandatory**:
  - `first`, `last`, `[0]`, `[-1]` → min-by or max-by (H10).
  - `take(…, k)` or `[:k]`, with `hi(k)` known → a sorted buffer of `hi(k)` pairs. `[-k:]`, and `reverse` followed by `take`, are the same from the other end.
  - `find(…, p)` → min-by over `filter(xs, p)`.
  - `sum`, `count`, `len`, `mean`, `min`, `max`, `any`, `all`, `none`, `one`, `in` → the sort is dropped. A float `sum` or `mean` may then differ in its last bits.
- Ties follow the stable sort. `first` of an ascending sort is the first minimum, and `last` is the last maximum.

**H3. `uniq`.**
- Charge: `n × size(T)`.
- `first` costs 1 item, and `take(…, k)` costs `hi(k)` items.

**H4. `median`.**
- Charge: `n × 8`. Values are held as f64.

**H5. `reverse` of a non-indexable pipeline.**
- Charge: none, if the source can be walked from its end (`Input`, `Layout`, `Literal`, `Range` or `Held`) and the stages are only `Map`, `Filter` and `Slice`.
- Otherwise `n × size(T)`, or 0 if the consumer ignores order.

**H6. `ys[i]`, where `ys` is a non-indexable pipeline.**
- **A single read, not in a loop over `ys`:** replay `ys` up to `i`. No charge.
- **A look-back inside a loop over `ys[c:]`** (constant `c ≥ 0`): `ys[#index + d]` with a constant `d ≤ c` reads `k = c − d` items back. It costs a ring buffer of `k + 1` items.
  - That covers `ys[#index − 1]` in `map(ys, …)`, and the pairs idiom `map(ys[1:], f(ys[#index], #))`.
  - If `#index + d − c` can be negative, expr wraps to the end of the list (probe: `map(l, l[#index − 1])` is `[3, 1, 2]`). The ring is then filled first from a replay of the last `k` items, for a charge of `2k + 1`.
  - A guard such as `#index >= k ? … : …` lets the range analysis keep the charge at `k + 1`.
  - `ys` must be the same list the loop iterates: the same `let` name, or a structurally identical expression.
- **A look-ahead** (`d > c`), or any index that only increases, is a second cursor over a replay. No charge.
- **Any other index:** `n × size(T)`.

**H7. A list used more than once.**
- Charge: none. Uses evaluated under the same condition are fused (§6.2). Other uses replay the list, which costs work, not memory.

**H8. A `reduce` accumulator.**
- Charge: `2 × size(acc)`. The two copies swap after each item, so the body can read the old `#acc` while it writes the new one.
- The accumulator's type comes from a fixed point over the body, with widening:
  - an int range still moving after two rounds becomes the full range;
  - a list bound that keeps growing is `E-ACC-GROWS`.
- `concat(#acc, [#])[-5:]` settles at 5.

**H9. Record temporaries.**
- Charge: `size` of every record that is built and not written straight into the output. That includes literals, record results of host builtins, and a whole record read out of a list (`last(xs)`).
- A record that is only taken apart may be replaced by its scalars, but the charge is still made.

**H10. Items kept across iterations.**
- Applies to `first`, `last`, `find`, `findLast`, `min`, `max`, min-by and max-by, and to a `reduce` that keeps a record or string item.
- Charge: one copy of the item, `size(T)`. Scalars live in locals at no arena cost.
- A `Str` or `Ref` slot never outlives the buffer it points into.

**H11. Lockstep walks.**
- Applies to aggregate builtins over several lists (§2.4) and to list `==`/`!=`. Each is a `Zip` (§6.1).
- Charge: none for the walk; the cursors live in locals. An aggregate builtin's `acc_bytes` is charged.

### 4.4 Peak memory and limits

```
program RAM = size(output)
            + arena                 (the watermark of H2–H11 buffers and temporaries)
            + 16 × (max_stack + locals)
            + code                  (0 if the host runs the program in place)
```

- Buffers whose lifetimes don't overlap share bytes (§6.3).
- Every `bound × size` product is computed in saturating u64.
- **The caller passes the limits** to the compiler: arena, output, stack and locals, total, and work. Exceeding any of them is `E-LIMIT`, reported with a breakdown.
- The language has no built-in budget.

### 4.5 Work

**Work** is the static instruction count at full bounds, using the same costs the VM charges (§8):
- loops multiply;
- each replay counts as a full loop;
- holds count their real work:
  - an insertion sort is `n²/2` moves;
  - `uniq` is `n × bound`;
  - top-k is `n × k`;
- host builtins cost their declared `cost`.

The compiler always reports the work. If the caller passes a work limit, work over it is `E-WORK`. Every loop runs over a list with a static bound, so the work is bounded at compile time; the VM has no instruction budget.

---

## 5. Diagnostics

Every diagnostic has:
- a stable rule id;
- a span;
- a `message` that states the facts;
- a `help` line with a concrete fix.

The JSON form:

```json
{"rule": "E-HOLD", "severity": "error",
 "span": {"start": {"line": 3, "col": 5}, "end": {"line": 3, "col": 33}},
 "message": "sortBy holds up to 10,000 items (330,000 bytes); the arena limit is 8,192 bytes",
 "help": "slice the input first (xs[-60:]), or keep only what you need: first(…), take(sortBy(…), k)",
 "data": {"bound": 10000, "element_bytes": 33, "bytes": 330000, "limit": 8192}}
```

The text form is `file:line:col: error[E-HOLD]: message`, then `  help: …`.

Profiles may add their own wording to the help, for example naming their inputs.

### 5.1 Compile-time rules

**E-PARSE.** expr's parser message, plus a fixed hint for each common slip:

| Slip | Hint |
|---|---|
| `map(xs, {v: #})` | "a record inside a predicate needs parentheses: `map(xs, ({v: #}))`" |
| a newline instead of `;` after `let` | "each `let` ends with `;`: `let a = 1; let b = 2; …`" |
| `if c { a }` | "`if` needs an `else`: `if c { a } else { b }`" |
| `then` | "write `c ? a : b`" |
| `=>` | "expr has no lambdas: use a predicate with `#`" |
| `xs.map(` | "write `map(xs, …)`" |
| `...` | "expr has no spread: write every field" |
| `round(x, 2)` | "`round` takes one argument: `round(x * 100) / 100`" |
| `max(xs, .v)` (also `min`, `mean`, `median`) | "these take no predicate: `max(map(xs, .v))`" |
| `===` | "use `==`" |
| `{a, b}` | "write every field: `{a: a, b: b}`" |
| `s.n ?? 1 + 1` | "wrap the `??`: `(s.n ?? 1) + 1`" |

**The other compile-time rules:**

| Rule | Example | Message / help |
|---|---|---|
| `E-NAME` | `prev.cout`, `pow(x, 2)`, `Math.floor`, `xs.length` | "`Prev` has no field `cout`" / "did you mean `count`?"; "`pow` isn't a builtin" / "use `x ** y`"; "lists have no fields" / "use `len(xs)`" |
| `E-REDECLARE` | `let max = …` | "`max` is a builtin and can't be a name" / "rename it, e.g. `max_v`" |
| `E-POINTER` | `filter(xs, #index > 2)` | "`#index` exists only in `map` and `reduce`" / "to skip items use a slice: `xs[3:]`; for one item use `xs[k]`" |
| `E-UNSUPPORTED` | `"a" + b`, `now()`, `m[k]` | "`{feature}` isn't available" / a fix for each feature. Strings: "output numbers; format them where they're displayed". Time: "pass time in as an input". Capped list: "`concat(xs, [x])[-N:]`" |
| `E-TYPE` | `elapsed_ms: t / 1000` | "`elapsed_ms` is an integer but this is a float (`/` always gives a float)" / "`int(t / 1000)`" |
| `E-TYPE` (condition) | `prev?.paused ? …` | "`prev?.paused` may be nil, so it can't be a condition" / "`prev?.paused ?? false`" |
| `E-TYPE` (nil into a position) | `best: max(xs)` | "`best` can't be nil, but `max(xs)` is nil for an empty list" / "`max(xs) ?? 0.0`, or make `best` optional" |
| `E-TYPE` (a nullable path into a position) | `{count: n}` | as built: "the output field `count` can't be nil, but this is an integer or nil" / "test it first: `n != nil ? … : …` (in the first branch `n` isn't nil), or give a default: `n ?? 0`" |
| `E-ARG` (a nullable path as an argument) | `dist(s?.last, p)` | as built: "the argument `a` of `dist`: this may be nil (…), but nil isn't allowed here" / "test it first: `s?.last != nil ? dist(…) : …` (in the first branch `s?.last` isn't nil), or give a default: `s?.last ?? …`" |
| `E-VARIANT` | `{phase: {idle: {}, racing: {gun: 1}}}`; `{phase: {}}`; `{phase: {racing: r}}` with `r` nullable | "the output field `phase` must have exactly one of `countdown`, `idle`, `racing`, but this sets 2 of them: `idle` and `racing`" / "set only one; to choose between them, use a condition: `c ? {a: …} : {b: …}`"; "… but this sets none of them" / "set one, e.g. `countdown: {…}`"; "… but the variant `racing` here may be nil" / "choose the variant with a condition, e.g. `r != nil ? {racing: r} : {countdown: …}`"; "… but this is a record {idle, racing} and isn't known to have exactly one" / "build it with one variant, e.g. `{countdown: {…}}`, or pass on a value that has these variants" |
| `E-TYPE` (`%`, an index) | `x % 60` with a float `x`; `xs[n / 2]` | "`%` needs ints" / "`int(x) % 60`"; "an index must be an int" / "`xs[int(n / 2)]`" |
| `E-ARG` | a builtin called with the wrong arity or types | "`{name}` takes ({params})" / the declaration's signature |
| `E-FIELD` | the result lacks `total` | "the output is missing `total`" / "`total: prev?.total ?? 0.0`" |
| `E-RANGE` | `n: prev.n + 1` where `maximum` is 10 | "`n` is at most 10 but this can be 11" / "clamp it: `min(prev.n + 1, 10)`" |
| `E-OUTPUT-BOUND` | `recent: filter(xs, …)` | "`recent` holds at most {maxItems} but this can have {bound}" / "keep the last N: `(…)[-{maxItems}:]`. If you count with `len(recent)`, keep a separate count: a capped list stops growing" |
| `E-HOLD` | `sortBy(filter(xs, …), .v)` | "`{builtin}` holds up to {bound} {items} ({bytes} bytes); the arena limit is {limit}" / "slice the input first (`xs[-N:]`), or take only what you need (`first`, `take(…, k)`, `min`, `max`)" |
| `E-HOLD` (index) | `ys[i]` in a loop, where `ys` is filtered | "`ys[…]` in this loop needs all of `ys` held ({bytes} bytes), because a filtered list can't be indexed" / "look back by a constant: `ys[#index - 1]` in `map(ys, …)`, or `ys[#index]` in `map(ys[1:], …)`" |
| `E-UNBOUNDED` | `take(sortBy(xs, .v), prev.n)` with no `maximum` on `n` | "the size of `{expr}` depends on `{value}`, which has no upper bound" / "add `maximum` to `{path}`, or cap it: `min({value}, N)`" |
| `E-ACC-GROWS` | `reduce(xs, concat(#acc, [#]), [])` | "this `reduce` accumulator grows with every item" / "`concat(#acc, [#])[-N:]`, or use `map`/`filter`" |
| `E-LIMIT` | (the totals) | "the program needs {total} bytes (output {o}, buffers {a}, stack {k}); the limit is {limit}" / the largest item's own help, with its span |
| `E-WORK` | `map(xs, count(xs, …))` against a work limit | "about {work} instructions (loops over {bounds} multiply)" / "slice the input (`xs[-N:]`), or carry a running total in the output" |
| `E-SCHEMA` | an array without `maxItems` | "`{path}`: {problem}" / "add `maxItems`: every list needs a maximum length" |
| `E-DUP-KEY` | `{a: 1, a: 2}` | "key `a` appears twice" / "keep one" |

### 5.2 Run-time errors

| Code | Meaning |
|---|---|
| `R-NIL` | `nil` where a value is needed: member access, arithmetic, `!`, or a list builtin |
| `R-INDEX` | an index out of range |
| `R-DIV0` | `% 0` |
| `R-ARG` | an invalid argument: a negative `take`, `reduce` with no `init` on an empty list, a `nil` sort key, an aggregate's lists of different lengths, or a host builtin's own failure |
| `R-RANGE` | a store into the output outside its schema (the safety net, §3.4) |
| `R-INPUT` | an input is malformed: the wrong size, an accessor failure, a required input missing, or a variant group without exactly one variant present |
| `R-TYPE` | any other tag mismatch, or a buffer at capacity: a compiler bug |

### 5.3 Sample runs in `langc check`

After the static checks pass, `langc check` can run the program on the host VM over **samples the caller supplies**:

```json
{"runs": [
  {"inputs": {"prev": null, "now": 1790000000000, "xs": []}},
  {"inputs": {"now": 1790000001000, "xs": []}, "chain": {"prev": "output"}}
]}
```

- `chain` feeds a run the output of the run before it, as the named input.
- A failed run is reported as an error, with its `R-*` code, its span, and that code's help. For example, `R-NIL`: "a nullable input is nil here: use `?.` and `??`".
- Profiles define which samples to supply.

---

## 6. Plan IR: fusion by construction

### 6.1 Pipelines

Every list-typed expression is lowered to a **pipeline**: a source, then stages, then exactly one sink. A list is never a value on the stack.

**Sources**
- `Input(i, path)`: a list in an input.
- `Layout(ref)`: a list in the output or the arena.
- `Literal(items)`.
- `Range(a, b)`: counts `b − a + 1` items, computed in i128, so it never loops past `MaxInt`.
- `Concat(p…)`.
- `Cond(c, p1, p2)`: `c` is re-evaluated on each replay. That is safe because `c` is pure.
- `Held(buffer)`.
- `Zip(p1, p2, …)`: several cursors walked together.

**Stages:** `Map(e)`, `Filter(p)`, `Slice(a, b)`, `Take(k)`, `Reverse`, `Flatten`.

**Sinks:** `Aggregate(kind)` (including host aggregates), `WriteList(dest)`, `Hold(buffer, mode)` with mode `plain`, `sorted(dir, k?)`, `unique` or `values`, and `Index(i)`.

**Indexable pipelines.** A pipeline is *indexable* (item `i` in O(1)) when:
- its source is `Input`, `Layout`, `Literal`, `Range` or `Held`, or a `Concat` of indexable parts; and
- its stages are only `Map`, `Slice`, `Take` and `Reverse`.

`Filter` and `Flatten` make a pipeline non-indexable.

**Replay sources** must not change while any pipeline reading them is live:
- inputs, which is a host guarantee (§1.2);
- `Const`;
- arena buffers outside their write window.

A `reduce` accumulator is double-buffered (H8), so replays can read it.

**Cursor state** lives in locals, and its size is static:
- one counter per stage;
- one part index per `Concat`;
- one index per `Flatten` level.

### 6.2 Lowering

- **Loops.** Each sink is one loop over its source, with the stages inlined. A predicate that uses another list becomes a nested loop.
- **Fusing aggregates.** Several aggregates of the same list fuse into one loop, with one accumulator each, **only when they are evaluated under the same condition**. In `len(ys) > 0 ? sum(ys) / len(ys) : 0.0`, the `sum` doesn't fuse with the guard's `len`.
- **Replay.** Any other repeated use replays the pipeline.
- **Sort rewrites** (H2) happen during lowering and are mandatory.
- **Negative slices.** A negative slice of a non-indexable pipeline counts the pipeline's length first, by a replay.
- **Equality.** List `==` lowers to a `Zip` with an `all` sink, and `x in xs` to an `any` sink.
- **Host builtins.** A host aggregate lowers to a `Zip` of its lists with an `AggStep` sink.

### 6.3 The static arena layout

Every hold buffer, ring buffer, record temporary, kept copy, accumulator and aggregate `acc` gets a fixed arena offset.
- An allocation lives from its first use to its last, *including every replay of any `let` that depends on it, transitively*.
- Allocations whose lifetimes don't overlap share bytes.
- The watermark is `arena_size`.
- Scalars live in locals.

Nothing is allocated at run time.

---

## 7. Bytecode and arena format

**Ownership.** This section fixes what the bytecode means. The `lang/bytecode` crate fixes the exact encodings:
- `Op` and its operands;
- `ErrorCode` numbers;
- layout-node kinds;
- `Slot`/`Tag`;
- the encoder, decoder and validator, with round-trip tests.

**One owner writes the crate and freezes it before the compiler and VM strands start.**

All numbers are little-endian. Offsets and indices are u32 unless stated otherwise.

### 7.1 Container

**Header**

| Field | Size | Contents |
|---|---|---|
| magic | 4 | `EXP1` |
| version | u16 | 1 |
| flags | u16 | 0 |
| arena_size | u32 | bytes |
| max_stack | u16 | slots |
| locals | u16 | slots |
| output_layout | u32 | an index into the layout table |
| output_size | u32 | must match the layout table |
| input_count | u8 | |
| inputs | | per input: `name` (a `CONST` offset), `layout` (an index), `size` (u32), `nullable` (u8) |
| interface_hash | 8 | §1.1. The host checks it; the VM ignores it. |
| section_count | u8 | |
| sections | | each is `kind u8, len u32, bytes` |

**Sections:** `1 CONST`, `2 LAYOUT`, `3 CODE`, `4 SPANS`, `5 IMPORTS`.

- **`CONST`** holds string records `[len u16][UTF-8]`, back to back. A string is referred to by its byte offset into this section.
- **`CODE`**: jump targets and the failure `at` are byte offsets into it.
- **`SPANS`** maps a `CODE` offset to `(line, col, end_line, end_col)`. It is optional, and the VM never needs it.
- **`IMPORTS`** has one entry per host builtin the program calls: its signature (§2.4, UTF-8 with a u16 length) and its argument and result leaf counts (u16 each, results at most 16). The VM links each to the host's builtin with exactly that signature when it loads the program (§8).

> **As built:** the container is a fixed sequence rather than tagged sections: magic, version, interface hash, `max_stack`, `locals`, the input leaves' checks, the output leaves' checks, the input and output variant groups (§7.3), the imports, the input lists, then the constants, the code and the spans (`lang/bytecode/src/lib.rs`).

### 7.2 Values

A `Slot` is 16 bytes, 8-aligned:

| Bytes | Contents |
|---|---|
| 0 | tag |
| 1 | space (`Str`/`Ref`) |
| 2–3 | padding |
| 4–7 | layout index (`Ref`) |
| 8–15 | payload: an i64, an f64, a bool (0 or 1), or the u32 offset (`Str`/`Ref`) |

**Tags:** `Nil`, `Bool`, `Int`, `Float`, `Str`, `Ref`.

**Spaces:**

| Space | Holds |
|---|---|
| `0` | Const |
| `1` | Output |
| `2` | Arena |
| `16 + i` | input `i` |

**Tag checks.** Ops check their operands' tags:
- `Nil` where a value is needed is `R-NIL`.
- Any other mismatch is `R-TYPE`.
- No op converts implicitly.

### 7.3 Layouts

> **As built (records slice):** records are values, never references. A record is flattened into **leaves**, one 16-byte slot per scalar field, in field-name order; an optional record has a presence leaf (true, or nil when absent, when all its leaves are nil too) before its fields. Inputs and the output are lists of leaves, each with its own check in the header, and `Return(n)` writes the output's `n` leaves. Binding a record copies its leaves into locals; reading a field loads that field's leaves. The exception: when a `let` body's value starts with the binding itself, laid out as it is (`let r = …; {a: r, b: r.x}`, also as the first field of a field, or past further `let`s), the binding is built right there on the stack, as the leading leaves of the body's value, and read where it lies with `Peek` (§7.4): it takes no locals and is never copied. Anywhere else (read before it's written out, written out in another position or layout, or not written out at all) it lives in locals. No arena is used for records. The packed layouts below remain the plan for held lists.
>
> **Variants** are laid out as what they are typed as: optional record fields, each a presence leaf then its fields, in field-name order among the record's other fields. A record with no fields (`idle: {}`) is its presence leaf alone. So a value with variants costs the leaves of all its variants, and the memory figures stay exact with no new rule. Sharing slots between variants (only one is ever present) is a later optimisation.
>
> **Variant groups in the header.** For the inputs and for the output, the header lists each variant group as `(guard, members)`: the members are the variants' presence leaves, and the guard is the presence leaf of the optional record the group sits in (a nullable input, an optional field, or a variant), or none. Before evaluation starts the VM checks that in each input group whose guard isn't nil exactly one member is non-nil (`R-INPUT` otherwise); at `Return` it checks the output's groups the same way (`R-RANGE`). The checks read leaves already in hand: no allocation, one pass over the group table.

Layouts are packed, with no alignment, and read and written by copying bytes.

| Type | Bytes |
|---|---|
| `int` | 8 (i64) |
| `float` | 8 (f64) |
| `bool` | 1 |
| `enum{…}` | 1: the index into the value list |
| `string[≤m]` | `2 + m`: a u16 length, the bytes, then zeros |
| `T?` | `1 + size(T)`: a presence byte, then T (zeroed when absent) |
| record | its fields in schema order (literals: in the order they're declared) |
| record with variants | as a record, each variant an optional record (as built; above) |
| `list<T>[≤n]` | `2 + n·size(T)`: a u16 length, then the slots (unused ones zeroed) |
| sort pair | the key (8 bytes, i64 or f64), then the item |
| aggregate `acc` | `acc_bytes`, opaque to the VM |

**Layout table.** The `LAYOUT` section is a table of nodes, each with a kind, a size, and details:
- children, with names and offsets;
- or an element node and a capacity;
- for ints, the range;
- for enums, the values as `CONST` offsets;
- for strings, the capacity.

The table is walkable, so hosts can convert layouts to and from the wire codec and JSON.

**Zeroing.** At the start of every evaluation the VM zero-fills the output and the arena. Stores keep unused bytes zero, so equal values are equal bytes.

### 7.4 Instructions

There is a value stack and a set of locals, both made of slots. Arguments are pushed left to right, so the last argument is on top.

| Family | Ops (operands) | Notes |
|---|---|---|
| Constants | `PushNil`, `PushBool b`, `PushInt i64`, `PushFloat f64`, `PushStr off` | |
| Locals, stack | `Load l`, `Store l`, `Pop`, `Dup`, `Swap` | |
| Stack reads (as built) | `Peek s` | Pushes a copy of stack slot `s`, counted from the bottom; `s` must be below the height there (checked at load). Reads a record the compiler built in place on the stack (§7.3). |
| Int | `AddI SubI MulI NegI AbsI` (wrapping), `ModI` (`wrapping_rem`; 0 → `R-DIV0`), `CmpI op` | |
| Float | `AddF SubF MulF DivF PowF NegF AbsF CmpF op`, `IntToFloat`, `FloatToInt` (truncates, saturates, NaN → 0), `Round Floor Ceil` | |
| Other | `EqStr NeStr EqBool Not IsNil` | |
| Control | `Jump t`, `JumpIf t`, `JumpIfNot t`, `JumpIfNil t`, `JumpIfNotNil t` | The conditional jumps pop their operand. `&& \|\| ?? ?: ?.` and chained comparisons compile to jumps. |
| Inputs | `PushInput i` | Pushes the input's value: a scalar, a `Str`, or a `Ref` to its root. A nullable input that is absent gives `Nil`. |
| Layout reads | `LoadField kind off`, `RefAt space off layout` | `kind` is one of `int float bool enum string opt(kind) ref(layout)`. An absent `opt` gives `Nil`. Input spaces are read through the host's accessor. |
| Layout writes | `StoreField kind off` (`Ref`, value →) | Into the output or the arena only. Into the output, it checks int ranges, enum membership and string capacity, raising `R-RANGE` on failure. Floats are stored as they are. Storing `Nil` into an `opt` clears it. |
| | `CopyLayout layout` (src, dst →) | Identical layouts only. |
| Lists (as built) | `PushInputList id`, `MakeList base len stride`, `ListLen`, `ListRead leaf` | `PushInputList` pushes a handle to input list `id` (nil if the host says it's absent); `MakeList` a handle to items in locals. `ListLen`: handle → int (nil is `R-NIL`). `ListRead leaf`: handle, index → that item's leaf; it needs `0 ≤ i < len`, else `R-INDEX` (the compiler adds `len` to a negative index first). Loops are ordinary backward jumps. Writing lists (outputs) comes with step 1b. |
| Errors (as built) | `Error code` | Fails the evaluation: 1 `R-NIL`, 2 `R-INDEX`, 3 `R-ARG`; ends a path. |
| Holds | `HoldSortInsert buf dir k`, `HoldSort buf dir`, `HoldUniqueInsert buf`, `HoldMedian buf` | Stable insertion. A `Nil` key is `R-ARG`. Each comparison or move costs 1. |
| Host builtins | `CallHost import args results` | Pops `args` leaves, calls the builtin linked as `import`, pushes its `results` leaves. The counts must equal the import's. A host failure is `R-ARG`. (Builtins over lists come with lists.) |
| Errors | `Error code` | |
| End | `Return` | The output is complete. |

**Who raises what.**
- **Ops raise:**
  - `R-NIL` and `R-TYPE` (tag checks);
  - `R-DIV0`;
  - `R-INDEX` (`ListElem`);
  - `R-RANGE` (`StoreField`);
  - `R-ARG` (hold keys, host builtins);
  - `R-INPUT` (input reads).
- **Emitted `Error` instructions raise:**
  - a negative `take`;
  - `reduce` with no `init` on an empty list;
  - a `Zip` whose lists have different lengths;
  - an index out of range on a non-indexable pipeline.

### 7.5 Load-time validation

The VM refuses to load the program (`LoadError`) unless every one of these holds:
- **Container:** the magic and version are known, and the section lengths sum to the program's size.
- **Sizes:** `arena_size`, `output_size`, `max_stack`, `locals` and each input's size are within the host's `Limits`.
- **Layout table:**
  - children lie inside their parents;
  - sizes follow §7.3;
  - capacities are at most 65,535;
  - the output and input layout indices exist and match the declared sizes.
- **Code:**
  - every instruction decodes;
  - every jump lands on an instruction;
  - every `CONST` offset, local, layout, buffer and import is valid.
- **Imports:** every `CallHost` names an import whose counts it matches (`LoadError::BadImport`), and linking finds a registered builtin with each import's exact signature and counts (`LoadError::Unlinked`, `LoadError::BadImport`).
- **Stack:** an abstract interpretation of stack heights shows that the height at each instruction is the same on every path, never negative, and never above `max_stack`, and that every `Peek s` reads a slot below the height there.
- **Static offsets** lie inside their space.

Offsets reached through a `Ref`, and buffer capacities, are checked at run time.

---

## 8. VM host interface

As built (`lang/vm/src/lib.rs`), for scalars, records, host builtins and list inputs:

```rust
pub enum Value<'a> { Nil, Bool(bool), Int(i64), Float(f64), Str(&'a [u8]), List(ListRef) }   // 16 bytes on the C6
pub enum ListRef { Input(u16), Local { base: u16, len: u16, stride: u16 } }   // a handle: an input list, or a literal's items in locals

/// A list input, read in place (e.g. from flash). Items are flattened like
/// records (§7.3): `read(i, k)` is leaf `k` of item `i`.
pub trait ListInput<'a> {
    fn len(&self) -> u32;
    fn read(&self, item: u32, leaf: u16) -> Value<'a>;
}

pub struct HostFn {
    pub signature: &'static str,     // must equal the declaration's (§2.4)
    pub args: u16,
    pub results: u16,
    pub call: for<'a> fn(&[Value<'a>], &mut [Value<'a>]) -> bool,   // false: R-ARG
}

pub fn load<'a>(bytes: &'a [u8], scratch: &mut [u16], limits: &Limits) -> Result<Program<'a>, LoadError>;
pub fn link<'h>(p: &Program, registry: &[&'h HostFn], linked: &mut [Option<&'h HostFn>]) -> Result<(), LoadError>;
pub fn run<'a>(p: &Program<'a>, host: &[Option<&HostFn>], inputs: &[Value<'a>],
               lists: &[Option<&dyn ListInput<'a>>], output: &mut [Value<'a>],
               stack: &mut [Value<'a>], locals: &mut [Value<'a>]) -> Result<Stats, Fail>;
```

- **Loading.** `load` parses and validates (§7.5); `scratch` holds at least one entry per code byte and is only used during the call. `link` matches the program's imports to the host's builtins by signature; `linked` holds `import_count` entries.
- **Inputs** are the input leaves (§7.3), one `Value` each, in interface order; they're checked against their declarations, and each variant group for exactly one variant present, before evaluation starts (`R-INPUT`). The host must keep them unchanged for the length of a `run`; strings are borrowed, never copied.
- **Output.** `output` receives the output's leaves; an int written to a float position is converted.
- **Buffers.** The host owns every buffer and sizes them once, from the header: `max_stack`, `locals`, `output_count`, `import_count`.
- **No budget.** Every loop runs over a list with a static bound, so an evaluation always ends; there is no instruction budget.
- **Failure handling.** The VM is `no_std`, has no `alloc`, and never panics. Every failure is a `Fail { code, pc }` or a `LoadError`; the program's span table maps `pc` to source.
- **List inputs.** Every list inside the inputs (a list input, or a list field of an input record; not inside list items) has an id, in leaf order, and the host passes an accessor for each in `lists`, or `None` when the list sits in an optional record that's absent. The input leaf at a list's position is ignored (pass nil). A list longer than its `maxItems`, or an item leaf that doesn't fit its schema, is `R-INPUT`: the checker's bounds rely on both.
- **List handles.** A list value is a handle (`ListRef`) to an input list, or to a list literal's items in locals. Both live for the whole evaluation (inputs don't change; each literal has its own block of locals, never reused), so a handle can be copied freely through `let`, `?:` and `??`. `map`/`filter` results are never stored: each consumer runs the pipeline as one loop.

What the host does with an output, or with a failure, is the profile's business.

---

## 9. The oracle and conformance

The oracle is Go expr pinned at `v1.17.9-0.20260412144015-3a46b1970b37` (v1.17.8 plus the `findLast` fix, §2), **run on linux/arm64 only**, where `int()` saturates.

**Compiling**

```
expr.Compile(src, expr.Env(env), expr.Optimize(false), functions…)
```

The optimizer is off. With it on, expr:
- rewrites `sum([a,b,c])` into a right fold;
- folds `1 % 0` into a compile error;
- merges predicates (probe).

**Test builtins.** Conformance registers the same small set of host builtins on both sides:
- in Go, with `expr.Function`;
- in Rust, as `HostFn`s.

The set covers:
- a scalar builtin;
- a record param;
- an optional param;
- a nullable record result;
- an aggregate over two lists;
- one builtin that fails on some inputs.

Each implementation must be exact: integer arithmetic, or floats with no transcendental functions.

> **As built:** the language's corpus has one test builtin, `dist(a, b, scale?)` = `|a.lat − b.lat| + |a.lon − b.lon|`, times `scale` if given (a record param, an optional param); a case lists the test builtins it calls in `builtins`. The profile's corpus registers the profile's builtins instead.

**Inputs** are built from JSON **according to their schemas**:
- an integer becomes Go `int`, and a number becomes `float64`;
- an absent optional field is left out of the map, never set to `nil`;
- an absent nullable input is `nil`;
- a record with variants is a map with its one variant's key (the variants' schemas are found through `oneOf`).

**Comparing results**
1. Normalise Go's result according to the output schema: an int in a float position becomes a float, and a key whose value is nil counts as absent.
2. A nil result, or a result that violates the schema, is an error.
3. Decode the VM's output through `LAYOUT`.
4. Compare:
   - ints, bools, enums, strings and list lengths exactly;
   - floats within 4 ulp or a relative 1e-12;
   - `-0.0` equals `0.0`, and NaN equals NaN.

**Comparing errors.** Go's error strings map to our codes:

| Go error string | Code |
|---|---|
| `integer divide by zero` | `R-DIV0` |
| `index out of range` / `out of bounds` | `R-INDEX` |
| `from <nil>` / `<nil>` operands | `R-NIL` |
| test-builtin errors, `reduce of empty`, `unknown order`, a negative `take` | `R-ARG` |

`R-RANGE`, `R-INPUT` and `R-TYPE` never appear in the corpus.

**Divergences**
- **Lazy:** Go fails where the VM succeeds (§2.7). This is reported separately, not counted as a failure.
- **Float edges:** a non-float result differs because of `**`. These are quarantined.

**The corpus**
- the probes;
- generated edge cases: nil paths, `#index`, empty lists, throw versus `nil`, every `??` precedence probe, and every test builtin;
- a rejection set: at least one program per rule in §5.1, each with its golden message.

Profiles add their own corpora on top of this.

---

## 10. Open decisions

**Resolved:**
- No nil rules beyond expr's own at run time; narrowing is a typing rule (§3.3).
- D10: `oneOf` is a record with variants, in inputs and outputs; the checker proves an output has exactly one (§3.2, §3.4).
- The sort is stable.
- Mixed int/float branches become float.
- NaN and ±Inf are ordinary values.
- `E-RANGE` is checked statically.
- Evaluation is lazy.
- Floats are compared within a tolerance.

**Open:**
- **D2: `median` over large inputs.** Hold within the limit only, or use multi-pass selection, or a remedian.
- **D13: the builtin ABI.** Only flat scalars cross it. Strings, enums and list results are not supported. Is that enough for the host builtins we expect?
- **D14: the interface file.** The format in §1.1 is the compiler's. Should profiles generate it, or should authors ever write it?

---

## Appendix: deferred review items

| Item | Source | Why it's deferred |
|---|---|---|
| Bit-exact floats | implementability review M2 | The user chose a tolerance. |
| Eager stage evaluation | implementability M6(a), soundness M6 | The user chose lazy evaluation. Condition-aware fusion is kept. |
| A lower charge from scalar replacement of records | soundness m5 | An optimisation only; the charge stays. |
| General monotone `#index` proven by range analysis | soundness m4 | Constant look-ahead is covered. Other cases hold for now. |
| A `Cond` source without duplicating code | soundness m8 | Left to the emitter. |
