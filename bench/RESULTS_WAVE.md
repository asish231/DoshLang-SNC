# Wave-13 Wave-Region Benchmark — the J(W) batched fold vs the S1 fallback merge

**Agent:** Impl-R, Cohort Delta (implementers), Wave 13 · 2026-11-15
**Feature measured:** the wave region — Theorem WS made executable (W2B §4, the wave-seam composition theorem; the typed read-mode surface W2B §3). The wave body stages contributions into a per-cell buffer (`contrib`), and the block exit is J(W) — the J2 batched canonical fold: the staged multiset folds into one offer cell and joins the seam cell through ONE atomic ⊔ (Lemma NI(ii): "contributions ARE atomic ⊔-merges on seam cells"; Thm WS Part 2). This benchmark prices that discipline against the S1 fallback path it replaces — K direct `lcell_inflate` payload applications on a plain fallback cell — and prices the read modes (`read_W` vs the fallback read).
**Machine:** Apple M4, macOS 27.2.1 (darwin27, arm64), 16 GB · single-threaded
**Toolchain:** snc 0.2.0 (worktree at HEAD past fe6555d) · Apple clang `-O2` (the snc default `.ll` link)
**Protocol:** K = 100,000 payloads per round; WAVE_ROUNDS = 1000 (GSET rows × 1/2, read rows × 2) so every timed op phase totals ≥ 100 ms (≥ 100× the .sn ms clock, per the W2F discipline); one warmup at rounds/10, **median of 5 timed runs**; every row prints its own deterministic checksum. Raw runs: `results/sn_wave_bench.txt` (committed).
**Re-run:** `WAVE_ONLY=1 ./run_bench.sh` (env `WAVE_ROUNDS`, `RUNS`).

## 1 · The shape, and what is held fixed

Each row moves the SAME K payloads (values `0..K−1`, the deterministic checksum inputs) into a Tier-L cell through one of the two write paths the seam admits:

- **wave_max / s1_max** — the O(1) MAX carrier: K staged `contrib`s + the batched fold (the offer build = K one-compare inflations on a fresh ⊥ cell, then ONE merge) vs K direct `lcell_inflate`s on the fallback twin (one compare each). Steady state: the HWM saturates at K−1 after round 1; every later payload is a losing offer on both paths.
- **wave_gset / s1_gset** — the GSET carrier at its idempotent steady state (after round 1 every element is present; every later fold/inflate is probe-only — the S5 retry/re-merge dividend priced at its cheapest).
- **wave_readw / read_s** — the read modes: K `read_W` calls inside a read-only wave (slot lookup + the frozen-κ value readout) vs K fallback `lcell_value` reads. The read-only wave stages nothing — its join is the identity fold.

The S1 rows use their own module cells (never captured): the E-R1 merge-position rule forbids fallback merges into a *claimed* seam cell except program-order-before the wave, so the honest same-shape comparison runs on twins. The wave/S1 pairs' **checksums must be identical** — both paths reach the same folded state (the determinism claim, self-checked in the benchmark): `wave_max == s1_max == 99,999,000`, `wave_gset == s1_gset == 50,000,000`, `wave_readw == read_s == 19,999,800,000,000`.

GC threshold pinned to 2×10⁹ (out of every timed phase). Single-threaded — slice-1 is single-tile by design (see §4).

## 2 · Results (median of 5; ns per payload of the K moved per round)

| Row | ns/op | ops/run | phase_ms (median) | checksum (5/5 identical) |
|---|---:|---:|---:|---:|
| timer (clock-pair overhead) | 10 | 100,000 | 1 | wall-clock sum (varies by construction) |
| **wave_max** (staging + the J2 batched fold) | **3** | 100,000,000 | 333 | 99,999,000 |
| **s1_max** (K direct S1 inflates) | **1** | 100,000,000 | 100 | 99,999,000 |
| **wave_gset** (the batched fold, GSET steady state) | **34** | 50,000,000 | 1,721 | 50,000,000 |
| **s1_gset** (direct S1 inflates, same steady state) | **8** | 50,000,000 | 409 | 50,000,000 |
| **wave_readw** (the frozen-κ read_W in-wave) | **1** | 200,000,000 | 249 | 19,999,800,000,000 |
| **read_s** (the fallback value read) | **≤1** | 200,000,000 | 198 | 19,999,800,000,000 |

