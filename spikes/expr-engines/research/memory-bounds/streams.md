## Bounded-memory stream queries

How to read the tags: **(read)** means I read the claim in the primary source myself; **(secondary)** means I got it from another document. Anything marked **(ours)** is my own reasoning about our language. No source makes those claims.

Our setting, recapped so this section stands alone. An engine is one pure expression `(state, event, timestamp, gps) -> state`. It runs once per second. `gps` is an append-only array of fixed 32-byte records in flash, sorted by `t`, capped at 61,440 records. Any record can be read in O(1), and the whole array can be re-read at any time. The scarce resource is **RAM**, which must be bounded statically. **CPU comes second but still counts**: when memory is within budget, take the cheapest compute. For that reason every trade-off below lists both sides.

---

### 1. Arasu, Babcock, Babu, McAlister, Widom: characterizing memory requirements

**Citation.** A. Arasu, B. Babcock, S. Babu, J. McAlister, J. Widom. "Characterizing Memory Requirements for Queries over Continuous Data Streams." *ACM Transactions on Database Systems* 29(1):162–194, March 2004. This extends the short paper of the same title in *Proc. PODS 2002*, pp. 221–232.
- Opened: https://www.microsoft.com/en-us/research/publication/characterizing-memory-requirements-for-queries-over-continuous-data-streams/ and the full TODS text at https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/mem.pdf. I read all of the main body. The proofs are in an electronic appendix, which I did not see.
- DOI 10.1145/974750.974756 redirects to dl.acm.org, but the landing page returned 403, so I have **not** confirmed that this DOI belongs to this title.
- I did **not** open the PODS 2002 version. Its page range comes from the TODS reference list (secondary).

**Model (read).**
- A stream is an unbounded **bag** of relational tuples over discrete, ordered domains (integers).
- Each stream is read **once**, in an order the evaluator does not control, and interleaved arbitrarily with the other streams. To use past data later, the evaluator must store it itself.
- "Memory" means *any* storage, including disk. One unit holds one attribute value or one count. A count is treated as one unit even though its width grows logarithmically.
- **Definition 3.1:** a query is *computable in bounded memory* if some constant M and some algorithm evaluate it using fewer than M units, for every possible instance and interleaving.
- They consider exact evaluation only. They point out that approximate quantiles fit in logarithmic memory (Greenwald–Khanna).
- Output of an SPJ query is streamed and not stored. For aggregate queries, the current answer *is* counted in memory, so an unbounded number of groups is automatically unbounded memory.

**Query class (read).**
- Select-Project-Join queries `Π_L(σ_P(S1 × … × Sn))`. P is a conjunction of atomic predicates of the form `Si.A op Sj.B` or `Si.A op k`, with op in {<, =, >}. ≤ and ≥ are a straightforward extension.
- Π is either duplicate-preserving or duplicate-eliminating.
- Self-joins are added in §8.1. Grouping and aggregation are added in §8.2.

**Main results (read).**
1. **Dichotomy.** Every query in this class needs either O(1) memory or memory linear in the input. There is nothing in between (§1, §3.3).
2. **Bounded and unbounded attributes (Definition 4.5).** An attribute is *bounded* if the transitive closure of P gives it both a constant upper bound and a constant lower bound.
3. **Theorem 5.3.** An SPJ query is bounded-memory iff every "Locally Totally Ordered" (LTO) query derived from it is bounded-memory. An LTO query is obtained by adding filters until, within each stream, all attributes and constants are totally ordered.
4. **Theorem 5.6, duplicate-preserving, more than one stream.** Bounded iff:
   - C1: every projected attribute is bounded;
   - C2: both sides of every equality join are bounded;
   - C3: for every stream, MaxRef and MinRef are both empty. MaxRef and MinRef are the unbounded attributes of that stream used in a non-redundant inequality join.

   A single-stream duplicate-preserving query is always bounded, because it is a filter.
