/* pinned_lane_bench.c — SWARM RABBIT HOLE, Wave 8 / Impl-K (Cohort Delta).
 * The pinned-lane vs heap recursion benchmark, C twin: the identical
 * recursive inout-accumulator shape driven directly against the SNlang
 * runtime API on an ns clock (SNlang's own clock is ms-resolution; the
 * reclaim phases are sub-ms there, so the precise numbers live here).
 *
 * Shape (matches bench/pinned_lane_bench.sn): CHAINS rounds, each builds
 * ONE list of N = depth*per int elements grown by a recursive call chain
 * of depth D (per pushes per level, values level*per+k = the exact set
 * {0..N-1}; len/sum checksums must match the .sn twin). Element storage
 * is the shared pool-backed path in both variants (W2A §1.5 E-NESTED:
 * list_grow + the 512KB slab windows + malloc fallback are identical) —
 * only the header's construct and death differ:
 *   pinned: sn_list_new_lane(kind, 2)  (header = lane bump memory) …
 *           sn_lane_restore(2, wm)     (the O(1) reclaim: 1 store)
 *   heap:   sn_list_new(kind)          (xmalloc header + gc_track) …
 *           sn_release(l)              (gc_untrack + free(items) + free)
 *   heap_gc: the collector alternative — the same dead lists swept by one
 *           full precise-root pass (sn_gc_collect_roots, the path the
 *           language's gc_collect() intrinsic lowers to: mark + sweep,
 *           no stack scan).
 *
 * The recursive grow is non-tail by construction (a per-level local read
 * after the recursive call) in BOTH twins, so the real depth-D call chain
 * survives clang -O2 — verified in the disassembly of both binaries (the
 * naive void tail recursion and `return f(n-1)+1` are both optimized into
 * loops; reported in RESULTS_PINNED_LANE.md).
 *
 * Rows (median of RUNS runs after one CHAINS/10 warmup; driver):
 *   timer         — 100_000 clock pairs (the clock overhead baseline).
 *   pin           — the pinned-lane chains: elem_alloc_ns (construct +
 *                   recursive pushes), elem_read_ns (sum pass), and
 *                   chain_reclaim_ns (the per-chain lane_restore — the
 *                   key number: 1 store, independent of N).
 *   heap          — the identical shape on the malloc'd header:
 *                   the same three phases; reclaim = per-list sn_release.
 *   heap_gc       — one full collection over CHAINS dead heap lists.
 *   pin_resetpair — 1M save+restore pairs on lane 2 (the O(1) bound).
 *   pin_life      — 1M empty-list pinned construct+reclaim lifecycles.
 *   heap_life     — 1M empty-list heap construct+release lifecycles.
 *
 * Protocol: gc threshold pinned to 2e9 so no collection interrupts the
 * timed phases (the sn_heap_bench.sn control); single-threaded; the
 * deterministic checksums (len_total, sum_total, depth_total) are printed
 * and must be identical across runs and the .sn twin.
 *
 * Build (the driver, mirroring how snc links .sn programs):
 *   clang -O2 pinned_lane_bench.c ../compiler/runtime.c -pthread
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

/* The runtime API (compiler/runtime.c, linked in beside this file). */
void *sn_list_new(int64_t elem_kind);
void *sn_list_new_lane(int64_t elem_kind, int64_t lane_id);
void sn_list_push_i64(void *p, int64_t v);
int64_t sn_list_len(void *p);
int64_t sn_list_get_i64(void *p, int64_t i);
void sn_release(void *p);
void *sn_lane_save(int64_t lane_id);
void sn_lane_restore(int64_t lane_id, void *watermark);
int64_t sn_lane_used(int64_t lane_id);
void sn_gc_root_push(void *p);
void sn_gc_collect_roots(void);
int64_t sn_gc_object_count(void);
void sn_gc_set_threshold(int64_t n);

