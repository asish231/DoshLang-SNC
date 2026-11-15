#!/bin/bash
# run_bench.sh — SWARM RABBIT HOLE, Wave-5 benchmark driver (Impl-C, Cohort Delta).
#
# Builds all five programs and runs each with N (default 10,000,000) after a
# small-N warmup run, reporting ns/op and peak RSS. See bench/RESULTS.md for
# what each row means and what is held fixed.
#
#   ./run_bench.sh              # full run, N=10,000,000, 5 timed runs, median
#   N=1000000 RUNS=3 ./run_bench.sh
#
# Controls implemented (D0 §7 pitfalls, wave-5 brief):
#   (a) memset asymmetry   — the SNlang lane rows run TWICE: raw (release
#                            build, post-DIFF-6 O(1) bump) and a second
#                            binary built with `snc --poison` (Wave 6,
#                            Impl-D: the compiler emits the T-FIBR 0xAA
#                            fill itself on every codegen path — inline
#                            bump, slow call, and the region-rewind
#                            re-poison — so the debug guarantee no longer
#                            depends on the path or on SN_POISON, which the
#                            inline bump never sees; bench/RESULTS.md §5).
#                            C: malloc vs calloc; Rust: alloc_layout vs
#                            alloc-with-zero-value; Go: every heap row is
#                            inherently zeroed (mallocgc zeroes).
#   (b) predictable branch  — freshlcg rows: sizes 16..63 from a fixed LCG
#                            (state=(state*1103515245+12345) mod 2^31, seed
#                            12345, draw before advance). lcg_state_final is
#                            printed by every program and MUST match.
#   (c) timer resolution    — every program reports its own clock-pair
#                            overhead (timer row) and the raw per-op
#                            arithmetic (elapsed*1e6/ops). SNlang's only
#                            clock is ms-resolution time_now_ms; its phases
#                            are sized >=100x that, and the C/Rust/Go twins
#                            carry the same-size phases on ns clocks.
#   (d) held fixed          — N, 16-byte records, the LCG sequence, checksum
#                            work (pointer sums), one scratch store per op in
#                            alloc phases, one 8-byte load per record in read
#                            passes, no payload stores in raw rows,
#                            single-threaded, same machine, median of RUNS
#                            timed runs after one warmup run.
#
# Cachegrind: valgrind has no darwin/arm64 support (the formula refuses to
# build: "Linux is required"); D1-miss rows are therefore NOT reported on this
# machine and the script does not block on it. On Linux the script runs
# `valgrind --tool=cachegrind` for the C and Rust binaries automatically.

set -u
cd "$(dirname "$0")"

N="${N:-10000000}"
WARMUP_N=$((N / 10))
RUNS="${RUNS:-5}"
ROOT="$(cd .. && pwd)"
SNC="$ROOT/snc"
OUT="$(pwd)/results"
mkdir -p "$OUT"

CLANG="${CLANG:-clang}"
CFLAGS="-O2"

echo "== SWARM RABBIT HOLE Wave-5 allocation benchmark =="
echo "machine: $(uname -sm) · $(sysctl -n machdep.cpu.brand_string 2>/dev/null)"
echo "clang: $($CLANG --version | head -1)"
echo "N=$N RUNS=$RUNS warmup_N=$WARMUP_N"
echo

# ---------- build ----------
echo "-- building"
if [ ! -x "$SNC" ]; then (cd "$ROOT" && make snc) || exit 1; fi

"$SNC" sn_lane_bench.sn -o "$OUT/sn_lane_bench" || exit 1
# Poison twin (Wave 6): compile-time T-FIBR emission — the fill is paid on
# the inline fast path too, which the old SN_POISON=1 env row never touched.
"$SNC" sn_lane_bench.sn --poison -o "$OUT/sn_lane_bench_poison" || exit 1
"$SNC" sn_heap_bench.sn   -o "$OUT/sn_heap_bench"   || exit 1

$CLANG $CFLAGS -Wall -o "$OUT/c_malloc_bench" c_malloc_bench.c || exit 1

(cd rust_bumpalo_bench && cargo build --release --offline >/dev/null 2>&1) || \
  (cd rust_bumpalo_bench && cargo build --release) || exit 1
cp rust_bumpalo_bench/target/release/rust_bumpalo_bench "$OUT/" || exit 1