5. **Theorem 5.10, duplicate-eliminating, any number of streams.** Same C1 and C2. C3 is relaxed to `|MaxRef(Si)|eq + |MinRef(Si)|eq ≤ 1`. Each stream may therefore have one unbounded attribute in inequality joins, kept as a running **max or min**.
   - The reason (§5.2): duplicate elimination only asks whether a join partner *exists*, so an extreme value is enough. The duplicate-preserving case must know *how many* partners there are, which needs the whole distribution.
6. **Synopsis construction (§5.1, §5.2, §7).**
   - Keep the projection of the filtered tuples onto the bounded attributes. For bags, store one count per distinct projected tuple. Its size is bounded by the product of the bounded domains.
   - Add one extreme value per bucket for the MaxRef or MinRef attribute.
   - The memory bound is therefore governed by the **sizes of the bounded domains**.
   - A polynomial checking algorithm (§6, Fig. 2) examines subsets of at most 4 attributes.
7. **Aggregation (§8.2).** Uses the Gray et al. classification:
   - *distributive*: SUM, COUNT, MIN, MAX;
   - *algebraic*: AVG, which has a fixed-size synopsis g (here sum and count);
   - *holistic*: COUNT-DISTINCT, MEDIAN.

   Distributive and algebraic aggregates run online in bounded memory. Holistic ones cannot.
   - **Theorem 8.5, single stream, standard aggregates.** Bounded iff every grouping attribute is bounded and no holistic aggregate is applied to an unbounded attribute. A holistic aggregate over a bounded attribute is fine: keep one count per value.
   - MIN, MAX and COUNT-DISTINCT are duplicate-insensitive. SUM, COUNT, AVG and MEDIAN are duplicate-sensitive.
   - **Theorem 8.6** gives the multi-stream case. SUM, COUNT and AVG can be pushed below the join, so they can aggregate an *unbounded* attribute. MAX(A) over an unbounded A is allowed only if A is the single MaxRef attribute.
8. **Sliding windows (§1.3).** A row-based window (the last N tuples) trivially needs only a bounded synopsis. A time-based window does *not*, because any number of tuples can arrive in a finite interval.

**How it maps to our builtins.**

- **Every one of our lists is finite.** Capacity is 61,440, schema lists have `maxItems`, `take n` is n. So in Arasu's sense *everything* we do is trivially bounded. Their theorem is useful here for a different reason (ours): it shows **which operators inherently need Θ(n) state in a single pass**. Those are exactly the places where we must choose between buffering (RAM) and replaying the source (CPU).
- **Their "read once" assumption does not hold for us (ours).** We can re-read `gps` from flash. Their unbounded cases therefore split into three options:
  - (a) materialise Θ(n) in RAM;
  - (b) replay, at extra passes or quadratic CPU, in O(1) RAM;
  - (c) carry a synopsis in `state` across ticks.
- **Our `state` is their synopsis (ours).**
  - An engine that keeps `state.maxAlt = max(state.maxAlt, last(gps).alt)` is their per-tuple bounded-memory evaluation, with the 1 Hz tick as the arrival.
  - Schema `maxItems` and fixed-size accumulators force engines into the distributive/algebraic class *for state carried across ticks*.
  - Anything holistic over an unbounded attribute cannot live in state. It must be recomputed from `gps` each tick.

**Builtin by builtin** (the Arasu classification is read; the "for us" column is ours):

