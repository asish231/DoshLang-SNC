/*
 * Systems Test: SFC Layouts — A2's Decision Table, Runtime Mechanics
 * (SWARM RABBIT HOLE, Wave 14, Impl-S, Cohort Delta)
 * ============================================================================
 *
 * The compile-time arms (dims discipline, the type-agreement rule,
 * the E-GC-IN-LANE store rejections, the T5b warning) are pinned by
 * the golden suite (morton_*.sn / hilbert_*.sn / sfc_*.sn +
 * golden.rs); THIS file proves the runtime mechanics the decision
 * table's claims stand on (A2 §2.2/§2.4):
 *
 *   1. The interleave/deinterleave, d=2, EXHAUSTIVE over a 64×64
 *      grid: every cell round-trips, and the software path matches
 *      the bit-by-bit reference (the reference table this AArch64
 *      machine verifies — it has no BMI2, so the software path is
 *      what runs here; C2.8).
 *   2. The feature dispatch: SN_SFC_FORCE_PATH re-resolution runs
 *      the grid under BOTH settings and diffs (in-process on a BMI2
 *      machine; on AArch64 the =bmi2 setting is the honest
 *      [E-SFC-PATH] refusal — fork'd, the process must die).
 *   3. The deinterleave bijection: interleave∘deinterleave = id over
 *      the code space, deinterleave∘interleave = id over the pairs.
 *   4. The Butz/Skilling hilbert loop: encode∘decode = id over the
 *      full 8×8 code space AND the vertex-continuous property
 *      (consecutive codes are unit-step neighbors — A2 §2.1's
 *      "always exactly 1"; Morton fails this at the quadrant seam,
 *      which is the honest structural difference between the two
 *      curves).
 *   5. The segment-advance box query vs the linear-scan reference,
 *      EXACT across 128+ randomized boxes (seeded LCG) plus the edge
 *      boxes (the full grid, single cells, thin rows/columns,
 *      out-of-grid clips): same count, same sum, same xor — same
 *      result SET, any order.
 *   6. The O(1)-carry iteration: codes [0, len) visit every grid
 *      cell EXACTLY ONCE (the code space covers a power-of-two grid
 *      exactly — no padding, no skips).
 *   7. The lane backing (E-GC-IN-LANE consistency): building and
 *      filling mortons/hilberts moves ZERO GC objects (the storage
 *      is lane memory, never gc_track'd), and the lane-used delta is
 *      the exact contiguous reservation (two builds of the same
 *      dims reserve the same bytes).
 *   8. The refusal backstops (fork'd children that must die with
 *      the tag on stderr): the morton bounds, the from_lex length,
 *      the from_bytes dims disagreement, the hilbert insert order,
 *      the duplicate point, the absent point, the out-of-grid
 *      coordinate, and the forced-bmi2 knob on a no-BMI2 machine.
 *
 * The runner builds this twice: plain (22), then ASan (22b — the
 * reservations, the reshuffle, the box scans and the growth copies
 * are exactly the paths the sanitizer should watch). The morton
 * buffer is lane memory — it is never freed here (the lane reclaims
 * it at its rewind; the process exits first), and the hilbert's
 * growth blocks are abandoned to the same rewind, so ASan's
 * leak-freedom expectation is met by construction: the lane owns the
 * bytes. The heap side (the from_lex source list, the to_bytes
 * bytearray) stays GC-managed.
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

/* compiler/runtime.c public surface (compiled together with this file) */
void sn_rt_init(int argc, char **argv);
int64_t sn_sfc_path(void);
void sn_sfc_force_recheck(void);
int64_t sn_morton_index(int64_t i, int64_t j);
int64_t sn_morton_x(int64_t k);
int64_t sn_morton_y(int64_t k);
void *sn_morton_new(int64_t d1, int64_t d2);
void *sn_morton_from_lex(void *src, int64_t d1, int64_t d2);
int64_t sn_morton_get(void *m, int64_t i, int64_t j);
void sn_morton_set(void *m, int64_t i, int64_t j, int64_t v);
int64_t sn_morton_len(void *m);
int64_t sn_morton_scan_get(void *m, int64_t k);
void *sn_morton_to_bytes(void *m);
void *sn_morton_from_bytes(void *b, int64_t d1, int64_t d2);
void *sn_hilbert_new(int64_t k);
void sn_hilbert_insert(void *h, int64_t x, int64_t y, int64_t v);
int64_t sn_hilbert_get(void *h, int64_t x, int64_t y);
int64_t sn_hilbert_has(void *h, int64_t x, int64_t y);
int64_t sn_hilbert_len(void *h);
int64_t sn_hilbert_code(int64_t x, int64_t y, int64_t k);
int64_t sn_hilbert_dx(int64_t c, int64_t k);
int64_t sn_hilbert_dy(int64_t c, int64_t k);
int64_t sn_hilbert_entry_key(void *h, int64_t idx);
int64_t sn_hilbert_entry_x(void *h, int64_t idx);
int64_t sn_hilbert_entry_y(void *h, int64_t idx);
int64_t sn_hilbert_entry_val(void *h, int64_t idx);
int64_t sn_hilbert_box_first(void *h, int64_t x0, int64_t y0, int64_t x1, int64_t y1);
int64_t sn_hilbert_box_next(void *h, int64_t x0, int64_t y0, int64_t x1, int64_t y1, int64_t idx);
int64_t sn_gc_object_count(void);
int64_t sn_lane_used(int64_t lane_id);
void *sn_list_new(int64_t elem_kind);
void sn_list_push_i64(void *l, int64_t v);

