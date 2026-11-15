# Wave-8 Pinned-Lane vs Heap Recursion Benchmark — SNlang SCC accumulators

**Agent:** Impl-K, Cohort Delta (benchmarkers), Wave 8 · 2026-11-15
**Feature measured:** the pinned-lane coloring, commit `e1f3b72` on `rabbit-hole-impl`
("check+ir: pinned-lane coloring + the real lane feed to IR" — the checker pre-colors a
recursive inout-grow SCC accumulator's allocation site onto a pinned lane L2..L6; the IR
constructs the growable's header ON the lane, `sn_list_new_lane` → `sn_lane_alloc(π_j)`;
compiler/tests/golden.rs:414 `scc_accum_pinned_lane_ok`).
**Machine:** Apple M4, macOS 27.2.1 (darwin27, arm64), 16 GB · single-threaded
**Toolchain:** snc 0.2.0 (worktree `7400d35`, includes `e1f3b72`) · Apple clang `-O2` for
both the `.ll` link (the snc default) and the C twin
**Protocol:** CHAINS = 200 rounds, one warmup round-set at CHAINS/10, **median of 5 timed
runs**; every timed phase totals ≥ 44 ms (≥ 100× the .sn ms clock, per the W2F discipline);
each program prints its own clock-pair overhead (timer row). Raw runs:
`results/sn_pinned_lane_bench.txt`, `results/c_pinned_lane_bench.txt` (committed).
Re-run: `PIN_ONLY=1 ./run_bench.sh` (env `PIN_CHAINS`, `RUNS`).

## 1 · The shape, and what is held fixed

Each round builds ONE `list<int>` of **N = 100,000 elements** pushed by a recursive inout
accumulator across a **real depth-D = 100 call chain** (1,000 pushes per level; values
`level*1000+k` = the exact set {0..N−1}). The two backings:

- **pinned-lane** — `list<int> acc = []` whose site the compiler pre-colored onto pinned
  lane L2: header = 48 B lane bump memory (`sn_list_new_lane`), reclaim at chain end =
  `sn_lane_restore(2, wm)` — **one store**.
- **heap** — the default `sn_list_new` path: 48 B `xmalloc` header + `gc_track`, reclaim =
  `sn_release` (rc → `gc_untrack` + `free(items)` + `free(header)`); plus the collector
  alternative: one precise-root pass (`sn_gc_collect_roots` — the path the `gc_collect()`
  intrinsic lowers to) over all the dead lists.

Held fixed across the two variants: the identical recursive `grow` (element pushes through
`sn_list_push_i64`, the shared `list_grow`, and the **E-NESTED pool-backed element
storage** — a fresh 512 KB slab window per list, one 512 KB→1 MB growth `memcpy` per chain
paid by both at N > 65,536), the same values/checksums, single-threaded, GC threshold
pinned to 2×10⁹ (the `sn_heap_bench.sn` control), same machine, median of 5.

**The recursion is real in both binaries — verified in the disassembly (exactly one
self-call in each).** Two naive forms were tried and rejected first: a void tail recursion
is TCO'd by clang -O2 into a loop, and even `return f(n−1) + 1` is rewritten into an
accumulator (0 self-calls, measured). The shipped shape reads a per-level local *after*
the recursive call, which defeats the accumulator rewrite — and doubles as the depth
checksum (`Σ level = 4,950` per chain). The C twin is `bench/pinned_lane_bench.c` linked
against `compiler/runtime.c` (the same runtime snc links); the .sn twin is
`bench/pinned_lane_bench.sn`, lowered by the actual compiler — its `pin_build` rounds run
the full check → color (`sn_list_new_lane(0,2)` in the IR) → lower → run loop.

## 2 · Results (median of 5; ns/elem = per element of the 20,000,000 pushed per row set)