static int64_t now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t)ts.tv_sec * 1000000000 + (int64_t)ts.tv_nsec;
}

/* The recursive inout-accumulator twin of the .sn `grow`: one list, grown
 * across a call chain of depth n+1, per pushes per level. Non-tail by
 * construction (the per-level local `here` is read after the recursive
 * call, so the frame must live across it): clang -O2 tail-call-optimizes
 * void tail recursion AND `return f(n-1)+1` (accumulator rewrite) into
 * loops — both verified — while this shape keeps the real call chain.
 * Returns the depth checksum: sum of levels = 4950 per chain at D=100. */
static int64_t grow_rec(void *xs, int64_t n, int64_t per) {
    for (int64_t k = 0; k < per; k++) sn_list_push_i64(xs, n * per + k);
    int64_t here = n;
    int64_t r = 0;
    if (n > 0) r = grow_rec(xs, n - 1, per);
    return r + here;
}

int main(int argc, char **argv) {
    int64_t chains = 200;
    if (argc > 1) {
        int64_t v = atoll(argv[1]);
        if (v > 0) chains = v;
    }
    const int64_t depth = 100, per = 1000, n = depth * per;
    const int64_t elems = chains * n;
    sn_gc_set_threshold((int64_t)2000000000);

    void **ls = malloc(sizeof(void *) * (size_t)chains);
    void **wms = malloc(sizeof(void *) * (size_t)chains);
    if (!ls || !wms) return 1;

    /* ---- timer overhead: 1M clock pairs (the c_malloc_bench.c idiom) */
    int64_t t0 = now_ns(), acct = 0;
    for (int64_t k = 0; k < 1000000; k++) acct += now_ns();
    int64_t t1 = now_ns();
    printf("BENCH row=timer clock_ns=%lld ops=1000000 checksum=%lld\n",
           (long long)((t1 - t0) / 1000000), (long long)acct);

    /* ---- pinned-lane chains: header on lane 2, reclaim = lane_restore */
    int64_t lane_used_first = 0, depth_total = 0;
    t0 = now_ns();
    for (int64_t c = 0; c < chains; c++) {
        wms[c] = sn_lane_save(2);
        ls[c] = sn_list_new_lane(0, 2);
        depth_total += grow_rec(ls[c], depth - 1, per);
        if (c == 0) lane_used_first = sn_lane_used(2);
    }
    t1 = now_ns();
    int64_t len_total = 0, sum_total = 0;
    for (int64_t c = 0; c < chains; c++) {
        len_total += sn_list_len(ls[c]);
        for (int64_t j = 0; j < n; j++) sum_total += sn_list_get_i64(ls[c], j);
    }
    int64_t t2 = now_ns();
    for (int64_t c = chains - 1; c >= 0; c--) sn_lane_restore(2, wms[c]);
    int64_t t3 = now_ns();
    printf("BENCH row=pin elem_alloc_ns=%lld elem_read_ns=%lld "
           "chain_reclaim_ns=%lld chain_alloc_us=%.2f chains=%lld elems=%lld "
           "len_total=%lld sum_total=%lld depth_total=%lld\n",
           (long long)((t1 - t0) / elems), (long long)((t2 - t1) / elems),
           (long long)((t3 - t2) / chains),
           (double)(t1 - t0) / 1000.0 / (double)chains,
           (long long)chains, (long long)elems,
           (long long)len_total, (long long)sum_total, (long long)depth_total);
    printf("pin_lane_used_bytes=%lld\n", (long long)lane_used_first);

    /* ---- heap chains: identical shape, malloc'd header, reclaim = release */
    depth_total = 0;
    t0 = now_ns();
    for (int64_t c = 0; c < chains; c++) {
        ls[c] = sn_list_new(0);
        depth_total += grow_rec(ls[c], depth - 1, per);
    }
    t1 = now_ns();
    len_total = 0;
    sum_total = 0;
    for (int64_t c = 0; c < chains; c++) {
        len_total += sn_list_len(ls[c]);
        for (int64_t j = 0; j < n; j++) sum_total += sn_list_get_i64(ls[c], j);
    }
    t2 = now_ns();
    for (int64_t c = 0; c < chains; c++) sn_release(ls[c]);
    t3 = now_ns();
    printf("BENCH row=heap elem_alloc_ns=%lld elem_read_ns=%lld "
           "chain_reclaim_ns=%lld chain_alloc_us=%.2f chains=%lld elems=%lld "
           "len_total=%lld sum_total=%lld depth_total=%lld\n",
           (long long)((t1 - t0) / elems), (long long)((t2 - t1) / elems),
           (long long)((t3 - t2) / chains),
           (double)(t1 - t0) / 1000.0 / (double)chains,
           (long long)chains, (long long)elems,
           (long long)len_total, (long long)sum_total, (long long)depth_total);

    /* ---- heap_gc: one full collection over the same shape's dead lists.
     * The precise-root path the language's gc_collect() intrinsic uses
     * (ir.rs → sn_gc_collect_roots): no stack scan, and no roots pushed —
     * the lists are referenced only through a malloc'd array — so the
     * pass sweeps them all; deterministic, unlike the conservative
     * sn_gc_collect stack scan. */
    for (int64_t c = 0; c < chains; c++) {
        ls[c] = sn_list_new(0);
        grow_rec(ls[c], depth - 1, per);
    }
    int64_t g0 = sn_gc_object_count();
    t0 = now_ns();
    sn_gc_collect_roots();
    t1 = now_ns();
    int64_t g1 = sn_gc_object_count();
    printf("BENCH row=heap_gc gc_ns=%lld gc_objs_before=%lld "
           "gc_objs_after=%lld gc_freed=%lld gc_elems=%lld\n",
           (long long)(t1 - t0), (long long)g0, (long long)g1,
           (long long)(g0 - g1), (long long)elems);

    /* ---- pin_resetpair: 1M save+restore pairs on the pinned lane */
    t0 = now_ns();
    for (int64_t k = 0; k < 1000000; k++) {
        void *wm = sn_lane_save(2);
        sn_lane_restore(2, wm);
    }
    t1 = now_ns();
    printf("BENCH row=pin_resetpair pair_ns=%lld ops=1000000\n",
           (long long)((t1 - t0) / 1000000));

    /* ---- pin_life: 1M empty-list pinned construct+reclaim lifecycles.
     * The O(1) restore reclaims the header; the 32-byte items backing is
     * pool policy (E-NESTED) — slab windows until the 256 MB slab's 512
     * windows exhaust, then a leaked xmalloc per cycle. Honest cost of
     * the lane header lifecycle; RSS is reported by the driver. */
    int64_t acc = 0;
    t0 = now_ns();
    for (int64_t k = 0; k < 1000000; k++) {
        void *wm = sn_lane_save(2);
        void *l = sn_list_new_lane(0, 2);
        acc += (l != 0);
        sn_lane_restore(2, wm);
    }
    t1 = now_ns();
    printf("BENCH row=pin_life cycle_ns=%lld ops=1000000 checksum=%lld\n",
           (long long)((t1 - t0) / 1000000), (long long)acc);

    /* ---- heap_life: 1M empty-list heap construct+release lifecycles */
    int64_t acc2 = 0;
    t0 = now_ns();
    for (int64_t k = 0; k < 1000000; k++) {
        void *l = sn_list_new(0);
        acc2 += (l != 0);
        sn_release(l);
    }
    t1 = now_ns();
    printf("BENCH row=heap_life cycle_ns=%lld ops=1000000 checksum=%lld\n",
           (long long)((t1 - t0) / 1000000), (long long)acc2);

    printf("PINNED_LANE_BENCH_DONE\n");
    free(ls);
    free(wms);
    return 0;
}
