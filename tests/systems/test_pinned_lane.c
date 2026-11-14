/*
 * Systems Test: Pinned-Lane Accumulator Headers (SWARM RABBIT HOLE, Wave 8)
 * ===========================================================================
 *
 * Proves, at the runtime level, the memory mechanics of the pinned-lane
 * coloring (Impl-I's sn_list_new_lane, commit e1f3b72) that the Wave-8
 * benchmark measured (Impl-K, bench/RESULTS_PINNED_LANE.md):
 *
 *   1. The pinned header is lane bump memory: sn_list_new_lane(0, 2)
 *      allocates exactly sizeof(List) = 48 B on lane 2 — the whole
 *      on-lane footprint (W2A §1.3 S6: the accumulator's allocation site
 *      is colored onto the pinned lane at compile time).
 *   2. E-NESTLED (W2A §1.5): element storage is pool-backed (the slab
 *      window), NOT lane bytes — 1,000 pushes leave lane_used(2) at 48.
 *   3. The O(1) SCC-exit reclaim: sn_lane_restore(2, wm) is ONE store;
 *      the header dies with the lane rewind (5 ns measured vs 240 ns
 *      for the heap twin's sn_release — bench/RESULTS_PINNED_LANE.md),
 *      and T-FIBR re-poisons the freed range 0xAA. SN_POISON=1 is set
 *      below, so this run also proves the pinned path is poison-clean
 *      end to end.
 *   4. The bump discipline: after the restore the SAME address is handed
 *      out again for the next pinned list (fresh-lane reuse).
 *   5. The feature's real shape: a recursive inout-accumulator (grow
 *      across a depth-10 chain — the bench's non-TCO shape) whose
 *      header survives the whole chain and dies once at the end.
 *   6. The batch reclaim: 32 pinned headers die with ONE restore (the
 *      whole 32×48 B range, one store) — the SCC-exit invariant.
 *   7. The GC-invisibility invariant (the runtime half of E-GC-IN-LANE,
 *      D0 gap #6): pinned headers are NEVER gc_track'd —
 *      sn_gc_object_count() is unchanged across the entire pinned
 *      lifecycle — while the heap twin's sn_list_new headers ARE
 *      tracked one-by-one and untracked at sn_release. The two memory
 *      spaces cannot reference each other safely BECAUSE the runtime
 *      keeps them separate; the checker's E-GC-IN-LANE rule (Impl-L,
 *      commit a307681) is the compile-time half of that contract.
 *
 * The runner (tests/systems/run_all.sh) builds this twice: plain, and
 * with -fsanitize=address (the ASan stress pass — no UAF, no reports
 * over the lane restore/reuse cycle; LSan is not supported on arm64
 * darwin, so the pass runs with detect_leaks=0 per the house pattern).
 * No `leaks(1)` check is needed: the pinned path performs ZERO mallocs
 * (headers are lane mmap bytes, elements are slab-window bytes); the
 * heap twin's headers are all sn_release'd.
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* compiler/runtime.c public surface (compiled together with this file) */
void sn_rt_init(int argc, char **argv);
void *sn_list_new(int64_t elem_kind);
void *sn_list_new_lane(int64_t elem_kind, int64_t lane_id);
void sn_list_push_i64(void *p, int64_t v);
int64_t sn_list_len(void *p);
int64_t sn_list_get_i64(void *p, int64_t i);
void *sn_lane_save(int64_t lane_id);
void sn_lane_restore(int64_t lane_id, void *watermark);
int64_t sn_lane_used(int64_t lane_id);
void sn_release(void *p);
int64_t sn_gc_object_count(void);
void sn_gc_set_threshold(int64_t n);

static void ck(int cond, const char *what) {
    if (!cond) {
        fprintf(stderr, "FAIL: %s\n", what);
        exit(1);
    }
}

/* The recursive inout-accumulator twin of the bench's grow_rec: pushes
 * `per` elements per level down a depth-(n+1) chain; reads a per-level
 * local AFTER the recursive call (non-tail by construction — the shape
 * clang cannot TCO) and returns the depth checksum (sum of levels). */
static int64_t grow_rec(void *xs, int64_t n, int64_t per) {
    for (int64_t k = 0; k < per; k++) sn_list_push_i64(xs, n * per + k);
    int64_t here = n;
    int64_t r = 0;
    if (n > 0) r = grow_rec(xs, n - 1, per);
    return r + here;
}

