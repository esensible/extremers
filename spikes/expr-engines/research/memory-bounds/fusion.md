## Fusion and lazy list evaluation

Scope: how the literature removes intermediate lists, what it actually *guarantees* about memory, where it stops working, and what each choice costs in CPU. Memory is our hard constraint and must be bounded statically. CPU is secondary but still matters, so for every memory/compute trade-off both sides are recorded.

Tags: **(read)** means I read it in the primary source at the URL given. **(secondary)** means it comes from a secondary description, or from an automated summary of the source that I could not check line by line. **(ours)** marks our own inference, not a claim made by the source.

---

### 1. Kiselyov, Biboudis, Palladinos, Smaragdakis: "Stream Fusion, to Completeness" (POPL 2017), and later strymonas work

**Citation.** O. Kiselyov, A. Biboudis, N. Palladinos, Y. Smaragdakis. *Stream Fusion, to Completeness.* POPL 2017. arXiv:1612.06668.
**Opened:** https://arxiv.org/abs/1612.06668 (PDF https://arxiv.org/pdf/1612.06668; HTML rendering https://ar5iv.labs.arxiv.org/html/1612.06668).
*Caveat:* the PDF did not text-extract, so I read the paper through the ar5iv HTML rendering via an extraction tool. Section numbers and short quotes below come from that rendering. Where a claim is the tool's inference rather than the paper's own text, I tag it (secondary).

**What it establishes**
- **The guarantee (read, §6.4, Theorem 1).** The generated code has no extra heap allocations in the main loop. Pipelines are built from `of_arr`/`unfold` producers, then `map`, `filter`, `take`, `flat_map`, `zip_with`, and end in a `fold` consumer. There is **one exception**: zipping two streams that are *both* nested (made by `flat_map`, or by `filter`, which is a special case of it). There the paper says some "rate-adjusting allocation is inevitable, even in hand-written code".
- **Supported combinators (read, Fig. 1, §2).** "Any combination of zipping, nesting, sub-ranging, filtering, mapping" of finite or infinite streams.
- **How it works (read, §5).** Staging in MetaOCaml. The stream is a compile-time (code-generation-time) description, and three steps turn it into a plain loop:
  1. The stepper is converted to continuation-passing style, so step shapes such as Yield/Skip are decided at generation time.
  2. Stream state becomes mutable cells allocated before the loop.
  3. Each producer becomes a `for` loop (`For` producer, with a known upper bound and an index function) or a `while` loop (`Unfold` producer).
  The no-allocation proof just inspects the generated code for tuples or closures.
- **Linear vs nested (read, §6.1, §6.3).**
  - A *linear* stream yields exactly one element per state advance. `map` keeps a stream linear.
  - `filter` produces a *nested* stream: it is a `flat_map` whose inner stream yields at most one element.
  - Zip of linear with linear shares one loop index.
  - Zip of linear with nested pushes the linear stepper into the nested one.
  - Zip of nested with nested must "reify" one side into a pull function returning an option. The paper says there is no way around capturing that continuation.
- **What is not covered (secondary).** Sorting, grouping, random access, reverse and stream reuse are not combinators in the paper. The paper describes its scope as data accessed strictly linearly and processed uniformly (§1), which matches the extraction tool's reading.
- **CPU side (read, §7).** The fused code matches hand-written loops. Staging cost (a few ms) is excluded from the benchmarks.

**Later work: Kiselyov, Kobayashi, Palladinos, "Complete Fusion for Stateful Streams: Equational Theory of Stateful Streams and Fusion as Normalization-by-Evaluation"**
arXiv:2412.15768, v1 Dec 2024, v2 Feb 2026. The abstract page gives no journal reference. A summary I could not verify named TOPLAS as the venue (secondary), so cite it as an arXiv preprint.
**Opened:** https://arxiv.org/abs/2412.15768v2 and https://arxiv.org/html/2412.15768v2.

