/*
 * Systems Test: the incremental view engine (SWARM RABBIT HOLE, Problem-5
 * second slice — Impl-H, Cohort Delta, Wave 7).
 * =====================================================================
 *
 * Drives the runtime delta-propagation circuit end to end with an
 * INDEPENDENT reference model (plain C arrays — no runtime view rule is
 * consulted for any expected value), and proves the claims the slice
 * owes:
 *
 *   1. THE CHAIN TEST (source -> FILTER -> MAP -> AGG_SUM), per round:
 *      (a) the work bound — probe counts are EXACT hash-map operation
 *          counts (never wall time): 3 ops per changed key per linear
 *          view; the downstream batch sizes are the |Δv| terms of the
 *          general law W <= c_F*(|dA| + Σ|Δv|) (W2C §2.1/§2.2);
 *      (b) every committed delta 𝒟(output) equals recomputing from
 *          scratch (the reference model's F(new) − F(old)): a key
 *          whose weight vanishes carries its NEGATIVE entry in the
 *          changelog — only exactly-cancelled deltas (net 0 within
 *          the round) are pruned;
 *      (c) retraction — negative deltas remove from the aggregate; a
 *          weight returning to exactly 0 vanishes from the filter
 *          output, and an entire aggregate group zeroed out round-by-
 *          round vanishes from the maintained output;
 *      (d) the invariant output = F(input) holds after every round.
 *   2. THE |A|-INDEPENDENCE TEST: the same batch against a 100-key and
 *      a 100,000-key source produces IDENTICAL probe counts — the
 *      O(|dA|) claim is a function of |dA| and the circuit shape, not
 *      of the state size (the probe-linear bound, W2C §2.2).
 *   3. AGGREGATE-COUNT: presence flips cost 1 + 2·[flip] ops each
 *      (negative weights are present — Tier-G honesty); group counts
 *      track the distinct present keys exactly.
 *   4. QUIESCENCE (W4C v2.1 repair 3, tier G): an empty batch costs 0
 *      probes and every commit returns 𝒟(∫α) = 0. The tier-L test
 *      F(s) ⊑ s is future work — no L carriers exist in this slice.
 *
 * Salt ζ (the fixpoint bracket, W2C §3) is explicitly future work:
 * nothing here exercises cycles or recursion.
 *
 * Built twice by tests/systems/run_all.sh: plain, then under ASan.
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

/* compiler/runtime.c public surface (compiled together with this file) */
void *sn_zset_new(int64_t key_kind);
void sn_zset_add(void *z, int64_t key, int64_t delta);
void *sn_zset_copy(void *z);
int64_t sn_zset_count(void *z, int64_t key);
int64_t sn_zset_total(void *z);
int64_t sn_zset_len(void *z);
void *sn_zset_keys(void *z);
void *sn_view_new(int64_t kind, void *input, int64_t p1, int64_t p2);
void sn_view_apply_delta(void *view, void *delta);
void *sn_view_commit(void *view);
void *sn_view_output(void *view);
void sn_view_release(void *view);
void sn_view_probe_reset(void);
int64_t sn_view_probe_count(void);
void sn_release(void *p);
/* only len/items of List are touched below (elem_kind 0 = i64 items) */
struct List { int64_t rc, kind, elem_kind, len, cap; int64_t *items; };

#define SN_VIEW_FILTER 1
#define SN_VIEW_MAP 2
#define SN_VIEW_AGG_SUM 3
#define SN_VIEW_AGG_COUNT 4

static void ck(int cond, const char *what) {
    if (!cond) {
        fprintf(stderr, "FAIL: %s\n", what);
        exit(1);
    }
}

/* ------------------- the independent reference model ------------------- */
/* Plain arrays. src[k] = source weight; f1[k] = filter output weight;
 * f2[y] = map output weight; f3[g] = aggregate sum. Arrays are sized
 * `cap` (room for out-of-fill keys added by later batches); only
 * 0..fill-1 receive initial weights. */
typedef struct {
    int64_t n;   /* array capacity / key span: 0..n-1 */
    int64_t thr; /* FILTER keeps k >= thr */
    int64_t m;   /* AGG modulus */
    int64_t a;   /* MAP: y = a*k (group homomorphism, b = 0) */
    int64_t *src, *f1, *f2, *f3;
} Model;