mkdir -p go_escape_bench
cp go_escape_bench.go go_escape_bench/main.go
(cd go_escape_bench && go build -gcflags=-m -o "$OUT/go_escape_bench" . 2>"$OUT/go_escape.log") || exit 1
echo "go escape analysis log: $OUT/go_escape.log"
echo

# ---------- helpers ----------
# time -l gives peak RSS on macOS (bytes); fall back to getrusage inside C.
peak_rss_kb() {
    /usr/bin/time -l "$@" 2>&1 >/dev/null | awk '/maximum resident set size/ {print $1/1024; exit}' | cut -d. -f1
}

# median of the nth whitespace field matching row=NAME
median_row() {
    local file="$1" row="$2" field="$3"
    grep "row=$row " "$file" | awk -v f="$field" '{for(i=1;i<=NF;i++) if($i ~ "^"f"=") {split($i,a,"="); print a[2]}}' \
        | sort -n | awk '{a[NR]=$1} END {if (NR%2) print a[(NR+1)/2]; else print (a[NR/2]+a[NR/2+1])/2}'
}

run_ns() {  # run_ns binary n [env...]  (kept for ad-hoc single-row runs)
    local bin="$1" n="$2"; shift 2
    for r in $(seq "$RUNS"); do
        env "$@" "$bin" "$n" 2>/dev/null
    done
}

# ---------- pinned-lane rows (Wave 8, Impl-K) ----------
# bench/pinned_lane_bench.sn — the compiler's pinned-lane coloring (e1f3b72)
# end-to-end: recursive inout-accumulator rounds pre-colored onto pinned
# lane L2 vs the default heap-backed list. bench/pinned_lane_bench.c — the
# identical recursive shape against the runtime API on an ns clock (SNlang's
# only clock is ms-resolution; the reclaim phases are sub-ms there, so the
# precise alloc/read/reclaim rows live in the C twin). PIN_CHAINS rounds
# (default 200: each round pushes N=100_000 elements across a depth-D=100
# recursive chain — every timed phase totals >= 100x the .sn ms clock, per
# the W2F discipline). Results: bench/RESULTS_PINNED_LANE.md.
# PIN_ONLY=1 ./run_bench.sh runs just these rows without re-running the
# five-language suite (which would need N=10M and would rewrite results/).
pinned_lane_rows() {
    echo "-- building pinned-lane bench (Wave 8, Impl-K)"
    "$SNC" pinned_lane_bench.sn -o "$OUT/sn_pinned_lane_bench" || exit 1
    $CLANG $CFLAGS -Wall -o "$OUT/c_pinned_lane_bench" \
        pinned_lane_bench.c "$ROOT/compiler/runtime.c" -pthread || exit 1
    local chains="${PIN_CHAINS:-200}"
    local warm=$((chains / 10))
    local b f r kb spec
    for b in sn_pinned_lane_bench c_pinned_lane_bench; do
        f="$OUT/$b.txt"
        : > "$f"
        echo "-- warmup $b (chains=$warm)" >&2
        "$OUT/$b" "$warm" >/dev/null 2>&1
        echo "-- timed $b x$RUNS (chains=$chains)" >&2
        for r in $(seq "$RUNS"); do
            "$OUT/$b" "$chains" >> "$f" 2>/dev/null
        done
    done
    echo "-- pinned-lane peak RSS (KB, chains=$chains)"
    for b in sn_pinned_lane_bench c_pinned_lane_bench; do
        kb=$(peak_rss_kb "$OUT/$b" "$chains")
        echo "rss $b $kb" >> "$OUT/rss.txt"
        echo "  $b: $kb KB"
    done
    echo "-- pinned-lane medians (chains=$chains, median of $RUNS):"
    for spec in \
        "sn_pinned_lane_bench timer ns_per_elem" \
        "sn_pinned_lane_bench pin_build ns_per_elem" \
        "sn_pinned_lane_bench pin_read ns_per_elem" \
        "sn_pinned_lane_bench pin_resetpair ns_per_elem" \
        "sn_pinned_lane_bench heap_build ns_per_elem" \
        "c_pinned_lane_bench timer clock_ns" \
        "c_pinned_lane_bench pin elem_alloc_ns" \
        "c_pinned_lane_bench pin elem_read_ns" \
        "c_pinned_lane_bench pin chain_reclaim_ns" \
        "c_pinned_lane_bench heap elem_alloc_ns" \
        "c_pinned_lane_bench heap elem_read_ns" \
        "c_pinned_lane_bench heap chain_reclaim_ns" \
        "c_pinned_lane_bench heap_gc gc_ns" \
        "c_pinned_lane_bench pin_resetpair pair_ns" \
        "c_pinned_lane_bench pin_life cycle_ns" \
        "c_pinned_lane_bench heap_life cycle_ns"; do
        set -- $spec
        echo "  $2.$3 = $(median_row "$OUT/$1.txt" "$2" "$3")"
    done
}