| Row | .sn in-language (ms clock) | C twin, runtime API (ns clock) |
|---|---:|---:|
| timer (clock-pair overhead) | 10 ns | 17 ns |
| **alloc**: construct header + recursive push of N=100K (per element) | **2** (45 ms/200 chains = 225 µs/chain) | pin **2** / heap **2** (219.4 / 226.5 µs/chain) |
| **read pass** (8 B load/record, sum) | split = 3 − 2 = **1** | pin **0** (<0.05) / heap **0** |
| **reclaim per chain** (the key number) | 0 ms — sub-clock; bounded by pin_resetpair | **pin 5 ns** vs **heap 240 ns** |
| reclaim, batched (one GC pass over 200 dead lists) | heap_gc: 0 ms — sub-clock; 254/257 objs swept, deterministic | **24 µs total = 120 ns/list** (200/200 swept) |
| pin_resetpair (1M save+restore pairs, lane 2) | 4 ns/pair | 4 ns/pair |
| header lifecycle, empty list (1M cycles: construct+reclaim) | — | **pin 17 ns** vs heap **34 ns** |
| protocol checksums (5/5 runs, both binaries) | len 20,000,000 · sum 999,990,000,000 · depth 990,000 | identical |
| lane 2 actually holds the header | `lane_used(2) = 48` | `sn_lane_used(2) = 48` |
| peak RSS (process, chains=200) | 737,056 KB | 614,464 KB |

Raw per-run spreads: pin reclaim {0, 5, 5, 5, 10} ns; heap reclaim {185, 220, 240, 285, 305} ns;
heap_gc {11, 12, 24, 26, 43} µs; pin/heap alloc 2 ns/elem in every run of both binaries.

## 3 · The key numbers

1. **The O(1) reclaim is real and measured: 5 ns vs 240 ns per chain — 48× at N=100K.**
   The pinned lane's exit path is one `sn_lane_restore(2, wm)` = a single top store plus
   the call pair (~4 ns, the pin_resetpair row at 1M repetitions) — **independent of N**:
   it touches no element data. The heap exit pays `gc_untrack` (a **linear scan of the
   tracked-object table** — ~200 ns at 200 tracked objects) + `free` of the N-sized items
   array (N-dependent) + `free` of the header. Amortized per element: 0.00005 vs
   0.0024 ns — and the gap **widens with N** (the heap side carries the growing free).
