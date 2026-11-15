#!/usr/bin/env bash
set -uo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$DIR/../.." && pwd)"
SNC="$ROOT/snc"

echo "================================================================="
echo "  SNLANG SYSTEMS & SECURITY VERIFICATION TEST SUITE"
echo "================================================================="
echo "Using compiler: $SNC"
echo ""

FAILURES=0

# 1. Fixed-Point dec(2) and dec(4) Arithmetic
echo "-----------------------------------------------------------------"
echo "1. FIXED-POINT ARITHMETIC (test_dec.sn)"
echo "-----------------------------------------------------------------"
OUT_DEC="/tmp/snc_test_dec"
rm -f "$OUT_DEC"
if "$SNC" "$DIR/test_dec.sn" -o "$OUT_DEC"; then
    OUTPUT=$("$OUT_DEC")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_dec: PASSED"
    else
        echo ">>> test_dec: FAILED (output did not contain PASS)"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_dec: COMPILATION FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 2. Reject Compound Assignment to Const Variables
echo "-----------------------------------------------------------------"
echo "2. REJECT COMPOUND ASSIGN TO CONST (test_const_assign.sn)"
echo "-----------------------------------------------------------------"
OUT_CONST="/tmp/snc_test_const_assign"
rm -f "$OUT_CONST"
COMPILE_ERR=$("$SNC" "$DIR/test_const_assign.sn" -o "$OUT_CONST" 2>&1 || true)
echo "$COMPILE_ERR"
if echo "$COMPILE_ERR" | grep -qi "cannot assign to const"; then
    echo ">>> test_const_assign: PASSED (compiler rejected compound assignment to const as expected)"
elif [ ! -f "$OUT_CONST" ]; then
    echo ">>> test_const_assign: PASSED (compiler rejected invalid const assignment)"
else
    echo ">>> test_const_assign: FAILED (compiled successfully when rejection was expected)"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 3. Reject Await on Non-Future Types
echo "-----------------------------------------------------------------"
echo "3. REJECT AWAIT ON NON-FUTURE (test_await_nonfuture.sn)"
echo "-----------------------------------------------------------------"
OUT_AWAIT="/tmp/snc_test_await_nonfuture"
rm -f "$OUT_AWAIT"
COMPILE_ERR=$("$SNC" "$DIR/test_await_nonfuture.sn" -o "$OUT_AWAIT" 2>&1 || true)
echo "$COMPILE_ERR"
if echo "$COMPILE_ERR" | grep -qi "cannot await non-future"; then
    echo ">>> test_await_nonfuture: PASSED (compiler rejected await on non-future type as expected)"
elif [ ! -f "$OUT_AWAIT" ]; then
    echo ">>> test_await_nonfuture: PASSED (compiler rejected invalid await)"
else
    echo ">>> test_await_nonfuture: FAILED (compiled successfully when rejection was expected)"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 4. Generational Slot Vault Pool (std.pool)
echo "-----------------------------------------------------------------"
echo "4. GENERATIONAL SLOT POOL (test_pool.sn)"
echo "-----------------------------------------------------------------"
OUT_POOL="/tmp/snc_test_pool"
rm -f "$OUT_POOL"
if "$SNC" "$DIR/test_pool.sn" -o "$OUT_POOL"; then
    OUTPUT=$("$OUT_POOL")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_pool: PASSED"
    else
        echo ">>> test_pool: FAILED (output did not contain PASS)"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_pool: COMPILATION FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 5. Region Block & Elastic Lane Rollback
echo "-----------------------------------------------------------------"
echo "5. REGION BLOCK & ELASTIC LANE ROLLBACK (test_region.sn)"
echo "-----------------------------------------------------------------"
OUT_REGION="/tmp/snc_test_region"
rm -f "$OUT_REGION"
if "$SNC" "$DIR/test_region.sn" -o "$OUT_REGION"; then
    OUTPUT=$("$OUT_REGION")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_region: PASSED"
    else
        echo ">>> test_region: FAILED (output did not contain PASS)"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_region: COMPILATION FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 6. Web Directory Traversal Defense in std.web
echo "-----------------------------------------------------------------"
echo "6. WEB PATH TRAVERSAL DEFENSE (test_web_traversal.sn)"
echo "-----------------------------------------------------------------"
OUT_WEB="/tmp/snc_test_web_traversal"
rm -f "$OUT_WEB"
if "$SNC" "$DIR/test_web_traversal.sn" -o "$OUT_WEB"; then
    OUTPUT=$("$OUT_WEB")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_web_traversal: PASSED"
    else
        echo ">>> test_web_traversal: FAILED (output did not contain PASS)"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_web_traversal: COMPILATION FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 7. Nursery Early Return Task Joining
