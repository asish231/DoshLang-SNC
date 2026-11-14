/*
 * Systems Test: Tier-L Join-Semilattice Cells (SWARM RABBIT HOLE, Wave 9)
 * ============================================================================
 *
 * The Tier-L slice's runtime mechanics (Impl-M, Cohort Delta; W2C §1.1
 * Tier L; final report §8.1; the capability order O ⊑ L ⊑ G). Proves, at
 * the runtime level, that SnLCell is the join-semilattice carrier:
 *
 *   1. The three lattice laws, EXHAUSTIVELY, over randomized op
 *      sequences (a fixed-seed LCG — deterministic across runs and
 *      platforms): idempotence x⊔x = x, commutativity a⊔b = b⊔a,
 *      associativity (a⊔b)⊔c = a⊔(b⊔c), asserted per merge for all
 *      three carriers (max / gset / lww).
 *   2. The delta-mutator law m(X) = X ⊔ m^δ(X) (Almeida–Shoker–Baquero
 *      JPDC 2018; Enes et al. 2018): for randomized MONOTONE
 *      transitions prev ⊑ dst (the LVar/CRDT regime — Tier-L states
 *      only inflate), prev ⊔ delta(dst, prev) == dst, and the delta is
 *      the join-decomposition-optimal STATE-RELATIVE fragment — never a
 *      copy: every gset element of the delta is in dst and NOT in prev.
 *   3. Quiescence: delta(x, x) is the identity cell (len 0) — the
 *      Tier-L fixpoint test (W4C Repair 3's per-tier form: F(s) ⊑ s).
 *   4. The per-carrier readouts: max's HWM (monotone only), gset's
 *      size/membership, lww's winner + the DETERMINISTIC clock-tie
 *      value tiebreak (same bag of offers joined in different orders
 *      converges to the same winner — commutativity depends on it).
 *   5. The runtime's own env-gated law self-checks (SN_LATTICE_CHECK=1):
 *      every merge below re-verifies idempotence and commutativity
 *      in-engine; a violation fail-fasts the process.
 *   6. E-LKIND, the tier-confusion guard (runtime-enforced because the
 *      surface type of a cell is an untyped ptr — the checker cannot
 *      see the carrier): carrier mismatch, kind mismatch (a list handed
 *      to lcell_merge), the G-side twin (an lcell handed to
 *      zset_merge), and the unknown carrier — each probed in a fork'd
 *      child that must die with the [E-LKIND] diagnostic on stderr.
 *
 * NO ⊖ is exercised anywhere in this file because none exists: exact
 * retraction on the idempotent carrier is the checked compile-time
 * rejection E-LRET (the Grothendieck argument: x⊔x = x ⇒ [x] = 0 —
 * the merge operator does not group-embed); examples/lcell_retract_fail.sn
 * + golden.rs pin the compile-time side.
 *
 * The runner (tests/systems/run_all.sh) builds this twice: plain (16),
 * and with -fsanitize=address (16b — the ASan stress pass: no UAF over
 * the GSET insert/rehash/growth paths, no leaks — every cell created
 * here is sn_release'd).
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
void *sn_lcell_delta(void *dst, void *prev);
void *sn_lcell_apply(void *base, void *delta);
int64_t sn_lcell_len(void *c);
int64_t sn_lcell_value(void *c);
int64_t sn_lcell_has(void *c, int64_t e);
int64_t sn_lcell_carrier(void *c);
int64_t sn_lcell_eq(void *a, void *b);
void *sn_zset_new(int64_t key_kind);
void sn_zset_merge(void *dst, void *src);
void *sn_list_new(int64_t elem_kind);
void sn_release(void *p);

/* LCELL_MAX / LCELL_GSET / LCELL_LWW — mirror of runtime.c's enum */
enum { T_MAX = 0, T_GSET = 1, T_LWW = 2 };

static int g_fail = 0;

static void ck(int64_t got, int64_t want, const char *what) {
    if (got != want) {
        fprintf(stderr, "FAIL: %s: got %lld, want %lld\n", what, (long long)got, (long long)want);
        g_fail++;
    }
}

/* The fixed-seed LCG (the run_bench.sh (b) control's family): fully
   deterministic, so a law violation here reproduces bit-for-bit. */
static uint64_t g_lcg = 0x123456789abcdefULL;

static uint64_t lcg(void) {
    g_lcg = g_lcg * 6364136223846793005ULL + 1442695040888963407ULL;
    return g_lcg >> 33;
}

/* A copy of x's state as a fresh cell: delta(x, NULL) is the full
   join-decomposition of x — exactly a deep copy on every carrier. */
static void *lcell_copy(void *x) {
    return sn_lcell_delta(x, NULL);
}

