#!/usr/bin/env bash
set -uo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$DIR/../.." && pwd)"
SNC="$ROOT/snc"

echo "================================================================="
echo "  MEMORY SAFETY EXPLOIT TEST SUITE: RUST vs SNLANG BASELINE"
echo "================================================================="

echo ""
echo "-----------------------------------------------------------------"
echo "1. STACK ESCAPE"
echo "-----------------------------------------------------------------"
echo ">>> Running rustc tests/memory_safety/stack_escape.rs (expects compile error):"
if rustc "$DIR/stack_escape.rs" -o /tmp/rust_stack_escape 2>&1; then
    echo "UNEXPECTED: Rust compilation succeeded!"
else
    echo "PASS: Rust rejected stack reference escape at compile time."
fi

echo ""
echo ">>> Running ./snc tests/memory_safety/stack_escape.sn (baseline behavior):"
"$SNC" "$DIR/stack_escape.sn" -o /tmp/sn_stack_escape && /tmp/sn_stack_escape

echo ""
echo "-----------------------------------------------------------------"
echo "2. LIST INVALIDATION DURING BORROW"
echo "-----------------------------------------------------------------"
echo ">>> Running rustc tests/memory_safety/list_invalidation.rs (expects compile error):"
if rustc "$DIR/list_invalidation.rs" -o /tmp/rust_list_invalidation 2>&1; then
    echo "UNEXPECTED: Rust compilation succeeded!"
else
    echo "PASS: Rust rejected mutation during borrow at compile time."
fi

echo ""
echo ">>> Running ./snc tests/memory_safety/list_invalidation.sn (baseline behavior):"
"$SNC" "$DIR/list_invalidation.sn" -o /tmp/sn_list_invalidation && /tmp/sn_list_invalidation

echo ""
echo "-----------------------------------------------------------------"
echo "3. HEAP USE-AFTER-FREE & DOUBLE-FREE"
echo "-----------------------------------------------------------------"
echo ">>> Running rustc tests/memory_safety/heap_uaf.rs (expects compile error):"
if rustc "$DIR/heap_uaf.rs" -o /tmp/rust_heap_uaf 2>&1; then
    echo "UNEXPECTED: Rust compilation succeeded!"
else
    echo "PASS: Rust rejected UAF and double-free via affine types at compile time."
fi

echo ""
echo ">>> Running ./snc tests/memory_safety/heap_uaf.sn (baseline behavior):"
"$SNC" "$DIR/heap_uaf.sn" -o /tmp/sn_heap_uaf && /tmp/sn_heap_uaf || true

echo ""
echo "-----------------------------------------------------------------"
echo "4. CONCURRENCY DATA RACE IN SPAWN"
echo "-----------------------------------------------------------------"
echo ">>> Running rustc tests/memory_safety/spawn_race.rs (expects compile error):"
if rustc "$DIR/spawn_race.rs" -o /tmp/rust_spawn_race 2>&1; then
    echo "UNEXPECTED: Rust compilation succeeded!"
else
    echo "PASS: Rust rejected data race across thread spawn at compile time."
fi

echo ""
echo ">>> Running ./snc tests/memory_safety/spawn_race.sn (baseline behavior):"
"$SNC" "$DIR/spawn_race.sn" -o /tmp/sn_spawn_race && /tmp/sn_spawn_race

echo ""
echo "-----------------------------------------------------------------"
echo "5. REFERENCE CYCLE LEAK (RUST) vs CYCLE COLLECTION (SNLANG)"
echo "-----------------------------------------------------------------"
echo ">>> Compiling and running rustc tests/memory_safety/cycle_leak.rs:"
rustc "$DIR/cycle_leak.rs" -o /tmp/rust_cycle_leak && /tmp/rust_cycle_leak

echo ""
echo ">>> Compiling and running ./snc tests/memory_safety/cycle_collect.sn:"
"$SNC" "$DIR/cycle_collect.sn" -o /tmp/sn_cycle_collect && /tmp/sn_cycle_collect

echo ""
echo "================================================================="
echo "  SUMMARY OF COMPARISON COMPLETED"
echo "================================================================="
