#!/usr/bin/env bash
set -uo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$DIR/../.." && pwd)"
SNC="$ROOT/snc"

echo "================================================================="
echo "  SNLANG PHASE 3 CONCURRENCY TEST SUITE"
echo "================================================================="
echo "Using compiler: $SNC"
echo ""

FAILURES=0

# 1. Structured Concurrency Nursery Test
echo "-----------------------------------------------------------------"
echo "1. STRUCTURED CONCURRENCY NURSERY (test_nursery.sn)"
echo "-----------------------------------------------------------------"
OUT_NURSERY="/tmp/snc_test_nursery"
rm -f "$OUT_NURSERY"
if "$SNC" "$DIR/test_nursery.sn" -o "$OUT_NURSERY"; then
    OUTPUT=$("$OUT_NURSERY")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_nursery: PASSED"
    else
        echo ">>> test_nursery: FAILED (output did not contain PASS)"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_nursery: COMPILATION FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 2. Move-on-Send Channels Test
echo "-----------------------------------------------------------------"
echo "2. MOVE-ON-SEND CHANNEL CHK (test_move_on_send.sn)"
echo "-----------------------------------------------------------------"
OUT_MOVE="/tmp/snc_test_move_on_send"
rm -f "$OUT_MOVE"
COMPILE_ERR=$("$SNC" "$DIR/test_move_on_send.sn" -o "$OUT_MOVE" 2>&1 || true)
echo "$COMPILE_ERR"
if echo "$COMPILE_ERR" | grep -qi "moved"; then
    echo ">>> test_move_on_send: PASSED (compiler rejected use of moved value as expected)"
elif [ ! -f "$OUT_MOVE" ]; then
    echo ">>> test_move_on_send: PASSED (compiler rejected invalid program)"
else
    echo ">>> test_move_on_send: FAILED (compiled successfully when rejection was expected)"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 3. Channel Close Non-Crashing Recv Test
echo "-----------------------------------------------------------------"
echo "3. CHANNEL CLOSE & RECV OK (test_chan_close.sn)"
echo "-----------------------------------------------------------------"
OUT_CLOSE="/tmp/snc_test_chan_close"
rm -f "$OUT_CLOSE"
if "$SNC" "$DIR/test_chan_close.sn" -o "$OUT_CLOSE"; then
    OUTPUT=$("$OUT_CLOSE")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> test_chan_close: PASSED"
    else
        echo ">>> test_chan_close: FAILED (output did not contain PASS)"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> test_chan_close: COMPILATION FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""

# 4. Spawning 10,000+ Green Tasks Benchmark
echo "-----------------------------------------------------------------"
echo "4. WORK-STEALING POOL SCALE BENCHMARK (bench_spawn_scale.sn)"
echo "-----------------------------------------------------------------"
OUT_BENCH="/tmp/snc_bench_spawn_scale"
rm -f "$OUT_BENCH"
if "$SNC" "$DIR/bench_spawn_scale.sn" -o "$OUT_BENCH"; then
    OUTPUT=$("$OUT_BENCH")
    echo "$OUTPUT"
    if echo "$OUTPUT" | grep -q "PASS:"; then
        echo ">>> bench_spawn_scale: PASSED"
    else
        echo ">>> bench_spawn_scale: FAILED (output did not contain PASS)"
        FAILURES=$((FAILURES + 1))
    fi
else
    echo ">>> bench_spawn_scale: COMPILATION FAILED"
    FAILURES=$((FAILURES + 1))
fi

echo ""
echo "================================================================="
if [ $FAILURES -eq 0 ]; then
    echo "  ALL CONCURRENCY TESTS PASSED (0 failures)"
    echo "================================================================="
    exit 0
else
    echo "  CONCURRENCY TESTS FAILED ($FAILURES failures)"
    echo "================================================================="
    exit 1
fi