| our pattern | Arasu class | single-pass memory | for us |
|---|---|---|---|
| `filter`, `map` (no `#index`), `count`, `sum`, `mean`, `min`, `max`, `first`, `find`, `findIndex`, `reduce` with a fixed-size accumulator | selection/projection, distributive/algebraic aggregates | O(1) | Fuse into one loop. O(1) RAM, one pass. A fixed-size `reduce` accumulator **is** Gray's "synopsis g", so our rule matches the algebraic class exactly. |
| `last`, `gps[-k:]` | row-based window | O(k) on a stream | O(1) on `gps`, because the length is known and access is random. On a *derived* recipe (`filter(gps,…)[-k:]`) the length is unknown. Options: two passes (count, then skip), a k-slot ring buffer, or iterating the source *in reverse* and taking k (filter and map commute with reverse; an order-insensitive consumer then needs O(1)). |
| `sum(map(xs[1:], distance(xs[#index], #)))` | self-join of xs with itself on position i and i−1, i.e. a row window of 2 | O(1) | O(1) when `xs` is randomly indexable (`gps`, schema lists). If `xs` is a filtered recipe, `xs[#index]` is **not** O(1). Compile constant offsets (`#index - k`) to a k-slot lag buffer. Non-constant indexing into a recipe costs O(n²) CPU through rescans, or materialisation. |
| `uniq(xs)`, `count(uniq(xs))` | duplicate elimination / COUNT-DISTINCT (holistic) | bounded **iff** the value domain is bounded (Thm 5.10 C1, Thm 8.5) | If the value domain is statically bounded (schema `enum`, an integer `minimum`/`maximum`, or a filter predicate that brackets the value; note this is a *value-domain* bound, not a length bound), use a bitmap or count array of domain size. Otherwise choose O(n) RAM (hash/sort), or O(1) RAM with O(n²) CPU (for each i, scan j<i). At n = 61,440 the quadratic option is ~1.9·10⁹ comparisons, which is not feasible at 1 Hz. If the input is **sorted** on the key: O(1) RAM, one pass. |
| `median(xs)` | holistic | bounded only over a bounded domain (Thm 8.5) | Not single-pass-bounded. Because the source is replayable, there is an O(1)-RAM exact option (ours, not from these papers): binary-search the value domain, counting `# ≤ pivot` on each pass. That costs ~32 passes for a 32-bit fixed-point key, i.e. ~2·10⁶ record reads at n = 61,440. Alternatively, approximate it in O(log n) RAM (Greenwald–Khanna, cited by Arasu, not read). |
| `sortBy(xs)` as a list output | not in their class (non-monotone). The blocking "sort" operator (see §2). | Θ(n) | Θ(n) if materialised. Even a u16 index permutation of full `gps` is ~120 KB, which is almost certainly over budget. Replaying via repeated selection is O(n²) CPU. Must be fused: `sortBy\|first` → min-by (O(1)); `sortBy\|take(k)` → top-k (O(k)). Otherwise reject at full `gps` length. |
| `any(xs, any(ys, #x.b < #y.d))` style existence across two lists | inequality join, duplicate-eliminating, one MaxRef/MinRef per side (Thm 5.10) | O(1): keep min(xs.b) and max(ys.d) | A **rewrite** taken directly from their synopsis: existence plus an inequality join reduces to `min(xs.b) < max(ys.d)`. That is O(n+m) CPU instead of O(n·m), and O(1) RAM. A counting version (duplicate-preserving) has no such shortcut (Thm 5.6 C3). |
| nested `map`/`filter` over two sources (join) | equi- or inequality join on unbounded attributes: unbounded | Θ(n) | Nested loops over replayable sources: O(1) RAM, O(n·m) CPU. Over two full-length `gps` scans that is ~3.8·10⁹, which is infeasible. A CPU bound is needed. |
| time windows: `filter(gps, #.t > timestamp - 60)` | time-based window, **not** bounded in their model | unbounded | `gps` is sorted by t, so binary-search to a slice: O(log n) CPU, O(1) RAM. The **length bound** of the result is the capacity (61,440) unless we have a **rate invariant** on the source (e.g. at most 1 fix per second, strictly increasing integer t). Then the bound is ~60. Their §1.3 point applies directly: a time predicate alone does not bound a length. |

**What to borrow.**
- **Gray's distributive / algebraic / holistic split**, as the static classification of every aggregate builtin and every `reduce`.
- **Duplicate-sensitive vs duplicate-insensitive**, which decides whether an existence-only consumer (`any`, `find`, `first`, `min`, `max`) allows a min/max synopsis in place of counts.
- **Value-domain boundedness** as a second analysis alongside length bounds. Borrow both their predicate-closure test (Defs. 4.1–4.5) and the generalisation in §9, where an attribute is bounded if the set of values it can take under P is a constant.

  This is what makes `uniq`, grouping, and holistic aggregates over enums or small integer ranges statically bounded.