echo "-----------------------------------------------------------------"
echo "7. NURSERY EARLY RETURN (test_nursery_early_return.sn)"
echo "-----------------------------------------------------------------"
OUT_NRET="/tmp/snc_test_nursery_early_return"
rm -f "$OUT_NRET"
if "$SNC" "$DIR/test_nursery_early_return.sn" -o "$OUT_NRET"; then
    OUTPUT=$("$OUT_NRET")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_nursery_early_return: PASSED"
    else
        echo ">>> test_nursery_early_return: FAILED (tasks orphaned on return)"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_nursery_early_return: COMPILATION FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 8. Region Early Return Lane Rewind
echo "-----------------------------------------------------------------"
echo "8. REGION EARLY RETURN (test_region_early_return.sn)"
echo "-----------------------------------------------------------------"
OUT_RRET="/tmp/snc_test_region_early_return"
rm -f "$OUT_RRET"
if "$SNC" "$DIR/test_region_early_return.sn" -o "$OUT_RRET"; then
    OUTPUT=$("$OUT_RRET")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_region_early_return: PASSED"
    else
        echo ">>> test_region_early_return: FAILED (lane memory leaked on return)"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_region_early_return: COMPILATION FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 9. Channel Move on Member Send
echo "-----------------------------------------------------------------"
echo "9. CHAN MOVE ON MEMBER SEND (test_chan_member_move.sn)"
echo "-----------------------------------------------------------------"
OUT_CMM="/tmp/snc_test_chan_member_move"
rm -f "$OUT_CMM"
COMPILE_ERR=$("$SNC" "$DIR/test_chan_member_move.sn" -o "$OUT_CMM" 2>&1 || true)
echo "$COMPILE_ERR"
if echo "$COMPILE_ERR" | grep -qi "use of moved value"; then
    echo ">>> test_chan_member_move: PASSED (compiler rejected access to moved container as expected)"
elif [ ! -f "$OUT_CMM" ]; then
    echo ">>> test_chan_member_move: PASSED (compiler rejected access to moved object)"
else
    echo ">>> test_chan_member_move: FAILED (compiled successfully when rejection was expected)"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "-----------------------------------------------------------------"
echo "10. NURSERY CHILD-LANE CARVE INTEGRATION (test_nursery_carve.sn)"
echo "-----------------------------------------------------------------"
OUT_NCARVE="/tmp/snc_test_nursery_carve"
rm -f "$OUT_NCARVE"
if "$SNC" "$DIR/test_nursery_carve.sn" -o "$OUT_NCARVE"; then
    OUTPUT=$("$OUT_NCARVE")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_nursery_carve(.sn): PASSED"
    else
        echo ">>> test_nursery_carve(.sn): FAILED (output did not contain PASS)"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_nursery_carve(.sn): COMPILATION FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 11. Nursery Child-Lane Carve — the O(1) join at the runtime level
#     (D0 gap #5: sn_nursery_loan_lane -> sn_lane_alloc(nu) -> the join's
#     1-store reset). Built twice: plain, then ASan (the UAF/no-heap-error
#     stress pass with concurrent + nested spawns). LSan is not supported
#     on arm64 darwin; leak-freedom is checked with macOS `leaks` below.
echo "-----------------------------------------------------------------"
echo "11. NURSERY CHILD-LANE CARVE, RUNTIME O(1) JOIN (test_nursery_carve.c)"
echo "-----------------------------------------------------------------"
OUT_CCARVE="/tmp/snc_test_nursery_carve_c"
rm -f "$OUT_CCARVE"
if clang -O1 -g -o "$OUT_CCARVE" "$DIR/test_nursery_carve.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$("$OUT_CCARVE")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_nursery_carve(.c): PASSED"
    else
        echo ">>> test_nursery_carve(.c): FAILED (output did not contain PASS)"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_nursery_carve(.c): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 11b. The ASan stress pass of the same runtime carve test
echo "-----------------------------------------------------------------"
echo "11b. NURSERY CHILD-LANE CARVE, ASAN STRESS (test_nursery_carve.c)"
echo "-----------------------------------------------------------------"
OUT_CCARVE_ASAN="/tmp/snc_test_nursery_carve_c_asan"
rm -f "$OUT_CCARVE_ASAN"
if clang -O1 -g -fsanitize=address -o "$OUT_CCARVE_ASAN" "$DIR/test_nursery_carve.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$(ASAN_OPTIONS=detect_leaks=0 "$OUT_CCARVE_ASAN" 2>&1)
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_nursery_carve(.c ASan): PASSED (no ASan reports under concurrent/nested spawns)"
    else
        echo ">>> test_nursery_carve(.c ASan): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_nursery_carve(.c ASan): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 11c. Leak check of the runtime carve test via macOS `leaks` (LSan is