int main(void) {
    /* T-FIBR on for this whole run: the 0xAA alloc fill and the rewind
       re-poison both execute, proving the pinned path poison-clean. */
    setenv("SN_POISON", "1", 1);
    sn_rt_init(0, NULL);
    sn_gc_set_threshold((int64_t)2000000000); /* keep the GC passive (the bench control) */

    /* 1. Fresh pinned lane (the compiler colors L2..L6; this test uses 2) */
    ck(sn_lane_used(2) == 0, "lane 2 not fresh at start");
    int64_t base_count = sn_gc_object_count();

    /* 2. The header is lane bump memory; elements are NOT (E-NESTLED) */
    void *wm = sn_lane_save(2);
    void *l = sn_list_new_lane(0, 2);
    ck(l != NULL, "sn_list_new_lane returned NULL");
    ck(sn_lane_used(2) == 48, "pinned header footprint != sizeof(List)=48B");
    int64_t sum = 0;
    for (int64_t i = 0; i < 1000; i++) {
        int64_t v = i * 7 + 1;
        sn_list_push_i64(l, v);
        sum += v;
    }
    ck(sn_list_len(l) == 1000, "pinned list length != 1000");
    ck(sn_lane_used(2) == 48, "elements leaked onto the lane (E-NESTLED violated)");
    int64_t reread = 0;
    for (int64_t i = 0; i < 1000; i++) reread += sn_list_get_i64(l, i);
    ck(reread == sum, "pinned element checksum mismatch");

    /* 3. The O(1) reclaim — one store — then the T-FIBR re-poison proof */
    ck(sn_gc_object_count() == base_count, "pinned header was gc_track'd (must be GC-invisible)");
    sn_lane_restore(2, wm);
    ck(sn_lane_used(2) == 0, "restore did not return the watermark (base)");
    ck(((uint8_t *)l)[0] == 0xAA, "T-FIBR rewind re-poison missing (expected 0xAA)");

    /* 4. The bump discipline: the same bytes are handed out again */
    void *l2 = sn_list_new_lane(0, 2);
    ck(l2 == l, "bump discipline: restored range not reused at the same address");
    ck(sn_list_len(l2) == 0, "reused pinned header not fresh (len != 0)");
    sn_list_push_i64(l2, 42);
    ck(sn_list_get_i64(l2, 0) == 42, "reused pinned list push/read mismatch");
    sn_lane_restore(2, wm);
    ck(sn_lane_used(2) == 0, "second restore did not return to base");

    /* 5. The feature's real shape: the recursive inout accumulator */
    void *acc = sn_list_new_lane(0, 2);
    ck(sn_lane_used(2) == 48, "accumulator header footprint != 48B");
    int64_t depth_sum = grow_rec(acc, 9, 100); /* depth-10 chain, 1,000 pushes */
    ck(depth_sum == 45, "depth checksum != 45 (sum of levels 0..9)");
    ck(sn_list_len(acc) == 1000, "accumulator lost elements across the chain");
    int64_t acc_sum = 0;
    for (int64_t i = 0; i < 1000; i++) acc_sum += sn_list_get_i64(acc, i);
    /* elements are n*100+k over n=0..9, k=0..99 → 10000*45 + 10*4950 */
    ck(acc_sum == 499500, "accumulator element checksum mismatch");
    sn_lane_restore(2, wm);
    ck(sn_lane_used(2) == 0, "accumulator reclaim did not return to base");

    /* 6. The batch reclaim: 32 headers die with ONE restore */
    void *ls[32];
    for (int i = 0; i < 32; i++) {
        ls[i] = sn_list_new_lane(0, 2);
        ck(ls[i] != NULL, "batch pinned header alloc failed");
        sn_list_push_i64(ls[i], i);
        ck(sn_lane_used(2) == (int64_t)(i + 1) * 48, "batch watermark drift");
    }
    ck(sn_gc_object_count() == base_count, "batch pinned headers must stay GC-invisible");
    sn_lane_restore(2, wm); /* ONE store — all 32 die */
    ck(sn_lane_used(2) == 0, "batch reclaim did not return to base");

    /* 7. The heap twin: tracked one-by-one, untracked at release */
    for (int i = 0; i < 32; i++) {
        void *h = sn_list_new(0);
        ck(sn_gc_object_count() == base_count + i + 1, "heap header not gc_track'd");
        sn_list_push_i64(h, i);
        ck(sn_list_len(h) == 1, "heap list push mismatch");
        ck(sn_list_get_i64(h, 0) == i, "heap list read mismatch");
        ls[i] = h;
    }
    for (int i = 0; i < 32; i++) {
        sn_release(ls[i]);
        ck(sn_gc_object_count() == base_count + 31 - i, "heap release untrack mismatch");
    }
    ck(sn_gc_object_count() == base_count, "heap twin left the GC count changed");

    printf("PASS: pinned-lane headers — O(1) SCC-exit reclaim, batch 32/1-store, GC-invisible\n");
    return 0;
}