- **The dichotomy as a diagnostic**: if an operator is not in the bounded class, the compiler should say which option it chose (materialise, replay, or reject), and at what cost.

**Where it contradicts or weakens our current thinking.**
- **Replay is not free (ours, motivated by the paper).** Replay turns their Θ(n) memory into extra *passes*. Several of our builtins (uniq, sortBy as a list, joins, `#index` into recipes) then become **O(n²) CPU**, which is infeasible at n = 61,440 within a 1 s tick. A memory-only static check would accept engines that never finish. We need a static **pass and cost bound** as well.
- **"Length-bound inference" is not enough.** `uniq` and holistic aggregates are bounded by **value-domain** size, not list length. Time filters bound nothing without a rate invariant.
- Their memory model counts disk. Ours counts only RAM. So `gps` itself is "free" storage that their model would count as Θ(n). This is the main reason their "unbounded" verdicts are often affordable for us.

---

### 2. Babcock, Babu, Datar, Motwani, Widom: models and issues in data stream systems

**Citation.** B. Babcock, S. Babu, M. Datar, R. Motwani, J. Widom. "Models and Issues in Data Stream Systems." *Proc. PODS 2002*, pp. 1–16. Page range is from the TODS reference list.
- Opened: https://www.cs.princeton.edu/courses/archive/spr04/cos598B/bib/BabcockBDMW.pdf (author preprint; I read pp. 1–10).

**What it establishes (read).**
- **§4.5, blocking operators.** A blocking operator cannot produce its first output tuple until it has seen all of its input. Sorting is the standard example, and so are SUM, COUNT, MIN, MAX and AVG.
- **Root vs interior.** A blocking operator *at the root* that produces one value (or a few) is manageable: update the answer as data arrives. *Interior* blocking operators are the real problem, because their consumers cannot act on intermediate results that may still change.
- **Mitigations.** Non-blocking approximations (the "juggle" partial-sort operator), and *punctuations*: in-stream assertions such as "all future daynumber ≥ 10", which let a group-by emit results and a join discard state.
  - The authors leave open how punctuations relate formally to memory bounds.
- **§4.3, sliding windows.** Presented both as an approximation and as intended semantics: well defined, deterministic, and favouring recent data.
- **§4.1** summarises Arasu et al.: without domain restrictions, joins need memory proportional to the input.
- **§4.4.** A synopsis structure is framed as `update(tuple)` plus `computeAnswer()`. That is exactly our state-carrying engine.

**How it maps.**
- **Root vs interior (ours).** Our aggregates (`sum`, `max`, `median`, …) at the *root* of a sub-expression produce a scalar, which matches the manageable root case.
- **`sortBy` and `uniq` are the interior blocking operators** whenever their *list* output feeds another list operator. This is exactly the set our fusion rules must eliminate (`sortBy|first`, `sortBy|take(k)`). Anything left over must materialise.
- **Sortedness as punctuation (ours).** "`gps` is sorted by t" is a standing punctuation. It lets us:
  - turn time filters into slices;
  - stream `uniq` and group-by on t-monotone keys;
  - stop a `find`/`filter` scan early once t passes a bound.

**Borrow.**
- A sortedness / monotonicity property on recipes. `filter` preserves it. `map` with a monotone function preserves it. `take` and slices preserve it.
- Use that property to make "blocking" operators streaming.

**Weakens.** Nothing in our current thinking. It confirms that fusion must target interior blocking operators specifically.

---

### 3. Datar, Gionis, Indyk, Motwani: maintaining stream statistics over sliding windows

**Citation.** M. Datar, A. Gionis, P. Indyk, R. Motwani. "Maintaining Stream Statistics over Sliding Windows." *SIAM Journal on Computing* 31(6):1794–1813, 2002. A preliminary version appeared at SODA 2002.
- Opened: https://moodle2.units.it/pluginfile.php/718390/mod_resource/content/0/Stream_statistics_sliding_window.pdf. This is a course-hosted copy of the SIAM published version. I read pp. 1794–1799 and 1809–1813.