static void model_init(Model *mo, int64_t cap, int64_t fill, int64_t thr,
                       int64_t m, int64_t a) {
    mo->n = cap;
    mo->thr = thr;
    mo->m = m;
    mo->a = a;
    mo->src = calloc((size_t)cap, sizeof(int64_t));
    mo->f1 = calloc((size_t)cap, sizeof(int64_t));
    mo->f2 = calloc((size_t)a * cap + 1, sizeof(int64_t));
    mo->f3 = calloc((size_t)m, sizeof(int64_t));
    ck(mo->src && mo->f1 && mo->f2 && mo->f3, "model alloc");
    for (int64_t k = 0; k < fill; k++) mo->src[k] = 1 + k % 3;
    for (int64_t k = 0; k < cap; k++) {
        if (k >= thr && mo->src[k] != 0) {
            mo->f1[k] = mo->src[k];
            mo->f2[a * k] += mo->f1[k];
            mo->f3[((a * k) % m + m) % m] += mo->f1[k];
        }
    }
}

static void model_free(Model *mo) {
    free(mo->src); free(mo->f1); free(mo->f2); free(mo->f3);
}

/* Accumulator for expected deltas: (key, delta), add-with-combine. */
typedef struct {
    int64_t keys[600];
    int64_t deltas[600];
    int64_t n;
} Batch;

static void batch_add(Batch *b, int64_t key, int64_t d) {
    for (int64_t i = 0; i < b->n; i++) {
        if (b->keys[i] == key) { b->deltas[i] += d; return; }
    }
    ck(b->n < 600, "expected-delta accumulator overflow");
    b->keys[b->n] = key;
    b->deltas[b->n] = d;
    b->n++;
}

/* Apply a source batch to the model, producing the three expected
 * commit deltas along the way (the scratch-recompute differences).
 * Robust to non-injective maps (first-occurrence accumulation). */
static void model_apply(Model *mo, Batch *in, Batch *d1, Batch *d2, Batch *d3) {
    d1->n = d2->n = d3->n = 0;
    /* expected d1: the changelog = F1(new) − F1(old) per key: for a
     * passing key the value IS d (nonzero by batch construction) — a
     * key whose weight vanishes gets its NEGATIVE entry, it does not
     * disappear from the changelog. Only exactly-cancelled deltas
     * (net 0 within the round) are pruned. */
    for (int64_t i = 0; i < in->n; i++) {
        int64_t k = in->keys[i], d = in->deltas[i];
        if (k >= mo->thr && d != 0) batch_add(d1, k, d);
    }
    /* expected d2: map the d1 entries; combine per image key; the
     * changelog value is the net (pruned only when the net is 0) */
    for (int64_t i = 0; i < d1->n; i++) {
        int64_t y = mo->a * d1->keys[i];
        int64_t j = 0;
        for (; j < i; j++) {
            if (mo->a * d1->keys[j] == y) break; /* already emitted */
        }
        if (j < i) continue;
        int64_t acc = 0;
        for (j = 0; j < d1->n; j++) {
            if (mo->a * d1->keys[j] == y) acc += d1->deltas[j];
        }
        if (acc != 0) batch_add(d2, y, acc);
    }
    /* expected d3: group the d2 entries; combine per group */
    for (int64_t i = 0; i < d2->n; i++) {
        int64_t g = ((d2->keys[i] % mo->m) + mo->m) % mo->m;
        int64_t j = 0;
        for (; j < i; j++) {
            if (((d2->keys[j] % mo->m) + mo->m) % mo->m == g) break;
        }
        if (j < i) continue;
        int64_t acc = 0;
        for (j = 0; j < d2->n; j++) {
            if (((d2->keys[j] % mo->m) + mo->m) % mo->m == g) acc += d2->deltas[j];
        }
        if (acc != 0) batch_add(d3, g, acc);
    }
    /* advance the model state */
    for (int64_t i = 0; i < in->n; i++) {
        int64_t k = in->keys[i], d = in->deltas[i];
        mo->src[k] += d;
        if (k >= mo->thr) {
            mo->f2[mo->a * k] += d;
            mo->f3[((mo->a * k) % mo->m + mo->m) % mo->m] += d;
        }
        mo->f1[k] = (k >= mo->thr) ? mo->src[k] : 0;
    }
}

/* Compare a runtime Z-set against expected (key, delta) pairs: every
 * entry present with the exact weight, correct support size and total. */
