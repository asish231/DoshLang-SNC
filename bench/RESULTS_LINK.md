# RESULTS_LINK — the self-relative link system's price rows

**SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P, Cohort Delta). Kills G1 §1 A2 → S4.**
Driver: `bench/run_bench.sh` (`LINK_ONLY=1`); raw runs in `bench/results/sn_link_bench.txt`.
Protocol (W2F, verbatim from RESULTS_GPOOL.md): 5 timed runs, median; one warmup
run at rounds/10 before timing; every row prints its own deterministic checksum,
and the checksums must match across all runs (decode-equality booleans and fixed
payload words only — never raw addresses, which vary per run); every phase
totals ≥ 100× the 1-ms `time_now_ms` clock.

- machine: `uname -sm` = `Darwin arm64` · `sysctl machdep.cpu.brand_string` = Apple M3
- `clang --version`: the build's first line (see the raw txt)
- N = 100,000 nodes per round (the brief's size), K = 10 passes per round,
  R = 200 rounds → 200,000,000 ops per phase per run (every phase
  393–988 ms — ≥ 100× the clock).

## The rows (median of 5 timed runs)

| row | ns/op | phase_ms | ops | checksum | what it prices |
|---|---|---|---|---|---|
| timer | 10 | 1 | 100,000 | (wall) | 100,000 `time_now_ms()` call pairs — the clock row |
| scan_bytes | 2 | 458 | 200,000,000 | 10 | `link_to_bytes(chain, 0, N)` — the O(N) sender-side self-containedness scan + the pack |
| raw_bytes | 2 | 424 | 200,000,000 | 10 | `link_to_bytes_raw(chain, 0, N)` — the pack WITHOUT the scan (the bench-only comparator) |
| reloc_range | 1 | 397 | 200,000,000 | 2000 | `relocate_range(src, dst, N)` — the constant-shift exemption: scan + memcpy + boundary re-encodes |
| reloc_node | 4 | 968 | 200,000,000 | 2000 | `N × relocate_node(nb, i, nb2, i)` — the per-node decode/copy/re-encode/round-check sequence |
| raw_copy | 3 | 651 | 200,000,000 | 2000 | `N × 2-word ptr copy nb → nb2` — the UNCHECKED by-value byte-copy the checker REJECTS |

## The honest claims

1. **The to_bytes scan overhead is sub-clock on this geometry** (scan_bytes 2
   ns/op vs raw_bytes 2 ns/op — the medians tie; the raw runs show the scan
   phase 433–492 ms vs the raw phase 406–479 ms, a ~4–7% phase-level
   difference). One decode + two comparisons per link over a chain of
   |δ| = 8 links is a predictable O(N) sender-side pass — the precondition
   that makes "100% legal bulk copy" TRUE instead of false by omission, priced
   here at a few percent of the pack itself.
2. **relocate_range vs per-node relocate_node — the constant-shift exemption's
   payoff** (1 ns/op vs 4 ns/op median; ~4× — 397 ms vs 968 ms per phase). The
   exemption is one scan + one memcpy + the boundary re-encodes; per-node
   relocation pays the decode/copy/re-encode/round-check sequence per node.
   Self-contained ranges should always take the bulk path.
3. **relocate_node vs the checked-copy it replaces — the honest price of the
   discipline** (4 ns/op vs raw_copy's 3 ns/op; ~1.3×). The E0801 checker
   rejects the raw byte-copy of HasSelfLink values; the legal move
   (relocate_node) costs about a third more than the forbidden shortcut —
   decode + re-encode + the i16 range check per link, and the source is
   preserved (no destructive move). That premium buys the soundness
   guarantee: no 2-byte delta is ever interpreted against an address it was
   not encoded for.

Note the geometry honesty: at 16 bytes/record the u16 reach is ±2046
elements, so a 100K-node *ring*'s wrap-around link (δ ≈ −1.6M) is
UNREPRESENTABLE by construction — the carriers are *chains* (every |δ| = 8,
the last link null) for the scan/pack/bulk rows, and self-link (δ = 0)
blocks for the per-node rows so the re-encode cost never depends on the two
blocks' malloc distance. Cross-slice and out-of-range rejections are
exercised for correctness in `tests/systems/test_link.c` (checks 19/19b),
not priced here.

Raw evidence: `bench/results/sn_link_bench.txt` (all 5 runs; checksum
columns identical 5/5 for every non-timer row).