if [ -n "${PIN_ONLY:-}" ]; then
    pinned_lane_rows
    echo
    echo "== done (PIN_ONLY). raw runs in $OUT/*.txt ; see RESULTS_PINNED_LANE.md =="
    exit 0
fi

# ---------- lcell rows (Wave 9, Impl-M) ----------
# bench/lcell_bench.sn — the Tier-L semilattice cell's ⊔ (join) costs,
# in-language: the gset union merge vs the Tier-G zset ⊕ join at the same
# N=100_000, the idempotent steady-state merge, the mutator-law
# delta+apply pair, and the O(1) carriers (max/lww) — "mergeable" is
# not free, priced per carrier (W2C §1.1 Tier L; final report §8.1).
# SNlang's only clock is ms-resolution; every phase is sized >= 100x it
# (per the W2F discipline). Results: bench/RESULTS_LCELL.md.
# LCELL_ONLY=1 ./run_bench.sh runs just these rows.
lcell_rows() {
    echo "-- building lcell bench (Wave 9, Impl-M)"
    "$SNC" lcell_bench.sn -o "$OUT/sn_lcell_bench" || exit 1
    local rounds="${LCELL_ROUNDS:-40}"
    local warm=$((rounds / 10))
    local f r b spec
    b=sn_lcell_bench
    f="$OUT/$b.txt"
    : > "$f"
    echo "-- warmup $b (rounds=$warm)" >&2
    "$OUT/$b" "$warm" >/dev/null 2>&1
    echo "-- timed $b x$RUNS (rounds=$rounds)" >&2
    for r in $(seq "$RUNS"); do
        "$OUT/$b" "$rounds" >> "$f" 2>/dev/null
    done
    echo "-- lcell medians (rounds=$rounds, median of $RUNS):"
    for spec in \
        "timer ns_per_elem" \
        "gset_inflate ns_per_elem" \
        "gset_merge_fresh ns_per_elem" \
        "gset_merge_same ns_per_elem" \
        "gset_delta_apply ns_per_elem" \
        "max_inflate ns_per_elem" \
        "lww_inflate ns_per_elem" \
        "zset_build ns_per_elem" \
        "zset_merge ns_per_elem" \
        "list_push ns_per_elem"; do
        set -- $spec
        echo "  $1.$2 = $(median_row "$OUT/sn_lcell_bench.txt" "$1" "$2")"
    done
}

if [ -n "${LCELL_ONLY:-}" ]; then
    lcell_rows
    echo
    echo "== done (LCELL_ONLY). raw runs in $OUT/sn_lcell_bench.txt ; see RESULTS_LCELL.md =="
    exit 0
fi