static void ck_zset_matches(void *z, Batch *exp, const char *what) {
    if (sn_zset_len(z) != exp->n) {
        fprintf(stderr, "FAIL: %s: len runtime=%lld model=%lld\n", what,
                (long long)sn_zset_len(z), (long long)exp->n);
        exit(1);
    }
    int64_t total = 0;
    for (int64_t i = 0; i < exp->n; i++) {
        int64_t got = sn_zset_count(z, exp->keys[i]);
        if (got != exp->deltas[i]) {
            fprintf(stderr, "FAIL: %s: key %lld runtime=%lld model=%lld\n", what,
                    (long long)exp->keys[i], (long long)got,
                    (long long)exp->deltas[i]);
            exit(1);
        }
        total += exp->deltas[i];
    }
    if (sn_zset_total(z) != total) {
        fprintf(stderr, "FAIL: %s: total runtime=%lld model=%lld\n", what,
                (long long)sn_zset_total(z), (long long)total);
        exit(1);
    }
}

/* Full-output invariant check against the model arrays. */
static void ck_full_output(void *z, Model *mo, int which, const char *what) {
    struct List *keys = sn_zset_keys(z);
    int64_t support = 0;
    for (int64_t i = 0; i < keys->len; i++) {
        int64_t k = keys->items[i];
        int64_t w = (which == 1) ? mo->f1[k] : (which == 2) ? mo->f2[k] : mo->f3[k];
        int64_t got = sn_zset_count(z, k);
        if (got != w) {
            fprintf(stderr, "FAIL: %s: key %lld runtime=%lld model=%lld\n",
                    what, (long long)k, (long long)got, (long long)w);
            exit(1);
        }
        if (w != 0) support++;
    }
    int64_t span = (which == 1) ? mo->n : (which == 2) ? mo->a * mo->n : mo->m;
    for (int64_t k = 0; k < span; k++) {
        int64_t w = (which == 1) ? mo->f1[k] : (which == 2) ? mo->f2[k] : mo->f3[k];
        if (w == 0) continue;
        char msg[128];
        snprintf(msg, sizeof msg, "%s: missing present key %lld", what, (long long)k);
        ck(sn_zset_count(z, k) == w, msg);
    }
    ck(keys->len == support, what);
    sn_release(keys);
}

/* --------------------------- the chain harness -------------------------- */

typedef struct {
    void *src, *v1, *v2, *v3;
} Chain;

static void chain_build(Chain *c, Model *mo) {
    c->src = sn_zset_new(0);
    for (int64_t k = 0; k < mo->n; k++) {
        if (mo->src[k] != 0) sn_zset_add(c->src, k, mo->src[k]);
    }
    /* the chain: each view OWNS the integration of its input stream
     * (DBSP's per-node integrator): a downstream view wraps its OWN
     * COPY of the upstream output. Sharing one object would
     * double-integrate every delta — the upstream apply and the
     * downstream fold would both write it. */
    c->v1 = sn_view_new(SN_VIEW_FILTER, c->src, 2, mo->thr);       /* k >= thr */
    void *in2 = sn_zset_copy(sn_view_output(c->v1));
    c->v2 = sn_view_new(SN_VIEW_MAP, in2, 2, 0);                   /* y = 2k   */
    sn_release(in2); /* the view retained its own reference */
    void *in3 = sn_zset_copy(sn_view_output(c->v2));
    c->v3 = sn_view_new(SN_VIEW_AGG_SUM, in3, mo->m, 0);
    sn_release(in3);
}

static void chain_release(Chain *c) {
    sn_view_release(c->v3);
    sn_view_release(c->v2);
    sn_view_release(c->v1);
    sn_release(c->src);
}

/* One round: build dA from a Batch, drive the chain, assert everything.
 * Returns the total probe count for the round. */