**What it establishes (read).**
- **Model.** Sliding window over the last N elements. Once an element arrives it cannot be re-read. Space is measured in bits.
- **BasicCounting** (number of 1s in the last N bits): "an exact solution requires Θ(N) bits of memory." An ε-approximation (the Exponential Histogram) uses O((1/ε) log² N) bits, and there is a **matching lower bound** Ω((1/ε) log² N) for deterministic and randomized algorithms.
- **Sum** of positive integers in [0, R]: matching bounds via a generalised EH.
- **Min/max over a window (§7.4).**
  - Lower bound: log C(N+R−1, N) ≥ N log(R/N) bits. With R polynomial in N, you must store essentially the whole window.
  - The exact algorithm keeps a list of (value, timestamp) pairs, increasing in both: O(N log R) worst case, O(log N · log R) expected for random arrival order.
- **Distinct values (§7.5).**
  - Approximate: a Flajolet–Martin bitmap with per-bit timestamps, O(m log² N) bits.
  - The BasicCounting lower bound transfers to distinct counting.
  - **With deletions**, even a factor-2 approximation needs Ω(N) bits (Claim 1).
- **Sum with positive and negative values (§7.6):** even a constant-factor approximation needs Θ(N) (Theorem 8).

**How it maps.**
- **Their lower bounds all rest on one assumption: the expiring element cannot be re-read.** For `gps` it can, because `gps[-N-1]` is in flash. Two consequences (ours):
  - **Recompute each tick:** `sum(gps[-N:])`, `max(gps[-N:])`, `median(gps[-N:])` are exact with O(1) RAM and O(N) CPU per tick. They need no window state.
  - **Incremental via state, for invertible aggregates (sum, count, mean):** `state.s = state.s + last(gps).x - gps[-N-1].x`. This gives O(1) RAM *and* O(1) CPU per tick, exactly, which beats DGIM's Θ(N) bound because the evicted value is free to read.

    Caveat (ours): with no FPU, use integer or fixed-point accumulators. Float add-then-subtract drifts, and the soft-float cost is real.
  - **Non-invertible aggregates (min, max, median, distinct):** cannot be maintained in O(1) state over a window. Per DGIM §7.4, min/max needs Θ(N)-ish state. Recompute from `gps` each tick instead: O(N) CPU, O(1) RAM (min/max) or O(passes · N) for median (see §1).
- **Exact vs approximate.** Approximate distinct or median sketches buy O(polylog) memory *and* O(n) CPU. Exact methods on a replayable source buy O(1) RAM at extra passes. For us, the choice is a language decision: do we offer an `approxUniq`/`approxMedian` at all? The papers show that the approximate versions are the only single-pass bounded options.

**Borrow.**
- The **invertible / non-invertible** split (a group vs a mere monoid) as a property of aggregates. This decides whether a windowed engine may be incremental through state.
- Expose the "subtract the expiring element" pattern as an idiom, or as a compiler rewrite.

**Weakens.** "Lists are lazy, restartable recipes; replay instead of buffering" is right for RAM. But DGIM's bounds tell us that **the window state we would otherwise keep is exactly the history `gps` already holds**. So the right default is to *recompute* from `gps` each tick (CPU O(N)), except for invertible aggregates, which can be O(1)/O(1) through `state`.

---

### 4. Polars and DuckDB: which operators break pipelines

**Polars (read, from source).**
- Opened: https://docs.pola.rs/user-guide/concepts/streaming. The user guide only says some operations are non-streaming and fall back to the in-memory engine, and that the plotted plan has a memory-intensity legend.
- Source files read:
  - https://raw.githubusercontent.com/pola-rs/polars/main/crates/polars-stream/src/physical_plan/fmt.rs
  - https://raw.githubusercontent.com/pola-rs/polars/main/crates/polars-stream/src/physical_plan/mod.rs
  - https://raw.githubusercontent.com/pola-rs/polars/main/crates/polars-expr/src/reduce/convert.rs