# ---------- zfix rows (Wave 9, Impl-N) ----------
# bench/zfix_bench.sn — Salt ζ: the incremental semi-naïve fixpoint
# bracket vs the NAIVE fixpoint (recompute from the full state every
# round — no delta tracking) at N=100K edges over a layered graph with
# known diameter (1000 chains x 100 edges; backward reach covers all
# 101K nodes in exactly 100 rounds — the ≤ height bound with
# equality). Rows: the bracket (zfix_run: the CSR materialization +
# semi-naïve rounds), the Tier-L GSET-closure variant (zfix_l_run),
# the naive twin (naive_run: (rounds+1) x |E| edge scans per
# bracket), and the relation build (zfix_build). The honest §3.4
# caveat: W_μ is the ROUND-SUM, not |ΔDB|-bounded — one 1000-fact
# seed batch drives 100 rounds and 100K derivations here (a 100x
# amplification). Results: bench/RESULTS_ZFIX.md.
# ZFIX_ONLY=1 ./run_bench.sh runs just these rows.
zfix_rows() {
    echo "-- building zfix bench (Wave 9, Impl-N)"
    "$SNC" zfix_bench.sn -o "$OUT/sn_zfix_bench" || exit 1
    local rounds="${ZFIX_ROUNDS:-20}"
    local warm=$((rounds / 10))
    if [ "$warm" -lt 2 ]; then warm=2; fi
    local f r b spec
    b=sn_zfix_bench
    f="$OUT/$b.txt"
    : > "$f"
    echo "-- warmup $b (rounds=$warm)" >&2
    "$OUT/$b" "$warm" >/dev/null 2>&1
    echo "-- timed $b x$RUNS (rounds=$rounds)" >&2
    for r in $(seq "$RUNS"); do
        "$OUT/$b" "$rounds" >> "$f" 2>/dev/null
    done
    echo "-- zfix medians (rounds=$rounds, median of $RUNS):"
    for spec in \
        "timer ns_per_op" \
        "zfix_build ns_per_op" \
        "zfix_run ns_per_op" \
        "zfix_run phase_ms" \
        "zfix_run probes" \
        "zfix_l_run ns_per_op" \
        "zfix_l_run phase_ms" \
        "naive_run ns_per_op" \
        "naive_run phase_ms"; do
        set -- $spec
        echo "  $1.$2 = $(median_row "$OUT/sn_zfix_bench.txt" "$1" "$2")"
    done
    echo "-- fixpoint agreement (must be identical across all runs):"
    grep -h "^agree=" "$OUT/sn_zfix_bench.txt" | sort | uniq -c
    grep -h "^zfix_len=" "$OUT/sn_zfix_bench.txt" | sort | uniq -c
    grep -h "^zfix_sum=" "$OUT/sn_zfix_bench.txt" | sort | uniq -c
}

if [ -n "${ZFIX_ONLY:-}" ]; then
    zfix_rows
    echo
    echo "== done (ZFIX_ONLY). raw runs in $OUT/sn_zfix_bench.txt ; see RESULTS_ZFIX.md =="
    exit 0
fi

# ---------- gpool rows (Wave 10, Impl-O) ----------
# bench/gpool_bench.sn — the global-pool discipline's per-op price
# (SWARM RABBIT HOLE, DIFF-4; kills G1 §1 A6c). Rows: the frozen read
# (the zero-lock shared read), the unfrozen-window mutation (the
# freeze/unfreeze pair amortized over N ops), the synchronized set
# (the vault mutex around the checked write), the synchronized
# insert+remove (the locked freelist mutation — the op the modes
# exist to serialize), and the thread_local insert+remove (the
# home-thread tag, no lock). N=1M ops/round, every phase >= 100x the
# ms clock (the W2F discipline); every row prints a deterministic
# checksum. The honest claim: the frozen default is the zero-cost
# path; synchronization prices what it buys. Results:
# bench/RESULTS_GPOOL.md.
# GP_ONLY=1 ./run_bench.sh runs just these rows.
gpool_rows() {
    echo "-- building gpool bench (Wave 10, Impl-O)"
    "$SNC" gpool_bench.sn -o "$OUT/sn_gpool_bench" || exit 1
    local rounds="${GP_ROUNDS:-10}"
    local warm=$((rounds / 10))
    if [ "$warm" -lt 1 ]; then warm=1; fi
    local f r b spec
    b=sn_gpool_bench    f="$OUT/$b.txt"
    : > "$f"
    echo "-- warmup $b (rounds=$warm)" >&2
    "$OUT/$b" "$warm" >/dev/null 2>&1
    echo "-- timed $b x$RUNS (rounds=$rounds)" >&2
    for r in $(seq "$RUNS"); do
        "$OUT/$b" "$rounds" >> "$f" 2>/dev/null
    done
    echo "-- gpool medians (rounds=$rounds, N=1M ops/round, median of $RUNS):"
    for spec in \
        "timer ns_per_op" \
        "frozen_read ns_per_op" \
        "frozen_set ns_per_op" \
        "sync_set ns_per_op" \
        "sync_insert ns_per_op" \
        "tl_insert ns_per_op"; do
        set -- $spec
        echo "  $1.$2 = $(median_row "$OUT/sn_gpool_bench.txt" "$1" "$2")"
    done
    echo "-- gpool checksums (must be identical across all runs):"
    grep -h "^values_ok=" "$OUT/sn_gpool_bench.txt" | sort | uniq -c
    grep -h "^BENCH row=frozen_read" "$OUT/sn_gpool_bench.txt" | sort | uniq -c
}