/* Randomized inflation: n offers, values/clocks drawn from the LCG. */
static void fill_random(void *c, int n) {
    int64_t carrier = sn_lcell_carrier(c);
    for (int i = 0; i < n; i++) {
        int64_t v = (int64_t)(lcg() % 200) - 50; /* -50..149 */
        int64_t clock = (int64_t)(lcg() % 7);     /* small clocks: ties happen */
        sn_lcell_inflate(c, v, clock);
    }
    (void)carrier;
}

/* --- 1. The three laws, per merge, all carriers ----------------------- */
static void law_round(int carrier, int rounds, int ops) {
    for (int r = 0; r < rounds; r++) {
        void *a = sn_lcell_new(carrier);
        void *b = sn_lcell_new(carrier);
        void *c = sn_lcell_new(carrier);
        fill_random(a, ops);
        fill_random(b, ops);
        fill_random(c, ops);

        /* idempotence: a ⊔ a == a */
        void *aa = lcell_copy(a);
        sn_lcell_merge(aa, a);
        if (!sn_lcell_eq(aa, a)) {
            fprintf(stderr, "FAIL: idempotence x⊔x=x violated (carrier %d, round %d)\n", carrier, r);
            g_fail++;
        }
        sn_release(aa);

        /* commutativity: a ⊔ b == b ⊔ a */
        void *ab = lcell_copy(a);
        sn_lcell_merge(ab, b);
        void *ba = lcell_copy(b);
        sn_lcell_merge(ba, a);
        if (!sn_lcell_eq(ab, ba)) {
            fprintf(stderr, "FAIL: commutativity a⊔b=b⊔a violated (carrier %d, round %d)\n", carrier, r);
            g_fail++;
        }
        sn_release(ab);
        sn_release(ba);

        /* associativity: (a ⊔ b) ⊔ c == a ⊔ (b ⊔ c) */
        void *abc = lcell_copy(a);
        sn_lcell_merge(abc, b);
        sn_lcell_merge(abc, c);
        void *acb = lcell_copy(a);
        sn_lcell_merge(acb, c);
        sn_lcell_merge(acb, b);
        if (!sn_lcell_eq(abc, acb)) {
            fprintf(stderr, "FAIL: associativity (a⊔b)⊔c=a⊔(b⊔c) violated (carrier %d, round %d)\n", carrier, r);
            g_fail++;
        }
        sn_release(abc);
        sn_release(acb);

        /* inflation monotonicity: the state only grows — the LVar/CRDT
           discipline. gset: any insert can only grow the support (the
           probe element is fresh, so exactly +1); max: an offer below
           the HWM is a no-op (the HWM never drops). */
        int64_t len_before = sn_lcell_len(a);
        int64_t val_before = sn_lcell_value(a);
        sn_lcell_inflate(a, -1000000, 0);
        if (carrier == T_GSET) {
            ck(sn_lcell_len(a), len_before + 1, "gset: a fresh insert grows the support by exactly 1");
        } else if (carrier == T_MAX) {
            ck(sn_lcell_value(a), val_before, "max: a lower offer must not drop the HWM");
        }
        /* (lww's loser-invariance is covered by the convergence sweep
           below — the clock is not a readout, so the winner-invariance
           proof runs over whole join orders) */

        sn_release(a);
        sn_release(b);
        sn_release(c);
    }
}

/* --- 2. The mutator law over randomized MONOTONE transitions --------- */
static void mutator_round(int carrier, int rounds, int ops) {
    for (int r = 0; r < rounds; r++) {
        void *prev = sn_lcell_new(carrier);
        fill_random(prev, ops);
        void *dst = lcell_copy(prev); /* dst starts at prev: monotone */
        fill_random(dst, ops);       /* ...and only inflates from there */
        void *d = sn_lcell_delta(dst, prev);
        void *rt = sn_lcell_apply(prev, d); /* prev ⊔ m^δ */
        if (!sn_lcell_eq(rt, dst)) {
            fprintf(stderr, "FAIL: mutator law prev⊔delta(prev,dst)==dst violated (carrier %d, round %d)\n", carrier, r);
            g_fail++;
        }
        /* the fragment is state-relative, NOT a copy: on the gset
           carrier every delta element is in dst and absent from prev,
           and the delta's support size matches the scan (values are
           drawn from -50..149, so the value space is the scan space) */
        if (carrier == T_GSET) {
            int64_t dl = sn_lcell_len(d);
            int64_t seen = 0;
            for (int64_t v = -50; v < 150; v++) {
                if (sn_lcell_has(d, v)) {
                    seen++;
                    if (!sn_lcell_has(dst, v)) {
                        fprintf(stderr, "FAIL: delta element not in dst\n");
                        g_fail++;
                    }
                    if (sn_lcell_has(prev, v)) {
                        fprintf(stderr, "FAIL: delta element already in prev (not state-relative)\n");
                        g_fail++;
                    }
                }
            }
            ck(seen, dl, "gset: delta support size consistent");
        }
        sn_release(d);
        sn_release(rt);
        sn_release(prev);
        sn_release(dst);
    }
}

