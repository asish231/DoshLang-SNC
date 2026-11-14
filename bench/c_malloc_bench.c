/*
 * C malloc/calloc twin — SWARM RABBIT HOLE, Impl-C (Cohort Delta, Wave 5).
 * D0 §7 #3: malloc/free loop with the same size mix, checksum work, scratch
 * store, and read pass as bench/sn_lane_bench.sn. Compiled with the same
 * clang at -O2 (the snc driver compiles its .ll + runtime.c with -O2 too).
 *
 * Rows (identical protocol across all five programs):
 *   timer      — one clock_gettime(CLOCK_MONOTONIC) pair, ns (K=1,000,000)
 *   emptyloop  — N iterations of acc++ : the loop tax every other row pays
 *   amort16    — malloc(16) x N into a scratch pointer array, checksum via
 *                pointer sums; read pass = one 8-byte load per record;
 *                reset = the free loop, timed separately (malloc has NO
 *                wholesale reset — the per-object cost is the honest number)
 *   amort16z   — the same via calloc(16): the zeroing twin (memset asymmetry
 *                control; on macOS large calloc runs come pre-zeroed from the
 *                kernel, the small-quanta ones are memset — this row reports
 *                what real code gets)
 *   fresh16    — malloc(16)+free per iteration (lane's region-per-iteration
 *                analog; the asymmetry: bump reset is O(1) wholesale, free is
 *                per-object — stated, not hidden)
 *   fresh16z   — calloc(16)+free per iteration
 *   fresh4k    — malloc(4096)+free per iteration
 *   freshlcg   — sizes 16..63 from the fixed LCG, malloc+free per iteration
 *
 * Held fixed vs the twins: N (argv[1], default 10,000,000), 16-byte records,
 * LCG constants (state=(state*1103515245+12345) mod 2^31, seed 12345, draw
 * BEFORE advance), checksum work, one scratch store per op inside the alloc
 * phase, one payload load per record in the read pass, no payload stores in
 * raw rows, single-threaded. Peak RSS via getrusage + /usr/bin/time -l.
 *
 * Valgrind/cachegrind note: this machine is Apple Silicon; valgrind has no
 * darwin27/arm64 bottle and does not build (see bench/RESULTS.md). The
 * cachegrind rows are wired in run_bench.sh for Linux; here the memory-side
 * story comes from the time rows plus the documented first-touch constant.
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <time.h>
#include <sys/resource.h>
#include <malloc/malloc.h> /* malloc_good_size (macOS) */

static long long now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (long long)ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

static void report(const char *row, long long alloc_ns, long long read_ns,
                   long long reset_ns, long long ops, long long ck) {
    printf("BENCH row=%s alloc_ns=%lld read_ns=%lld reset_ns=%lld ops=%lld checksum=%lld\n",
           row, alloc_ns, read_ns, reset_ns, ops, ck);
}

static long long rss_kb(void) {
    struct rusage ru;
    getrusage(RUSAGE_SELF, &ru);
    return ru.ru_maxrss; /* KB on darwin */
}

int main(int argc, char **argv) {
    long long n = 10000000;
    if (argc > 1 && atoll(argv[1]) > 0) n = atoll(argv[1]);

    void **scratch = malloc((size_t)n * sizeof(void *));
    if (!scratch) { fprintf(stderr, "scratch alloc failed\n"); return 1; }

    /* warmup: n/100 mallocs + frees (icache + branch state; amortized rows
       still fault their big ranges inside the timed region — same in twins) */
    for (long long i = 0; i < n / 100; i++) { void *p = malloc(16); free(p); }

    /* ---- timer overhead: 1M clock pairs */
    long long t0 = now_ns(), acct = 0;
    for (int i = 0; i < 1000000; i++) acct += now_ns();
    long long t1 = now_ns();
    report("timer", (t1 - t0) / 1000000, 0, 0, 1000000, acct);

    /* ---- empty loop tax */
    t0 = now_ns();
    long long acc0 = 0;
    for (long long i = 0; i < n; i++) acc0 += 1;
    t1 = now_ns();
    report("emptyloop", (t1 - t0) / n, 0, 0, n, acc0);

    /* ---- amort16: malloc all, read, free all (free timed separately) */
    t0 = now_ns();
    long long acc1 = 0;
    for (long long i = 0; i < n; i++) {
        void *p = malloc(16);
        scratch[i] = p;
        acc1 += (long long)(intptr_t)p;
    }
    t1 = now_ns();
    long long sum1 = 0;
    for (long long i = 0; i < n; i++) sum1 += *(long long *)scratch[i];
    long long t2 = now_ns();
    for (long long i = 0; i < n; i++) free(scratch[i]);
    long long t3 = now_ns();
    report("amort16", (t1 - t0) / n, (t2 - t1) / n, (t3 - t2) / n, n, acc1 + sum1);

    /* ---- amort16z: calloc all (zeroing twin), read, free all */
    t0 = now_ns();
    long long acc2 = 0;
    for (long long i = 0; i < n; i++) {
        void *p = calloc(1, 16);
        scratch[i] = p;
        acc2 += (long long)(intptr_t)p;
    }
    t1 = now_ns();
    long long sum2 = 0;
    for (long long i = 0; i < n; i++) sum2 += *(long long *)scratch[i];
    t2 = now_ns();
    for (long long i = 0; i < n; i++) free(scratch[i]);
    t3 = now_ns();
    report("amort16z", (t1 - t0) / n, (t2 - t1) / n, (t3 - t2) / n, n, acc2 + sum2);

    /* ---- fresh16: malloc+free per iteration */
    t0 = now_ns();
    long long acc3 = 0;
    for (long long i = 0; i < n; i++) {
        void *p = malloc(16);
        acc3 += (long long)(intptr_t)p;
        free(p);
    }
    t1 = now_ns();
    report("fresh16", (t1 - t0) / n, 0, 0, n, acc3);

    /* ---- fresh16z: calloc+free per iteration */
    t0 = now_ns();
    long long acc4 = 0;
    for (long long i = 0; i < n; i++) {
        void *p = calloc(1, 16);
        acc4 += (long long)(intptr_t)p;
        free(p);
    }
    t1 = now_ns();
    report("fresh16z", (t1 - t0) / n, 0, 0, n, acc4);

    /* ---- fresh4k: malloc(4096)+free per iteration */
    t0 = now_ns();
    long long acc5 = 0;
    for (long long i = 0; i < n; i++) {
        void *p = malloc(4096);
        acc5 += (long long)(intptr_t)p;
        free(p);
    }
    t1 = now_ns();
    report("fresh4k", (t1 - t0) / n, 0, 0, n, acc5);

    /* ---- freshlcg: irregular sizes from the fixed LCG (identical sequence
       to the SNlang/Rust/Go twins; state verified via lcg_state_final) */
    uint32_t st = 12345;
    t0 = now_ns();
    long long acc6 = 0;
    for (long long i = 0; i < n; i++) {
        size_t sz = 16 + (size_t)(st % 48u);
        void *p = malloc(sz);
        acc6 += (long long)(intptr_t)p;
        free(p);
        st = (st * 1103515245u + 12345u) & 0x7fffffffu;
    }
    t1 = now_ns();
    report("freshlcg", (t1 - t0) / n, 0, 0, n, acc6);
    printf("lcg_state_final=%u\n", st);

    free(scratch);
    printf("rss_kb=%lld\n", rss_kb());
    printf("malloc_good_size_16=%zu\n", malloc_good_size(16));
    printf("malloc_good_size_4096=%zu\n", malloc_good_size(4096));
    printf("C_BENCH_DONE\n");
    return 0;
}