#      unsupported on arm64 darwin). The test pauses on
#      SN_CARVE_LEAK_PAUSE so `leaks` can attach to the live process.
echo "-----------------------------------------------------------------"
echo "11c. NURSERY CHILD-LANE CARVE, LEAK CHECK (macOS leaks(1))"
echo "-----------------------------------------------------------------"
if command -v leaks >/dev/null 2>&1; then
    CARVE_PID=""
    MallocStackLogging=1 SN_CARVE_LEAK_PAUSE=15 "$OUT_CCARVE" > /tmp/snc_carve_leaks_out.txt 2>&1 &
    CARVE_PID=$!
    sleep 7
    LEAK_LINE=$(leaks "$CARVE_PID" 2>/dev/null | grep -E "[0-9]+ leaks? for [0-9]+ total leaked bytes" | head -1)
    wait "$CARVE_PID" 2>/dev/null
    echo "$LEAK_LINE"
    if echo "$LEAK_LINE" | grep -q "0 leaks for 0 total leaked bytes"; then
        echo ">>> test_nursery_carve(leaks): PASSED (0 leaked bytes)"
    else
        echo ">>> test_nursery_carve(leaks): FAILED (leaks reported)"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_nursery_carve(leaks): SKIPPED (leaks(1) unavailable)"
fi

echo ""

# 12. The incremental view engine — DBSP-style delta propagation
#     (Problem-5 second slice: the O(|dA|) maintained-view circuit;
#     chain test with exact probe counts, scratch-recompute deltas,
#     retraction, |A|-independence, quiescence).
echo "-----------------------------------------------------------------"
echo "12. INCREMENTAL VIEW ENGINE (test_view_engine.c)"
echo "-----------------------------------------------------------------"
OUT_VIEW="/tmp/snc_test_view_engine"
rm -f "$OUT_VIEW"
if clang -O1 -g -o "$OUT_VIEW" "$DIR/test_view_engine.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$("$OUT_VIEW")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_view_engine: PASSED"
    else
        echo ">>> test_view_engine: FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_view_engine: BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 12b. The ASan stress pass of the same view-engine test
echo "-----------------------------------------------------------------"
echo "12b. INCREMENTAL VIEW ENGINE, ASAN STRESS (test_view_engine.c)"
echo "-----------------------------------------------------------------"
OUT_VIEW_ASAN="/tmp/snc_test_view_engine_asan"
rm -f "$OUT_VIEW_ASAN"
if clang -O1 -g -fsanitize=address -o "$OUT_VIEW_ASAN" "$DIR/test_view_engine.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$(ASAN_OPTIONS=detect_leaks=0 "$OUT_VIEW_ASAN" 2>&1)
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_view_engine(ASan): PASSED (no ASan reports under delta propagation)"
    else
        echo ">>> test_view_engine(ASan): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_view_engine(ASan): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 13. Tier-G Z-set delta collections — the stdlib surface (P5 slice 1).
echo "-----------------------------------------------------------------"
echo "13. TIER-G Z-SET DELTA COLLECTIONS, STDLIB (test_zset_delta.sn)"
echo "-----------------------------------------------------------------"
OUT_ZSET_SN="/tmp/snc_test_zset_delta_sn"
rm -f "$OUT_ZSET_SN"
if "$SNC" "$DIR/test_zset_delta.sn" -o "$OUT_ZSET_SN"; then
    OUTPUT=$("$OUT_ZSET_SN")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS: test_zset_delta"; then
        echo ">>> test_zset_delta(.sn): PASSED"
    else
        echo ">>> test_zset_delta(.sn): FAILED (output did not contain PASS)"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_zset_delta(.sn): COMPILATION FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 14. Tier-G Z-set engine — the runtime mechanics (P5 slice 1; W2C §1.1).
#     Built twice: plain, then ASan (the UAF/no-heap-error stress pass
#     over the swap-with-last removal and the prune/retain paths). LSan is
#     not supported on arm64 darwin; leak-freedom is checked with `leaks`.
echo "-----------------------------------------------------------------"
echo "14. TIER-G Z-SET DELTA, RUNTIME MECHANICS (test_zset_delta.c)"
echo "-----------------------------------------------------------------"
OUT_ZSET_C="/tmp/snc_test_zset_delta_c"
rm -f "$OUT_ZSET_C"
if clang -O1 -g -o "$OUT_ZSET_C" "$DIR/test_zset_delta.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$("$OUT_ZSET_C")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_zset_delta(.c): PASSED"
    else
        echo ">>> test_zset_delta(.c): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_zset_delta(.c): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 14b. The ASan stress pass of the same runtime z-set test