if [ -n "${GP_ONLY:-}" ]; then
    gpool_rows
    echo
    echo "== done (GP_ONLY). raw runs in $OUT/sn_gpool_bench.txt ; see RESULTS_GPOOL.md =="
    exit 0
fi

# ---------- link rows (Wave 11, DIFF-5, Impl-P) ----------
# The self-relative link system's price rows, in-language: the to_bytes
# sender-side scan (scan_bytes) vs the raw pack without it
# (raw_bytes) — the O(N) precondition that makes "100% legal bulk
# copy" TRUE instead of false by omission; relocate_range (the
# constant-shift bulk exemption: one scan + one memcpy + boundary
# re-encodes) vs per-node relocate_node (the decode/copy/re-encode/
# round-check sequence per node) — the exemption's payoff; and the
# raw 2-word ptr copy (raw_copy) — the unchecked by-value byte-copy
# the E0801 checker REJECTS on HasSelfLink values, priced as the
# floor (reloc_node - raw_copy = the honest price of the discipline).
# N=100K nodes/round (the brief's size), every phase >= 100x the ms
# clock (the W2F discipline); every row prints a deterministic
# checksum. Results: bench/RESULTS_LINK.md.
# LINK_ONLY=1 ./run_bench.sh runs just these rows.
link_rows() {
    echo "-- building link bench (Wave 11, Impl-P)"
    "$SNC" link_bench.sn -o "$OUT/sn_link_bench" || exit 1
    # 200 rounds x 100K ops keeps every phase >= 100ms (>= 100x the
    # 1-ms clock, the W2F discipline; the timer row keeps the
    # gpool/lcell 100_000-pair shape).
    local rounds="${LINK_ROUNDS:-200}"
    local warm=$((rounds / 10))
    if [ "$warm" -lt 1 ]; then warm=1; fi
    local f r b spec
    b=sn_link_bench
    f="$OUT/$b.txt"
    : > "$f"
    echo "-- warmup $b (rounds=$warm)" >&2
    "$OUT/$b" "$warm" >/dev/null 2>&1
    echo "-- timed $b x$RUNS (rounds=$rounds)" >&2
    for r in $(seq "$RUNS"); do
        "$OUT/$b" "$rounds" >> "$f" 2>/dev/null
    done
    echo "-- link medians (rounds=$rounds, N=100K node ops/round, median of $RUNS):"
    for spec in \
        "timer ns_per_op" \
        "scan_bytes ns_per_op" \
        "raw_bytes ns_per_op" \
        "reloc_range ns_per_op" \
        "reloc_node ns_per_op" \
        "raw_copy ns_per_op"; do
        set -- $spec
        echo "  $1.$2 = $(median_row "$OUT/sn_link_bench.txt" "$1" "$2")"
    done
    echo "-- link checksums (must be identical across all runs):"
    grep -h "^BENCH row=" "$OUT/sn_link_bench.txt" | grep -v "row=timer" | sort | uniq -c
}

if [ -n "${LINK_ONLY:-}" ]; then
    link_rows
    echo
    echo "== done (LINK_ONLY). raw runs in $OUT/sn_link_bench.txt ; see RESULTS_LINK.md =="
    exit 0
fi

