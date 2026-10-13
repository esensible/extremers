## Languages and analyses with static memory bounds

Scope: prior art for statically bounding the memory needed by one step of an engine
`(state, event, timestamp, gps) -> state` (an expr-lang expression evaluated once per second on an
ESP32-C6). Memory is the hard constraint and compute is secondary but not free, so the trade-offs
between the two are noted wherever a source covers them.

Markers: **(read)** means I read it in the primary source at the URL given. **(secondary)** means I
took it from an abstract page, a search snippet, or a citation inside another paper. **(inference)**
means it is my own reasoning about our language.

---

### 1. Lustre (synchronous dataflow)

**Citation.** N. Halbwachs, P. Caspi, P. Raymond, D. Pilaud. "The Synchronous Data Flow Programming
Language LUSTRE." *Proceedings of the IEEE* 79(9):1305–1320, September 1991.
URL opened: https://homepage.divms.uiowa.edu/~tinelli/classes/181/Fall20/Papers/Halb91.pdf (author-course copy of the IEEE paper)

**Citation.** E. Jahier, P. Raymond, N. Halbwachs. *The Lustre V6 Reference Manual*, software v6.112.1.
URL opened: https://www-verimag.imag.fr/DIST-TOOLS/SYNCHRONE/lustre-v6/doc/lv6-ref-man.pdf

**What it establishes**
- `pre(E)` is the only way to reach the past. It holds exactly one previous value, so each `pre`
  costs one memory cell. `->` costs one boolean `_init` flag (read, §II-B, §III-C). The paper calls
  `pre` "acts as a memory".
- The compiler checks that the program fits in bounded memory and does not leave this to chance
  (read, §III-A):
  - no recursive node calls, so only static networks can be written;
  - every cycle must pass through a `pre`;
  - a clock calculus rejects programs that would need unbounded memory. The example is
    `y = x + (x when b)`: computing y at step n needs x at step 2n, so a reactive system "will
    certainly overflow".
- Code generation produces a single loop or an automaton. Memory consists of `_pre_x` variables
  copied at the end of each cycle. The compiler avoids a memory variable when it can: in the paper's
  example, `pre(is_set)` needed no separate cell (read, §III-C remarks).
- Lustre V6 arrays have sizes that are compile-time constants (`int^3^3`). Slices take static
  bounds (`array12[1..10]`). The iterators `map<<N; n>>`, `red`, `fill` and `fillred` (fold) all
  take the size `n` as a static parameter, and their semantics are given by full unfolding (read,
  §3.5–3.6, pp. 33–36). Recursive nodes exist only as static recursion on constants: the `with`
  construct is expanded at compile time (read, §3.4). The manual notes that typing the n-ary `map`
  properly would need dependent types (read, §3.6.7).
- Lustre V6 has no filter iterator and no variable-length arrays (read: absent from the §3.6
  iterator list).

**Mapping to our language**
- Our `state` is the set of Lustre `pre` variables. `event`, `timestamp` and `gps` are inputs. One
  engine evaluation is one Lustre cycle (inference).
- `gps` is an append-only history in flash. Lustre has no such thing: there the only history is what
  the program keeps in `pre` cells. For us, history held in RAM is exactly the state; history in
  flash costs no RAM (inference).
- `xs[#index - k]` inside `map` reads k elements back within the same list. Lustre has no in-cycle
  equivalent. The closest analogue is `pre` applied k times to a stream, which costs k cells (a ring
  buffer of depth k). This works only when k is a static constant (inference).

**Borrow**
- Make "no recursion, cycles only through state" a checked rule. Our grammar already guarantees it.
- Reject constructs whose memory depends on run-time values, the way the clock calculus does.
- Copy only the state fields that the step actually reads later. Lustre avoids `_pre_` cells it
  does not need, so we should not hold two full copies of the state when one copy plus the changed
  fields would do.

**Contradicts or weakens our thinking**
- In Lustre-family languages every size is exact and static. The "filter keeps the source's bound"
  idea has no precedent in them. It is sound as an upper bound, but it can be very loose: a filter
  over `gps` has a bound of 61,440 (inference).

---

### 2. SCADE 6 / Lucid Synchrone

