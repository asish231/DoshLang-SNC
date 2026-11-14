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
echo "================================================================="
if [ $FAILURES -eq 0 ]; then
    echo "  ALL SYSTEMS & SECURITY TESTS PASSED (13 checks)"
    exit 0
else
    echo "  $FAILURES SYSTEMS TESTS FAILED"
    exit 1
fi