# ---------- wave rows (Wave 13, Impl-R) ----------
# bench/wave_bench.sn — the wave region's J(W) batched fold vs the S1
# fallback merge path at the same payload shape (SWARM RABBIT HOLE,
# Wave 13; W2B §4 — Thm WS Part 2's two write paths into a seam cell).
# Rows: K=100K payloads per round — wave_max (staging + the batched
# fold on the MAX seam cell) vs s1_max (K direct lcell_inflates on
# the fallback twin), wave_gset / s1_gset (the GSET steady state —
# after round 1 every element is present, the idempotent re-merge
# dividend), and wave_readw / read_s (the frozen-κ read_W vs the
# fallback read). The CROSS-ROW CHECKSUMS MUST MATCH (both paths reach
# the identical folded state — the determinism claim, self-checked).
# Every phase >= 100x the ms clock (the W2F discipline).
# WAVE_ONLY=1 ./run_bench.sh runs just these rows.
wave_rows() {
    echo "-- building wave bench (Wave 13, Impl-R)"
    "$SNC" wave_bench.sn -o "$OUT/sn_wave_bench" || exit 1
    local rounds="${WAVE_ROUNDS:-1000}"
    local warm=$((rounds / 10))
    if [ "$warm" -lt 1 ]; then warm=1; fi
    local f r b spec
    b=sn_wave_bench
    f="$OUT/$b.txt"
    : > "$f"
    echo "-- warmup $b (rounds=$warm)" >&2
    "$OUT/$b" "$warm" >/dev/null 2>&1
    echo "-- timed $b x$RUNS (rounds=$rounds)" >&2
    for r in $(seq "$RUNS"); do
        "$OUT/$b" "$rounds" >> "$f" 2>/dev/null
    done
    echo "-- wave medians (rounds=$rounds, K=100K payloads/round, median of $RUNS):"
    for spec in \
        "timer ns_per_op" \
        "wave_max ns_per_op" \
        "s1_max ns_per_op" \
        "wave_gset ns_per_op" \
        "s1_gset ns_per_op" \
        "wave_readw ns_per_op" \
        "read_s ns_per_op"; do
        set -- $spec
        echo "  $1.$2 = $(median_row "$OUT/$b.txt" "$1" "$2")"
    done
    echo "-- wave checksums (must be identical across all runs, and the wave/S1 pairs must agree):"
    grep -h "^BENCH row=" "$OUT/$b.txt" | grep -v "row=timer" | sort | uniq -c
}

if [ -n "${WAVE_ONLY:-}" ]; then
    wave_rows
    echo
    echo "== done (WAVE_ONLY). raw runs in $OUT/sn_wave_bench.txt ; see RESULTS_WAVE.md =="
    exit 0
fi

# ---------- sfc rows (Wave 14, Impl-S) ----------
# bench/sfc_bench.sn — the SFC layouts' price rows, in-language (A2's
# space-filling-geometry decision table, priced; SWARM RABBIT HOLE,
# Wave 14). Rows: lex_build vs from_lex (the row-major build vs the
# one blocked reshuffle of the same 1M values — the 2–5%-of-runtime
# layout price ICS'99 names), the 5-point stencil over a 1024×1024
# grid (8 MB — past L2) in three orders/layouts (lexicographic
# row-major, lexicographic morton, curve-ordered morton — the honest
# direction report: Abel–Mark says row-major wins the mean
# neighbor-gap; Chatterjee's morton claim is recursive-kernel dyadic
# contiguity, not flat stencils), the hilbert box queries (100 random
# + 400 dyadic boxes over a fully-populated 512×512 map — the
# segment advance vs the full linear scan over the same entries; the
# CHECKSUMS MUST AGREE: linear == random + dyadic, the exactness
# claim self-checked via the sfc_box_split line), and the pure
# interleave cost (software-path on this AArch64 machine — no BMI2,
# C2.8; the sfc_path line prints the resolved path). Every op phase
# >= 100x the ms clock (the W2F discipline). Results:
# bench/RESULTS_SFC.md. SFC_ONLY=1 ./run_bench.sh runs just these rows.
sfc_rows() {
    echo "-- building sfc bench (Wave 14, Impl-S)"
    "$SNC" sfc_bench.sn -o "$OUT/sn_sfc_bench" || exit 1
    local rounds="${SFC_ROUNDS:-5}"
    local warm=$((rounds / 10))
    if [ "$warm" -lt 1 ]; then warm=1; fi
    local f r b spec
    b=sn_sfc_bench
    f="$OUT/$b.txt"
    : > "$f"
    echo "-- warmup $b (base=$warm)" >&2
    "$OUT/$b" "$warm" >/dev/null 2>&1
    echo "-- timed $b x$RUNS (base=$rounds)" >&2
    for r in $(seq "$RUNS"); do
        "$OUT/$b" "$rounds" >> "$f" 2>/dev/null
    done
    echo "-- sfc medians (base=$rounds, median of $RUNS):"
    for spec in \
        "timer ns_per_op" \
        "lex_build ns_per_op" \
        "from_lex ns_per_op" \
        "stencil_lex_rowmajor ns_per_op" \
        "stencil_lex_morton ns_per_op" \
        "stencil_curve_morton ns_per_op" \
        "hilbert_box ns_per_op" \
        "hilbert_box_dyadic ns_per_op" \
        "hilbert_linear ns_per_op" \
        "interleave ns_per_op"; do
        set -- $spec
        echo "  $1.$2 = $(median_row "$OUT/$b.txt" "$1" "$2")"
    done
    echo "-- sfc dispatch + checksums (must be identical across all runs; the box split must satisfy linear == random + dyadic):"
    grep -h "^sfc_path=" "$OUT/$b.txt" | sort | uniq -c
    grep -h "^sfc_box_split=" "$OUT/$b.txt" | sort | uniq -c
    grep -h "^BENCH row=" "$OUT/$b.txt" | grep -v "row=timer" | sort | uniq -c
}