echo "-----------------------------------------------------------------"
echo "14b. TIER-G Z-SET DELTA, ASAN STRESS (test_zset_delta.c)"
echo "-----------------------------------------------------------------"
OUT_ZSET_ASAN="/tmp/snc_test_zset_delta_c_asan"
rm -f "$OUT_ZSET_ASAN"
if clang -O1 -g -fsanitize=address -o "$OUT_ZSET_ASAN" "$DIR/test_zset_delta.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$(ASAN_OPTIONS=detect_leaks=0 "$OUT_ZSET_ASAN" 2>&1)
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_zset_delta(.c ASan): PASSED (no ASan reports over the z-set engine)"
    else
        echo ">>> test_zset_delta(.c ASan): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_zset_delta(.c ASan): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 14c. Leak check of the runtime z-set test via macOS `leaks`. The test
#      pauses on SN_ZSET_LEAK_PAUSE so `leaks` can attach.
echo "-----------------------------------------------------------------"
echo "14c. TIER-G Z-SET DELTA, LEAK CHECK (macOS leaks(1))"
echo "-----------------------------------------------------------------"
if command -v leaks >/dev/null 2>&1; then
    ZSET_PID=""
    MallocStackLogging=1 SN_ZSET_LEAK_PAUSE=15 "$OUT_ZSET_C" > /tmp/snc_zset_leaks_out.txt 2>&1 &
    ZSET_PID=$!
    sleep 7
    LEAK_LINE=$(leaks "$ZSET_PID" 2>/dev/null | grep -E "[0-9]+ leaks? for [0-9]+ total leaked bytes" | head -1)
    wait "$ZSET_PID" 2>/dev/null
    echo "$LEAK_LINE"
    if echo "$LEAK_LINE" | grep -q "0 leaks for 0 total leaked bytes"; then
        echo ">>> test_zset_delta(leaks): PASSED (0 leaked bytes)"
    else
        echo ">>> test_zset_delta(leaks): FAILED (leaks reported)"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_zset_delta(leaks): SKIPPED (leaks(1) unavailable)"
fi

# 15. Pinned-lane accumulator headers — the Wave-8 coloring's runtime
#     mechanics (Impl-I's sn_list_new_lane + the O(1) SCC-exit reclaim
#     Impl-K measured). Built twice: plain, then ASan (the UAF stress pass
#     over the restore/reuse cycle; the pinned path performs ZERO mallocs,
#     so no leaks(1) check is needed — the heap twin's headers are all
#     sn_release'd).
echo "-----------------------------------------------------------------"
echo "15. PINNED-LANE ACCUMULATOR HEADERS (test_pinned_lane.c)"
echo "-----------------------------------------------------------------"
OUT_PIN="/tmp/snc_test_pinned_lane"
rm -f "$OUT_PIN"
if clang -O1 -g -o "$OUT_PIN" "$DIR/test_pinned_lane.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$("$OUT_PIN")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_pinned_lane(.c): PASSED"
    else
        echo ">>> test_pinned_lane(.c): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_pinned_lane(.c): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 15b. The ASan stress pass of the same pinned-lane test
echo "-----------------------------------------------------------------"
echo "15b. PINNED-LANE ACCUMULATOR HEADERS, ASAN STRESS (test_pinned_lane.c)"
echo "-----------------------------------------------------------------"
OUT_PIN_ASAN="/tmp/snc_test_pinned_lane_asan"
rm -f "$OUT_PIN_ASAN"
if clang -O1 -g -fsanitize=address -o "$OUT_PIN_ASAN" "$DIR/test_pinned_lane.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$(ASAN_OPTIONS=detect_leaks=0 "$OUT_PIN_ASAN" 2>&1)
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_pinned_lane(.c ASan): PASSED (no ASan reports over the lane restore/reuse cycle)"
    else
        echo ">>> test_pinned_lane(.c ASan): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_pinned_lane(.c ASan): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 16. Tier-L join-semilattice cells — the semilattice tier's runtime
#     mechanics (SWARM RABBIT HOLE, Wave 9, Impl-M; W2C §1.1 Tier L).
#     The three lattice laws (idempotence/commutativity/associativity)
#     asserted per merge over randomized seeded-LCG op sequences, the
#     delta-mutator law m(X) = X ⊔ m^δ(X), quiescence, lww tiebreak
#     determinism, the SN_LATTICE_CHECK=1 in-engine law self-checks,
#     and the E-LKIND tier-confusion probes (fork'd children must die
#     with the diagnostic — carrier mismatch, kind mismatch, and the
#     G-side zset_merge twin).
echo "-----------------------------------------------------------------"
echo "16. TIER-L SEMILATTICE CELLS, RUNTIME MECHANICS (test_lcell.c)"
echo "-----------------------------------------------------------------"
OUT_LCELL_C="/tmp/snc_test_lcell_c"
rm -f "$OUT_LCELL_C"
if clang -O1 -g -o "$OUT_LCELL_C" "$DIR/test_lcell.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$("$OUT_LCELL_C")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_lcell(.c): PASSED"
    else
        echo ">>> test_lcell(.c): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_lcell(.c): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 16b. The ASan stress pass of the same Tier-L cell test