2. **Allocation is a per-element wash — by design.** 2 ns/element in *both* backings,
   in-language and in C (the .sn 225 µs/chain matches the C twin's 219 µs/chain to 3%):
   the elements are pool-backed in both (W2A §1.5 E-NESTED), so the pinned-lane feature
   buys nothing per push. The construct difference is **per header**: the empty-list
   lifecycle row isolates it at **17 vs 34 ns (2.0×)** — lane bump + restore vs
   xmalloc + `gc_track` + `gc_untrack` + free.
3. **The GC pass alternative: 24 µs for 200 lists = 120 ns/list** — cheaper per list than
   240 individual releases, because the sweep frees without the per-object untrack scan
   (one table compaction instead of 200 linear-scan untracks). Still **24× the lane
   restore**, and it is a batch cost: the collector must run over *all* tracked objects,
   and the mark walk is O(len) only for ptr-typed elements (int lists mark O(1)/list).
4. **Cross-language protocol ✓:** identical checksums in 5/5 runs of each binary and
   across binaries (`len_total=20,000,000`, `sum_total=999,990,000,000`,
   `depth_total=990,000` = 200 × 4,950); `lane_used(2)=48` (the header) in both;
   GC counters deterministic (C: 200/200 swept; .sn: 254 freed / 3 survivors — the args
   list + its strings; the .sn row also sweeps the report machinery's garbage strings).

## 4 · The honest caveats

- **The O(1) restore does NOT return the element storage.** `sn_list_new_lane`'s items
  array stays malloc/slab-backed by design (E-NESTED), and nothing on the lane-reset path
  frees it: each chain's 1 MB grown array (outgrowing its 512 KB slab window at
  N = 100,000 > 65,536) is **leaked** by the pinned variant, while the heap variant's
  `sn_release` frees it. Measured: 200 chains → ~300 MB of abandoned windows+arrays on
  the lane path; process peaks 737 MB (.sn) / 614 MB (C) — both binaries run both
  variants, so the delta vs a heap-only process is the ~200 MB of leaked grown arrays.
  **The mandate's O(1) reclaim is confirmed for the header; element-pool reclamation is
  the open gap this measurement exposes** (a per-lane items-pool, or header-at-restore
  epoch sweep, would close it).
  **→ CLOSED in Wave 12 (Impl-Q):** the lane-aware items pool (a per-lane registry of
  lane-backed list headers; `sn_lane_restore` frees the grown items array of every
  header the rewind covers — the §6 re-run supersedes this caveat's numbers). Only the
  abandoned slab *windows'* touched pages stay resident by design: windows are carved
  from the process-lifetime 256 MB slab, never freed.
- **The mandate's "N frees" is not what this heap does.** SNlang lists pool their
  elements: a heap list's death is 1 untrack + 2 frees — not 100,000 frees. The
  per-object-free shape (one `new`-blueprint object per element, freed individually) was
  priced in Wave 5 (`RESULTS.md` §2: C 5 ns/free). The list-vs-list comparison here is
  therefore 240 ns vs 5 ns, not N×5 ns vs 5 ns.
- **The in-language heap twin cannot be the same recursive inout shape anymore:** the
  coloring pins *every* recursive inout-grow SCC accumulator — `examples/scc_accum_ok.sn`
  (the Wave-6 "heap twin") now also lowers to `sn_list_new_lane(0,2)` (probed on this
  worktree). The .sn `heap_build` row is therefore the default push-loop into `sn_list_new`
  (no recursion), and the C twin carries the identical recursive chain for both backings.
- **The heap reclaim's 240 ns is mostly the untrack tax at this table size** (a linear
  scan over ~200 tracked objects — runtime.c `gc_index_of`); at an empty table the
  heap lifecycle is 34 ns (heap_life). Both numbers are reported so the O(1) comparison
  is not flattered by an artificially large table.
- The .sn `heap_gc` row's object count includes the report machinery's garbage strings
  (254 freed = 200 lists + 54 strings); the C twin isolates the pure list pass (200/200).
- pin reclaim's per-run quantization (0–10 ns): the phase is 200 restores ≈ 1 µs measured
  with one ~17 ns clock pair; the pin_resetpair row (1M pairs, 4 ns median) is the
  higher-confidence bound, and the two agree.

## 5 · What the numbers mean (against the mandate's O(1)-reclaim claim)

1. **Confirmed, and now measured at the feature level:** the pinned-lane SCC accumulator's
   exit is one store — 5 ns at N=100K vs the heap's 240 ns — and unlike the heap side it
   does not grow with N. This is the same asymmetry Wave 5 measured for regions
   (4 ns resetpair vs 5 ns/object free loop), now landing through the compiler's
   automatic coloring rather than explicit `region {}`.
2. **The allocation side of the mandate does not apply to growables:** element storage was
   never per-object heap memory, so "heap→lane" is 2→2 ns/elem. The feature's alloc win is
   the header only (2.0× lifecycle) — the honest expectation for callers is *reclaim*
   (48×) plus the E0802 safety the coloring buys, not a Wave-5-style 4.7× alloc speedup.
3. **The leak is the actionable finding:** O(1) reclaim ≠ O(1) memory return. Until the
   items pool is lane-aware (or list headers carry an epoch the restore can sweep), a
   long-running program that cycles pinned-lane accumulators grows RSS linearly with
   chains — 200 MB per 200×100K-element rounds, measured.
4. **In-language measurement limits are documented, not hidden:** the .sn clock cannot
   resolve a 5 ns reclaim (it reads 0 ms); the C twin exists precisely to price it, and
   the two twins agree everywhere both can see (2 ns/elem alloc, 225 vs 219 µs/chain).

**Bottom line:** the pinned-lane coloring delivers the mandate's O(1) SCC-exit reclaim —
**5 ns vs 240 ns per N=100K chain (48×), N-independent vs N-carrying** — at zero measured
alloc regression (2 ns/elem both) and one real cost the swarm should schedule: the
element arrays the 1-store restore leaves behind (~1 MB leaked per 100K-element chain,
measured in RSS).

## 6 · Wave-12 addendum (Impl-Q): the lane-aware items pool — §4's first caveat CLOSED

**Fix (compiler/runtime.c, Wave 12, Impl-Q):** a per-lane TLS registry of lane-backed
list headers (`lane_grown_register` at `sn_list_new_lane`; `SN_TLS` layout frozen — the
registry is a parallel array, not a SnLane field). `sn_lane_restore` pops the registry
suffix covered by the rewind — registry order is bump order, so the rewound range
[watermark, top) is exactly the entries created after the watermark — and frees each
dead header's items array, gated on `!list_is_slab_backed`. **Reclaim invariant: a
lane-backed list's header dies at the rewind that covers it, and the same rewind frees
its grown items array** (no ref to lane memory can escape — E-GC-IN-LANE — so header
and array are unreachable at the rewind; the pop is O(#freed), never O(#live)).
Slab windows (lists under 65,536 elements) are NEVER freed — carved from the shared
256 MB slab, not malloc'd; heap-backed lists never enter the registry (sn_release/gc_free
own their arrays). Runtime checks: tests/systems/test_lane_items.c (20/20b, ASan-clean);
the direct probe: 32 grown chains at HEAD → +32,768 KB in the default malloc zone, fixed
→ +0 KB.

**Re-run:** same machine (M4), same protocol (`PIN_ONLY=1 PIN_CHAINS=200 RUNS=5`,
medians of 5; the runner's raw files refreshed in `results/`, Wave-8 originals kept as
`results/*_w8_implk.txt`). Checksums identical in 5/5 runs of each binary:
`len_total=20,000,000`, `sum_total=999,990,000,000`, `depth_total=990,000`,
`pin_lane_used_bytes=48`.

| Row | Wave 8 (committed) | Wave 12 (post-fix) | Honest reading |
|---|---:|---:|---|
| peak RSS .sn (chains=200) | 737,056 KB | **421,840 KB** | **−315,216 KB (−42.8%)**: the grown arrays reclaimed per rewind |
| peak RSS C twin | 614,464 KB | **424,176 KB** | **−190,288 KB (−31.0%)**: same |
| pin chain reclaim (N=100K grown) | 5 ns | **225 ns** {190,205,225,355,510} | the old 5 ns **never reclaimed the 1 MB array — it leaked it**; the honest reclaim pays its `free` (~200 ns), landing next to the heap's |
| heap chain reclaim | 240 ns | **170 ns** {160,170,170,180,245} | same free + untrack + header free; the 48× gap for *grown* lists was the leak's artifact |
| pin_resetpair (1M pairs, no lists) | 4 ns | **5 ns** (5/5 runs) | the registry's empty-registry check: **+1 ns per rewind** |
| pin_life (1M empty pinned lifecycles) | 17 ns | **26 ns** {25,26,26,27,31} | +9 ns: register append + suffix-pop + slab test per lifecycle |
| heap_life (control, untouched path) | 34 ns | **42 ns** {39,41,42,42,50} | moved without any code change on it — ±8 ns on the small rows is session drift |
| heap_gc (one pass, 200 lists) | 24–26 µs | **26 µs** | unchanged |
| alloc rows (pin/heap, both binaries) | 2 ns/elem | **2 ns/elem** | unchanged; the fix touches no push path |

**What changed in the story:** for lists *under* the 65,536-element window the reclaim is
still the 1-store O(1) rewind (5 ns, `pin_resetpair` +1 ns for the registry check; the
window is reused-not-freed by design). For lists that GROW past the window, the pinned
reclaim now costs what honest reclamation costs — the array free — and the 48× headline
applies only to the header + under-window case, no longer to grown chains whose memory
used to simply never come back. RSS at chains=200 stops growing at the grown-array half
of the leak; the abandoned slab windows' touched pages (~512 KB/list) stay resident by
slab design (a future madvise/decommit at the grow transition is the remaining, out-of-
scope half of the footprint).