if [ -n "${SFC_ONLY:-}" ]; then
    sfc_rows
    echo
    echo "== done (SFC_ONLY). raw runs in $OUT/sn_sfc_bench.txt ; see RESULTS_SFC.md =="
    exit 0
fi

# ---------- run + collect ----------
collect() {  # collect NAME -- cmd...
    local name="$1"; shift; shift
    local f="$OUT/$name.txt"
    : > "$f"
    echo "-- warmup $name (N=$WARMUP_N)" >&2
    "$@" "$WARMUP_N" >/dev/null 2>&1
    echo "-- timed $name x$RUNS (N=$N)" >&2
    for r in $(seq "$RUNS"); do
        "$@" "$N" >> "$f" 2>/dev/null
    done
}

collect sn_lane_raw        -- "$OUT/sn_lane_bench"
collect sn_lane_poison     -- "$OUT/sn_lane_bench_poison"
collect sn_heap            -- "$OUT/sn_heap_bench"
collect c_malloc           -- "$OUT/c_malloc_bench"
collect rust_bumpalo       -- "$OUT/rust_bumpalo_bench"
collect go_escape          -- "$OUT/go_escape_bench"

# pinned-lane rows (Wave 8, Impl-K): see pinned_lane_rows above
pinned_lane_rows

# lcell rows (Wave 9, Impl-M): see lcell_rows above
lcell_rows

# zfix rows (Wave 9, Impl-N): see zfix_rows above
zfix_rows

# link rows (Wave 11, Impl-P): see link_rows above
link_rows

# wave rows (Wave 13, Impl-R): see wave_rows above
wave_rows

# sfc rows (Wave 14, Impl-S): see sfc_rows above
sfc_rows

# per-run peak RSS (one run each; RSS is stable)
echo "-- peak RSS (KB)"
for b in sn_lane_bench sn_heap_bench c_malloc_bench rust_bumpalo_bench go_escape_bench; do
    kb=$(peak_rss_kb "$OUT/$b" "$N")
    echo "rss $b $kb" >> "$OUT/rss.txt"
    echo "  $b: $kb KB"
done
# poisoned-lane RSS (write pattern identical -> same touched pages)
kb=$(/usr/bin/time -l "$OUT/sn_lane_bench_poison" "$N" 2>&1 >/dev/null | awk '/maximum resident set size/ {print $1/1024; exit}' | cut -d. -f1)
echo "rss sn_lane_bench_poison $kb" >> "$OUT/rss.txt"
echo "  sn_lane_bench_poison (--poison): $kb KB"

# ---------- cachegrind (Linux only) ----------
if command -v valgrind >/dev/null 2>&1; then
    if valgrind --tool=cachegrind --version >/dev/null 2>&1; then
        echo "-- cachegrind (deterministic D1 misses; small N)"
        for b in c_malloc_bench rust_bumpalo_bench sn_lane_bench; do
            valgrind --tool=cachegrind --cachegrind-out-file="$OUT/cg_$b.out" "$OUT/$b" 1000000 >/dev/null 2>&1 \
                && cg_annotate "$OUT/cg_$b.out" 2>/dev/null | grep -E "D1 +misses|LL +misses" | head -2 >> "$OUT/cachegrind.txt"
        done
    fi
else
    echo "-- cachegrind SKIPPED: no valgrind on this machine (Apple Silicon macOS: the brew formula requires Linux; see RESULTS.md)"
fi

echo
echo "== done. raw runs in $OUT/*.txt ; see RESULTS.md =="