- These are the `main` branch as of 2026-10-12. I read them through a fetch tool that summarises pages, so treat exact names as near-verbatim.

What the source shows:
- **`NodeStyle` in `fmt.rs`** puts each physical node into one of three classes:
  - **"potentially memory-intensive node"**: InMemorySource, InputIndependentSelect, **NegativeSlice**, InMemorySink, **Sort**, **GroupBy**, EquiJoin, SemiAntiJoin, CrossJoin, Multiplexer, Window, **Gather**, RangeJoin, MergeSorted;
  - **"in-memory engine fallback"**: InMemoryMap, InMemoryJoin, ColumnarFunction;
  - **generic streaming node**: everything else, including **Filter, Select, Reduce, StreamingSlice, TopK**.
- **Physical nodes** include `TopK { k, by_column, … }` and `Sort { …, slice: Option<(i64, usize)> }`, so sort and slice are fused. They also include `SortedUnique`, `SortedGroupBy`, `PeakMinMax`, `CumAgg` and `RollingFixedWindowFunction`.
- **Streaming reductions** (`convert.rs`):
  - supported: Sum, Mean, Min, Max, Var, Std, First, Last, Count, Len, **MinBy/MaxBy**, **ApproxNUnique**, **ApproxQuantileSketch**, and others;
  - **`Median` and `NUnique` are `todo!()`**, so they are not streaming reductions.

**Mapping to our builtins (ours).**

| ours | Polars counterpart | Polars class |
|---|---|---|
| `filter`, `map`, `take`/positive slice | Filter, Select, StreamingSlice | streaming |
| `sum`, `count`, `mean`, `min`, `max`, `first`, `last` | Reduce | streaming |
| `sortBy\|first` (min-by) | MaxBy/MinBy reduction | streaming |
| `sortBy\|take(k)` | TopK, or Sort with fused slice | streaming (O(k)) |
| `sortBy` | Sort | memory-intensive |
| `uniq` | GroupBy/unique, or **SortedUnique** if sorted | memory-intensive, or streaming if sorted |
| `median` | Median (not a streaming reduction) | falls back / memory-intensive; only the approximate quantile streams |
| `gps[-30:]` | **NegativeSlice** | **memory-intensive**, because a stream has unknown length. Ours is O(1) on `gps`: known length plus random access. |
| `xs[#index]` random access | Gather | memory-intensive |

**DuckDB (read).**
- Opened: https://duckdb.org/docs/current/guides/performance/how_to_tune_workloads
  - Blocking operators: GROUP BY, JOIN, ORDER BY, and window functions. All of them support larger-than-memory processing by spilling.
  - `list()`, `string_agg()` and sort-based ("holistic") aggregates cannot spill and may run out of memory.
- Opened: https://raw.githubusercontent.com/duckdb/duckdb/main/src/optimizer/topn_optimizer.cpp (read through the same summarising fetch tool)
  - Rewrites LIMIT over ORDER BY into a TopN operator. Requires a constant limit and an absent or constant offset.
  - **Skips the rewrite on cost grounds** when the limit is above 5,000 rows *and* above 0.7% of the estimated child cardinality, in which case a full sort is cheaper.
  - Also pushes a dynamic filter from TopN into scans.

**Mapping.**
- DuckDB's breaker set (sort, grouping, join, window, holistic aggregates) is the same as Polars'. It is also the same as Arasu's unbounded set and Babcock's blocking set.
- Four independent sources agree on our breakers: **`sortBy` (unfused), `uniq`, `median`, nested-source joins, and `#index` gathers into non-indexable lists**.
- **DuckDB's TopN threshold is a CPU heuristic.** For large k, a heap of k is slower than sorting. In our case memory comes first, so: top-k whenever k is statically bounded and k × record size fits the budget. When k is near n, top-k needs about as much RAM as a sort. Then the only O(1)-RAM option is multi-pass selection (find the k-th key by binary search over values, then filter), at k-independent extra passes.

