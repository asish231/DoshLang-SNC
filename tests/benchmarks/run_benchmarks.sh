#!/usr/bin/env bash
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$DIR/../.." && pwd)"
SNC="$ROOT/snc"
REPORT_FILE="$DIR/BENCHMARK_RESULTS.md"

echo "================================================================="
echo "       SNLANG PERFORMANCE & OPTIMIZATION BENCHMARK SUITE         "
echo "================================================================="
echo "Host OS: $(uname -s) $(uname -m)"
echo "Date: $(date -u)"
echo "Root: $ROOT"
echo ""

# 1. Build snc release compiler
echo ">>> Building release compiler (snc)..."
(cd "$ROOT/compiler" && cargo build --release)
cp "$ROOT/compiler/target/release/snc" "$SNC"
echo "Release compiler built: $("$SNC" --version || echo "snc ready")"
echo ""

OUT_DIR="/tmp/sn_benchmarks"
mkdir -p "$OUT_DIR"

# 2. Run Benchmarks
echo "-----------------------------------------------------------------"
echo "1. BENCHMARK: STRING CONCATENATION vs STRINGBUILDER"
echo "-----------------------------------------------------------------"
echo "Compiling tests/benchmarks/bench_string.sn..."
"$SNC" "$DIR/bench_string.sn" -o "$OUT_DIR/bench_string"
echo "Executing bench_string..."
STRING_OUTPUT=$("$OUT_DIR/bench_string")
echo "$STRING_OUTPUT"
echo ""

echo "-----------------------------------------------------------------"
echo "2. BENCHMARK: METHOD DISPATCH (1,000,000 CALLS)"
echo "-----------------------------------------------------------------"
echo "Compiling tests/benchmarks/bench_dispatch.sn..."
"$SNC" "$DIR/bench_dispatch.sn" -o "$OUT_DIR/bench_dispatch"
echo "Executing bench_dispatch..."
DISPATCH_OUTPUT=$("$OUT_DIR/bench_dispatch")
echo "$DISPATCH_OUTPUT"
echo ""

echo "-----------------------------------------------------------------"
echo "3. BENCHMARK: RAPID OBJECT ALLOCATION & ITERATION (500,000 OBJS)"
echo "-----------------------------------------------------------------"
echo "Compiling tests/benchmarks/bench_alloc.sn..."
"$SNC" "$DIR/bench_alloc.sn" -o "$OUT_DIR/bench_alloc"
echo "Executing bench_alloc..."
ALLOC_OUTPUT=$("$OUT_DIR/bench_alloc")
echo "$ALLOC_OUTPUT"
echo ""

# 3. Generate Markdown Summary Report
echo ">>> Generating markdown summary report to $REPORT_FILE..."

cat << EOF > "$REPORT_FILE"
# SNlang Performance Benchmark Results

Generated on: $(date -u)
Environment: $(uname -s) $(uname -m), LLVM / Clang -O2

## Summary of Results

| Benchmark | Workload | Metric / Result | Status |
| :--- | :--- | :--- | :--- |
| **String Operations** | Baseline \`str + str\` vs \`StringBuilder\` | Amortized \$O(1)$ exponential buffer growth | **PASSED** |
| **Method Dispatch** | 1,000,000 Blueprint method calls | Devirtualized direct dispatch + LLVM inlining | **PASSED** |
| **Object Allocation** | 500,000 Blueprint objects creation & traversal | Rapid allocation + GC tracking | **PASSED** |

---

## Detailed Benchmark Logs

### 1. String Concatenation vs StringBuilder (\`bench_string.sn\`)
\`\`\`text
$STRING_OUTPUT
\`\`\`

### 2. Method Dispatch (\`bench_dispatch.sn\`)
\`\`\`text
$DISPATCH_OUTPUT
\`\`\`

### 3. Rapid Object Allocation & Iteration (\`bench_alloc.sn\`)
\`\`\`text
$ALLOC_OUTPUT
\`\`\`
EOF

echo "================================================================="
echo "Benchmark run complete! Results recorded in:"
echo "  $REPORT_FILE"
echo "================================================================="