static int64_t chain_round(Chain *c, Model *mo, Batch *in, int round) {
    void *dA = sn_zset_new(0);
    for (int64_t i = 0; i < in->n; i++) {
        if (in->deltas[i] != 0) sn_zset_add(dA, in->keys[i], in->deltas[i]);
    }
    int64_t nb = sn_zset_len(dA);

    Batch e1, e2, e3;
    model_apply(mo, in, &e1, &e2, &e3);

    /* (a) the work bound: exact probe counts, per view. FILTER costs
     * 3 ops per passing key and 1 per dropped key (input fold only). */
    int64_t pass = 0;
    for (int64_t i = 0; i < in->n; i++) {
        if (in->deltas[i] != 0 && in->keys[i] >= mo->thr) pass++;
    }
    sn_view_probe_reset();
    sn_view_apply_delta(c->v1, dA);
    int64_t p1 = sn_view_probe_count();
    ck(p1 == 3 * pass + (nb - pass), "v1 probe count != 3|passing|+1|dropped|");

    void *d1 = sn_view_commit(c->v1);
    ck_zset_matches(d1, &e1, "commit(v1): delta != scratch-recompute diff");

    sn_view_probe_reset();
    sn_view_apply_delta(c->v2, d1);
    int64_t p2 = sn_view_probe_count();
    ck(p2 == 3 * sn_zset_len(d1), "v2 probe count != 3|Δv1|");

    void *d2 = sn_view_commit(c->v2);
    ck_zset_matches(d2, &e2, "commit(v2): delta != scratch-recompute diff");

    sn_view_probe_reset();
    sn_view_apply_delta(c->v3, d2);
    int64_t p3 = sn_view_probe_count();
    ck(p3 == 3 * sn_zset_len(d2), "v3 probe count != 3|Δv2|");

    void *d3 = sn_view_commit(c->v3);
    ck_zset_matches(d3, &e3, "commit(v3): delta != scratch-recompute diff");

    /* (d) the invariant: output = F(input) after every round */
    ck_full_output(sn_view_output(c->v1), mo, 1, "invariant: v1.output != F1(src)");
    ck_full_output(sn_view_output(c->v2), mo, 2, "invariant: v2.output != F2(F1)");
    ck_full_output(sn_view_output(c->v3), mo, 3, "invariant: v3.output != F3(F2)");

    printf("round %d: |dA|=%lld probes(v1)=%lld |d1|=%lld probes(v2)=%lld "
           "|d2|=%lld probes(v3)=%lld |d3|=%lld\n",
           round, (long long)nb, (long long)p1, (long long)sn_zset_len(d1),
           (long long)p2, (long long)sn_zset_len(d2), (long long)p3,
           (long long)sn_zset_len(d3));

    sn_release(dA); sn_release(d1); sn_release(d2); sn_release(d3);
    return p1 + p2 + p3;
}

/* -------------------------------- main ---------------------------------- */