**Borrow.**
- A **three-way node classification** in our compiler: streaming / bounded-buffer(k) / breaker (materialise or replay).
- **Sortedness-specialised operators** (Polars SortedUnique, SortedGroupBy).
- **Dynamic-filter pushdown from top-k into the scan:** once the heap is full, skip records worse than the current k-th. This is a CPU win.

---

### 5. Incremental view maintenance: DBSP and differential dataflow

**DBSP.**
- Citation: M. Budiu, T. Chajed, F. McSherry, L. Ryzhyk, V. Tannen. "DBSP: Automatic Incremental View Maintenance for Rich Query Languages." *PVLDB* 16(7):1601–1614, 2023. doi:10.14778/3587136.3587137.
- Opened: https://www.vldb.org/pvldb/vol16/p1601-budiu.pdf. I read only the first page, to confirm the citation.
- The technical content below I read in the arXiv version, https://arxiv.org/abs/2203.16684 (v1, March 2022, pp. 1–8 and 11–13). Its wording may differ from the VLDB version.

What it establishes (read, arXiv v1):
- **The incremental version of a query is `Q^Δ = D ∘ Q ∘ I`**: integrate the changes, apply Q, then differentiate. The chain rule lets you incrementalise operator by operator.
- **Linear operators** (filter, projection, map, sum/count as linear aggregates) satisfy `Q^Δ = Q` and keep **zero state**.
- **Bilinear join** costs O(|ΔA|·|ΔB|)-ish work but **O(|R|) memory**: it keeps the integrated inputs.
- **Incremental `distinct`** takes O(|Δ|) time but **O(|R|) memory** (§4.5). The authors explicitly try to minimise how many distinct operators they use, because of the memory cost.
- **MIN/MAX are not incremental under deletions.** They are implemented by brute force: recompute over the integrated input, O(|R|) work per step (§7.2).
- **ORDER BY** is not efficiently incrementalisable in DBSP. It is left as future work (§7.2).
- **Fixed-size sliding-window aggregates** are expressed as a chain of delays, a convolution (§7.6.1).

**Differential dataflow.**
- Citation: F. McSherry, D. G. Murray, R. Isaacs, M. Isard. "Differential dataflow." *Proc. CIDR 2013*.
- Opened: https://www.cidrdb.org/cidr2013/Papers/CIDR13_Paper111.pdf (pp. 1–7).

What it establishes (read):
- Operators keep their **full input difference traces**, indexed by key, then version, then record. These are needed to reconstruct a collection at any version. The traces grow without bound unless they are consolidated once no earlier versions can arrive (§3.4).
- **Select, Where, Concat and Except are linear**: they are pipelined and **keep no state** (§4.3).
- Join, GroupBy and the aggregates Count, Sum and Min are keyed, and do work only on keys present in the change.
- **Space vs CPU in their own measurement:** the trace was only 1.5% bigger than the label set in their Twitter example, and the update work was 0.003% of a full prioritised re-evaluation. They trade **memory for compute**.

**How it maps (ours).**
- Our tick is "re-run the same query after one append". That is IVM with an insert-only change of size 1 on `gps`.
  - Linear parts (`filter`, `map`, `sum`, `count`) can be incrementalised through `state` with O(1) state.
  - Under insert-only changes, MIN/MAX are also O(1)-state incremental. DBSP's problem with them is deletions only.
  - Sliding windows *are* deletions. DBSP's brute-force answer (recompute over the window) is what we get for free from `gps`.
- `distinct`/`uniq`, joins and sort need **O(|R|) state** to be incremental, and that state does not fit under `maxItems`-bounded schemas. Recompute them instead.
- **Full DD/DBSP-style automatic incrementalisation goes the wrong way for us.** It buys CPU with RAM, and we hold RAM fixed and spend CPU.
  - The useful part is the *classification*: linear means zero-state incremental; distinct, join, sort and holistic aggregates mean state proportional to the data.
  - If we ever auto-incrementalise, do it only for operators whose incremental state is O(1): linear, distributive or algebraic, over an append-only source.

---

### Implications for us