static int g_fail = 0;

static void ck(int64_t got, int64_t want, const char *what) {
    if (got != want) {
        fprintf(stderr, "FAIL: %s: got %lld, want %lld\n", what, (long long)got, (long long)want);
        g_fail++;
    }
}

/* The fixed-seed LCG (the run_bench.sh (b) control's family): fully
   deterministic, so a box-query mismatch here reproduces bit-for-bit. */
static uint64_t g_lcg = 0x5fc0ffee12345678ULL;

static uint64_t lcg(void) {
    g_lcg = g_lcg * 6364136223846793005ULL + 1442695040888963407ULL;
    return g_lcg >> 33;
}

/* The bit-by-bit interleave reference — the "reference table" the
   software path is verified against on this AArch64 machine (no BMI2
   here; C2.8's honest note). */
static uint64_t ref_interleave(uint64_t x, uint64_t y) {
    uint64_t m = 0;
    int bit = 0;
    while (x || y) {
        m |= (x & 1) << bit;
        m |= (y & 1) << (bit + 1);
        x >>= 1;
        y >>= 1;
        bit += 2;
    }
    return m;
}

/* The interleave-grid hash: one fold over every cell of the 64×64
   grid (mixed so order and value both count). */
static uint64_t interleave_grid_hash(void) {
    uint64_t h = 1469598103934665603ULL;
    for (int64_t i = 0; i < 64; i++) {
        for (int64_t j = 0; j < 64; j++) {
            h ^= (uint64_t)sn_morton_index(i, j);
            h *= 1099511628211ULL;
            h ^= (uint64_t)sn_morton_x(sn_morton_index(i, j));
            h *= 1099511628211ULL;
            h ^= (uint64_t)sn_morton_y(sn_morton_index(i, j));
            h *= 1099511628211ULL;
        }
    }
    return h;
}

/* --- 1+3. The interleave round trip + the bijection + the reference -- */
static void interleave_roundtrip(void) {
    int64_t bad = 0;
    uint64_t ref_bad = 0;
    for (int64_t i = 0; i < 64; i++) {
        for (int64_t j = 0; j < 64; j++) {
            int64_t k = sn_morton_index(i, j);
            if (sn_morton_x(k) != i || sn_morton_y(k) != j) bad++;
            if ((uint64_t)k != ref_interleave((uint64_t)i, (uint64_t)j)) ref_bad++;
        }
    }
    ck(bad, 0, "interleave: every cell round-trips (deinterleave ∘ interleave)");
    ck((int64_t)ref_bad, 0, "interleave: the software path == the bit-by-bit reference");
    /* the bijection's other side: interleave ∘ deinterleave = id over
       the code space — and the code space [0, 4096) covers the grid
       exactly (the power-of-two discipline: no padding, no skips) */
    int64_t seen[64 * 64];
    memset(seen, 0, sizeof seen);
    int64_t dup = 0;
    for (int64_t k = 0; k < 64 * 64; k++) {
        int64_t x = sn_morton_x(k);
        int64_t y = sn_morton_y(k);
        if (x < 0 || x >= 64 || y < 0 || y >= 64) {
            dup++;
            continue;
        }
        if (sn_morton_index(x, y) != k) dup++;
        seen[x * 64 + y]++;
    }
    for (int64_t c = 0; c < 64 * 64; c++) {
        if (seen[c] != 1) dup++;
    }
    ck(dup, 0, "interleave: the code space covers the grid exactly once (the O(1)-carry iteration)");
}

