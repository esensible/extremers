# Bounding engine memory statically: prior art and what it means for us

## The question

An engine is one pure Go-expr expression, evaluated once per second on the ESP32-C6:

    (state, event, timestamp, gps, velocity) -> new state

- `gps` is the run's fixes `{t, lat, lon, alt}`, sorted by `t`, up to 61,440 records of 32 B. They are read in place from the inactive OTA slot, so `gps[i]` is one O(1) flash read and costs no RAM.
- `state` and `event` are bounded by the engine's JSON schemas (every list has `maxItems`).
- The language has expr's list builtins (`map`, `filter`, `reduce`, `sum`, `count`, `min`, `max`, `mean`, `median`, `sortBy`, `uniq`, `first`, `last`, `find`, `take`, slices, `#index` inside `map`/`reduce` predicates) and no recursion or user functions.

**Memory is the hard constraint.** The compiler must work out the most RAM an engine can need and refuse engines over budget. **Compute still counts**: within the memory budget, take the cheapest option.

The question: do existing libraries or research already do this, and what should we take from them?

## Answer in brief

No existing system bounds *peak memory* for a language with data-dependent list lengths (`filter`, `uniq`, `take n`) using sizes from schemas:
- **Lustre, SCADE and Copilot** guarantee static memory, but only because every size is a compile-time constant and they have no `filter` ([static-bounds §1–3](static-bounds.md)).
- **CEL** does compile-time checks with schema-derived sizes, but it budgets abstract *cost*, not memory ([static-bounds §7](static-bounds.md)).
- **expr itself** materialises every `map`/`filter`/`sortBy` result. Its only memory control is a counter at run time ([fusion §6](fusion.md)).

Every part we need is established, though, and they fit together:

