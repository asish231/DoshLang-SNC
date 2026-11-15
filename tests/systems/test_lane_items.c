/*
 * Systems Test: The Lane-Aware Items Pool (SWARM RABBIT HOLE, Wave 12)
 * ===========================================================================
 *
 * Checks 20/20b (tests/systems/run_all.sh; the suite's tally 29 -> 31).
 *
 * Closes the honest caveat Impl-K's Wave-8 benchmark pinned
 * (bench/RESULTS_PINNED_LANE.md): the 1-store lane restore
 * (sn_lane_restore) reclaimed the pinned list HEADER (48 B lane bump
 * memory) but NOT the GROWN items arrays. Each list gets one 512 KB
 * slab window (65,536 elements, W2A §1.5 E-NESTLED: element storage is
 * pool-backed, not lane bytes); past the window list_grow mallocs a
 * bigger array and abandons the window — and a lane-backed header has
 * NO release path (lane bytes die at the rewind, never free()), so at
 * N=100K per chain ~1 MB leaked per chain (the grown 1 MB array; the
 * abandoned window's touched pages stay resident by slab design).
 *
 * The Wave-12 fix (Impl-Q): a per-lane registry of lane-backed list
 * headers (lane_grown_register at sn_list_new_lane); sn_lane_restore
 * walks it and frees the items array of every header inside the
 * rewound range, gated on !list_is_slab_backed.
 *
 * RECLAIM INVARIANT (asserted throughout below): a lane-backed list's
 * header dies at the rewind that covers it, and the SAME rewind frees
 * its grown items array. No ref to lane memory can escape
 * (E-GC-IN-LANE), so the header and its array are unreachable at the
 * rewind: freeing both there is safe and free of double frees. Slab
 * windows (lists under 65,536 elements) are NEVER freed — they are
 * carved from the shared 256 MB slab, not malloc'd.
 *
 * This run proves, at the runtime level:
 *   (a) grown arrays are reclaimed: sn_lane_items_freed() advances by
 *       exactly one per grown list whose header the rewind covers, and
 *       the registry drains (sn_lane_lists_live() back to 0);
 *   (b) the header reclaim is still the O(1) 1-store rewind (every
 *       restore returns lane_used to its watermark), now composed with
 *       the T-FIBR re-poison (SN_POISON=1: the registry walk reads the
 *       headers BEFORE the poison fill — the 0xAA proof below);
 *   (c) lists UNDER the window (slab-backed) are NOT freed at rewind —
 *       the freed counter stays 0 across their whole lifecycle (the
 *       slab window is reused, not freed; ASan would flag a free of
 *       non-malloc slab bytes in the 20b pass);
 *   (d) nested watermarks: an outer-watermark list survives an inner
 *       rewind and dies at the rewind that covers IT;
 *   (e) heap-backed lists (sn_list_new) never enter the registry: their
 *       grown arrays die at sn_release, not at any lane rewind.
 *
 * Also carries the release-side proof of the Wave-12 push_ptr pointee
 * fix (Impl-J's flag): a borrow-kind (elem_kind 2) list of stack-slot
 * refs releases WITHOUT touching the slots (sn_release's element loop
 * is owned-kind-only), while an owned-kind (elem_kind 1) list of
 * tracked string objects still releases every element (no
 * over-correction — the legitimate ARC use).
 *
 * The runner builds this twice: plain, then -fsanitize=address (check
 * 20b: no reports over the grown-array free / nested rewinds / slab
 * non-frees; LSan is unsupported on arm64 darwin, detect_leaks=0 per
 * the house pattern — the freed-array accounting is the counter's job).
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
void sn_list_push_ptr(void *p, void *v);
int64_t sn_list_len(void *p);
int64_t sn_list_get_i64(void *p, int64_t i);
void *sn_lane_save(int64_t lane_id);
void sn_lane_restore(int64_t lane_id, void *watermark);
int64_t sn_lane_used(int64_t lane_id);
void sn_release(void *p);
int64_t sn_gc_object_count(void);
void sn_gc_set_threshold(int64_t n);
/* Wave 12 (Impl-Q): the lane-aware items pool's test surface */
int64_t sn_lane_items_freed(void);
int64_t sn_lane_lists_live(int64_t lane_id);
/* a tracked object to push into the owned-kind list (release-side proof) */
void *sn_str_from_cstr(const char *s);