Raw per-run spreads: wave_max {3,3,3,3,3} · s1_max {0,0,1,1,1} · wave_gset {33,34,34,34,35} · s1_gset {8,8,8,8,8} · wave_readw {1,1,1,1,1} · read_s {0,0,0,1,1} · timer {10,10,10,10,10}.

## 3 · What the numbers say about the composition design

1. **The batched fold costs ~3× the direct S1 path on the O(1) carrier (3 vs 1 ns/payload)** — and the premium is exactly the discipline: per payload, the wave path pays one buffer append (the staging write) + one offer inflation (the batch's internal fold) + a share of the single atomic ⊔, where the S1 path pays one compare against the live cell. That is the price of NI(ii)'s event shape — *contributions are one atomic ⊔-merge per cell per wave*, never K interleaved merges — and of J(W) being a single join point the runtime can make atomic under the future parallel form.
2. **The GSET row shows where the premium really goes: 34 vs 8 ns/element.** The S1 steady state is probe-only (8 ns ≈ the lcell_bench `gset_merge_same` row's 7 ns — reproduced). The wave path's offer build is a *fresh* ⊥ cell, so the batch's internal fold pays the FULL insert pass (~23–25 ns/element, the `gset_merge_fresh` cost) before the one merge (probe-only against the seam). The batch is not free — it buys atomicity and order-independence, and the benchmark says so.
3. **The read modes are the same price: read_W ≈ read_S (1 vs ≤1 ns).** At slice-1's single-tile arity (one slot), the wave slot lookup is one pointer compare — the typed mode is free. The frozenness in slice-1 is *static* (the R1/E0812 discipline guarantees nothing changes the cell mid-wave), so read_W is the fallback read plus a mode check; the runtime *snapshot* that the parallel form needs is exactly what is NOT paid here — see §4.
4. **The checksum agreement is the determinism witness:** each wave/S1 pair moves 100M–200M payloads through two different paths and lands bit-identical folded states, five runs out of five. That is Thm WS Part 2's fold grade, executed.

## 4 · The honest caveats

- **Single-tile, single-threaded — by design, not by omission.** Slice-1's execution model runs the wave body ONCE, on the calling thread (the wave IS the parallelism in the model); there are no concurrent-fold contention rows because there is no concurrent fold yet. The parallel-tile form (spawn-per-tile + the live-arity barrier, the (sender,seq) k-way merge) reuses exactly this staging buffer + join as its payload interface, and the fold already ignores arrival order by construction (Part 2(i) is a law, not a schedule) — but its *runtime* costs (the snapshot the frozen κ needs once tiles race, the barrier, the fold's lock or lock-free join) are future rows, honestly unmeasured.
- **The timer row is the clock-pair measurement itself** (inherently sub-ms; its checksum sums wall-clock readings and varies by construction — the pinned-lane precedent). Every *op* phase is ≥ 100 ms and every *op* checksum is bit-identical across runs.
- **The `s1_max`/`read_s` rows at ≤1 ns/op sit at the clock's arithmetic floor** (phase_ms × 10⁶ / ops with phase_ms ≈ 100–250, ops = 10⁸–2×10⁸); treat "1" as "≤1.5" honestly, per the lcell_bench precedent.
- **The MAX rows measure the steady state** (the HWM saturates after round 1; every later payload is a losing offer — a compare on both paths). The first-contact fold is priced by the GSET rows, where the offer build pays the full insert pass every round.
- **The wave premium is measured against the SAME carrier shapes the seam admits** (the O(1) MAX and the GSET); an LWW row would sit between them (the offer is one lexicographic compare per payload) and is omitted for brevity — the two extremes bound it.