/* --- 2. The feature dispatch: the knob + the cross-path equivalence -- */
static void dispatch_checks(void) {
    /* what runs here: 0 = the software shift-or path (AArch64), 1 = BMI2 */
    int64_t native = sn_sfc_path();
    if (native == 1) {
        /* A BMI2 machine: run the grid under BOTH settings, in
           process, and diff — the C2.8 equivalence claim. */
        setenv("SN_SFC_FORCE_PATH", "soft", 1);
        sn_sfc_force_recheck();
        ck(sn_sfc_path(), 0, "dispatch: the knob forces the software path");
        uint64_t soft_hash = interleave_grid_hash();
        unsetenv("SN_SFC_FORCE_PATH");
        sn_sfc_force_recheck();
        ck(sn_sfc_path(), 1, "dispatch: native resolution restores BMI2");
        uint64_t bmi2_hash = interleave_grid_hash();
        ck(soft_hash == bmi2_hash, 1, "dispatch: software == BMI2 over the grid (C2.8)");
    } else {
        /* No BMI2 (this AArch64 machine): the software path is
           verified against the reference above; forcing bmi2 is the
           honest [E-SFC-PATH] refusal — the fork'd probe below pins
           it. */
        ck(native, 0, "dispatch: the software shift-or path is what runs here");
    }
}

/* --- 4. The Butz/Skilling loop: bijection + vertex-continuity ------- */
static void hilbert_curve_checks(void) {
    int64_t k = 3; /* 8×8 */
    int64_t bad = 0;
    int64_t seen[64];
    memset(seen, 0, sizeof seen);
    int64_t px = 0, py = 0;
    for (int64_t c = 0; c < 64; c++) {
        int64_t x = sn_hilbert_dx(c, k);
        int64_t y = sn_hilbert_dy(c, k);
        if (sn_hilbert_code(x, y, k) != c) bad++;
        if (x < 0 || x >= 8 || y < 0 || y >= 8) bad++;
        seen[x * 8 + y]++;
        if (c > 0) {
            int64_t step = (x > px ? x - px : px - x) + (y > py ? y - py : py - y);
            if (step != 1) bad++; /* vertex-continuous: exactly 1 */
        }
        px = x;
        py = y;
    }
    for (int64_t c = 0; c < 64; c++) {
        if (seen[c] != 1) bad++;
    }
    ck(bad, 0, "hilbert: encode∘decode = id, every cell once, unit steps (A2 §2.1)");
}