echo "-----------------------------------------------------------------"
echo "16b. TIER-L SEMILATTICE CELLS, ASAN STRESS (test_lcell.c)"
echo "-----------------------------------------------------------------"
OUT_LCELL_ASAN="/tmp/snc_test_lcell_c_asan"
rm -f "$OUT_LCELL_ASAN"
if clang -O1 -g -fsanitize=address -o "$OUT_LCELL_ASAN" "$DIR/test_lcell.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$(ASAN_OPTIONS=detect_leaks=0 "$OUT_LCELL_ASAN" 2>&1)
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_lcell(.c ASan): PASSED (no ASan reports over the GSET insert/rehash/growth paths)"
    else
        echo ">>> test_lcell(.c ASan): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_lcell(.c ASan): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 17. Salt ζ — the fixpoint-bracket engine's runtime mechanics
#     (SWARM RABBIT HOLE, Wave 9, Impl-N; W2C §3.2–§3.5). The bracket
#     over randomized seeded-LCG graphs AND known shapes (the chain,
#     the star), both tiers (Tier-G Z-set presence / Tier-L GSET ⊔):
#     closure correctness vs a reference bitset BFS, the semi-naïve
#     claim probe-counted (each round's delta == the BFS level EXACTLY;
#     each round's adjacency probes == the delta's indeg sum — Thm 4b),
#     rounds ≤ height with equality on the chain, the per-tier
#     quiescence tests (𝒟(∫α)=0 / F(s) ⊑ s via the mutator law), a
#     collection fired mid-bracket (the tracked-engine design), and
#     the E-LKIND guard probes. Built twice: plain, then ASan.
echo "-----------------------------------------------------------------"
echo "17. SALT ZETA FIXPOINT BRACKET, RUNTIME MECHANICS (test_zfix.c)"
echo "-----------------------------------------------------------------"
OUT_ZFIX_C="/tmp/snc_test_zfix_c"
rm -f "$OUT_ZFIX_C"
if clang -O1 -g -o "$OUT_ZFIX_C" "$DIR/test_zfix.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$("$OUT_ZFIX_C")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_zfix(.c): PASSED"
    else
        echo ">>> test_zfix(.c): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_zfix(.c): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 17b. The ASan stress pass of the same Salt-ζ bracket test
echo "-----------------------------------------------------------------"
echo "17b. SALT ZETA FIXPOINT BRACKET, ASAN STRESS (test_zfix.c)"
echo "-----------------------------------------------------------------"
OUT_ZFIX_ASAN="/tmp/snc_test_zfix_c_asan"
rm -f "$OUT_ZFIX_ASAN"
if clang -O1 -g -fsanitize=address -o "$OUT_ZFIX_ASAN" "$DIR/test_zfix.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$(ASAN_OPTIONS=detect_leaks=0 "$OUT_ZFIX_ASAN" 2>&1)
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_zfix(.c ASan): PASSED (no ASan reports over the CSR build / round loop / mutator-law fragments)"
    else
        echo ">>> test_zfix(.c ASan): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_zfix(.c ASan): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 18. Global-pool discipline — the DIFF-4 runtime mechanics (SWARM
#     RABBIT HOLE, Wave 10, Impl-O; kills G1 §1 A6c). The frozen
#     concurrent read (N threads, stable generations, exact
#     checksums), the synchronized vault under N-thread mutation
#     (the lock serializes; final state exact), the unfrozen
#     window's freeze/unfreeze depth discipline (nested windows,
#     LIFO unwind, no underflow), and the E0803.1 home-thread
#     backstop probe (fork'd, the E-LKIND pattern). Built twice:
#     plain, then ASan (18b).
echo "-----------------------------------------------------------------"
echo "18. GLOBAL-POOL DISCIPLINE, RUNTIME MECHANICS (test_gpool.c)"
echo "-----------------------------------------------------------------"
OUT_GPOOL_C="/tmp/snc_test_gpool_c"
rm -f "$OUT_GPOOL_C"
if clang -O1 -g -o "$OUT_GPOOL_C" "$DIR/test_gpool.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$("$OUT_GPOOL_C")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_gpool(.c): PASSED"
    else
        echo ">>> test_gpool(.c): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_gpool(.c): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 18b. The ASan stress pass of the same global-pool test
echo "-----------------------------------------------------------------"
echo "18b. GLOBAL-POOL DISCIPLINE, ASAN STRESS (test_gpool.c)"
echo "-----------------------------------------------------------------"
OUT_GPOOL_ASAN="/tmp/snc_test_gpool_c_asan"
rm -f "$OUT_GPOOL_ASAN"
if clang -O1 -g -fsanitize=address -o "$OUT_GPOOL_ASAN" "$DIR/test_gpool.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$(ASAN_OPTIONS=detect_leaks=0 "$OUT_GPOOL_ASAN" 2>&1)
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_gpool(.c ASan): PASSED (no ASan reports over the concurrent frozen read / locked mutation / window cycles)"
    else
        echo ">>> test_gpool(.c ASan): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_gpool(.c ASan): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 19. The self-relative link system — the DIFF-5 runtime mechanics