- **Definition 1.1 (read).** A pipeline is "completely fused" when it runs without function calls or memory allocations, "provided each individual operator may be executed without calls and allocations". The guarantee is therefore *conditional on each operator being constant-space*.
- **Claimed guarantees (read, §1.3).** No calls, closures, tuples or intermediate structures, and constant space. This is "statically assured for all pipelines".
- **Combinators (read).** `map`, `filter`, `map_accum`/`scan`, `take`/`take_while`/`drop`/`drop_while`, `flat_map`, `zip`/`zip_with`, and sliding windows. The sliding windows are detailed in a companion paper (Kobayashi & Kiselyov 2024), which I did not open.
- **Deliberate exclusion (read, §1.1).** strymonas provides no `unzip` or other splitting operator. Splitting "admits pathological pipelines" that need unlimited buffering. Only lockstep split-join, done by tupling, is allowed.
- **Sharing (read, §2.1).** Let-expressions in the metalanguage share stream *descriptions*, not the streams themselves. Binding a stream once and using it twice therefore **re-runs the producer**. In other words, strymonas has replay semantics.
- **Normalization-by-evaluation (NbE) (read, §1.3, §3).** NbE computes a unique normal form that maps to a state machine, and it is certain to terminate. The authors contrast this with rewrite rules, which can be non-confluent and non-terminating. Zip-conversion removes `zip` whenever one side is linear (§1).

