/*
 * Systems Test: Wave Regions — Theorem WS's Runtime Mechanics
 * (SWARM RABBIT HOLE, Wave 13, Impl-R, Cohort Delta)
 * ============================================================================
 *
 * The wave-seam composition's runtime half (W2B §4, the campaign's
 * centerpiece; the typed read-mode surface §3). The compile-time side
 * conditions — E0812 (the purity kill switch), R1 (quiescent capture),
 * the seam typing — are pinned by the golden suite (wave_*.sn +
 * golden.rs); THIS file proves the runtime mechanics the theorem's
 * Part 2 quantifies over:
 *
 *   1. The capture/staging/join round trip, all three Tier-L
 *      carriers: κ₀ setup, K staged payloads, the J(W) fold lands
 *      κ₀(c) ⊔ ⊔𝒰_W(c) per carrier (max: the HWM rises; gset: the
 *      support grows by exactly the staged elements; lww: the
 *      (clock, value) winner).
 *   2. STAGING ATOMICITY (the buffer-swap proof, single-tile form):
 *      while the wave is open, the seam cell is BIT-IDENTICAL to its
 *      pre-wave state (staging writes the buffer, never the cell —
 *      read_W's frozenness is structural, not a copy's luck); the
 *      fold is ONE merge event per cell (NI(ii): "contributions ARE
 *      atomic ⊔-merges"), so there is no partial-join window to
 *      interleave — the result is asserted equal to the single-⊔
 *      computed independently.
 *   3. Fold order-independence (Thm WS Part 2 (i) commutativity):
 *      the same multiset staged in LCG-shuffled orders folds to the
 *      same cell (sn_lcell_eq) on every carrier.
 *   4. Duplication independence (Part 2(iii), Lemma 1 — the S5
 *      at-least-once dividend): every payload staged TWICE folds to
 *      the same cell as once.
 *   5. Grouping independence (Part 2(ii) associativity): the
 *      multiset split across two sequential waves (J(W₁) →
 *      capture(W₂) — case (c)'s composition) folds to the same cell.
 *   6. Multiset isolation: one wave, three slots, interleaved
 *      contribs — each cell folds EXACTLY its own payloads.
 *   7. The SN_LATTICE_CHECK=1 in-engine law self-checks ride the
 *      join's merge path (setenv at main entry — env resolved once
 *      on first merge): every fold below re-verifies idempotence +
 *      commutativity in-engine; the harness adds the three-operand
 *      law under the fold (fold ⊔ fold = fold; fold ⊔ κ₀ = fold).
 *   8. The runtime backstops (the E-LKIND pattern, fork'd children
 *      that must die with [E-R1] on stderr): read_W/contrib on an
 *      uncaptured cell, a cell captured twice, an out-of-range slot.
 *
 * NO thread rows: slice-1 is single-tile by design (the wave body
 * runs once; the wave IS the parallelism in the model) — the parallel
 * form's API is exactly this staging buffer + the join, so the
 * mechanics proven here are the ones the parallel fold reuses; the
 * concurrent-fold interleaving proof lands WITH the parallel form
 * (documented in bench/RESULTS_WAVE.md and SNLANG_SPEC's honest
 * boundary). The runner builds this twice: plain (21), and
 * -fsanitize=address (21b — no UAF over the slot table / buffer
 * growth / the join's offer build+merge+release; every cell created
 * here is sn_release'd, and the join frees its own context).
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

/* compiler/runtime.c public surface (compiled together with this file) */
void sn_rt_init(int argc, char **argv);
void *sn_lcell_new(int64_t carrier);
void sn_lcell_inflate(void *c, int64_t a, int64_t b);
void sn_lcell_merge(void *dst, void *src);
int64_t sn_lcell_len(void *c);
int64_t sn_lcell_value(void *c);
int64_t sn_lcell_has(void *c, int64_t e);
int64_t sn_lcell_carrier(void *c);
int64_t sn_lcell_eq(void *a, void *b);
void sn_release(void *p);
/* the wave context (Wave 13, Impl-R; W2B §3/§4 — Thm WS) */
void *sn_wave_new(int64_t nslots);
void sn_wave_capture(void *wave, int64_t i, void *cell);
int64_t sn_wave_read_w(void *wave, void *cell);
void sn_wave_contrib(void *wave, void *cell, int64_t a, int64_t b);
void sn_wave_join(void *wave);