| Part | Taken from |
|---|---|
| Fusion that is *guaranteed*, computed on the plan actually produced | strymonas (Kiselyov et al. 2017, 2024); Coutts et al. 2007 shows best-effort rules fail silently |
| Classifying operators as streaming, bounded buffer or breaker | Babcock et al. 2002; Steno 2011; Polars and DuckDB source |
| Which aggregates fit in a fixed-size accumulator | Arasu et al. 2004 (Gray's distributive/algebraic/holistic); DGIM 2002 (invertible or not) |
| Size intervals with element sizes, from schemas | CEL `SizeEstimate`; Kubernetes CRD validation |
| Integer ranges for computed sizes (`take(xs, n)`) | Faust interval analysis (Herrou et al. 2024) |
| Peak as a high watermark, not a sum | RAML (Hoffmann, Aehlig, Hofmann 2012) |
| `xs[#index - k]` as a fixed-depth delay line | Lustre `pre`, Copilot `++`/`drop` checks, Faust `@` |
| Replay instead of sharing, so bounds compose | Gustavsson & Sands 1999; strymonas shares descriptions, not streams |
| A static bound backed by a hard limit at run time | Hume `within`; CEL `CostLimit` |

## What this means for the design

1. **Bounds are computed on a plan the compiler builds deterministically, not on rewrites that might fire.**
   - The compiler lowers each expression to a closed plan language: source, transform, filter, sink or buffer, aggregate. That is Steno's QUIL; strymonas reaches it by normalisation-by-evaluation.
   - The memory bound is read off that plan.
   - Coutts et al. measured best-effort fusion rules silently missing, and allocation getting *worse* where they did. A bound that assumes fusion happened would then be unsound.
   - Sources: [fusion §1–2, §4](fusion.md).

2. **Every list in the plan carries capabilities, and the compiler propagates them.**

   | Capability | Meaning | Kept by | Lost by |
   |---|---|---|---|
   | indexable | item i in O(1) | slices, `take`, `map` | `filter`, `uniq`, `sortBy` |
   | double-ended | can be walked backwards | `filter`, `map`, slices | |
   | restartable | can be replayed from the start | everything over immutable sources | |
   | sorted-by key | order is known | `filter`, slices, `take`, monotone `map` | non-monotone `map` |

   `gps` has all four, sorted by `t`. These come from Rust's iterator traits (`ExactSizeIterator`, `DoubleEndedIterator`, `Clone`), Polars' sorted flag and Steno's ordered `GroupBy`. Sources: [fusion §4–5](fusion.md), [streams §2, §4](streams.md).

3. **Each operator falls into one of three classes.**
   - **Streaming, O(1):** `map`, `filter`, slices, `take`, and every aggregate with a fixed-size accumulator (`sum`, `count`, `min`, `max`, `mean`, `first`, `last`, `find`, `findIndex`, `any`/`all`, `reduce`).
   - **Bounded buffer, O(k):**
     - `sortBy | take(k)` becomes a top-k heap;
     - `xs[#index - k]` over a list that isn't indexable becomes a ring buffer of k;
     - a negative slice or `last` over a derived list becomes a reverse scan of the source plus `take(k)`.
   - **Breaker:**
     - `sortBy` whose whole output is needed;
     - `uniq` and `median` with no value-domain bound;
     - nested loops joining two long sources.

     A breaker must buffer (up to its length bound), replay (O(1) RAM, extra passes), or be rejected.

   Four independent sources agree on the breaker set: Arasu, Babcock, Polars and DuckDB ([streams §4](streams.md)).

4. **Fusion rules that shrink breakers once the consumer is known:**
   - `sortBy | first/last` → min/max-by, O(1).
   - `sortBy | take(k)` → a streaming top-k heap: O(k) memory, O(n log k) CPU. Add DuckDB's skip-worse filter.
   - `sortBy | count/min/max` → drop the sort. Drop it before `sum`/`mean` only for integer or fixed-point values, because float rounding depends on order. Never drop it before `reduce`.
   - `sortBy(…, .t)` over `gps` → no-op.
   - `uniq`/grouping on a key the input is already sorted by → adjacent comparison, O(1) (Polars `SortedUnique`, Steno §4.3).
   - `any(xs, any(ys, x.b < y.d))` → `min(xs.b) < max(ys.d)` (Arasu Thm 5.10).
   - A list bound by `let` and consumed by several aggregates → one pass with several accumulators (Steno `GroupByAggregate`; strymonas only allows lockstep splits).

   Sources: [fusion §3–4, §6](fusion.md), [streams §1, §4](streams.md).

5. **Index access depends on how the index moves as `#index` grows.** `#index` exists only inside a `map`/`reduce` predicate, so every index is relative to a loop the compiler can see.

   | Index | On an indexable list | On a filtered list |
   |---|---|---|
   | `#index - k`, constant k | free | a ring buffer of k: a delay line, like Lustre `pre`, Copilot `++`, Faust `@` |
   | moves only forward (`#index + k`, `#index * 2`) | free | a second cursor over a replay of the same list: O(1) RAM, about 2× the work |
   | the same in every iteration (`xs[state.k]`) | free | read once before the loop |
   | jumps backwards without bound, or depends on data | free | buffer (up to the length bound) or rescan per read (O(n²)) |

   *Resolution (ours).* The fusion section says a non-constant offset such as `xs[#index * 2]` has no constant-memory option. That holds only for offsets that aren't monotone. A forward-only index over a *restartable* list is a zip with a second cursor: strymonas' "reify one side into a pull function", which holds constant state, not a buffer. The cost is CPU, not RAM.

   Sources: [fusion §1, §5](fusion.md), [static-bounds §1, §3–4](static-bounds.md).

6. **Aggregates are classified at compile time** (Gray's classes, via Arasu §8.2; invertibility from DGIM):
   - **Distributive or algebraic** (`sum`, `count`, `min`, `max`, `mean`, a fixed-shape `reduce`) → one fused loop, O(1). Our rule "`reduce` accumulators must have fixed-size types" *is* the algebraic class. Enforce it as a checked side condition: no growing list or string accumulators (CEL gets this wrong for literal accumulators).
   - **Holistic** (`median`, `uniq`, count-distinct) → bounded only through a *value-domain* bound: a schema `enum`, an integer `minimum`/`maximum`, or a filter that brackets the value. That allows a count array the size of the domain. Otherwise:
     - `median` can use multi-pass selection (O(1) RAM, about 32 passes);
     - `uniq` without a domain bound or sorted input has no cheap O(1)-RAM option.
   - **Invertible** (`sum`, `count`, `mean`) over a sliding window can be kept in `state`: add the new fix and subtract `gps[-N-1]`, which is O(1) RAM and CPU. Use integers or fixed point, because there's no FPU and floats drift. **Not invertible** (`min`, `max`, `median`, distinct): recompute from `gps[-N:]` each tick. DGIM's lower bounds don't apply, because the expired fix can still be read.

   Sources: [streams §1, §3](streams.md), [static-bounds §7](static-bounds.md).

7. **Bound inference tracks three things: list lengths, element sizes and integer ranges.**
   - **Lengths** are intervals with saturating arithmetic:
     - `gps` → 61,440; a schema list → `maxItems`;
     - `filter` → `[0, n]`; `map` → `n`; `take(k)` → `min(k, n)`; nested lists multiply.

     (CEL `SizeEstimate`; Rust `size_hint`.)
   - **Element sizes**, CEL's `Elem`/`Key`: every string in a schema needs `maxLength`, worth up to 4 B per code point; every map needs `maxProperties`.
   - **Integer ranges** for computed sizes and offsets (`take(xs, state.n)`, `gps[-state.w:]`), seeded from schema `minimum`/`maximum` (Faust interval analysis). An unbounded interval is a compile error that names the expression. RAML can't do this because it sees only sizes, and CEL bounds slices only when the arguments are literals.

   Sources: [static-bounds §4, §6–7](static-bounds.md), [fusion §5](fusion.md).

8. **Peak memory is a high watermark over the evaluation, not a sum of fixed terms.**
   - My earlier formula, `2 × state + event + stack`, holds only when nothing outside state is ever buffered.
   - The real bound is the maximum, over every point in the plan, of what is live at that point:
     - the state being built;
     - accumulators of enclosing `reduce`s;
     - live buffers (top-k, ring buffers, materialised breakers);
     - `let`-bound buffers.
   - Composition rule: running e1 then e2 needs `max(q1, held1 + q2)`, not `q1 + q2` (RAML).
   - Lustre also shows that two full copies of the state aren't needed: copy only fields that are overwritten while their old value is still live.

   Sources: [static-bounds §1, §6](static-bounds.md).

9. **The default is replay, with a work bound beside the memory bound.**
   - **Replay (call-by-name)** costs O(1) RAM per cursor, whatever the evaluation order. That's what makes bounds compose. Under sharing (call-by-need), the space needed depends on evaluation order: `head xs + last xs` runs in constant space, swapped it takes O(n) (Gustavsson & Sands).
   - **Replay isn't free.** A replayed scan inside a per-item predicate multiplies, e.g. `map(xs, count(filter(gps, …)))` is |xs| × 61,440. Several breakers replayed over the whole `gps` come to 10⁹–10¹⁰ operations, which can't finish in a tick.
   - So the compiler also computes a work estimate (CEL-style `range × step`, RAML with a different cost metric). It **buffers when the buffer fits and replay would be super-linear**, and **rejects engines whose work bound is infeasible**.

   Sources: [fusion §7](fusion.md), [streams §1](streams.md), [static-bounds §3, §7](static-bounds.md).

10. **Incremental work across ticks happens only through state the author writes, and only where that state is O(1).**
    - Running totals, extremes over the whole run, and invertible window sums all fit that.
    - Automatic incrementalisation (DBSP, differential dataflow) keeps state proportional to the data for `distinct`, joins and sort. That trades RAM for CPU, the opposite of what we need.

    Sources: [streams §5](streams.md).

11. **Back the static bound with hard limits at run time.**
    - **Arena:** each step runs in a fixed arena. If it overflows, the step fails, the old state is kept and the failure is reported. This catches bugs in the analysis itself (Hume `within`, CEL `CostLimit`, expr's `MemoryBudget`).
    - **Native stack:** the VM's own stack, if `eval` recurses on the AST, is bounded once by stack analysis (StackAnalyzer, DATE 2005), and the compiler caps AST depth.

    Sources: [static-bounds §5, §8](static-bounds.md).

12. **Observable results must match expr's eager evaluation**, including when to throw and when to return nil:
    - `filter(…)[0]` throws on an empty result;
    - `first(filter(…))` returns nil.

    Keep expr's 14 optimizer rewrites as plan regression tests. They're a minimum, not a model. Source: [fusion §6](fusion.md).

## Corrections to what I said earlier in this conversation

- **Compute is not free.** It is secondary, but replay can make engines infeasible, so the compiler needs a work bound too (point 9).
- **The memory formula** `2 × state + event + stack` is the special case where nothing outside state is buffered. The real bound is a watermark (point 8).
- **`gps[-k:]` is free only on `gps` itself.** On a derived list, Polars treats a negative slice as memory-intensive. We get O(1) by scanning the source backwards (point 3).
- **Time filters don't bound length.** `filter(gps, .t >= timestamp - 10000)` is bounded by 61,440 unless `gps` has a stated rate invariant (Arasu §1.3, Copilot MTL). This matters whenever the result is buffered.
- **"Filter keeps its source's bound" is right but loose.** Value-domain bounds (point 6) and integer ranges (point 7) are needed alongside it.

## Decisions this raises

1. **A rate invariant on `gps`:** for example, `t` strictly increasing with at least 1 s between fixes. Without one, time windows have no tighter bound than the whole buffer.
2. **Approximate builtins:** do we offer `approxMedian`/`approxUniq` sketches? They're the only single-pass bounded options when the domain is unbounded. The alternative is to keep only exact versions with replay strategies.
3. **Arithmetic:** fixed point or integers for accumulators that are subtracted from (invertible windows), given soft-float and drift.
4. **Budgets:** the arena size per step and the work budget per tick on the C6.

## References

**R** = read in the primary source; **P** = partly read (see the section for which pages); **S** = secondary only.

**Stream queries and bounded memory**
- **R** A. Arasu, B. Babcock, S. Babu, J. McAlister, J. Widom. "Characterizing Memory Requirements for Queries over Continuous Data Streams." *ACM TODS* 29(1):162–194, 2004. Extends PODS 2002, pp. 221–232. https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/mem.pdf. Main body read; the proof appendix was not.
- **P** B. Babcock, S. Babu, M. Datar, R. Motwani, J. Widom. "Models and Issues in Data Stream Systems." *PODS 2002*. https://www.cs.princeton.edu/courses/archive/spr04/cos598B/bib/BabcockBDMW.pdf
- **P** M. Datar, A. Gionis, P. Indyk, R. Motwani. "Maintaining Stream Statistics over Sliding Windows." *SIAM J. Comput.* 31(6):1794–1813, 2002. https://moodle2.units.it/pluginfile.php/718390/mod_resource/content/0/Stream_statistics_sliding_window.pdf
- **R** Polars streaming: https://docs.pola.rs/user-guide/concepts/streaming, plus `crates/polars-stream/src/physical_plan/{fmt,mod}.rs` and `crates/polars-expr/src/reduce/convert.rs` on `main` (read through a summarising fetch).
- **R** DuckDB: https://duckdb.org/docs/current/guides/performance/how_to_tune_workloads and `src/optimizer/topn_optimizer.cpp` (read through a summarising fetch).
- **P** M. Budiu, T. Chajed, F. McSherry, L. Ryzhyk, V. Tannen. "DBSP: Automatic Incremental View Maintenance for Rich Query Languages." *PVLDB* 16(7):1601–1614, 2023. https://arxiv.org/abs/2203.16684 (v1)
- **P** F. McSherry, D. G. Murray, R. Isaacs, M. Isard. "Differential dataflow." *CIDR 2013*. https://www.cidrdb.org/cidr2013/Papers/CIDR13_Paper111.pdf

**Fusion and lazy evaluation**
- **R** O. Kiselyov, A. Biboudis, N. Palladinos, Y. Smaragdakis. "Stream Fusion, to Completeness." *POPL 2017*. https://arxiv.org/abs/1612.06668 (read through the ar5iv HTML).
- **R** O. Kiselyov, T. Kobayashi, N. Palladinos. "Complete Fusion for Stateful Streams: Equational Theory of Stateful Streams and Fusion as Normalization-by-Evaluation." arXiv:2412.15768 (v2 2026). https://arxiv.org/abs/2412.15768v2
- **R** D. Coutts, R. Leshchinskiy, D. Stewart. "Stream Fusion: From Lists to Streams to Nothing at All." *ICFP 2007*, pp. 315–326. https://www.cs.tufts.edu/~nr/cs257/archive/duncan-coutts/stream-fusion.pdf
- **S** P. Wadler. "Deforestation: Transforming Programs to Eliminate Trees." *TCS* 73:231–248, 1990. https://homepages.inf.ed.ac.uk/wadler/topics/deforestation.html. The text couldn't be extracted; the summary comes via Gill et al.
- **P** A. Gill, J. Launchbury, S. L. Peyton Jones. "A Short Cut to Deforestation." *FPCA 1993*, pp. 223–232. https://users.cs.northwestern.edu/~robby/courses/395-495-2017-winter/deforestation-short-cut.pdf
- **P** D. G. Murray, M. Isard, Y. Yu. "Steno: Automatic Optimization of Declarative Queries." *PLDI 2011*. https://microsoft.com/en-us/research/wp-content/uploads/2011/06/paper-pldi.pdf
- **R** Rust `std::iter`: https://doc.rust-lang.org/std/iter/trait.Iterator.html, https://doc.rust-lang.org/std/iter/struct.MapWindows.html, plus `library/core/src/iter/adapters/{cycle,filter,map_windows}.rs`.
- **R** expr-lang/expr v1.17.8: `optimizer/*.go`, `compiler/compiler.go`, `vm/vm.go`. https://github.com/expr-lang/expr/tree/v1.17.8/optimizer
- **P** J. Gustavsson, D. Sands. "A Foundation for Space-Safe Transformations of Call-by-Need Programs." *HOOTS '99*, ENTCS 26. https://hjemmesider.diku.dk/~henglein/space/space2001/papers/DavidSands.pdf
- **S** J. Gustavsson, D. Sands. "Possibilities and Limitations of Call-by-Need Space Improvement." *ICFP 2001*. Abstract only.

**Static memory bounds**
- **R** N. Halbwachs, P. Caspi, P. Raymond, D. Pilaud. "The Synchronous Data Flow Programming Language LUSTRE." *Proc. IEEE* 79(9):1305–1320, 1991. https://homepage.divms.uiowa.edu/~tinelli/classes/181/Fall20/Papers/Halb91.pdf
- **R** E. Jahier, P. Raymond, N. Halbwachs. *The Lustre V6 Reference Manual*. https://www-verimag.imag.fr/DIST-TOOLS/SYNCHRONE/lustre-v6/doc/lv6-ref-man.pdf
- **R** J.-L. Colaço, B. Pagano, M. Pouzet. "Scade 6: A Formal Language for Embedded Critical Software Development." *TASE 2017*. https://www.di.ens.fr/~pouzet/bib/tase17.pdf
- **R** I. Perez, F. Dedden, A. Goodloe. *Copilot 3.* NASA/TM-2020-220587. https://ntrs.nasa.gov/api/citations/20200003164/downloads/20200003164.pdf
- **P** Faust syntax manual (`@`, `mem`, tables): https://faustdoc.grame.fr/manual/syntax/ (read through a summarising fetch). **R** A. Herrou et al. "Towards Fixed-Point Formats Determination for Faust Programs." *JIM 2024*. https://arxiv.org/pdf/2403.06527
- **S** K. Hammond, G. Michaelson. "Predictable Space Behaviour in FSM-Hume." *IFL 2002*, LNCS 2670. https://doi.org/10.1007/3-540-44854-3_1. Abstract only. **R** K. Hammond, G. Michaelson, R. Pointon. *The Hume Report, Version 1.1*. https://www.macs.hw.ac.uk/~greg/hume/hume11.pdf
- **R** J. Hoffmann, K. Aehlig, M. Hofmann. "Multivariate Amortized Resource Analysis." *ACM TOPLAS* 34(3), 2012. https://www.cs.yale.edu/homes/hoffmann/papers/HAH12Toplas.pdf
- **R** cel-go cost model: https://raw.githubusercontent.com/cel-expr/cel-go/HEAD/docs/cost-model.md and `common/cost/{estimator,cost}.go`. CEL language definition: https://raw.githubusercontent.com/google/cel-spec/master/doc/langdef.md
- **R** Kubernetes CRD validation rule cost: https://kubernetes.io/docs/tasks/extend-kubernetes/custom-resources/custom-resource-definitions/ and `staging/src/k8s.io/apiserver/pkg/cel/common/schemas.go`
- **R** R. Heckmann, C. Ferdinand. "Verifying Safety-Critical Timing and Memory-Usage Properties of Embedded Software by Abstract Interpretation." *DATE 2005*. https://arxiv.org/pdf/0710.4753

**Not read** (paywalled or blocked):
- the ACM DOI pages (403);
- N. Mitchell, "Leaking Space" (ACM, 403);
- the Hume IFL 2002 full text;
- Copilot RV 2010;
- Kobayashi & Kiselyov 2024 on windowing;
- Regehr, Reid, Webb, EMSOFT 2003.

## Detailed sections

Each source has a citation, the URLs opened, what it establishes (tagged read, secondary or ours), how it maps to our builtins, what to borrow, and where it weakens our thinking.

- [streams.md](streams.md): which stream queries fit in bounded memory, and incremental evaluation (Arasu, Babcock, DGIM, Polars, DuckDB, DBSP, differential dataflow).
- [fusion.md](fusion.md): fusion and lazy list evaluation (strymonas, stream fusion, deforestation, Steno, Rust iterators, expr's optimizer and VM, call-by-need space).
- [static-bounds.md](static-bounds.md): languages and analyses with static memory bounds (Lustre, SCADE, Copilot, Faust, Hume, RAML, CEL and Kubernetes, StackAnalyzer).