#     (SWARM RABBIT HOLE, Wave 11, Impl-P; kills G1 §1 A2 → S4). The
#     E0801 seam closure's runtime half: relocate_node's decode/re-
#     encode (in-links fixed to the new interior, out-links re-based to
#     their absolute targets), relocate_range (the constant-shift bulk
#     exemption) BIT-IDENTICAL to the per-link re-encode reference
#     (memcmp over the whole block), the to_bytes sender-side
#     self-containedness scan accepting exactly the self-contained
#     ranges and rejecting exactly the cross-slice ones (the E0801
#     (serialize) fail-fast, fork'd probe), the round trip (bytes ->
#     memcpy -> constant-shift relocation), and the i16-bound rejection
#     (E0801 (relocate), fork'd). Built twice: plain, then ASan (19b).
echo "-----------------------------------------------------------------"
echo "19. SELF-RELATIVE LINK SYSTEM, RUNTIME MECHANICS (test_link.c)"
echo "-----------------------------------------------------------------"
OUT_LINK_C="/tmp/snc_test_link_c"
rm -f "$OUT_LINK_C"
if clang -O1 -g -o "$OUT_LINK_C" "$DIR/test_link.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$("$OUT_LINK_C")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_link(.c): PASSED"
    else
        echo ">>> test_link(.c): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_link(.c): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 19b. The ASan stress pass of the same link-system test
echo "-----------------------------------------------------------------"
echo "19b. SELF-RELATIVE LINK SYSTEM, ASAN STRESS (test_link.c)"
echo "-----------------------------------------------------------------"
OUT_LINK_ASAN="/tmp/snc_test_link_c_asan"
rm -f "$OUT_LINK_ASAN"
if clang -O1 -g -fsanitize=address -o "$OUT_LINK_ASAN" "$DIR/test_link.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$(ASAN_OPTIONS=detect_leaks=0 "$OUT_LINK_ASAN" 2>&1)
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_link(.c ASan): PASSED (no ASan reports over the lane-backed carriers / re-encode paths / scan+pack)"
    else
        echo ">>> test_link(.c ASan): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_link(.c ASan): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

# 20. The lane-aware items pool — the Wave-12 runtime mechanics
#     (SWARM RABBIT HOLE, Wave 12, Impl-Q; closing Impl-K's Wave-8
#     benchmark caveat in bench/RESULTS_PINNED_LANE.md). The per-lane
#     registry of lane-backed list headers: grown (malloc'd) items
#     arrays are freed at the rewind that covers their header (the
#     reclaim invariant), slab windows (lists under 65,536 elements)
#     are NEVER freed, nested watermarks free exactly the covering
#     set, heap-backed lists stay off the registry, and the release
#     side of the push_ptr pointee law is proven (borrow-kind lists
#     release without touching the slots; owned-kind lists still ARC
#     their elements). Built twice: plain, then ASan (20b).
echo "-----------------------------------------------------------------"
echo "20. LANE-AWARE ITEMS POOL (test_lane_items.c)"
echo "-----------------------------------------------------------------"
OUT_LANEITEMS="/tmp/snc_test_lane_items"
rm -f "$OUT_LANEITEMS"
if clang -O1 -g -o "$OUT_LANEITEMS" "$DIR/test_lane_items.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$("$OUT_LANEITEMS")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_lane_items(.c): PASSED"
    else
        echo ">>> test_lane_items(.c): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_lane_items(.c): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 20b. The ASan stress pass of the same lane-aware items pool test
echo "-----------------------------------------------------------------"
echo "20b. LANE-AWARE ITEMS POOL, ASAN STRESS (test_lane_items.c)"
echo "-----------------------------------------------------------------"
OUT_LANEITEMS_ASAN="/tmp/snc_test_lane_items_asan"
rm -f "$OUT_LANEITEMS_ASAN"
if clang -O1 -g -fsanitize=address -o "$OUT_LANEITEMS_ASAN" "$DIR/test_lane_items.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$(ASAN_OPTIONS=detect_leaks=0 "$OUT_LANEITEMS_ASAN" 2>&1)
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_lane_items(.c ASan): PASSED (no ASan reports over the grown-array reclaim / nested rewinds / slab non-frees)"
    else
        echo ">>> test_lane_items(.c ASan): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_lane_items(.c ASan): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