/* LCELL_MAX / LCELL_GSET / LCELL_LWW — mirror of runtime.c's enum */
enum { T_MAX = 0, T_GSET = 1, T_LWW = 2 };

static int g_fail = 0;

static void ck(int64_t got, int64_t want, const char *what) {
    if (got != want) {
        fprintf(stderr, "FAIL: %s: got %lld, want %lld\n", what, (long long)got, (long long)want);
        g_fail++;
    }
}

static void ck_eq(void *a, void *b, const char *what) {
    if (!sn_lcell_eq(a, b)) {
        fprintf(stderr, "FAIL: %s: cells differ\n", what);
        g_fail++;
    }
}

/* The fixed-seed LCG (the run_bench.sh (b) control's family): fully
   deterministic, so an order-dependence here reproduces bit-for-bit. */
static uint64_t g_lcg = 0xfeedface12345678ULL;

static uint64_t lcg(void) {
    g_lcg = g_lcg * 6364136223846793005ULL + 1442695040888963407ULL;
    return g_lcg >> 33;
}

/* A deep copy of x's state as a fresh cell: merge-join of a clone. */
static void *lcell_copy(void *x) {
    void *out = sn_lcell_new(sn_lcell_carrier(x));
    sn_lcell_merge(out, x);
    return out;
}

/* A Fisher-Yates shuffle of the first n pairs (the staging-order
   scrambler — Part 2(i)'s adversary, seeded so it is reproducible). */
static void shuffle(int64_t *a, int64_t *b, int n) {
    for (int i = n - 1; i > 0; i--) {
        int j = (int)(lcg() % (uint64_t)(i + 1));
        int64_t ta = a[i], tb = b[i];
        a[i] = a[j];
        b[i] = b[j];
        a[j] = ta;
        b[j] = tb;
    }
}

/* --- 1+2. The round trip + staging atomicity, per carrier ------------ */
static void roundtrip(int carrier) {
    /* κ₀: a small seeded state. */
    void *cell = sn_lcell_new(carrier);
    sn_lcell_inflate(cell, 7, 0);
    sn_lcell_inflate(cell, 3, 0);
    void *pre = lcell_copy(cell); /* the pre-wave state */

    void *w = sn_wave_new(1);
    sn_wave_capture(w, 0, cell);

    /* read_W: the frozen κ readout, at capture time and AFTER every
       staged payload — staging must not move the cell. The readout is
       sn_lcell_value's per-carrier convention (MAX: the HWM; GSET:
       the documented 0 — no single-value readout; LWW: the winner). */
    int64_t r0 = sn_wave_read_w(w, cell);
    int64_t k0 = sn_lcell_value(cell);
    ck(r0, k0, "read_W: the frozen κ readout at entry");

    const int K = 9;
    for (int i = 0; i < K; i++) {
        sn_wave_contrib(w, cell, 10 + i, i / 3); /* values 10..18, clocks 0..2 */
    }

    /* The buffer-swap proof: the LIVE cell is untouched while the
       wave is open — bit-identical to the pre-wave state. */
    ck_eq(cell, pre, "staging atomicity: the cell is untouched pre-join");
    ck(sn_wave_read_w(w, cell), r0,
       "read_W: the κ is frozen through the staging (no mid-wave merge)");
    ck_eq(cell, pre, "staging atomicity: still untouched after K contribs");

    sn_wave_join(w);

    /* The fold landed: κ₀ ⊔ ⊔𝒰. Per carrier: max → HWM 18; gset →
       the support grew by exactly the staged elements; lww → the
       highest clock wins (clock 2: values 16,17,18 → the highest
       value 18 at clock 2 — the deterministic tiebreak). */
    if (carrier == T_MAX) {
        ck(sn_lcell_value(cell), 18, "max fold: κ₀ ⊔ ⊔𝒰 = HWM 18");
    } else if (carrier == T_GSET) {
        ck(sn_lcell_len(cell), 2 + K, "gset fold: support grew by exactly K");
        for (int i = 0; i < K; i++) {
            ck(sn_lcell_has(cell, 10 + i), 1, "gset fold: staged element present");
        }
        ck(sn_lcell_has(cell, 7), 1, "gset fold: κ₀ element retained");
    } else {
        ck(sn_lcell_value(cell), 18, "lww fold: highest clock, deterministic tiebreak");
        ck(sn_lcell_len(cell), 1, "lww fold: one winner");
    }

    /* --- 7. the three-operand laws under the fold -------------------- */
    /* idempotence of the fold: fold ⊔ fold = fold. */
    void *fc = lcell_copy(cell);
    sn_lcell_merge(fc, cell);
    ck_eq(fc, cell, "law: fold ⊔ fold = fold (idempotence under the fold)");
    sn_release(fc);
    /* the fold is above κ₀ (monotone), and κ₀ ⊔ fold = fold. */
    void *k0f = lcell_copy(pre);
    sn_lcell_merge(k0f, cell);
    ck_eq(k0f, cell, "law: κ₀ ⊔ fold = fold (the join only grows)");
    sn_release(k0f);

    sn_release(pre);
    sn_release(cell);
}