static void ck(int cond, const char *what) {
    if (!cond) {
        fprintf(stderr, "FAIL: %s\n", what);
        exit(1);
    }
}

/* Pushes n values v = i*7+1 and returns their sum (the read-back checksum). */
static int64_t fill(void *l, int64_t n) {
    int64_t sum = 0;
    for (int64_t i = 0; i < n; i++) {
        int64_t v = i * 7 + 1;
        sn_list_push_i64(l, v);
        sum += v;
    }
    return sum;
}

static int64_t reread(void *l, int64_t n) {
    int64_t sum = 0;
    for (int64_t i = 0; i < n; i++) sum += sn_list_get_i64(l, i);
    return sum;
}

int main(void) {
    /* T-FIBR on for the whole run, the test_pinned_lane pattern: the
       registry walk must read the headers BEFORE the rewind re-poison. */
    setenv("SN_POISON", "1", 1);
    sn_rt_init(0, NULL);
    sn_gc_set_threshold((int64_t)2000000000); /* keep the GC passive */

    /* ---- 0. fresh lane, zero reclaim history ---- */
    ck(sn_lane_used(2) == 0, "lane 2 not fresh at start");
    ck(sn_lane_lists_live(2) == 0, "lane 2 registry not empty at start");
    ck(sn_lane_items_freed() == 0, "items freed counter not 0 at start");

    /* ---- 1. fix-1 release side: the pointee retention law ---- */
    {
        /* Borrow-kind (2) list of stack-slot refs: sn_release must not
           touch the slots (the old elem loop treated &x as an Obj
           header and decremented the target's value). */
        int64_t x = 99, y = -5, z = 1;
        void *borrows = sn_list_new(2);
        sn_list_push_ptr(borrows, &x);
        sn_list_push_ptr(borrows, &y);
        sn_list_push_ptr(borrows, &z);
        ck(sn_list_len(borrows) == 3, "borrow list length != 3");
        sn_release(borrows);
        ck(x == 99 && y == -5 && z == 1, "borrow-kind release touched the target slots");

        /* Owned-kind (1) list of tracked objects: the legitimate ARC —
           every element is released at list death (the GC count returns
           to base; no over-correction). */
        int64_t base = sn_gc_object_count();
        void *owned = sn_list_new(1);
        for (int i = 0; i < 8; i++) {
            void *s = sn_str_from_cstr("lane-items");
            sn_list_push_ptr(owned, s); /* the list's reference (retain) */
            sn_release(s);              /* the creator's reference */
        }
        ck(sn_gc_object_count() == base + 1 + 8, "owned list + 8 strings not all tracked");
        sn_release(owned);
        ck(sn_gc_object_count() == base, "owned-kind release left tracked objects behind");
    }

    /* ---- 2. (c) under-window slab lists are NOT freed at rewind ---- */
    {
        void *wm = sn_lane_save(2);
        int64_t base_used = sn_lane_used(2);
        for (int i = 0; i < 4; i++) {
            void *l = sn_list_new_lane(0, 2);
            int64_t sum = fill(l, 10000); /* 10,000 < 65,536: slab-backed */
            ck(reread(l, 10000) == sum, "under-window lane list checksum mismatch");
            ck(sn_lane_lists_live(2) == i + 1, "under-window list not registered");
        }
        sn_lane_restore(2, wm);
        ck(sn_lane_used(2) == base_used, "under-window restore missed the watermark");
        ck(sn_lane_lists_live(2) == 0, "under-window rewind left registry entries");
        ck(sn_lane_items_freed() == 0, "slab window was freed at rewind (must never be)");
    }

    /* ---- 3. (a) grown arrays are freed at the rewind ---- */
    {
        void *wm = sn_lane_save(2);
        int64_t base_used = sn_lane_used(2);
        for (int c = 0; c < 3; c++) {
            void *l = sn_list_new_lane(0, 2);
            int64_t sum = fill(l, 70000); /* > 65,536: window -> malloc */
            ck(sn_list_len(l) == 70000, "grown lane list length != 70000");
            ck(reread(l, 70000) == sum, "grown lane list checksum mismatch");
        }
        ck(sn_lane_lists_live(2) == 3, "grown lists not registered");
        int64_t freed_before = sn_lane_items_freed();
        sn_lane_restore(2, wm);
        ck(sn_lane_used(2) == base_used, "grown restore missed the watermark");
        ck(sn_lane_items_freed() == freed_before + 3, "grown arrays not freed at rewind (x3)");
        ck(sn_lane_lists_live(2) == 0, "grown rewind left registry entries");
    }

    /* ---- 4. (d) nested watermarks: the covering rewind owns the free ---- */
    {
        int64_t freed0 = sn_lane_items_freed();
        void *wm1 = sn_lane_save(2);
        void *outer = sn_list_new_lane(0, 2);
        (void)fill(outer, 70000); /* grown under the OUTER watermark */
        void *wm2 = sn_lane_save(2);
        void *inner = sn_list_new_lane(0, 2);
        (void)fill(inner, 70000); /* grown under the INNER watermark */
        ck(sn_lane_lists_live(2) == 2, "nested lists not both registered");
        sn_lane_restore(2, wm2); /* inner rewind: only the inner list dies */
        ck(sn_lane_items_freed() == freed0 + 1, "inner rewind freed the wrong count");
        ck(sn_lane_lists_live(2) == 1, "inner rewind lost the outer list");
        sn_lane_restore(2, wm1); /* outer rewind: the outer list dies */
        ck(sn_lane_items_freed() == freed0 + 2, "outer rewind freed the wrong count");
        ck(sn_lane_lists_live(2) == 0, "outer rewind left registry entries");
    }

    /* ---- 5. (e) heap-backed lists never enter the registry ---- */
    {
        int64_t base = sn_gc_object_count();
        int64_t freed0 = sn_lane_items_freed();
        void *h = sn_list_new(0);
        (void)fill(h, 70000); /* grown heap list: same list_grow, no lane */
        sn_release(h);
        ck(sn_gc_object_count() == base, "heap list release left the GC count changed");
        ck(sn_lane_items_freed() == freed0, "heap list reclaim went through the lane registry");
        ck(sn_lane_lists_live(2) == 0, "heap list entered the lane registry");
    }

    /* ---- 6. the bench shape: one N=100K chain, reclaim end to end ---- */
    {
        void *wm = sn_lane_save(2);
        int64_t base_used = sn_lane_used(2);
        void *l = sn_list_new_lane(0, 2);
        int64_t sum = fill(l, 100000);
        ck(sn_list_len(l) == 100000, "bench-shape chain length != 100000");
        ck(reread(l, 100000) == sum, "bench-shape chain checksum mismatch");
        int64_t freed0 = sn_lane_items_freed();
        sn_lane_restore(2, wm);
        ck(sn_lane_used(2) == base_used, "bench-shape restore missed the watermark");
        ck(sn_lane_items_freed() == freed0 + 1, "bench-shape chain's grown array not reclaimed");
        ck(((uint8_t *)l)[0] == 0xAA, "T-FIBR rewind re-poison missing (expected 0xAA)");
        ck(sn_lane_lists_live(2) == 0, "bench-shape rewind left registry entries");
    }

    printf("PASS: lane-aware items pool — grown arrays reclaimed at rewind, slab windows kept, nested watermarks exact\n");
    return 0;
}