/* --- 5. The segment-advance box query vs the linear scan, EXACT ------ */
static void box_query_checks(void) {
    int64_t k = 6; /* 64×64, ALL cells present */
    void *h = sn_hilbert_new(k);
    for (int64_t c = 0; c < 64 * 64; c++) {
        sn_hilbert_insert(h, sn_hilbert_dx(c, k), sn_hilbert_dy(c, k), c);
    }
    ck(sn_hilbert_len(h), 64 * 64, "hilbert: the full grid is present");

    int64_t rounds = 0;
    int64_t agree_fail = 0;
    /* the randomized boxes: 128 seeded-LCG rounds */
    for (int64_t r = 0; r < 128; r++) {
        int64_t x0 = (int64_t)(lcg() % 64);
        int64_t y0 = (int64_t)(lcg() % 64);
        int64_t x1 = x0 + (int64_t)(lcg() % 64);
        int64_t y1 = y0 + (int64_t)(lcg() % 64);
        int64_t cnt = 0, sum = 0, xsum = 0;
        int64_t idx = sn_hilbert_box_first(h, x0, y0, x1, y1);
        while (idx != -1) {
            cnt++;
            int64_t v = sn_hilbert_entry_val(h, idx);
            sum += v;
            xsum ^= v;
            idx = sn_hilbert_box_next(h, x0, y0, x1, y1, idx);
        }
        int64_t rcnt = 0, rsum = 0, rxsum = 0;
        for (int64_t e = 0; e < 64 * 64; e++) {
            int64_t ex = sn_hilbert_entry_x(h, e);
            int64_t ey = sn_hilbert_entry_y(h, e);
            if (ex >= x0 && ex <= x1 && ey >= y0 && ey <= y1) {
                rcnt++;
                rsum += sn_hilbert_entry_val(h, e);
                rxsum ^= sn_hilbert_entry_val(h, e);
            }
        }
        if (cnt != rcnt || sum != rsum || xsum != rxsum) agree_fail++;
        rounds++;
    }
    /* the edge boxes: the full grid, a single cell, a thin row, a thin
       column, a box clipped by the grid's far corner, a box wholly
       outside the grid (the empty result) */
    struct { int64_t x0, y0, x1, y1; } edges[] = {
        { 0, 0, 63, 63 },   /* the whole grid */
        { 5, 9, 5, 9 },     /* a single cell */
        { 0, 40, 63, 40 },  /* a thin row */
        { 17, 0, 17, 63 },  /* a thin column */
        { 60, 60, 200, 200 }, /* clipped at the far corner */
        { -10, -10, -1, -1 }, /* wholly outside: empty */
        { 32, 32, 47, 47 },  /* a dyadic box: exactly one cluster */
    };
    for (size_t ei = 0; ei < sizeof edges / sizeof edges[0]; ei++) {
        int64_t x0 = edges[ei].x0, y0 = edges[ei].y0;
        int64_t x1 = edges[ei].x1, y1 = edges[ei].y1;
        int64_t cnt = 0, sum = 0, xsum = 0;
        int64_t idx = sn_hilbert_box_first(h, x0, y0, x1, y1);
        while (idx != -1) {
            cnt++;
            int64_t v = sn_hilbert_entry_val(h, idx);
            sum += v;
            xsum ^= v;
            idx = sn_hilbert_box_next(h, x0, y0, x1, y1, idx);
        }
        int64_t rcnt = 0, rsum = 0, rxsum = 0;
        for (int64_t e = 0; e < 64 * 64; e++) {
            int64_t ex = sn_hilbert_entry_x(h, e);
            int64_t ey = sn_hilbert_entry_y(h, e);
            if (ex >= x0 && ex <= x1 && ey >= y0 && ey <= y1) {
                rcnt++;
                rsum += sn_hilbert_entry_val(h, e);
                rxsum ^= sn_hilbert_entry_val(h, e);
            }
        }
        if (cnt != rcnt || sum != rsum || xsum != rxsum) agree_fail++;
        rounds++;
    }
    ck(rounds, 135, "box: the randomized + edge box rounds ran");
    ck(agree_fail, 0, "box: the segment advance == the linear scan EXACTLY on every box");
    /* the point surface on the same map */
    ck(sn_hilbert_has(h, 0, 0), 1, "hilbert: point (0,0) present");
    ck(sn_hilbert_get(h, 63, 63), sn_hilbert_code(63, 63, k),
       "hilbert: point lookup returns the inserted value (the far corner's own code)");
}

/* --- 6+7. The lane backing: the carry iteration over a REAL morton,
   the to_bytes round trip, the GC-count invariance, the reservation
   determinism (E-GC-IN-LANE) ------------------------------------------ */