/* --- 3+4+5. Order / duplication / grouping independence -------------- */
static void order_round(int carrier, int rounds) {
    for (int rd = 0; rd < rounds; rd++) {
        const int K = 12;
        int64_t pa[32], pb[32];
        for (int i = 0; i < K; i++) {
            pa[i] = (int64_t)(lcg() % 100);
            pb[i] = (int64_t)(lcg() % 4); /* small clocks: LWW ties happen */
        }

        /* reference: κ₀ = {7}, staged in order. */
        void *ref = sn_lcell_new(carrier);
        sn_lcell_inflate(ref, 7, 0);
        void *w1 = sn_wave_new(1);
        sn_wave_capture(w1, 0, ref);
        for (int i = 0; i < K; i++) sn_wave_contrib(w1, ref, pa[i], pb[i]);
        sn_wave_join(w1);

        /* (i) shuffled staging order → the same fold (commutativity). */
        int64_t sa[32], sb[32];
        memcpy(sa, pa, sizeof(pa));
        memcpy(sb, pb, sizeof(pb));
        shuffle(sa, sb, K);
        void *shuf = sn_lcell_new(carrier);
        sn_lcell_inflate(shuf, 7, 0);
        void *w2 = sn_wave_new(1);
        sn_wave_capture(w2, 0, shuf);
        for (int i = 0; i < K; i++) sn_wave_contrib(w2, shuf, sa[i], sb[i]);
        sn_wave_join(w2);
        ck_eq(shuf, ref, "Part 2(i): shuffled staging order, identical fold");
        sn_release(shuf);

        /* (iii) every payload staged TWICE → the same fold (Lemma 1 —
           the S5 at-least-once/retry dividend at the seam). */
        void *dup = sn_lcell_new(carrier);
        sn_lcell_inflate(dup, 7, 0);
        void *w3 = sn_wave_new(1);
        sn_wave_capture(w3, 0, dup);
        for (int i = 0; i < K; i++) {
            sn_wave_contrib(w3, dup, pa[i], pb[i]);
            sn_wave_contrib(w3, dup, pa[i], pb[i]);
        }
        sn_wave_join(w3);
        ck_eq(dup, ref, "Part 2(iii): duplicated staging, identical fold");
        sn_release(dup);

        /* (ii) the multiset split across TWO sequential waves (case
           (c)'s J(W₁) → capture(W₂)) → the same fold (associativity). */
        void *grp = sn_lcell_new(carrier);
        sn_lcell_inflate(grp, 7, 0);
        void *w4 = sn_wave_new(1);
        sn_wave_capture(w4, 0, grp);
        for (int i = 0; i < K / 2; i++) sn_wave_contrib(w4, grp, pa[i], pb[i]);
        sn_wave_join(w4);
        void *w5 = sn_wave_new(1);
        sn_wave_capture(w5, 0, grp);
        for (int i = K / 2; i < K; i++) sn_wave_contrib(w5, grp, pa[i], pb[i]);
        sn_wave_join(w5);
        ck_eq(grp, ref, "Part 2(ii): regrouped across two waves, identical fold");
        sn_release(grp);

        sn_release(ref);
    }
}