The sibling paper "Highest-performance Stream Processing" (Kiselyov, Kobayashi, Biboudis, Palladinos; arXiv:2211.13461, opened at https://ar5iv.labs.arxiv.org/html/2211.13461) mentions windowing only in passing. I found no buffer-size details there (secondary).

**Mapping to our builtins (ours)**

| Our builtin | Fusion status |
|---|---|
| `map`, `filter`, `take`, slices `gps[a:b]`, `first`, `find`, `findIndex`, `count`, `sum`, `min`, `max`, `mean`, `reduce` | All within the strymonas model: a linear or nested producer plus a fold. Constant memory, single pass. |
| `#index` | A zip with a counter, which is linear. Free. |
| `last` | A fold that keeps the last element: O(1) memory. |
| `xs[#index]` with `xs` a random-access source (`gps`, `gps[1:]`) | Not a stream operation; it is an O(1) indexed read from flash. Free. |
| `xs[#index - k]` with `xs` a filtered recipe | A zip of two nested streams (the filter, and itself shifted by k): exactly the case strymonas says needs reification. Constant k: a k-slot ring buffer is the hand-written optimum. |
| `sortBy`, `median`, `uniq` | Outside the model entirely. |

**Borrow**
- Fusion as *normalisation by construction*: a deterministic compile-time evaluation of the pipeline into a loop plan. This is better than rewrite rules applied opportunistically.
- The linear/nested classification of every recipe.
- strymonas's refusal to provide splitting.
- The condition in Definition 1.1, applied as our admissibility rule: each operator must itself be constant-space, or else its bound gets added to the budget.

**Where it weakens our thinking**
- The "complete" guarantee is about *allocation*, not about *re-computation*. strymonas makes sharing descriptions-only, so a recipe used twice costs two producer runs. That is our replay model, with its CPU cost made explicit.
- The literature offers no constant-memory answer for an index into a nested stream at a *non-constant* offset, e.g. `ys[#index * 2]` or `ys[f(#)]` over a filter. Our design must reject this or buffer it.

---

### 2. Coutts, Leshchinskiy, Stewart: "Stream Fusion: From Lists to Streams to Nothing at All" (ICFP 2007)

**Citation.** D. Coutts, R. Leshchinskiy, D. Stewart. ICFP '07, pp. 315–326. DOI 10.1145/1291151.1291199. I did not open the DOI page.
**Opened:** the author PDF mirror at https://www.cs.tufts.edu/~nr/cs257/archive/duncan-coutts/stream-fusion.pdf. I read all 12 pages.

**What it establishes (all read)**
- **Representation (§2).** A stream is `Stream (s -> Step a s) s`, where `Step = Done | Yield a s | Skip s`.
- **Why `Skip` exists (§3).** It lets every stepper be *non-recursive*. Without it, `filter` would have to loop internally until it found a match. Non-recursive steppers are what let the compiler fuse with ordinary optimisations. Only the final consumer (`foldl`/`foldr`) is recursive.
- **Fusion mechanism (§2.2).** List functions are written as `unstream . f_s . stream`, and a single rewrite rule `stream (unstream s) = s` removes the conversions between adjacent functions.
- **What fuses (§3–5).** `zip` (it buffers *one* element from the first stream in its state, `Maybe a`, because either side may `Skip`), left folds, `append`, `concatMap`, and list comprehensions. These are cases that foldr/build cannot fuse well (§1.1).
- **Allocation is not automatically removed (§7).** Stream fusion "does not, however, reduce allocation" by itself: it swaps lists for `Step` values and state constructors such as `Either`/`Maybe`. Removing those needs inlining, case-of-case and constructor specialisation (§7.1). Nested `concatMap` additionally needs either static argument transformation or specialisation on partial applications, which GHC did not do robustly at the time (§7.2).
- **Results (§8).** Mean speedup was 3%, and allocation fell by an average of 4.4%. Where `Step` constructors were not eliminated, programs got *worse*: the 'paraffins' benchmark ran 24% slower, and two programs allocated far more.

**Mapping to us (ours)**
- `Skip` is our `filter` problem in miniature. A filtered recipe cannot be advanced by a fixed number of elements, so it is not indexable.
- Zipping two filtered streams needs one buffered element *and* per-step extra state.
- `#index` over `filter(...)` counts *output* positions, so it needs its own counter.

**Borrow**
- The stepper model (Done/Yield/Skip) is a good fit for a no_std Rust VM's pull interface.
- The paper's own lesson: fusion that *depends on an optimiser succeeding* is not a guarantee.

**Contradicts / weakens us**
- If our static bound assumed "fused" whenever a rewrite *might* fire, it would be unsound. The bound has to be computed on the plan the compiler *actually* produced. That requires a closed, decidable plan language (strymonas style), not best-effort rules.

---

### 3. Wadler, "Deforestation" (1988/1990), and Gill, Launchbury, Peyton Jones, "A Short Cut to Deforestation" (FPCA 1993)

**Wadler citation.** P. Wadler. *Deforestation: transforming programs to eliminate trees.* Theoretical Computer Science 73:231–248, 1990 (ESOP '88 special issue).
**Opened:** https://homepages.inf.ed.ac.uk/wadler/topics/deforestation.html and the PostScript at https://homepages.inf.ed.ac.uk/wadler/papers/deforest/deforest.ps. The PostScript uses bitmap fonts and I could not extract its text, so **the treeless-form details below are (secondary)**. They come from Gill et al. §2, which I did read.
- **Treeless form (secondary, via Gill et al. §2).**
  - A restrictive, first-order form in which "all variables must be used linearly" and no internal data structures are allowed.
  - Under that restriction Wadler proves two things: the result is again treeless, so no intermediate structures are built, and the algorithm terminates.
  - Outside treeless form the algorithm may not terminate. Fold steps must detect repeated calls, which is costly and "not foolproof".

**Gill citation.** A. Gill, J. Launchbury, S. L. Peyton Jones. *A Short Cut to Deforestation.* FPCA 1993, pp. 223–232.
**Opened:** https://users.cs.northwestern.edu/~robby/courses/395-495-2017-winter/deforestation-short-cut.pdf. I read pp. 1–4.
- **The rule (read, §3).** `foldr k z (build g) = g k z`.
  - Consumers are written as `foldr`. This covers `sum`, `and`, `elem`, `map`, `filter`, `++`, `concat`, and `foldl` via a higher-order `foldr`.
  - Producers are written as `build`.
- **Correctness (read, §3.4).** The rule holds when `g` is polymorphic in the result type, by the "free theorem" for that type (parametricity).
- **Limits (read).** The method does not guarantee removing *all* intermediate structures (§3).
- **Limits (secondary, via Coutts §1.1).** Left folds and zips are notable exceptions that build/foldr cannot fuse effectively.

**Mapping to us (ours)**
- **Wadler's linearity condition is precisely the line between free fusion and our hard cases.** Every recipe that is consumed *once, in order*, is "treeless" and fuses.
- A recipe that is used twice (`let t = filter(gps, ...); count(t) + sum(map(t, ...))`) violates linearity. We must then choose, per use site:
  - **Replay**, which is call-by-name: 0 extra memory, 2× producer CPU.
  - **Buffer**, which is call-by-need: memory = max length × element size, 1× CPU.

**Borrow**
- Treat "linear use" as the precondition for fusion and make non-linear uses explicit decisions in the plan.

---

### 4. Murray, Isard, Yu: "Steno: Automatic Optimization of Declarative Queries" (PLDI 2011)

**Citation.** D. G. Murray, M. Isard, Y. Yu. PLDI '11. DOI 10.1145/1993498.1993513 (not opened).
**Opened:** https://microsoft.com/en-us/research/wp-content/uploads/2011/06/paper-pldi.pdf. I read pp. 1–6.

**What it establishes (read)**
- **Intermediate language (§4.1, Table 1).** LINQ operators are lowered to QUIL symbols:
  - `Src`: sources such as Range and Repeat.
  - `Trans`: `Select`, i.e. map.
  - `Pred`: `Where`/`Take`/`Skip`, i.e. filter-like.
  - `Sink`: `GroupBy`, `OrderBy`, etc. Its Haskell analogue is `foldl` into an intermediate collection, which Sink "typically" builds up in memory.
  - `Agg`: `Aggregate`/`Min`/`Sum`.
  - `Ret`.
  - Nested operators `SelectMany`/`Join` correspond to concatMap.
- **Grammar (§4.1).** `Src (Trans | Pred | Sink | <query>)* Agg? Ret`. Without nesting this is a regular language, recognised by a 5-state finite state machine (Fig. 4). Nesting makes it a pushdown automaton with a stack of loop insertion points (§5.1).
- **Blocking operators (§4.2).** A `Sink` ends the current loop: its collection is declared in the prelude and filled in the body. The next operator then **starts a new loop over the sink collection**. Blocking operators therefore always materialise.
- **Operator specialisation (§4.3).** `GroupBy` followed by an aggregating result selector becomes `GroupByAggregate`, which keeps per-key partial aggregates instead of the groups themselves. Memory drops further to O(1) keys "if it is known that the collection is ordered by the same key as the grouping".
- **CPU side (Fig. 1, §1).** About 7.4× speedup over LINQ iterators, by removing virtual calls per element per operator.

**Mapping to us (ours)**
- `sortBy` (and `uniq`, and grouping inside `reduce`) is a QUIL Sink. Its memory is O(n × element) unless the consumer that follows lets us specialise it, exactly as `GroupBy|Agg` became `GroupByAggregate`:
  - `sortBy|first`/`last` → min-by/max-by, O(1) memory.
  - `sortBy|take(k)` → a bounded top-k, O(k) memory.
  - `sortBy|sum`/`count`/`min`/`max`/`mean` → drop the sort.
- **Order-awareness is a fusion source.** `gps` is sorted by `t`. So `sortBy(gps[...], .t)` is a no-op, and grouping/uniq on keys monotone in `t` can run in O(1), per Steno §4.3.

**Borrow**
- The explicit Src/Trans/Pred/Sink/Agg classification as our plan IR.
- The regular-language view: a pipeline is a word over these symbols, and each Sink adds a buffer term to the memory bound unless the Sink→consumer pattern is specialised.

**Weakens us**
- Steno never removes a Sink. It only shrinks one when a specific consumer follows. Every `sortBy` that is not followed by one of our recognised consumers, or whose result is stored into state, costs a full buffer. Its maxItems × record size must come from length-bound inference.

---

### 5. Rust `std::iter` as the practical pull model

**Opened:**
- https://doc.rust-lang.org/std/iter/trait.Iterator.html
- https://doc.rust-lang.org/std/iter/struct.MapWindows.html
- https://doc.rust-lang.org/std/primitive.slice.html
- Source files on rust-lang/rust master: `library/core/src/iter/adapters/{cycle,filter,map_windows}.rs`, `library/core/src/slice/mod.rs`, `library/alloc/src/slice.rs`.

**What it establishes**
- **Iterators are lazy (read).** Adapters (`map`, `filter`, `take`, `skip`, `step_by`, `zip`, `scan`, `flat_map`, `take_while`, `fuse`) do no work until consumed. Each holds O(1) state: its inner iterator plus a closure.
- **Consumers (read).** `sum`, `count`, `min_by_key`, `max_by_key`, `last`, `nth`, `fold` consume in one pass with O(1) state.
- **`nth` (read).** It *consumes* the preceding elements, so random access on a general iterator is O(n) time.
- **`Filter::size_hint` (read, filter.rs).** Returns `(0, upper)`, i.e. it keeps the upper bound and drops the lower bound. This is the same rule our length-bound inference needs.
- **`Filter::count` (read).** Specialised to a branchless `map(p as usize).sum()`. This is an example of a library-level peephole fusion.
- **`rev` (read).** Requires `DoubleEndedIterator`. `Filter` is double-ended only when its inner iterator is (`impl<I: DoubleEndedIterator, P> DoubleEndedIterator for Filter<I, P>`, implemented with `rfind`).
  - Backwards traversal of a recipe over a random-access source is free, and composes through `filter`.
  - (ours) This is how `last(filter(gps, p))` becomes a backward scan, like expr's `findLast`.
- **`cycle` (read, cycle.rs).** Requires `Clone`. It stores `orig: I` and, on exhaustion, does `self.iter = self.orig.clone()`.
  - **Restarting a Rust iterator means cloning its unstarted state.** That is replay.
  - The clone is O(state size), but every restart re-runs the whole producer.
- **No sort on `Iterator` (read).** Sorting requires `collect` into a `Vec` and then a slice sort.
- **Slice sorts (read, slice source docs).**
  - `slice::sort` (driftsort) allocates auxiliary memory: none for short slices, `len` for medium ones, clamped at `len/2` beyond that.
  - `sort_unstable` is in-place and does not allocate.
  - `select_nth_unstable` is in-place and O(n).
  - The unstable, nightly-only `partial_sort_unstable` (feature `slice_partial_sort_unstable`) is in-place and O(n + k log k).
  - All of these need the data already materialised in a mutable slice.
- **Windows (read).** `slice::windows` exists only on slices. The iterator equivalent `map_windows` is nightly-only (`iter_map_windows`, issue #87155), takes a const generic `N`, and internally keeps a buffer that is "semantically `[MaybeUninit<T>; 2 * N]`" (map_windows.rs). So a window of N over a non-random-access stream costs 2N elements of fixed buffer.
- **`peekable` (read).** It advances the underlying iterator on the first `peek`, so it holds one look-ahead element.

**Mapping to us (ours)**
- Each recipe in our VM should carry Rust-like capability flags:
  - *indexable*, like a slice or `ExactSizeIterator` plus O(1) `nth`;
  - *double-ended*;
  - *restartable*, like `Clone`.
- Capabilities propagate:
  - A source, or a slice of one, has all three.
  - `map` preserves all three.
  - `filter` keeps *double-ended* and *restartable* but loses *indexable*.
  - `take(k)` keeps *indexable* if its input has it.
- `xs[#index - k]` is legal without a buffer only if `xs` is indexable. Otherwise it needs a ring buffer of k+1 elements. Rust's own `map_windows` uses 2N for move efficiency; we can use N plus a head index.

**Borrow**
- A static capability lattice (indexable / double-ended / restartable / sequential-only).
- The `size_hint` upper-bound rules.
- Clone-as-restart.
- For top-k: Rust gives only materialise-then-partial-sort. We need a streaming bounded heap of k, which costs O(k) memory and O(n log k) CPU.

---

### 6. expr-lang/expr v1.17.8: its optimizer and VM

**Opened (read in full):**
- https://github.com/expr-lang/expr/tree/v1.17.8/optimizer (the directory listing).
- Raw sources at `https://raw.githubusercontent.com/expr-lang/expr/v1.17.8/` for:
  - `optimizer/{optimizer,fold,filter_map,filter_len,filter_first,filter_last,sum_map,sum_range,sum_array,count_any,count_threshold,predicate_combination,in_array,in_range}.go`
  - the relevant parts of `compiler/compiler.go` and `vm/vm.go`.

**Exact rewrites (read)**

`Optimize` runs these in order:
1. `inArray`
2. `fold`, repeated up to 1000 times until nothing changes
3. `constExpr`, only when `ConstFns` are configured
4. `inRange`
5. `filterMap`
6. `filterLen`
7. `filterLast`
8. `filterFirst`
9. `predicateCombination`
10. `sumRange`
11. `sumArray`
12. `sumMap`
13. `countAny`
14. `countThreshold`

What each one does:
- **`fold`.**
  - Constant-folds unary and binary arithmetic, string `+`, `==`, `and`/`or` with a constant side, and literal arrays of scalars (to a constant).
  - **`filter(filter(xs, p), q)` → `filter(xs, p && q)`**.
- **`filterMap`.** `map(filter(xs, p), f)` → a single `filter` node with a `Map` field. It is **skipped if `f` contains `#index`**: `Find(..., isIndexPointer)`. The reason is that the output index differs from the input index.
- **`filterLen`.** `len(filter(xs, p))` → `count(xs, p)`.
- **`filterFirst`.**
  - `filter(xs, p)[0]` → `find(xs, p)`, which throws if not found.
  - `first(filter(xs, p))` → `find(xs, p)`, which returns nil if not found.
  - Both carry any `Map`.
- **`filterLast`.** Same, with `[-1]` and `last`, → `findLast`. `findLast` is compiled as a **backwards loop** (`emitLoopBackwards`).
- **`predicateCombination`.**
  - `all(x,p) && all(x,q)` → `all(x, p && q)`.
  - `any(x,p) || any(x,q)` → `any(x, p || q)`.
  - `none(x,p) && none(x,q)` → `none(x, p || q)`.
  - It applies only when both collections stringify identically.
- **`sumRange`.**
  - `sum(m..n)`, and `sum(m..n, # ± k | # * k)`, with integer constants → closed-form arithmetic series.
  - `reduce(m..n, # + #acc [, init])` → the same.
- **`sumArray`.** `sum([a, b, c])` → `a + (b + c)`.
- **`sumMap`.** `sum(map(xs, f))` → `sum(xs, f)`, a single loop. It has **no `#index` guard**, which is fine because the loop source is still `xs`.
- **`countAny`.** `count(xs, p) > 0` and `count(xs, p) >= 1` → `any(xs, p)`, with early exit.
- **`countThreshold`.** `count(xs,p) {>,>=,<,<=} N` sets a `Threshold` on the count node, so the loop exits early once the count reaches N (or N+1).
- **`inArray`.** `x in [ints...]` / `[strings...]` → a constant set map.
- **`inRange`.** `x in m..n`, for int x → `x >= m and x <= n`.

**VM materialisation (read)**
- Every `filter` and `map` builtin compiles to a loop that pushes each kept or mapped element onto the stack, then `OpArray`:
  - It pops `size` values into a new `[]any`.
  - It charges `vm.memGrow(size)` first.
  - **Every non-rewritten `map`/`filter` result is a fully materialised array.**
- `sortBy` appends every item and its key into a `runtime.SortBy` accumulator (`OpSortBy`), then runs `sort.Sort` (`OpSort`), also charging `memGrow(len)`.
- `groupBy` builds a map.
- **`sum`, `count`, `find`, `findIndex`, `findLast`, `reduce` do not allocate.** Each is one loop over the *input* array held in the scope, with an accumulator or count.
- `memGrow` adds to `vm.memory` and panics with "memory budget exceeded" at `MemoryBudget`. **expr's memory control is a dynamic counter in elements, not a static bound.**
- `#index` is `OpGetIndex`: the current loop's index into its own input array.

**Mapping to us (ours)**
- expr's optimizer is a small set of one-level peephole patterns. It is not a fusion framework.
- Chains such as:
  - `sum(map(filter(...)))`: `filterMap` gives `filter{Map}`, but there is no `sum(filter)` rewrite, so the filter is still materialised.
  - `first(sortBy(...))`
  - `map(map(...))`

  all still allocate.
- Our recipe/fusion design strictly subsumes these rewrites, and we should keep every one of them as a regression test of expected plans.
- The `#index` guard in `filterMap` is the same hazard we face: fusing changes what `#index` means.

**Borrow**
- The rewrite list as a minimum.
- `findLast` as a backward loop over a double-ended source.
- Early-exit `count` thresholds, as a CPU saving.

**Contradicts nothing**
- But it confirms that the upstream semantics are *eager and materialising*. Our lazy semantics must match them observably: same results, same nil/throw behaviour, e.g. `filter(...)[0]` throws but `first(filter(...))` returns nil.

---

### 7. Space behaviour of laziness: replay (call-by-name) vs sharing (call-by-need)

**Primary source.** J. Gustavsson, D. Sands. *A Foundation for Space-Safe Transformations of Call-by-Need Programs.* HOOTS '99; the published version is in ENTCS vol. 26, 1999. I did not open the ENTCS page.
**Opened:** the author draft (dated 2 July, submitted to HOOTS '99) at https://hjemmesider.diku.dk/~henglein/space/space2001/papers/DavidSands.pdf. I read pp. 1–3.

**What it establishes (read, §1, §4)**
- Call-by-need refines call-by-name by evaluating each argument at most once. This reduces time, "often dramatically". But the extra sharing *prolongs the lifetime of data*, often at the cost of space.
- **The canonical example (§1).** `let xs = [1..n]; x = head xs; y = last xs in x + y`:
  - It runs in **constant** space if `x` is evaluated first: the list is generated and collected as `last` walks it.
  - It takes **O(n)** space for `y + x`: while `y` walks the list, `x`'s thunk still references its head.
  - So the innocent law `x + y = y + x` is *not* space-safe under sharing. Space depends on evaluation order and on which thunks still hold references.
- Space behaviour under call-by-need depends on the implementation, e.g. GC/abstract-machine details (§4). The authors measure heap and stack separately, and count update markers as heap.
- They prove that inlining *affine-linear* (used-once) bindings is work- and space-safe (abstract).

**Supporting source (abstract only).** J. Gustavsson, D. Sands. *Possibilities and Limitations of Call-by-Need Space Improvement.* ICFP 2001, pp. 265–276. DOI 10.1145/507635.507667.
**Opened:** abstract page https://icfpconference.org/archived/icfp2001/cristal.inria.fr/ICFP2001/Abstracts/60.html.
- (read, abstract) "Innocent-looking program transformations can easily change the space complexity" of lazy programs.
- (read, abstract) The asymptotic space-improvement relation is semantically badly behaved.

**Not used.** N. Mitchell, "Leaking Space" (ACM Queue / CACM 2013) is the standard practitioner reference: the `mean = sum xs / length xs` leak. Both ACM URLs returned HTTP 403, so I could not open it, and I cite nothing from it.

**Mapping to us (ours): the trade-off, both sides**
- **Sharing (memoised lazy list, call-by-need).**
  - Memory = the *peak live* prefix of the list, which depends on consumer order. It is hard to bound statically without modelling evaluation order and GC reachability. That is what Gustavsson & Sands show to be fragile.
  - CPU = 1× producer.
- **Replay (restartable recipe, call-by-name).**
  - Memory = O(1) per live cursor, independent of evaluation order. **This makes static bounds compositional**, which is our main goal.
  - CPU = one producer run per use. Uses *inside a per-element predicate* multiply. For example, `map(xs, count(filter(gps, p(#))))` replays a 61,440-element scan for every element of `xs`, which is O(|xs| × n).
  - Replays of replays nest multiplicatively.
- **Buffering (explicit materialisation).**
  - Memory = upper bound of length × element size, known statically from maxItems and length inference.
  - CPU = 1×, plus a copy.
- **A decision rule follows (ours).**
  1. Default to replay.
  2. Compute a static *work* estimate too: number of producer passes times source length.
  3. Buffer only when the replay cost is super-linear *and* the buffer fits the remaining memory budget.
  4. Otherwise reject, or ask the author to restructure.
  Gustavsson & Sands's affine-linear inlining result is the formal backing for "a recipe used once can always be fused, at no cost".

---

### Implications for us

1. **Our "fused" means strymonas Definition 1.1, conditionally.** Zero intermediate storage is guaranteed only for **single-consumer, in-order** pipelines of `map`/`filter`/`take`/slice/`#index` into a fold (`sum`/`count`/`min`/`max`/`mean`/`reduce`/`first`/`find`/`last`). Everything else needs an explicit memory term. Fusion must be **by construction** (a deterministic plan normaliser, NbE-style), not by optimistic rewrite rules. Coutts et al. showed rule-based fusion silently missing and making allocation *worse*. A static bound is only sound if it is computed on the plan actually produced.
2. **`#index` access is the zip problem, and our current thinking is incomplete there.**
   - `xs[#index ± k]` is free only when `xs` is **indexable**: a source, a slice of one, or a `map` over an indexable recipe.
   - Over a `filter`ed recipe it is a zip of two nested streams. Constant k means a k-slot ring buffer, as planned. A non-constant offset (`xs[#index*2]`, `xs[g(#)]`, `#index` from a different loop) cannot be fused in constant memory. It must be **buffered** (bounded by maxItems) or **replayed per access**, which is O(n) per access and O(n²) total.
   - We need an explicit capability lattice: indexable ⊃ double-ended ⊃ restartable ⊃ sequential. `filter` drops "indexable".
   - Also follow expr's rule that `#index` must refer to the position in the *output* of the recipe the predicate iterates. Fusing `map∘filter` must not change it, which is why expr's `filterMap` refuses to fuse when `#index` appears.
3. **`sortBy` is a blocking Sink (Steno, Rust, expr all materialise).** It is removable only when the consumer that follows is recognised:
   - `first`/`last` → min/max-by, O(1).
   - `take(k)` → a streaming bounded heap. The memory is O(k) and CPU O(n log k). Rust's in-place partial sort still needs the full slice, so it does not apply.
   - Order-insensitive aggregates (`count`, `min`, `max`, `sum` over integers) → drop the sort.
   - Caveats:
     - `sortBy|sum` over **floats** is not exactly order-insensitive (rounding), so drop the sort only for integer or fixed-point values, or document the change. (ours)
     - `reduce` with an arbitrary reducer is order-dependent, so its sort cannot be dropped.
     - `median` needs either a buffer (O(n)) or a replay-based selection: repeated counting passes over the source, O(1) memory and O(n · passes) CPU. This is a clean memory/CPU trade-off to expose. (ours)
     - `uniq` needs a seen-set of size O(distinct), unless the input is sorted on the key, in which case it is O(1) by adjacent comparison.
4. **Exploit known order (Steno §4.3).** `gps` is sorted by `t`. `sortBy(gps…, .t)` is the identity, `last(gps)` is the max-t fix, and group/uniq on keys monotone in t is O(1). Carry a "sorted-by" property on recipes.
5. **Replay vs sharing.**
   - The literature backs replay as the only choice whose space is **compositional and independent of evaluation order**. Gustavsson & Sands: sharing makes `x+y` vs `y+x` differ asymptotically. strymonas also chose to share descriptions, not streams.
   - But replay is *not free*. Its CPU cost multiplies under nesting, e.g. a replayed scan inside a per-element predicate.
   - The compiler should compute a static work bound alongside the memory bound, and switch to a bounded buffer when the buffer fits and replay is super-linear.
6. **Never provide splitting / fan-out.** strymonas excludes `unzip`-like operators because they need unbounded buffering. If our language allows `let t = <recipe>` used by two consumers *in lockstep* inside one expression, compile it as tupling (one pass, two accumulators). Example: `count(t)` and `sum(t)` → a single fold computing both. This is the Steno `GroupByAggregate` and the classic `mean` fix. Otherwise replay.
7. **Length-bound inference rules exist in practice.** Rust `size_hint`:
   - `filter` → `(0, upper)`
   - `map` → exact
   - `take(k)` → `min(k, n)`
   - `flat_map` → product of upper bounds

   These give the buffer sizes needed whenever something must materialise. Only lists **stored into the new state** must materialise in the end, and their maxItems comes from the schema.
8. **expr upstream gives no memory guarantee.** Its optimizer is ~14 one-level peephole rewrites, and its VM materialises every `map`/`filter`/`sortBy` result, with only a *runtime* element counter (`MemoryBudget`). Our design must keep observable equivalence with this eager semantics, including the throw-vs-nil distinctions (`filter(...)[0]` throws, `first(filter(...))` is nil). It should treat expr's rewrites as a floor, not a model.