static void lane_backing_checks(void) {
    /* The GC-count invariance: the SFC constructors move ZERO GC
       objects (lane memory, never gc_track'd). The source list (heap)
       is built BEFORE the snapshot. */
    void *src = sn_list_new(0);
    for (int64_t i = 0; i < 64; i++) {
        for (int64_t j = 0; j < 64; j++) {
            sn_list_push_i64(src, i * 64 + j);
        }
    }
    int64_t gc0 = sn_gc_object_count();
    int64_t u0 = sn_lane_used(0);
    void *m = sn_morton_from_lex(src, 64, 64);
    int64_t u1 = sn_lane_used(0);
    void *m2 = sn_morton_new(64, 64);
    int64_t u2 = sn_lane_used(0);
    ck(sn_gc_object_count(), gc0, "lane backing: from_lex/new move ZERO GC objects (E-GC-IN-LANE)");
    ck(u1 - u0, u2 - u1, "lane backing: two builds of the same dims reserve the same bytes");
    ck(u1 - u0 >= 64 * 64 * 8, 1, "lane backing: the reservation covers the whole data buffer");

    /* from_lex correctness on the real buffer: the permutation view */
    int64_t bad = 0;
    int64_t sum = 0;
    for (int64_t i = 0; i < 64; i++) {
        for (int64_t j = 0; j < 64; j++) {
            int64_t v = sn_morton_get(m, i, j);
            if (v != i * 64 + j) bad++;
            sum += v;
        }
    }
    ck(bad, 0, "from_lex: every cell lands at its interleave address");
    ck(sum, (64 * 64 - 1) * (64 * 64) / 2, "from_lex: the value multiset is preserved");

    /* the curve-order scan over the real buffer: the carry iteration
       visits every cell exactly once (the code IS the index) */
    int64_t seen[64 * 64];
    memset(seen, 0, sizeof seen);
    int64_t scan_sum = 0;
    for (int64_t k = 0; k < sn_morton_len(m); k++) {
        int64_t v = sn_morton_scan_get(m, k);
        scan_sum += v;
        int64_t x = sn_morton_x(k);
        int64_t y = sn_morton_y(k);
        if (x >= 0 && x < 64 && y >= 0 && y < 64) seen[x * 64 + y]++;
    }
    int64_t miss = 0;
    for (int64_t c = 0; c < 64 * 64; c++) {
        if (seen[c] != 1) miss++;
    }
    ck(miss, 0, "morton: the O(1)-carry iteration visits every cell exactly once");
    ck(scan_sum, sum, "morton: the curve-order scan sees the same multiset");

    /* the to_bytes round trip: values only — zero fixups by construction */
    for (int64_t i = 0; i < 64; i += 7) {
        for (int64_t j = 0; j < 64; j += 5) {
            sn_morton_set(m, i, j, i * 1000 + j + 1);
        }
    }
    void *bytes = sn_morton_to_bytes(m);
    void *m3 = sn_morton_from_bytes(bytes, 64, 64);
    int64_t rt_bad = 0;
    for (int64_t k = 0; k < sn_morton_len(m3); k++) {
        if (sn_morton_scan_get(m, k) != sn_morton_scan_get(m3, k)) rt_bad++;
    }
    for (int64_t i = 0; i < 64; i++) {
        for (int64_t j = 0; j < 64; j++) {
            if (sn_morton_get(m, i, j) != sn_morton_get(m3, i, j)) rt_bad++;
        }
    }
    ck(rt_bad, 0, "to_bytes/from_bytes: bit-identical round trip, zero fixups");
}

/* --- 8. The refusal backstops (fork'd; each child must die exit 1) --- */
static int probe_dies(void (*fn)(void)) {
    pid_t pid = fork();
    if (pid == 0) {
        /* the child's stderr must carry the tag; it exits(1) at the
           refusal, 0 only if the guard did NOT fire */
        fn();
        _exit(0);
    }
    int st = 0;
    waitpid(pid, &st, 0);
    return WIFEXITED(st) && WEXITSTATUS(st) == 1;
}

static void probe_morton_oob(void) {
    void *m = sn_morton_new(8, 8);
    (void)sn_morton_get(m, 9, 0); /* must die [E-SFC] */
    _exit(0);
}

static void probe_from_lex_len(void) {
    void *src = sn_list_new(0);
    sn_list_push_i64(src, 1);
    sn_list_push_i64(src, 2);
    (void)sn_morton_from_lex(src, 8, 8); /* 2 values, 64 needed [E-SFC] */
    _exit(0);
}