int main(void) {
    /* ---------------- the main chain: fill 2000 ---------------- */
    enum { FILL = 2000, CAP = 4096, THR = FILL / 2, MOD = 7 };
    Model mo;
    model_init(&mo, CAP, FILL, THR, MOD, 2);

    Chain c;
    chain_build(&c, &mo);
    ck_full_output(sn_view_output(c.v1), &mo, 1, "init: v1.output != F1(src)");
    ck_full_output(sn_view_output(c.v2), &mo, 2, "init: v2.output != F2(F1)");
    ck_full_output(sn_view_output(c.v3), &mo, 3, "init: v3.output != F3(F2)");

    /* round 0: the commits of the INITIAL materialization: the ℐ step
     * leaves no pending — a fresh view commits the empty delta. */
    void *i1 = sn_view_commit(c.v1);
    void *i2 = sn_view_commit(c.v2);
    void *i3 = sn_view_commit(c.v3);
    ck(sn_zset_len(i1) == 0 && sn_zset_len(i2) == 0 && sn_zset_len(i3) == 0,
       "init: fresh views must commit empty deltas");
    sn_release(i1); sn_release(i2); sn_release(i3);

    /* ---- round 1: mixed additions, updates, filtered-out keys ---- */
    Batch in1 = {0};
    batch_add(&in1, 10, 5);      /* below THR: filtered out        */
    batch_add(&in1, 1100, 4);    /* passing: update                */
    batch_add(&in1, 1200, -2);   /* passing: partial retraction    */
    batch_add(&in1, 1500, -50);  /* NEGATIVE weight: honest Tier-G */
    batch_add(&in1, 3000, 7);    /* new key outside the key space  */
    batch_add(&in1, 1999, 1);
    chain_round(&c, &mo, &in1, 1);

    /* ---- round 2: (c) retraction — a passing key zeroed EXACTLY ---- */
    Batch in2 = {0};
    batch_add(&in2, 1100, -mo.src[1100]); /* weight -> 0: must VANISH */
    batch_add(&in2, 1300, 3);
    batch_add(&in2, 20, 1);               /* below THR: no effect    */
    int64_t gone = 1100;
    chain_round(&c, &mo, &in2, 2);
    ck(sn_zset_count(sn_view_output(c.v1), gone) == 0,
       "retraction: zeroed key still present in filter output");
    printf("retraction: key %lld weight -> 0, pruned from filter output "
           "(support now %lld)\n",
           (long long)gone, (long long)sn_zset_len(sn_view_output(c.v1)));

    /* ---- round 3: churn, including full retraction of round-1 keys ---- */
    Batch in3 = {0};
    for (int64_t k = THR + 1; k < THR + 12; k++) batch_add(&in3, k, (k % 5) - 2);
    batch_add(&in3, 3000, -7); /* retract round-1's new key entirely */
    batch_add(&in3, 1500, 50);  /* back to zero from the negative    */
    chain_round(&c, &mo, &in3, 3);
    ck(sn_zset_count(sn_view_output(c.v1), 3000) == 0,
       "retraction: fully retracted new key still present");

    /* ---- round 4: (c) an ENTIRE aggregate group zeroed out ----
     * Target group g* = 0: retract every passing key k with
     * (2k mod 7) == g* to weight exactly 0. The group's sum returns to
     * exactly 0 and the group must vanish from the aggregate output. */
    int64_t g_target = 0;
    Batch in4 = {0};
    for (int64_t k = THR; k < mo.n; k++) {
        if (((2 * k) % MOD) == g_target && mo.src[k] != 0) {
            batch_add(&in4, k, -mo.src[k]);
        }
    }
    chain_round(&c, &mo, &in4, 4);
    ck(sn_zset_count(sn_view_output(c.v3), g_target) == 0,
       "group retraction: zeroed group still present in aggregate");
    printf("group retraction: group %lld sum -> 0, pruned from aggregate "
           "(groups left: %lld of %d)\n",
           (long long)g_target, (long long)sn_zset_len(sn_view_output(c.v3)), MOD);

    /* ---- round 5: re-derive the group (re-additions) ---- */
    Batch in5 = {0};
    for (int64_t k = THR; k < THR + 10; k++) {
        if (((2 * k) % MOD) == g_target) batch_add(&in5, k, 2 + k % 3);
    }
    chain_round(&c, &mo, &in5, 5);
    ck(sn_zset_count(sn_view_output(c.v3), g_target) != 0,
       "group re-derivation: re-added group did not return");
    printf("group re-derivation: group %lld sum = %lld\n", (long long)g_target,
           (long long)sn_zset_count(sn_view_output(c.v3), g_target));

    /* ---- quiescence (W4C repair 3, tier G): empty batch ---- */
    void *empty = sn_zset_new(0);
    sn_view_probe_reset();
    sn_view_apply_delta(c.v1, empty);
    sn_view_apply_delta(c.v2, empty);
    sn_view_apply_delta(c.v3, empty);
    ck(sn_view_probe_count() == 0, "quiescence: empty batch must probe nothing");
    void *q1 = sn_view_commit(c.v1);
    void *q2 = sn_view_commit(c.v2);
    void *q3 = sn_view_commit(c.v3);
    ck(sn_zset_len(q1) == 0 && sn_zset_len(q2) == 0 && sn_zset_len(q3) == 0,
       "quiescence: settled chain must commit empty deltas (D(integral)=0)");
    printf("quiescence: empty batch -> 0 probes, all commits empty "
           "(tier-G test D(int alpha) = 0)\n");
    sn_release(empty); sn_release(q1); sn_release(q2); sn_release(q3);

    chain_release(&c);
    model_free(&mo);

    /* ---------------- |A|-independence: 100 vs 100,000 ----------------
     * Same transformation (thr=40 for both), same batches; only the
     * source size differs. Probe counts must be IDENTICAL. */
    enum { SMALL = 100, BIG = 100000, IND_THR = 40 };
    Model m_small, m_big;
    model_init(&m_small, SMALL, SMALL, IND_THR, MOD, 2);
    model_init(&m_big, BIG, BIG, IND_THR, MOD, 2);
    Chain cs, cb;
    chain_build(&cs, &m_small);
    chain_build(&cb, &m_big);

    for (int r = 0; r < 3; r++) {
        /* the SAME 10-key batch for both chains */
        Batch b = {0};
        for (int64_t i = 0; i < 10; i++) {
            int64_t k = 20 + 7 * i; /* crosses thr: mix of dropped/passing */
            batch_add(&b, k, (i % 3) - 1);
        }
        int64_t ps = chain_round(&cs, &m_small, &b, 100 + r);
        int64_t pb = chain_round(&cb, &m_big, &b, 200 + r);
        ck(ps == pb, "|A|-independence: probe counts differ across source sizes");
        printf("|A|-independence: round %d, same dA: probes small(%d keys) == "
               "big(%d keys) == %lld\n",
               r, SMALL, BIG, (long long)ps);
    }
    chain_release(&cs);
    chain_release(&cb);
    model_free(&m_small);
    model_free(&m_big);

    /* ---------------- AGGREGATE-COUNT: presence flips ---------------- */
    enum { NC = 64, CN_MOD = 5 };
    Model mc;
    mc.n = NC; mc.thr = 0; mc.m = CN_MOD; mc.a = 1;
    mc.src = calloc(NC, sizeof(int64_t));
    mc.f1 = calloc(NC, sizeof(int64_t));
    mc.f2 = calloc(NC, sizeof(int64_t));
    mc.f3 = calloc(CN_MOD, sizeof(int64_t));
    ck(mc.src && mc.f1 && mc.f2 && mc.f3, "count model alloc");

    void *src_c = sn_zset_new(0);
    void *vc = sn_view_new(SN_VIEW_AGG_COUNT, src_c, CN_MOD, 0);
    ck(sn_zset_len(sn_view_output(vc)) == 0, "count: fresh view not empty");

    struct { int64_t key, delta, flips; const char *what; } ops[] = {
        {3, 5, 1, "add 0->5: presence begins"},
        {3, 3, 0, "add 5->8: no flip, 1 probe"},
        {3, -8, 1, "retract 8->0: presence ends"},
        {3, 2, 1, "re-add 0->2"},
        {3, -2, 1, "retract 2->0 again"},
        {8, 1, 1, "key 8 (group 3): count 0->1"},
        {13, 1, 1, "key 13 (group 3): count 1->2"},
        {8, -1, 1, "retract key 8: count 2->1"},
        {13, -1, 1, "retract key 13: count 1->0, group vanishes"},
        {3, -4, 1, "NEGATIVE weight 0->-4: present (Tier-G honesty)"},
        {3, 4, 1, "retract the negative: back to 0, present ends"},
    };
    int64_t expect_count[CN_MOD];
    for (int64_t g = 0; g < CN_MOD; g++) expect_count[g] = 0;
    for (unsigned long i = 0; i < sizeof ops / sizeof ops[0]; i++) {
        int64_t k = ops[i].key, d = ops[i].delta;
        void *one = sn_zset_new(0);
        sn_zset_add(one, k, d);
        int64_t old = mc.src[k];
        mc.src[k] += d;
        int64_t g = ((k % CN_MOD) + CN_MOD) % CN_MOD;
        if ((old != 0) != (mc.src[k] != 0)) {
            expect_count[g] += (mc.src[k] != 0) ? 1 : -1;
        }
        sn_view_probe_reset();
        sn_view_apply_delta(vc, one);
        int64_t p = sn_view_probe_count();
        ck(p == 1 + 2 * ops[i].flips, ops[i].what);
        ck(sn_zset_count(sn_view_output(vc), g) == expect_count[g], ops[i].what);
        printf("count op %-44s probes=%lld group(%lld)=%lld\n", ops[i].what,
               (long long)p, (long long)g, (long long)expect_count[g]);
        sn_release(one);
    }
    /* the count view's full invariant: group g == # present keys */
    for (int64_t g = 0; g < CN_MOD; g++) {
        int64_t live = 0;
        for (int64_t k = 0; k < NC; k++) {
            if (((k % CN_MOD) + CN_MOD) % CN_MOD == g && mc.src[k] != 0) live++;
        }
        ck(sn_zset_count(sn_view_output(vc), g) == live,
           "count invariant: group != # present keys");
    }
    /* quiescence for the count view too */
    void *empty_c = sn_zset_new(0);
    sn_view_probe_reset();
    sn_view_apply_delta(vc, empty_c);
    ck(sn_view_probe_count() == 0, "count quiescence: empty batch probed");
    void *qc = sn_view_commit(vc);
    ck(sn_zset_len(qc) == 0, "count quiescence: commit not empty");
    sn_release(empty_c); sn_release(qc);

    sn_view_release(vc);
    sn_release(src_c);
    model_free(&mc);

    printf("PASS: incremental view engine — delta propagation is O(|dA|) "
           "probes, chain deltas match scratch, retraction exact\n");
    return 0;
}