# 21. Wave regions — the Theorem WS runtime mechanics (SWARM RABBIT
#     HOLE, Wave 13, Impl-R; W2B §3/§4 — the wave-seam composition
#     theorem, the campaign's centerpiece). The SnWave context's
#     mechanics: the capture/staging/join round trip on all three
#     Tier-L carriers, the staging-atomicity buffer-swap proof (the
#     live cell is bit-identical to its pre-wave state while the wave
#     is open — staging writes the buffer, never the cell), the J(W)
#     fold's order/duplication/grouping independence (Thm WS Part 2
#     (i)/(ii)/(iii) — shuffled staging orders, doubled payloads,
#     and the multiset split across two sequential waves all fold
#     to the identical cell), multiset isolation across slots, the
#     lattice laws under the fold (SN_LATTICE_CHECK=1 rides the
#     join's merge path), and the [E-R1] runtime backstops (fork'd
#     probes). Built twice: plain, then ASan (21b).
echo "-----------------------------------------------------------------"
echo "21. WAVE REGIONS, RUNTIME MECHANICS (test_wave.c)"
echo "-----------------------------------------------------------------"
OUT_WAVE="/tmp/snc_test_wave_c"
rm -f "$OUT_WAVE"
if clang -O1 -g -o "$OUT_WAVE" "$DIR/test_wave.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$("$OUT_WAVE")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_wave(.c): PASSED"
    else
        echo ">>> test_wave(.c): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_wave(.c): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "-----------------------------------------------------------------"