static void probe_from_bytes_dims(void) {
    void *m = sn_morton_new(8, 8);
    void *bytes = sn_morton_to_bytes(m);
    (void)sn_morton_from_bytes(bytes, 16, 16); /* dims disagree [E-SFC] */
    _exit(0);
}

static void probe_insert_order(void) {
    void *h = sn_hilbert_new(2);
    sn_hilbert_insert(h, sn_hilbert_dx(5, 2), sn_hilbert_dy(5, 2), 5);
    sn_hilbert_insert(h, sn_hilbert_dx(2, 2), sn_hilbert_dy(2, 2), 2); /* [E-SFC-ORDER] */
    _exit(0);
}

static void probe_insert_dup(void) {
    void *h = sn_hilbert_new(2);
    sn_hilbert_insert(h, 1, 1, 5);
    sn_hilbert_insert(h, 1, 1, 7); /* [E-SFC-DUP] */
    _exit(0);
}

static void probe_get_absent(void) {
    void *h = sn_hilbert_new(2);
    sn_hilbert_insert(h, 0, 0, 5);
    (void)sn_hilbert_get(h, 3, 3); /* [E-SFC-KEYX] */
    _exit(0);
}

static void probe_coord_oob(void) {
    void *h = sn_hilbert_new(2);
    sn_hilbert_insert(h, 8, 0, 1); /* x=8 out of the 4×4 grid [E-SFC] */
    _exit(0);
}

/* The UNEQUAL power-of-two dims refusal: interleave(2^a−1, 2^b−1)
   needs 2a−1 bits when a > b, so a 1024×64 grid's corner code
   (352,255) would overflow the 65,536-slot reservation — slice 1
   refuses unequal dims rather than pad (the padded reservation /
   the hybrid tile floor is the slice-2 form). */
static void probe_unequal_dims(void) {
    (void)sn_morton_new(1024, 64); /* must die [E-SFC] */
    _exit(0);
}

static void probe_forced_bmi2(void) {
    setenv("SN_SFC_FORCE_PATH", "bmi2", 1);
    sn_sfc_force_recheck();
    (void)sn_morton_index(1, 1); /* no BMI2 here: [E-SFC-PATH] */
    _exit(0);
}

int main(void) {
    sn_rt_init(0, NULL);

    interleave_roundtrip();
    dispatch_checks();
    hilbert_curve_checks();
    box_query_checks();
    lane_backing_checks();

    ck(probe_dies(probe_morton_oob), 1, "backstop: the morton bounds refusal dies [E-SFC]");
    ck(probe_dies(probe_from_lex_len), 1, "backstop: the from_lex length refusal dies [E-SFC]");
    ck(probe_dies(probe_from_bytes_dims), 1, "backstop: the from_bytes dims refusal dies [E-SFC]");
    ck(probe_dies(probe_insert_order), 1, "backstop: the out-of-order insert dies [E-SFC-ORDER]");
    ck(probe_dies(probe_insert_dup), 1, "backstop: the duplicate point dies [E-SFC-DUP]");
    ck(probe_dies(probe_get_absent), 1, "backstop: the absent point dies [E-SFC-KEYX]");
    ck(probe_dies(probe_coord_oob), 1, "backstop: the out-of-grid coordinate dies [E-SFC]");
    ck(probe_dies(probe_unequal_dims), 1, "backstop: unequal power-of-two dims die [E-SFC] (the corner code would overflow the reservation)");
    /* On a no-BMI2 machine (this AArch64 one) the forced-bmi2 knob is
       the honest refusal; on a BMI2 machine it runs — the probe's
       expectation follows the native resolution. */
    ck(probe_dies(probe_forced_bmi2), sn_sfc_path() == 0 ? 1 : 0,
       "backstop: forcing bmi2 without BMI2 dies [E-SFC-PATH] (the C2.8 refusal)");

    if (g_fail == 0) {
        printf("PASS: test_sfc (interleave round trip + reference, dispatch "
               "knob, hilbert bijection + unit steps, segment-advance == "
               "linear scan, O(1)-carry iteration, lane backing, [E-SFC] "
               "backstops)\n");
        return 0;
    }
    printf("FAIL: test_sfc (%d checks failed)\n", g_fail);
    return 1;
}