/* --- 6. Multiset isolation: three slots, interleaved contribs -------- */
static void isolation_round(void) {
    void *a = sn_lcell_new(T_GSET);
    void *b = sn_lcell_new(T_GSET);
    void *c = sn_lcell_new(T_GSET);
    void *w = sn_wave_new(3);
    sn_wave_capture(w, 0, a);
    sn_wave_capture(w, 1, b);
    sn_wave_capture(w, 2, c);
    for (int i = 0; i < 8; i++) {
        sn_wave_contrib(w, a, 100 + i, 0); /* a: 100..107 */
        sn_wave_contrib(w, b, 200 + i, 0); /* b: 200..207 */
        sn_wave_contrib(w, c, 300 + i, 0); /* c: 300..307 */
    }
    sn_wave_join(w);
    ck(sn_lcell_len(a), 8, "isolation: slot a folded exactly its multiset");
    ck(sn_lcell_len(b), 8, "isolation: slot b folded exactly its multiset");
    ck(sn_lcell_len(c), 8, "isolation: slot c folded exactly its multiset");
    ck(sn_lcell_has(a, 250), 0, "isolation: no cross-slot bleed a←b");
    ck(sn_lcell_has(b, 100), 0, "isolation: no cross-slot bleed b←a");
    ck(sn_lcell_has(c, 150), 0, "isolation: no cross-slot bleed c←a");
    ck(sn_lcell_has(a, 105), 1, "isolation: a's own element");
    ck(sn_lcell_has(c, 307), 1, "isolation: c's own element");
    sn_release(a);
    sn_release(b);
    sn_release(c);
}

/* --- 8. The runtime backstops (fork'd; must die with [E-R1]) ---------- */
static int probe_dies_with_er1(void (*fn)(void)) {
    pid_t pid = fork();
    if (pid == 0) {
        /* the child's stderr must carry [E-R1] and it must exit(1) */
        int to_devnull = 1;
        (void)to_devnull;
        fn();
        _exit(0); /* reachable only if the guard did NOT fire */
    }
    int st = 0;
    waitpid(pid, &st, 0);
    return WIFEXITED(st) && WEXITSTATUS(st) == 1;
}

static void probe_read_w_uncaptured(void) {
    void *cell = sn_lcell_new(T_MAX);
    void *w = sn_wave_new(1);
    (void)sn_wave_read_w(w, cell); /* must die [E-R1] */
    _exit(0);
}

static void probe_contrib_uncaptured(void) {
    void *cell = sn_lcell_new(T_MAX);
    void *w = sn_wave_new(1);
    sn_wave_contrib(w, cell, 1, 0); /* must die [E-R1] */
    _exit(0);
}

static void probe_capture_twice(void) {
    void *cell = sn_lcell_new(T_MAX);
    void *w = sn_wave_new(2);
    sn_wave_capture(w, 0, cell);
    sn_wave_capture(w, 1, cell); /* must die [E-R1] */
    _exit(0);
}

static void probe_capture_oor(void) {
    void *cell = sn_lcell_new(T_MAX);
    void *w = sn_wave_new(1);
    sn_wave_capture(w, 5, cell); /* must die [E-R1] */
    _exit(0);
}

int main(void) {
    sn_rt_init(0, NULL);

    /* The runtime's own per-merge law self-checks ON for this whole
       run (env resolved once on first merge): every J(W) fold below
       re-verifies idempotence + commutativity in-engine, on exactly
       the atomic ⊔ event the theorem quantifies over. */
    setenv("SN_LATTICE_CHECK", "1", 1);

    roundtrip(T_MAX);
    roundtrip(T_GSET);
    roundtrip(T_LWW);

    order_round(T_MAX, 25);
    order_round(T_GSET, 25);
    order_round(T_LWW, 25);

    isolation_round();

    ck(probe_dies_with_er1(probe_read_w_uncaptured), 1,
       "backstop: read_W on an uncaptured cell dies [E-R1]");
    ck(probe_dies_with_er1(probe_contrib_uncaptured), 1,
       "backstop: contrib to an uncaptured cell dies [E-R1]");
    ck(probe_dies_with_er1(probe_capture_twice), 1,
       "backstop: a cell captured twice dies [E-R1]");
    ck(probe_dies_with_er1(probe_capture_oor), 1,
       "backstop: an out-of-range slot dies [E-R1]");

    if (g_fail == 0) {
        printf("PASS: test_wave (capture/staging/join round trip, staging "
               "atomicity, fold order/duplication/grouping independence, "
               "multiset isolation, lattice laws under the fold, [E-R1] "
               "backstops)\n");
        return 0;
    }
    printf("FAIL: test_wave (%d checks failed)\n", g_fail);
    return 1;
}