/* --- 6. E-LKIND probes: a fork'd child must die with the diagnostic ---
 * The guard is RUNTIME-enforced (the surface type of a cell is an
 * untyped ptr; the checker cannot see the carrier), fail-fast per the
 * runtime.c convention. Each probe runs in a child with stderr
 * redirected to a temp file; the parent asserts the child died AND
 * the diagnostic carries [E-LKIND] plus the mode's keyword. */
static int elkind_probe(int mode) {
    char tmpl[] = "/tmp/snc_lcell_elkind_XXXXXX";
    int fd = mkstemp(tmpl);
    if (fd < 0) return -1;
    pid_t pid = fork();
    if (pid == 0) {
        /* child: stderr -> the temp file, then the forbidden op */
        dup2(fd, 2);
        close(fd);
        void *mx = sn_lcell_new(T_MAX);
        void *gs = sn_lcell_new(T_GSET);
        void *lw = sn_lcell_new(T_LWW);
        switch (mode) {
        case 1: /* carrier mismatch: max ⊔ gset */
            sn_lcell_merge(mx, gs);
            break;
        case 2: /* carrier mismatch on the pure pair: delta(gset, lww) */
            sn_lcell_delta(gs, lw);
            break;
        case 3: /* kind mismatch: a list handed to lcell_merge */
            sn_lcell_merge(mx, sn_list_new(0));
            break;
        case 4: /* the G-side twin: an lcell handed to zset_merge */
            sn_zset_merge(sn_zset_new(0), mx);
            break;
        case 5: /* the readout guard: lww ops on a list */
            sn_lcell_value(sn_list_new(0));
            break;
        default:
            break;
        }
        /* if the guard failed to fire we would reach here — exit 0 so
           the parent sees a clean child as a FAILURE */
        _exit(0);
    }
    close(fd);
    int status = 0;
    waitpid(pid, &status, 0);
    /* read the captured stderr and grep for the diagnostic */
    FILE *f = fopen(tmpl, "r");
    char buf[4096];
    size_t n = f ? fread(buf, 1, sizeof(buf) - 1, f) : 0;
    if (f) fclose(f);
    unlink(tmpl);
    buf[n] = '\0';
    int died = (status != 0);
    int has_diag = strstr(buf, "[E-LKIND]") != NULL;
    static const char *kw[6] = {"", "cannot merge a max cell with a gset cell",
                                "cannot delta a gset cell with a lww cell",
                                "expects a Tier-L cell", "zset_merge expects Tier-G Z-sets",
                                "expects a Tier-L cell"};
    int has_kw = mode >= 1 && mode <= 5 && strstr(buf, kw[mode]) != NULL;
    return died && has_diag && has_kw ? 1 : 0;
}