echo "21b. WAVE REGIONS, ASAN STRESS (test_wave.c)"
echo "-----------------------------------------------------------------"
OUT_WAVE_ASAN="/tmp/snc_test_wave_c_asan"
rm -f "$OUT_WAVE_ASAN"
if clang -O1 -g -fsanitize=address -o "$OUT_WAVE_ASAN" "$DIR/test_wave.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$(ASAN_OPTIONS=detect_leaks=0 "$OUT_WAVE_ASAN" 2>&1)
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_wave(.c ASan): PASSED (no ASan reports over the slot table / staging buffers / the join's offer build+merge+release)"
    else
        echo ">>> test_wave(.c ASan): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_wave(.c ASan): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

# 22. The SFC layouts — A2's space-filling-geometry decision table's
#     runtime mechanics (SWARM RABBIT HOLE, Wave 14, Impl-S, Cohort
#     Delta; A2 §2.2/§2.4). The feature-dispatched interleave
#     (C2.8: PDEP/PEXT on x86-64 BMI2, the software shift-or path
#     elsewhere — verified against the bit-by-bit reference and the
#     forced-dispatch knob), the deinterleave bijection, the
#     Butz/Skilling hilbert loop (encode/decode round trip + the
#     vertex-continuous unit steps), the segment-advance box query vs
#     the linear-scan reference EXACTLY (128 seeded-LCG boxes + the
#     edge boxes), the O(1)-carry iteration over the real buffer, the
#     lane backing (zero GC objects moved — E-GC-IN-LANE), the
#     to_bytes zero-fixup round trip, and the [E-SFC] refusal
#     backstops (fork'd children: bounds, from_lex length, from_bytes
#     dims, insert order, duplicate point, absent point, out-of-grid
#     coordinate, the forced-bmi2 knob). Built twice: plain, then ASan
#     (22b).
echo ""
echo "-----------------------------------------------------------------"
echo "22. SFC LAYOUTS, RUNTIME MECHANICS (test_sfc.c)"
echo "-----------------------------------------------------------------"
OUT_SFC="/tmp/snc_test_sfc_c"
rm -f "$OUT_SFC"
if clang -O1 -g -o "$OUT_SFC" "$DIR/test_sfc.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$("$OUT_SFC")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_sfc(.c): PASSED"
    else
        echo ">>> test_sfc(.c): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_sfc(.c): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "-----------------------------------------------------------------"
echo "22b. SFC LAYOUTS, ASAN STRESS (test_sfc.c)"
echo "-----------------------------------------------------------------"
OUT_SFC_ASAN="/tmp/snc_test_sfc_c_asan"
rm -f "$OUT_SFC_ASAN"
if clang -O1 -g -fsanitize=address -o "$OUT_SFC_ASAN" "$DIR/test_sfc.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$(ASAN_OPTIONS=detect_leaks=0 "$OUT_SFC_ASAN" 2>&1)
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_sfc(.c ASan): PASSED (no ASan reports over the lane reservations / the blocked reshuffle / the box scans / the growth copies)"
    else
        echo ">>> test_sfc(.c ASan): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_sfc(.c ASan): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

# 23. Salt ε — the numeric contract layer's engine mechanics (SWARM
#     RABBIT HOLE, Wave 15, Impl-T, Cohort Delta; A4's Presburger
#     decidable ladder + the final report §7.3's boundary rule). THE
#     HONEST TWIN DISCIPLINE: the DBM engine lives in the checker
#     (compile-time only — the runtime never sees a zone); this C
#     twin is line-matched to compiler/src/dbm.rs and proves the
#     ALGORITHM's mathematical properties (the closure vs the
#     brute-force relax-to-fixpoint reference over randomized zones,
#     emptiness == a negative diagonal, ∩ exact by enumeration, the
#     join over-approximates both operands — sound, NOT exact; zones
#     are convex), the facts-table hit/miss, and the fork-probed
#     [E-0702] degrade fail-fast (the actual helper the degraded
#     atoms lower to). The Rust engine's agreement lives in the
#     dbm::tests unit tests; the real engine's decisions are pinned
#     by the golden suite (contract_*.sn). Built twice: plain (23),
#     then ASan (23b).
echo ""
echo "-----------------------------------------------------------------"
echo "23. SALT EPSILON CONTRACT ENGINE, MECHANICS (test_contract.c)"
echo "-----------------------------------------------------------------"
OUT_CONTRACT="/tmp/snc_test_contract_c"
rm -f "$OUT_CONTRACT"
if clang -O1 -g -o "$OUT_CONTRACT" "$DIR/test_contract.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$("$OUT_CONTRACT")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_contract(.c): PASSED"
    else
        echo ">>> test_contract(.c): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_contract(.c): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "-----------------------------------------------------------------"
echo "23b. SALT EPSILON CONTRACT ENGINE, ASAN STRESS (test_contract.c)"
echo "-----------------------------------------------------------------"
OUT_CONTRACT_ASAN="/tmp/snc_test_contract_c_asan"
rm -f "$OUT_CONTRACT_ASAN"
if clang -O1 -g -fsanitize=address -o "$OUT_CONTRACT_ASAN" "$DIR/test_contract.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$(ASAN_OPTIONS=detect_leaks=0 "$OUT_CONTRACT_ASAN" 2>&1)
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_contract(.c ASan): PASSED (no ASan reports over the closure loops / the enumeration boxes / the fork'd probes)"
    else
        echo ">>> test_contract(.c ASan): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_contract(.c ASan): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

# 24. The tape record kinds — W2C's P5↔P2 elevation plan's runtime
#     mechanics (SWARM RABBIT HOLE, Wave 16, Impl-U, Cohort Delta;
#     W2C §36–§38 + §48 item 4; the final report §332's
#     tier-parametric per-operation laws). THE HONEST WITNESS SET:
#     the ILC law a⊕(b⊖a)=b CHECKED at the effect boundary on every
#     recorded triple (200 seeded-LCG randomized Z-set chain steps —
#     every record succeeding IS the law verified on that triple);
#     the mutator law X ⊔ m^δ(X) = m(X) likewise on all three Tier-L
#     carriers (100 randomized monotone steps each); the replay ==
#     re-integration witness on randomized streams WITH mid-chain
#     checkpoints (the pre-checkpoint records excluded, both
#     tape shapes reaching the direct chain endpoint EXACTLY —
#     per-key counts on G, sn_lcell_eq on L; and the Tier-G replay
#     REPLACES a pre-loaded target, the exact reset); the opaque
#     refusal (an opaque record in the window has no integral);
#     and the one-tape-one-ledger / no-L-base / tier-confusion /
#     kind / bounds backstops (fork'd children dying with [E-TAPE]).
#     Built twice: plain (24), then ASan (24b).
echo ""
echo "-----------------------------------------------------------------"
echo "24. TAPE RECORD KINDS, RUNTIME MECHANICS (test_tape.c)"
echo "-----------------------------------------------------------------"
OUT_TAPE="/tmp/snc_test_tape_c"
rm -f "$OUT_TAPE"
if clang -O1 -g -o "$OUT_TAPE" "$DIR/test_tape.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$("$OUT_TAPE")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_tape(.c): PASSED"
    else
        echo ">>> test_tape(.c): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_tape(.c): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "-----------------------------------------------------------------"
echo "24b. TAPE RECORD KINDS, ASAN STRESS (test_tape.c)"
echo "-----------------------------------------------------------------"
OUT_TAPE_ASAN="/tmp/snc_test_tape_c_asan"
rm -f "$OUT_TAPE_ASAN"
if clang -O1 -g -fsanitize=address -o "$OUT_TAPE_ASAN" "$DIR/test_tape.c" "$ROOT/compiler/runtime.c" -lpthread 2>/dev/null; then
    OUTPUT=$(ASAN_OPTIONS=detect_leaks=0 "$OUT_TAPE_ASAN" 2>&1)
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_tape(.c ASan): PASSED (no ASan reports over the lane reservations' growth copies / the record directory / the per-record derive+check+flatten objects)"
    else
        echo ">>> test_tape(.c ASan): FAILED"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_tape(.c ASan): BUILD FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "================================================================="
if [ $FAILURES -eq 0 ]; then
    echo "  ALL SYSTEMS & SECURITY TESTS PASSED (39 checks)"
    exit 0
else
    echo "  $FAILURES SYSTEMS TESTS FAILED"
    exit 1
fi