1. **Classify every aggregate at compile time as distributive / algebraic / holistic (Gray, via Arasu §8.2), and as invertible or not (DGIM).**
   - Distributive and algebraic: fused single loop, O(1) RAM.
   - Holistic (`median`, `uniq`/count-distinct, unfused `sortBy`): needs a stated strategy.

   Our "reduce accumulators must be fixed-size" rule *is* the algebraic class. This validates it.
2. **Add value-domain bounds alongside length bounds.** Holistic operators are bounded by *domain* size (Arasu Thms 5.10 and 8.5), not by list length. Infer domains from schema `enum`, integer `minimum`/`maximum`, and bracketing filter predicates. Over a bounded domain, `uniq` and `median` become count arrays.
3. **Current thinking is incomplete: replay trades RAM for passes, and some replays are quadratic.**
   - Joins between two full-length sources, `uniq` without a domain bound or sortedness, `sortBy` materialised by repeated selection, and non-constant `#index` into a filtered recipe all become ~10⁹–10¹⁰ operations at n = 61,440.
   - The static check must bound **memory (hard) and passes/CPU (soft, but must reject the infeasible)**. A memory-only check would accept engines that cannot finish a tick.
4. **Track sortedness/monotonicity on recipes.**
   - `gps` is sorted by t. `filter`, slices, `take` and monotone `map` preserve sortedness.
   - With it: time filters become binary-search slices; `uniq` and group-by on sorted keys stream in O(1) (Polars SortedUnique and SortedGroupBy); scans can stop early.
5. **Time filters do not bound length without a rate invariant on `gps`** (Arasu §1.3). Decide and document an invariant such as "t strictly increasing, at most 1 fix/s". Otherwise `filter(gps, #.t > ts-60)` has a length bound of 61,440.
6. **Sliding windows: recompute from `gps`, don't keep window state.**
   - DGIM's Θ(N) lower bounds assume the expired element cannot be re-read. Ours can.
   - Non-invertible aggregates (min, max, median, distinct) over `gps[-N:]`: O(1) RAM, O(N) CPU per tick.
   - Invertible aggregates (sum, count, mean): can be O(1) RAM and O(1) CPU through `state` (add the new value, subtract `gps[-N-1]`). Use integer or fixed-point, because there is no FPU and floats drift.
7. **Negative slices and `last` on *derived* recipes are breakers** (Polars NegativeSlice), even though they are O(1) on `gps` itself. Compile them as reverse iteration of the source plus take(k): O(1) for order-insensitive consumers, a k-buffer otherwise.
8. **`#index` must be analysed by offset.**
   - Constant offset into an indexable source (`gps`, schema lists): free.
   - Constant offset into a recipe: a k-slot lag buffer.
   - Non-constant offset into a recipe: materialise or reject. Polars treats a gather as memory-intensive.
9. **Fusion targets are confirmed by four independent sources.** The breakers in Arasu, Babcock, Polars and DuckDB are the same: sort, distinct, holistic aggregates, joins and gathers.
   - Add **existence + inequality → min/max** (Arasu Thm 5.10), e.g. `any(xs, any(ys, #x.b < #y.d))` → `min(xs.b) < max(ys.d)`.
   - Add **top-k with a skip-worse filter** (DuckDB dynamic filter).
   - When k approaches n, top-k is no longer a memory win. Fall back to multi-pass selection, or reject.
10. **Exact vs approximate is a language decision.** The only single-pass bounded-memory options for distinct and quantiles are sketches (DGIM §7.5, Polars ApproxNUnique and ApproxQuantileSketch). On a replayable source, exact `median` has an O(1)-RAM multi-pass algorithm (ours: binary search on value, ~32 passes). Exact `uniq` without a domain bound or sortedness has no cheap O(1)-RAM option.
11. **Don't adopt DBSP/DD-style general incrementalisation.** It stores O(|R|) state for distinct, join and sort, which is the opposite of our constraint. Incrementalise only through author-written `state`, and only for O(1)-state (linear, algebraic, or invertible-window) computations.