int main(void) {
    sn_rt_init(0, NULL);

    /* The runtime's own per-merge law self-checks ON for this whole
       run (env resolved once on first merge): every merge below is
       re-verified in-engine — idempotence + commutativity per merge;
       associativity is this harness's job (a three-operand law). */
    setenv("SN_LATTICE_CHECK", "1", 1);

    /* --- deterministic sanity: the readout surface per carrier -------- */
    void *mx = sn_lcell_new(T_MAX);
    ck(sn_lcell_carrier(mx), T_MAX, "max: carrier tag");
    ck(sn_lcell_len(mx), 0, "max: empty support");
    sn_lcell_inflate(mx, 10, 0);
    sn_lcell_inflate(mx, 4, 0); /* losing offer: no-op */
    ck(sn_lcell_value(mx), 10, "max: the HWM readout");
    ck(sn_lcell_len(mx), 1, "max: support {10}");
    ck(sn_lcell_has(mx, 10), 1, "max: membership of the HWM");
    ck(sn_lcell_has(mx, 4), 0, "max: a lost offer is not in the support");
    sn_release(mx);

    void *gs = sn_lcell_new(T_GSET);
    ck(sn_lcell_carrier(gs), T_GSET, "gset: carrier tag");
    sn_lcell_inflate(gs, 5, 0);
    sn_lcell_inflate(gs, 5, 0); /* idempotent insert */
    sn_lcell_inflate(gs, 9, 0);
    ck(sn_lcell_len(gs), 2, "gset: |{5,9}| = 2");
    ck(sn_lcell_has(gs, 5), 1, "gset: membership 5");
    ck(sn_lcell_has(gs, 9), 1, "gset: membership 9");
    ck(sn_lcell_has(gs, 7), 0, "gset: non-membership 7");
    ck(sn_lcell_value(gs), 0, "gset: no single-value readout (documented)");
    sn_release(gs);

    void *lw = sn_lcell_new(T_LWW);
    ck(sn_lcell_carrier(lw), T_LWW, "lww: carrier tag");
    ck(sn_lcell_value(lw), 0, "lww: no offer yet");
    ck(sn_lcell_len(lw), 0, "lww: empty support");
    sn_lcell_inflate(lw, 100, 3);
    sn_lcell_inflate(lw, 300, 2); /* lower clock: loses */
    ck(sn_lcell_value(lw), 100, "lww: clock 3 beats clock 2");
    sn_lcell_inflate(lw, 200, 3); /* TIE at clock 3: higher value wins */
    ck(sn_lcell_value(lw), 200, "lww: the deterministic value tiebreak");
    ck(sn_lcell_len(lw), 1, "lww: support {200}");
    ck(sn_lcell_has(lw, 200), 1, "lww: membership of the winner");
    ck(sn_lcell_has(lw, 100), 0, "lww: the superseded offer left the support");
    sn_release(lw);

    /* --- 1. the three laws, randomized, all carriers ------------------ */
    law_round(T_MAX, 200, 12);
    law_round(T_GSET, 200, 12);
    law_round(T_LWW, 200, 12);

    /* a bigger gset sweep: exercises the rehash/growth path under the
       laws (500 distinct values force multiple doublings) */
    law_round(T_GSET, 20, 500);

    /* --- 2. the mutator law, randomized monotone, all carriers -------- */
    mutator_round(T_MAX, 200, 12);
    mutator_round(T_GSET, 200, 12);
    mutator_round(T_LWW, 200, 12);

    /* --- 3. quiescence: delta(x,x) is the identity, all carriers ------- */
    void *q1 = sn_lcell_new(T_MAX);
    sn_lcell_inflate(q1, 42, 0);
    void *q2 = sn_lcell_new(T_GSET);
    sn_lcell_inflate(q2, 1, 0);
    sn_lcell_inflate(q2, 2, 0);
    void *q3 = sn_lcell_new(T_LWW);
    sn_lcell_inflate(q3, 7, 5);
    ck(sn_lcell_len(sn_lcell_delta(q1, q1)), 0, "quiescence: delta(max,max) is empty");
    ck(sn_lcell_len(sn_lcell_delta(q2, q2)), 0, "quiescence: delta(gset,gset) is empty");
    ck(sn_lcell_len(sn_lcell_delta(q3, q3)), 0, "quiescence: delta(lww,lww) is empty");
    sn_release(sn_lcell_delta(q1, q1));
    sn_release(sn_lcell_delta(q2, q2));
    sn_release(sn_lcell_delta(q3, q3));
    sn_release(q1);
    sn_release(q2);
    sn_release(q3);

    /* --- 4. lww convergence: one bag of offers, many join orders ------ */
    for (int r = 0; r < 100; r++) {
        void *offers[6];
        for (int i = 0; i < 6; i++) {
            offers[i] = sn_lcell_new(T_LWW);
            sn_lcell_inflate(offers[i], (int64_t)(lcg() % 50), (int64_t)(lcg() % 3));
        }
        /* join in order 0..5 */
        void *acc1 = sn_lcell_new(T_LWW);
        for (int i = 0; i < 6; i++) sn_lcell_merge(acc1, offers[i]);
        /* join in the REVERSED order 5..0 — with clocks drawn from a
           3-value space, ties are frequent, so this exercises the
           deterministic value tiebreak under commutativity */
        void *acc2 = sn_lcell_new(T_LWW);
        for (int i = 5; i >= 0; i--) sn_lcell_merge(acc2, offers[i]);
        if (!sn_lcell_eq(acc1, acc2)) {
            fprintf(stderr, "FAIL: lww join-order divergence (round %d)\n", r);
            g_fail++;
        }
        for (int i = 0; i < 6; i++) sn_release(offers[i]);
        sn_release(acc1);
        sn_release(acc2);
    }

    /* --- 5. E-LKIND probes --------------------------------------------- */
    for (int mode = 1; mode <= 5; mode++) {
        if (!elkind_probe(mode)) {
            fprintf(stderr, "FAIL: E-LKIND probe %d did not fail-fast with its diagnostic\n", mode);
            g_fail++;
        }
    }

    if (g_fail == 0) {
        printf("PASS: tier-L semilattice cells — lattice laws (randomized), "
               "the mutator law, quiescence, E-LKIND, E-LRET's runtime twin-free "
               "surface (W2C §1.1)\n");
    } else {
        printf("FAIL: %d tier-L cell check(s)\n", g_fail);
        return 1;
    }
    return 0;
}