**Citation.** J.-L. Colaço, B. Pagano, M. Pouzet. "Scade 6: A Formal Language for Embedded Critical
Software Development." *TASE 2017* (invited paper). HAL: https://hal.archives-ouvertes.fr/hal-01666470
URL opened: https://www.di.ens.fr/~pouzet/bib/tase17.pdf

**What it establishes**
- An array size must be a compile-time static expression, for example `float64^3^7`. Size equality
  constraints are checked after static expansion (read, §IV-A).
- A well-typed program never indexes an array out of bounds (read, Property 1).
- A well-clocked SCADE 6 model can be implemented in bounded memory, provided the imported functions
  are also bounded (read, corollary to Property 2).
- `merge` replaces `current` partly because `merge` "does not use any memory" (read, §III).
- Array iterators (`map`, `fold`, functional update) are free of side effects. The paper says they
  "enjoy several optimizations like the elimination of intermediate copies". Their semantics is full
  unfolding, but they compile to a `for` loop (read, §VI).
- The scheduler in dataflow compilation uses heuristics to limit memory size (read, §VII-A).

**Mapping to our language**
- Eliminating intermediate copies is the same idea as our lazy, restartable recipes. SCADE gets it
  through iterator composition and fusion at compile time (read, §VII-A lists "iterator
  compositions").
- `fold` corresponds to our `reduce`. SCADE accumulators have static types and sizes, which matches
  our rule that reduce accumulators must have fixed-size types (inference).

**Borrow**
- Fuse `map`/`filter`/`reduce` chains at compile time so they become one pass with no intermediate
  buffer.
- Treat memory-minimising scheduling as an explicit compiler goal.

**Contradicts or weakens our thinking**
- Same as Lustre: SCADE has no data-dependent lengths, so it does not cover `filter`, `uniq` or
  `take(n)` with a computed n.

---

### 3. Copilot (NASA runtime-verification stream language)

**Citation.** I. Perez, F. Dedden, A. Goodloe. *Copilot 3.* NASA/TM-2020-220587, NASA Langley
Research Center, April 2020.
URL opened: https://ntrs.nasa.gov/api/citations/20200003164/downloads/20200003164.pdf

The original paper is L. Pike, A. Goodloe, R. Morisset, S. Niller, "Copilot: A Hard Real-Time Runtime
Monitor", RV 2010. I saw it only in search results, where it is described as generating
"constant-time and constant-space C programs" (secondary).

**What it establishes** (all read)
- Abstract: Copilot generates "C code with static memory requirements".
- §3.1.3: unbounded delays leak memory. To keep memory predictable, a stream can be delayed only by
  prepending a fixed number of samples with `(++) :: [a] -> Stream a -> Stream a`.
- §3.1.4: `drop` is checked at compile time.
  - Dropping from an `extern` stream (a sensor) gives "Drop applied to non-append operation!".
  - Dropping more samples than were prepended gives "Drop index overflow!".
  - Both checks reject non-causal programs, i.e. reads of the future.
- §3.3: arrays carry their length in the type (`Array 16 Int64`). This allows "C without dynamic
  memory allocation".
  - An out-of-range index that is a constant stream is caught at compile time.
  - An index that depends on an external variable is not caught.
- §4.2.2–4.2.3: bounded LTL needs enough prepended samples, so every temporal operator has a finite
  look-ahead.
  - The MTL operators take a time range `[n1, n2]`, a clock stream, and a "minimum time delta
    between samples".
  - Together these turn a time window into a bounded sample count.
- §6.3, code generation:
  - Global buffers hold each delay. Each buffer starts with exactly the prepended elements.
  - Each stream is compiled to a function that recomputes its current sample on demand. Its value is
    not stored in a variable.
  - The step function first copies all external variables, which keeps execution reentrant and
    causal. It then checks triggers, updates the buffers, and advances the indices.

**Mapping to our language**
- `state` corresponds to Copilot's delay buffers. Copying the externals at the start of a step is our
  "snapshot `event`/`state`, then compute the new state", which is where the 2× state term comes from
  (inference).
- Computing streams on demand rather than storing them is our replay-instead-of-buffer choice. It
  trades RAM for repeated computation (inference).
- `xs[#index - k]` with k > 0 is a delay of k, and Copilot would give it a buffer of depth k.
  `xs[#index + k]` reads the future, and Copilot's causality check rejects that. We have the whole
  list, so a future read is legal for us, but it needs either replay or a look-ahead buffer of k
  (inference).
- Time windows such as `filter(gps, .t > timestamp - 60)` have no sample bound unless we know a
  minimum spacing between fixes. Copilot's MTL makes the user pass that minimum delta explicitly
  (read, §4.2.3). With 1 Hz fixes and a declared minimum spacing, the window is at most 61 fixes
  (inference).

**Borrow**
- Compile-time checks of the form "offset ≤ buffer depth" for `#index ± k` and for slices.
- An explicit minimum spacing between GPS fixes, used to turn time windows into count bounds.
- Recompute on demand by default, and buffer only where a delay forces it.

**Contradicts or weakens our thinking**
- Copilot recomputes a stream every time it is used. If a lazy recipe is consumed twice, or consumed
  inside another iteration, the CPU cost multiplies. Our model needs a matching CPU estimate so the
  compiler can decide when to materialise instead (inference).

---

### 4. Faust (audio DSP)

**Citation.** Faust documentation, "Faust Syntax" (`@` delay, `'`, `mem`, `rdtable`/`rwtable`).
URL opened: https://faustdoc.grame.fr/manual/syntax/

**Citation.** A. Herrou, F. de Dinechin, S. Letz, Y. Orlarey, A. Volkova. "Towards Fixed-Point Formats
Determination for Faust Programs." *Journées d'Informatique Musicale 2024*. arXiv:2403.06527.
URL opened: https://arxiv.org/pdf/2403.06527

**What it establishes**
- The manual says `@` delays must be bounded and must not be negative.
  - A slider is an acceptable source for a variable delay, because its declared range bounds it.
  - `'` and `mem` are one-sample delays.
  - Table sizes must be constant numerical expressions.
  - Source: the doc page, read through a fetch summary. The exact error behaviour for an unbounded
    `@` is not stated on that page (read).
- The compiler runs interval analysis during symbolic propagation (read, Herrou et al. §4.2).
  - Each interval holds a low bound, a high bound and a precision.
  - Source signals have known ranges: audio is [-1, 1], a slider is [min, max], a constant is [K, K].
- The paper says the interval library "was primarily introduced to bound the size of delay lines".
  It is also used to detect division by 0 and negative delays (read, §4.2).
- Interval analysis may not converge to a tight interval through recursive loops. The compiler then
  falls back to a default (read, §3.1–3.2).

**Mapping to our language**
- `gps[-k:]`, `take(xs, n)` and `xs[#index - k]` all have a size or offset that is an integer
  expression.
  - When the expression is a literal, the bound is trivial.
  - When it is computed (`take(xs, state.n)`, `gps[-state.window:]`), we need a value-range analysis
    in the Faust style.
  - The ranges can come from schema `minimum`/`maximum` on state and event fields, and from
    arithmetic on them (inference).

**Borrow**
- Value-range analysis over integers. Schema `minimum`/`maximum` play the role of Faust's slider
  ranges.
- A size whose interval has no finite upper bound is a compile error, and the error names the
  expression responsible.

**Contradicts or weakens our thinking**
- Our current list of bound sources (gps capacity, maxItems, `take n`, `push_last n`) assumes n is
  known. Without range analysis, `take` with a computed n can only fall back to the bound of the
  source list (inference).

---

### 5. Hume (Hammond and Michaelson)

**Citation.** K. Hammond, G. Michaelson. "Predictable Space Behaviour in FSM-Hume." In *Implementation
of Functional Languages, IFL 2002*, LNCS 2670, pp. 1–16, Springer, 2003.
DOI: https://doi.org/10.1007/3-540-44854-3_1
Opened: the Heriot-Watt portal abstract only, https://researchportal.hw.ac.uk/en/publications/predictable-space-behaviour-in-fsm-hume-2/
The full text was paywalled and I did not read it.

**Citation.** K. Hammond, G. Michaelson, R. Pointon. *The Hume Report, Version 1.1.*
URL opened: https://www.macs.hw.ac.uk/~greg/hume/hume11.pdf

**What it establishes**
- The IFL paper gives a cost model for stack and heap usage in FSM-Hume and checks it on a mine
  drainage controller. Its empirical results suggest the model predicts usage well (secondary:
  abstract only).
- Hume is defined in levels that trade expressiveness for costability (read, Report §1.2.1 and
  Fig. 1.1):
  - **HW-Hume**: no functions, non-recursive data.
  - **FSM-Hume**: adds non-recursive first-order functions, conditionals and local definitions.
  - **Template-Hume**: FSM-Hume plus *predefined* higher-order functions, polymorphism and
    inductive data structures, but no user-defined higher-order or recursive functions.
  - **PR-Hume**: adds primitive recursion.
  - **Full Hume**: Turing-complete.
- `HeapOverflow` and `StackOverflow` are raised only by code whose heap or stack costs "have not been
  certified", and then only inside a `within` constraint (read, §2.1.4). Version 1.0 extended
  `within` to space as well as time (read, §1.7). Certified bounds and run-time guards therefore
  coexist.
- Version 1.1 changed vectors to be "specified with a size rather than a bound", and made strings
  fixed-size immutable objects (read, §1.6).
- Hume uses a static process network, so the total cost of the system can be specified (read,
  §1.4.2).
- The expression layer is strict, which "allows tight cost functions" (read, §2.4).
- Space cost models exist for HW-Hume and FSM-Hume, and the HAM back end provides guaranteed
  hard-space bounds for FSM-Hume (read, §1.3.2).

**Mapping to our language**
- Our language is almost exactly **Template-Hume**: predefined `map`/`filter`/`reduce`/`sortBy`, no
  user functions and no recursion. In Hume this is the level just above the one with proven space
  models, and the levels below it are where certification is reported (inference from Fig. 1.1 and
  §1.3.2).
- Each engine is one box executing once per tick, inside a static network. Box state lives on wires,
  which is our `state` (inference).

**Borrow**
- Combine static certification with a run-time space guard, as Hume's `within` does. The VM should
  run each step in a fixed arena and fail safe (keep the previous state and report) if the arena
  overflows, even when the static check passed. This covers bugs in the analysis itself.
- Exact sizes are simpler to analyse than bounds. Hume moved from bounds to exact sizes, which
  suggests keeping data-dependent sizes to the few builtins that genuinely need them.

**Contradicts or weakens our thinking**
- Hume's proven models cover FSM-Hume, which has no higher-order iteration. Higher-order builtins
  (Template-Hume) are exactly where per-builtin rules have to be written and justified by hand. Each
  of our builtins needs its own memory rule; no general theorem provides them (inference).

---

### 6. Automatic amortised resource analysis (RAML)

**Citation.** J. Hoffmann, K. Aehlig, M. Hofmann. "Multivariate Amortized Resource Analysis." *ACM
TOPLAS* 34(3), Article 14, 2012.
URL opened: https://www.cs.yale.edu/homes/hoffmann/papers/HAH12Toplas.pdf
Project page opened: https://www.raml.co/

**What it establishes** (read unless marked)
- The analysis applies to Resource Aware ML, a first-order OCaml fragment with integers, lists, trees
  and recursion.
  - It infers concrete worst-case bounds as multivariate polynomials in the sizes of the inputs.
  - The resource metric is a parameter: heap cells, stack or evaluation steps.
  - Inference reduces to linear programming (abstract, §1).
- The semantics records a pair (q, q'): q is the "high watermark" of resources needed and q' is what
  remains afterwards. This handles resources that are given back, such as freed memory or a popped
  stack, as well as ones that are not, such as time (§3, pp. 10–11).
- The cost constants for stack-space, heap-space and clock cycles were measured for the Hume abstract
  machine on a Renesas M32C/85U, citing Jost et al. 2009 (§3; the Jost paper itself is secondary).
- Limitations (§9):
  - Bounds must be resource polynomials, so n·log n and 2^n are excluded.
  - Whether a bound exists is undecidable in general.
  - Bounds can depend only on sizes of data structures, never on integer values. A loop guarded by
    `n > m` cannot be bounded.

**Mapping to our language**
- Our "bounds from schema sizes" idea is RAML's model with the size variables fixed: maxItems,
  capacity 61,440 and so on. Because the sizes are fixed, the polynomial collapses to a number, and
  we need no LP inference (inference).
- The (q, q') watermark is the right model for peak memory. Peak is not the sum of all allocations,
  because a lazy recipe that finishes gives its memory back. Our analysis should compute the maximum
  over program points of live memory. Summing terms is wrong (inference).
- RAML cannot see integer values, so `take(xs, state.n)` is out of its reach. That reinforces the
  Faust point: we need integer ranges as well as sizes (inference).

**Borrow**
- Watermark composition: running e1 then e2 needs `max(q1, consumed1 + q2)`, not `q1 + q2`.
- Bound both memory and steps with one framework under different metrics. The CPU correction to the
  brief suggests computing a step bound as well.
- Per-node cost constants measured on the real VM.

**Contradicts or weakens our thinking**
- "Max memory = 2 × max(state) + max(event) + evaluation stack" is a sum of fixed terms. RAML's
  semantics shows the real quantity is a high watermark over the evaluation. That watermark includes
  every value live at once:
  - `let`-bound lists;
  - partial outputs of `map` that are destined for state;
  - accumulators of enclosing `reduce`s;
  - materialised `sortBy`/`median`/`uniq` buffers.

  The formula is a special case that holds only when nothing is materialised outside state
  (inference).

---

### 7. CEL static cost estimator (cel-go)

**Citation.** cel-go, the canonical repository, now at github.com/cel-expr/cel-go (module
`cel.dev/cel-go`). The old repository github.com/google/cel-go is a read-only alias shim: I read its
README and `checker/cost.go`, which only forwards to `common/cost`.

Files opened:
- https://raw.githubusercontent.com/cel-expr/cel-go/HEAD/docs/cost-model.md
- https://raw.githubusercontent.com/cel-expr/cel-go/HEAD/common/cost/estimator.go (function `costComprehension`)
- https://raw.githubusercontent.com/cel-expr/cel-go/HEAD/common/cost/cost.go
- https://raw.githubusercontent.com/cel-expr/cel-go/HEAD/checker/cost.go (alias shim)
- https://raw.githubusercontent.com/google/cel-spec/master/doc/langdef.md (language definition; still served from the old repo path)

**Citation.** Kubernetes documentation, "Extend the Kubernetes API with CustomResourceDefinitions",
section on validation rule resource use.
URL opened: https://kubernetes.io/docs/tasks/extend-kubernetes/custom-resources/custom-resource-definitions/
Source opened: https://raw.githubusercontent.com/kubernetes/kubernetes/master/staging/src/k8s.io/apiserver/pkg/cel/common/schemas.go

**What it establishes** (read)
- The cost model serves two purposes from one declaration:
  - static estimation returns an interval `[Min, Max]` of cost and is used to reject expressions over
    budget;
  - run-time tracking halts evaluation when a limit is exceeded.

  Both are generated from one `OverloadModel` per function.
- A `SizeEstimate` is an interval of lengths. It also carries `Elem` and `Key` size metadata, so
  sizes propagate through nested containers. `math.MaxUint64` means unbounded, and all arithmetic
  saturates.
- Input sizes come from an `Estimator.EstimateSize(node)` hook that looks up field paths. The
  documented example uses hints keyed by paths such as `request.auth.claims.@keys`.
- In `costComprehension`, used for `map`/`filter`/`all`/`exists`:
  - cost is `rangeCnt × (stepCost + loopCost)` plus the range, init and result costs;
  - if the accumulator is a list or map, the result size is `rangeCnt`, with `Elem` taken from the
    step;
  - if the accumulator is initialised with a literal, the result size is the size of that literal.
- Specific builtins:
  - `slice(2, 8)` gets size `[0, 6]` only when both arguments are literals. Otherwise the bound is
    unbounded or comes from the target.
  - `sort`/`sortBy`/`distinct` are charged about 2.1·n² comparisons.
  - Cost charges allocations through `ListAlloc`.
- CEL does not track peak memory. It tracks abstract cost and result sizes; the design doc mentions
  protecting "compute and memory resources", but the only budgeted quantity is cost. The spec says
  space is "the sum of the space complexity of its sub-expressions, plus a constant", and that macros
  can become exponential when nested (langdef.md).
- Kubernetes CRDs apply this with OpenAPI schemas (doc and `schemas.go`):
  - `maxItems`, `maxLength` and `maxProperties` become size bounds.
  - Without them the estimator assumes the worst case and the CRD is rejected with "exceeded budget
    by more than 100x (try ... adding maxItems, maxProperties, and maxLength ...)".
  - A missing `maxItems` falls back to `(maxRequestSizeBytes - 2) / (minElemSerializedSize + 1)`.
  - A string's `maxLength` counts code points, so it is multiplied by 4 to get bytes.

**Mapping to our language**
- This is the closest precedent for our approach: schema bounds give sizes, the sizes propagate
  through comprehensions, and expressions over budget are rejected at compile time.
- CEL's propagation rule for list accumulators (result length = source length) is the same as our
  "filter/map keep the source's bound".
- Our `reduce` with a fixed-size accumulator corresponds to CEL's literal-accumulator case. CEL simply
  takes the literal's size. That is correct only if the step never grows the accumulator, which is
  the side condition our fixed-size-type rule enforces explicitly. A string accumulator built by
  concatenation would break it (inference).

**Borrow**
- Intervals of sizes with saturating arithmetic. `Elem` and `Key` metadata, so that a list of strings
  has both a length bound and a per-element byte bound.
- Path-keyed size hints taken from the JSON schemas of `state` and `event`.
- One declarative model per builtin that drives both the static estimate and an optional run-time
  meter.
- A CPU estimate of the form `range × step`, used to choose between replay and materialisation.
- The Kubernetes fallback: when a schema has no `maxItems`, bound the list by the storage budget
  divided by the minimum element size, instead of rejecting outright.

**Contradicts or weakens our thinking**
- Element size matters as much as length. A list of 100 strings has no byte bound without a string
  `maxLength`, and JSON-schema `maxLength` counts code points, so the byte bound is up to 4 × code
  points in UTF-8 (read, `schemas.go`).
- CEL budgets cost, not memory. No production CEL implementation provides a peak-memory analysis to
  copy, so ours is new work (read: no memory metric in the cost model).

---

### 8. Stack and WCET analysis for embedded code

**Citation.** R. Heckmann, C. Ferdinand. "Verifying Safety-Critical Timing and Memory-Usage Properties
of Embedded Software by Abstract Interpretation." *DATE 2005*. arXiv:0710.4753.
URL opened: https://arxiv.org/pdf/0710.4753

**What it establishes** (read)
- AbsInt's StackAnalyzer applies value analysis to the stack pointer along all control-flow paths and
  derives worst-case stack usage per task.
- aiT computes WCET in phases: CFG reconstruction, value analysis, loop-bound analysis, cache and
  pipeline analysis, and ILP path analysis.
- Measurement cannot guarantee the maximum, because it covers only the inputs tested.

Related but only seen in search results: J. Regehr, A. Reid, K. Webb, "Eliminating stack overflow by
abstract interpretation", EMSOFT 2003 and ACM TECS 2005 (secondary).

**Mapping to our language**
- Our bound covers the VM's value stack and arena. The native Rust stack of the interpreter also
  grows with AST nesting if `eval` recurses. That stack shares the 512 KB with the radio stacks, so
  it must be bounded too. Bounding it means measuring stack bytes per nesting level of the VM's
  `eval` with a machine-code stack analysis, then capping AST depth (inference).

**Borrow**
- Bound the interpreter's own native stack once, by analysis, rather than by testing. Then cap AST
  depth in the compiler.

---

### Implications for us

1. **The fixed formula `2·S + E + stack` is incomplete.** RAML's (q, q') semantics shows peak memory
   is a high watermark over the evaluation: the maximum over program points of all live values. It
   must include:
   - `let`-bound materialised lists;
   - the partially built output of a `map` or `filter` whose result is stored into state;
   - accumulators of every enclosing `reduce`;
   - buffers that any non-streamable builtin needs.

   The fixed formula is only the special case where nothing is materialised outside state.

2. **Some of our builtins cannot be lazy, so they need either memory or CPU.**
   - `sortBy`, `median`, `uniq` and `findIndex`-after-`sortBy` need the whole input. For a bound of
     n they require O(n) buffer memory, or O(1) memory and O(n²) CPU via repeated selection.
   - `sortBy` over a 61,440-fix `gps` filter would need about 2 MB if materialised at 32 B per fix.
     "sortBy keeps the source's bound" gives the right length but says nothing about the buffer.
   - For each such builtin the compiler must pick materialise or recompute and charge memory or CPU
     accordingly. CEL already charges `sort` about 2.1·n² comparisons.

3. **`xs[#index - k]` is a delay line,** in the sense of Lustre's `pre`, Copilot's `++` and Faust's
   `@`.
   - On a recipe that cannot be randomly accessed, such as the output of a `filter`, it costs either
     a ring buffer of k elements or a replay from the start at O(n·k) CPU.
   - k needs a finite upper bound, checked statically. Copilot rejects reads past what is buffered
     and Faust rejects unbounded delays.
   - Direct `gps[i]` access is O(1) from flash and needs no buffer.

4. **Integer-valued sizes need range analysis.** RAML cannot bound by integer values, and CEL bounds
   `slice` only for literal arguments. `take(xs, n)`, `gps[-k:]` and `#index - k` with computed n or
   k need a Faust-style interval analysis seeded by schema `minimum`/`maximum`. An unbounded interval
   is a compile error that names the expression responsible.

5. **Time windows need a declared minimum fix spacing.**
   `filter(gps, .t > timestamp - 60)` is bounded only by 61,440 unless the GPS source declares a
   minimum Δt between fixes, as Copilot's MTL operators require. With that declaration the bound is
   `window / Δt_min + 1`. This matters only when the result is materialised or sorted.

6. **Size intervals must carry element sizes, not just lengths,** as CEL's `Elem`/`Key` metadata do.
   - Every string in the `state` and `event` schemas needs `maxLength`, and the byte bound is 4× that
     in UTF-8. Maps need `maxProperties`.
   - Following Kubernetes, a schema missing a bound can fall back to the storage budget divided by the
     minimum element size, with a warning, instead of being treated as unbounded.

7. **The reduce rule is right, but it must be a checked side condition.** CEL gives a literal
   accumulator the literal's size, which is safe only if the step never grows it. Our rule that
   accumulators have fixed-size types is that side condition. List or string accumulators should be
   rejected, or bounded by an explicit `take`/`push_last n`.

8. **Avoid two full copies of the state where possible.** Lustre keeps `_pre_` cells only for
   variables that are read later, and SCADE schedules equations to limit memory. 2·S is a safe upper
   bound. A tighter one copies only the fields the new state overwrites while their old values are
   still live.

9. **Fuse lazy recipes, but account for CPU.** SCADE eliminates intermediate copies by composing
   iterators, and Copilot recomputes streams on demand.
   - A recipe consumed twice, or consumed inside another iteration (`map(xs, count(filter(gps, …)))`),
     multiplies CPU: about 61,440 × outer length steps.
   - We should estimate CPU as CEL does (`range × step`) and materialise when the memory is available
     and the CPU saving is large.

10. **Keep a run-time guard as defence in depth.** Hume's `within` space constraints and CEL's
    run-time `CostLimit` both pair a static estimate with enforcement at run time. The VM should run
    each step in a fixed arena with a hard cap and a fail-safe: keep the previous state and report.

11. **Bound the interpreter's native stack as well.** Our evaluation-stack term covers the VM's value
    stack. If `eval` recurses on the AST, the native Rust stack also grows per nesting level, and it
    shares the 512 KB with the radio stacks. Measure bytes per level with stack analysis, not tests,
    and have the compiler enforce a maximum AST depth.

12. **This is new territory.** Lustre, SCADE and Copilot avoid data-dependent lengths altogether,
    RAML cannot see integer values, and CEL budgets cost but not memory. Bounding peak memory with
    data-dependent lengths (`filter`, `uniq`, `take n`) taken from schema bounds is therefore our own
    work, assembled from these parts:
    - CEL's size intervals with element metadata;
    - Faust's integer ranges;
    - RAML's watermark composition;
    - Copilot's causality and buffer-depth checks.
