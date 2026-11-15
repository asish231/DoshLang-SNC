/*
 * Systems Test: Salt ζ — the Fixpoint-Bracket Engine (SWARM RABBIT HOLE, Wave 9)
 * ============================================================================
 *
 * The ζ slice's runtime mechanics (Impl-N, Cohort Delta; W2C §3.2–§3.5).
 * Proves, at the runtime level, that the bracket IS the convergence
 * theorem made executable:
 *
 *   1. CLOSURE CORRECTNESS vs a reference bitset BFS: over randomized
 *      seeded-LCG graphs (deterministic across runs and platforms) AND
 *      known-shape graphs (the chain, the star), both tiers (Tier G:
 *      the Z-set presence carrier; Tier L: the GSET ⊔=union carrier)
 *      reach exactly the reference backward closure — size, per-node
 *      membership, and the full state set.
 *
 *   2. SEMI-NAÏVE DELTAS — the Thm 4b claim, probe-counted: driving
 *      the bracket round by round, each round's derived delta equals
 *      the reference BFS LEVEL size exactly, and each round's
 *      adjacency-probe count equals Σ indeg over ONLY that round's
 *      input delta — per-round work O(|δ|), never O(|state|). Each
 *      reached node enters a delta exactly once, so its adjacency is
 *      probed exactly once across the whole bracket: Σ probes is
 *      exactly Σ indeg over the reached set — the naive engine's
 *      (rounds+1)·|E| rescans are the counterfactual (asserted on the
 *      chain, where both numbers are exact).
 *
 *   3. THE ≤ HEIGHT BOUND: rounds ≤ |U| (the presence carrier's
 *      height over the universe), with EQUALITY on the chain: the
 *      chain v=64 seeded at the far end takes exactly 63 productive
 *      rounds = the diameter = the longest derivation chain. The star
 *      takes 1 round for 257 facts — height prices DEPTH, not width.
 *
 *   4. THE PER-TIER QUIESCENCE TESTS, both exercised:
 *      Tier G — 𝒟(∫α) = 0: the terminating round's delta Z-set is
 *      empty (the round's return), and sn_zfix_quiescent_g's direct
 *      whole-state form holds after the run.
 *      Tier L — F(s) ⊑ s, the MUTATOR-LAW test (𝒟 presupposes ⊖,
 *      which does not and cannot exist on Tier L — Grothendieck
 *      triviality, the E-LRET rejection): sn_zfix_quiescent_l holds
 *      after the run, and the raw mutator-law form is asserted
 *      directly: lcell_delta(x, x) is the empty fragment (⊥, no
 *      inflation).
 *
 *   5. THE §3.4 AMPLIFICATION, observed: on the chain, ONE seed fact
 *      (|ΔDB| = 1) drives 63 rounds and 63 derivations — W_μ is the
 *      ROUND-SUM c·(|δ₀^(r)| + Σ_v |Δv^(r)|), not bounded by |ΔDB|
 *      (the fraud-propagation shape; W2C §3.4).
 *
 *   6. GC SAFETY MID-BRACKET: the engine is a first-class tracked
 *      object (KIND_ZFIX) whose gc_mark arm traces its carriers — a
 *      collection fired between rounds must not disturb the bracket
 *      (the reason the engine is tracked where SnView is not: SNlang
 *      programs trigger threshold collections mid-bracket).
 *
 *   7. E-LKIND, the guard probes (fork'd children must die with the
 *      diagnostic): a non-engine handed to zfix ops; the per-tier
 *      quiescence predicates cross-called; a string-keyed edge Z-set
 *      (the ζ node universe is int64).
 *
 * The two divergent bodies (counting- / negation-through-recursion)
 * have NO runtime surface at all — the checker always rejects them
 * with E-ZADM (examples/zfix_count_fail.sn + zfix_negate_fail.sn +
 * golden.rs pin the compile-time side); there is nothing to probe here.
 *
 * The runner (tests/systems/run_all.sh) builds this twice: plain (17),
 * and with -fsanitize=address (17b — the ASan stress pass: no UAF over
 * the CSR build / round loop / mutator-law fragments, no leaks — every
 * engine, Z-set, cell, and list created here is released).
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

/* compiler/runtime.c public surface (compiled together with this file) */
void sn_rt_init(int argc, char **argv);
void *sn_zset_new(int64_t key_kind);
void sn_zset_add(void *p, int64_t key, int64_t delta);
int64_t sn_zset_len(void *p);
void *sn_zset_keys(void *p);
void *sn_zfix_new(void *edges, void *seed);
void *sn_zfix_new_l(void *edges, void *seed);
int64_t sn_zfix_round(void *fix);
int64_t sn_zfix_run(void *fix);
int64_t sn_zfix_rounds(void *fix);
int64_t sn_zfix_len(void *fix);
int64_t sn_zfix_has(void *fix, int64_t node);
int64_t sn_zfix_probes(void *fix);
void *sn_zfix_state(void *fix);
int64_t sn_zfix_quiescent_g(void *fix);
int64_t sn_zfix_quiescent_l(void *fix);
void sn_zfix_release(void *fix);
void *sn_lcell_delta(void *dst, void *prev);
int64_t sn_lcell_len(void *c);
void *sn_list_new(int64_t elem_kind);
int64_t sn_list_len(void *l);
int64_t sn_list_get_i64(void *l, int64_t i);
void sn_release(void *p);
void sn_gc_collect(void);
void sn_gc_root_push(void *p);
void sn_gc_collect_roots(void);

static int g_fail = 0;

static void ck(int64_t got, int64_t want, const char *what) {
    if (got != want) {
        fprintf(stderr, "FAIL: %s: got %lld, want %lld\n", what, (long long)got, (long long)want);
        g_fail++;
    }
}

/* The fixed-seed LCG (test_lcell's family): deterministic across runs
   and platforms, so any mismatch here reproduces bit-for-bit. */
static uint64_t g_lcg = 0x123456789abcdefULL;

static uint64_t lcg(void) {
    g_lcg = g_lcg * 6364136223846793005ULL + 1442695040888963407ULL;
    return g_lcg >> 33;
}

/* ------------------------- the reference ------------------------- */

/* A deterministic backward-closure reference: the dedup'd edge list,
 * the CSR in-neighbor lists, and the BFS levels from the seeds.
 * level_size[r] = |{v : dist(v) == r}| is the SEMI-NAÏVE per-round
 * delta the bracket must produce exactly; level_probes[r] =
 * Σ indeg over level r is the per-round adjacency-probe count (each
 * round touches only ITS delta's adjacency — the Thm 4b claim). */
typedef struct Ref {
    int64_t v;             /* the node universe size */
    int64_t e;             /* the DISTINCT edge count */
    int64_t *ea, *eb;      /* the distinct edge list */
    int64_t *indeg;        /* per node */
    int64_t *off, *pred;   /* CSR: node -> its in-neighbors */
    int64_t nseeds;
    int64_t *seeds;
    int64_t *dist;         /* BFS backward distance; -1 unreached */
    int64_t *level_size;   /* r = 0..levels */
    int64_t *level_probes; /* P_r = Σ indeg over {v : dist(v) == r} */
    int64_t levels;        /* max dist over the reached = expected rounds */
    int64_t reached;
    int64_t total_probes;  /* Σ_{r=0..levels} P_r */
} Ref;

static void ref_free(Ref *r) {
    free(r->ea);
    free(r->eb);
    free(r->indeg);
    free(r->off);
    free(r->pred);
    free(r->seeds);
    free(r->dist);
    free(r->level_size);
    free(r->level_probes);
}

/* The random shape: `want` LCG draws dedup'd through a v×v bitset
 * (the Z-set coalesces duplicate packed keys the same way — presence
 * is a set). Seeds: `nseeds` distinct LCG nodes. */
static void ref_build_random(Ref *r, int64_t v, int64_t want, int64_t nseeds, uint64_t lcg_seed) {
    memset(r, 0, sizeof(*r));
    g_lcg = lcg_seed;
    r->v = v;
    uint8_t *seen = (uint8_t *)calloc((size_t)v * (size_t)v, 1);
    r->ea = (int64_t *)malloc(sizeof(int64_t) * (size_t)want);
    r->eb = (int64_t *)malloc(sizeof(int64_t) * (size_t)want);
    r->indeg = (int64_t *)calloc((size_t)v, sizeof(int64_t));
    while (r->e < want) {
        int64_t a = (int64_t)(lcg() % (uint64_t)v);
        int64_t b = (int64_t)(lcg() % (uint64_t)v);
        if (seen[a * v + b]) continue;
        seen[a * v + b] = 1;
        r->ea[r->e] = a;
        r->eb[r->e] = b;
        r->indeg[b]++;
        r->e++;
    }
    free(seen);
    r->nseeds = 0;
    r->seeds = (int64_t *)malloc(sizeof(int64_t) * (size_t)nseeds);
    int64_t *sseen = (int64_t *)calloc((size_t)v, sizeof(int64_t));
    while (r->nseeds < nseeds) {
        int64_t s = (int64_t)(lcg() % (uint64_t)v);
        if (sseen[s]) continue;
        sseen[s] = 1;
        r->seeds[r->nseeds++] = s;
    }
    free(sseen);
}

/* The chain shape: 0→1→…→v-1, seed {v-1} — the height observable with
 * equality (rounds == v-1 == the diameter). */
static void ref_build_chain(Ref *r, int64_t v) {
    memset(r, 0, sizeof(*r));
    r->v = v;
    r->e = v - 1;
    r->ea = (int64_t *)malloc(sizeof(int64_t) * (size_t)r->e);
    r->eb = (int64_t *)malloc(sizeof(int64_t) * (size_t)r->e);
    r->indeg = (int64_t *)calloc((size_t)v, sizeof(int64_t));
    for (int64_t i = 0; i < r->e; i++) {
        r->ea[i] = i;
        r->eb[i] = i + 1;
        r->indeg[i + 1] = 1;
    }
    r->nseeds = 1;
    r->seeds = (int64_t *)malloc(sizeof(int64_t));
    r->seeds[0] = v - 1;
}

/* The star shape: i→0 for i = 1..v-1, seed {0} — 1 round, v facts:
 * height prices depth, not width. */
static void ref_build_star(Ref *r, int64_t v) {
    memset(r, 0, sizeof(*r));
    r->v = v;
    r->e = v - 1;
    r->ea = (int64_t *)malloc(sizeof(int64_t) * (size_t)r->e);
    r->eb = (int64_t *)malloc(sizeof(int64_t) * (size_t)r->e);
    r->indeg = (int64_t *)calloc((size_t)v, sizeof(int64_t));
    for (int64_t i = 0; i < r->e; i++) {
        r->ea[i] = i + 1;
        r->eb[i] = 0;
        r->indeg[0]++;
    }
    r->nseeds = 1;
    r->seeds = (int64_t *)malloc(sizeof(int64_t));
    r->seeds[0] = 0;
}

/* CSR + BFS + level accounting, shared by every shape. */
static void ref_finish(Ref *r) {
    int64_t v = r->v;
    r->off = (int64_t *)calloc((size_t)v + 1, sizeof(int64_t));
    int64_t acc = 0;
    for (int64_t i = 0; i < v; i++) {
        r->off[i] = acc;
        acc += r->indeg[i];
    }
    r->off[v] = acc;
    r->pred = (int64_t *)malloc(sizeof(int64_t) * (size_t)(acc > 0 ? acc : 1));
    int64_t *cur = (int64_t *)calloc((size_t)v, sizeof(int64_t));
    for (int64_t i = 0; i < r->e; i++) {
        int64_t b = r->eb[i];
        r->pred[r->off[b] + cur[b]++] = r->ea[i];
    }
    free(cur);

    /* BFS backward from the seeds over the in-neighbor lists */
    r->dist = (int64_t *)malloc(sizeof(int64_t) * (size_t)v);
    for (int64_t i = 0; i < v; i++) r->dist[i] = -1;
    int64_t *q = (int64_t *)malloc(sizeof(int64_t) * (size_t)v);
    int64_t qh = 0, qt = 0;
    for (int64_t i = 0; i < r->nseeds; i++) {
        if (r->dist[r->seeds[i]] == -1) {
            r->dist[r->seeds[i]] = 0;
            q[qt++] = r->seeds[i];
        }
    }
    r->reached = qt;
    while (qh < qt) {
        int64_t u = q[qh++];
        for (int64_t k = r->off[u]; k < r->off[u + 1]; k++) {
            int64_t a = r->pred[k];
            if (r->dist[a] == -1) {
                r->dist[a] = r->dist[u] + 1;
                r->reached++;
                q[qt++] = a;
            }
        }
    }
    r->levels = 0;
    for (int64_t i = 0; i < v; i++) {
        if (r->dist[i] > r->levels) r->levels = r->dist[i];
    }
    r->level_size = (int64_t *)calloc((size_t)r->levels + 2, sizeof(int64_t));
    r->level_probes = (int64_t *)calloc((size_t)r->levels + 2, sizeof(int64_t));
    for (int64_t i = 0; i < v; i++) {
        if (r->dist[i] < 0) continue;
        r->level_size[r->dist[i]]++;
        r->level_probes[r->dist[i]] += r->indeg[i];
    }
    r->total_probes = 0;
    for (int64_t rr = 0; rr <= r->levels; rr++) {
        r->total_probes += r->level_probes[rr];
    }
    free(q);
}

/* --------------------- one full config, one tier --------------------- */

static void run_config(const Ref *r, int tier, const char *name, int gc_midflight) {
    void *edges = sn_zset_new(0);
    for (int64_t i = 0; i < r->e; i++) {
        sn_zset_add(edges, r->ea[i] * 4294967296LL + r->eb[i], 1);
    }
    void *seed = sn_zset_new(0);
    for (int64_t i = 0; i < r->nseeds; i++) {
        sn_zset_add(seed, r->seeds[i], 1);
    }
    void *fix = tier == 0 ? sn_zfix_new(edges, seed) : sn_zfix_new_l(edges, seed);

    /* Drive round-by-round: the per-round delta must equal the BFS
       level EXACTLY, and the per-round probes must equal the level's
       indeg sum — each round touched ONLY its input delta's adjacency
       (Thm 4b). Iteration i consumes level i. */
    int64_t iter = 0;
    for (;;) {
        int64_t p0 = sn_zfix_probes(fix);
        int64_t dsize = sn_zfix_round(fix);
        int64_t dp = sn_zfix_probes(fix) - p0;
        if (gc_midflight && iter == 5) {
            /* the mid-bracket collection: the engine is a tracked
               object; this must not disturb the bracket (check 6).
               PRECISE roots, the substrate's own gc_collect discipline
               (the .sn codegen pushes every live pointer-holding local
               before collecting — the conservative stack scan cannot
               see a -O1 register-held pointer, so a raw
               sn_gc_collect here would sweep a LIVE engine). */
            /* (a) an ABANDONED engine — deliberately NOT rooted: the
               sweep must free it through gc_free's KIND_ZFIX arm (the
               CSR arrays + the shared block free) without corrupting
               the heap — the bench's unrooted loop-local shape. */
            {
                void *dead_edges = sn_zset_new(0);
                void *dead_seed = sn_zset_new(0);
                sn_zset_add(dead_seed, 7, 1);
                void *dead = sn_zfix_new(dead_edges, dead_seed);
                (void)dead; /* abandoned: not pushed, not released */
            }
            sn_gc_root_push(fix);
            sn_gc_root_push(edges);
            sn_gc_root_push(seed);
            sn_gc_collect_roots();
            /* the live engine is untouched; the abandoned one is
               gone — a fresh engine now proves the heap is sane. */
            {
                void *e2 = sn_zset_new(0);
                void *s2 = sn_zset_new(0);
                sn_zset_add(s2, 7, 1);
                void *f2 = sn_zfix_new(e2, s2);
                ck(sn_zfix_len(f2), 1, "the post-sweep engine sanity: the heap is intact");
                sn_zfix_release(f2);
                sn_release(s2);
                sn_release(e2);
            }
        }
        ck(dp, r->level_probes[iter], "per-round probes == the level's indeg sum (semi-naïve)");
        int64_t expect = (iter + 1 <= r->levels) ? r->level_size[iter + 1] : 0;
        ck(dsize, expect, "the round's delta == the next BFS level");
        if (dsize == 0) {
            ck(iter, r->levels, "the terminating round consumed the last level");
            break;
        }
        iter++;
    }

    /* 3. the ≤ height bound: presence over U has height |U|; the
       chain attains equality (diameter). */
    ck(sn_zfix_rounds(fix), r->levels, "rounds == the longest derivation chain");
    ck(sn_zfix_rounds(fix) <= r->v, 1, "rounds <= height (|U| for presence)");
    ck(sn_zfix_len(fix), r->reached, "|reach| == the reference closure");
    int64_t memb_bad = 0;
    for (int64_t i = 0; i < r->v; i++) {
        if (sn_zfix_has(fix, i) != (r->dist[i] >= 0)) memb_bad++;
    }
    ck(memb_bad, 0, "per-node membership vs the reference BFS");

    /* 4. the per-tier quiescence tests */
    if (tier == 0) {
        ck(sn_zfix_quiescent_g(fix), 1, "Tier-G stop test: 𝒟(∫α) = 0 (whole-state form)");
        void *st = sn_zfix_state(fix);
        ck(sn_zset_len(st), r->reached, "the state copy's support");
        void *ks = sn_zset_keys(st);
        ck(sn_list_len(ks), r->reached, "the state keys count");
        int64_t outside = 0;
        for (int64_t i = 0; i < sn_list_len(ks); i++) {
            int64_t k = sn_list_get_i64(ks, i);
            if (k < 0 || k >= r->v || r->dist[k] < 0) outside++;
        }
        ck(outside, 0, "every state key is a reached node (set equality by size+membership)");
        sn_release(ks);
        sn_release(st);
    } else {
        ck(sn_zfix_quiescent_l(fix), 1, "Tier-L stop test: F(s) ⊑ s (whole-state form)");
        void *st = sn_zfix_state(fix);
        ck(sn_lcell_len(st), r->reached, "the Tier-L state clone's support");
        /* the raw mutator-law form: delta(x, x) = ⊥ — no inflation */
        void *frag = sn_lcell_delta(st, st);
        ck(sn_lcell_len(frag), 0, "the mutator-law quiescence: delta(x,x) = ⊥");
        sn_release(frag);
        sn_release(st);
    }

    /* the total work observable: Σ probes == Σ indeg over the reached
       set (each reached node's adjacency visited exactly once) */
    ck(sn_zfix_probes(fix), r->total_probes, "Σ probes == Σ indeg over the reached (each node probed once)");

    printf("  %s tier=%s: v=%lld e=%lld seeds=%lld reached=%lld rounds=%lld probes=%lld%s\n",
           name, tier == 0 ? "G" : "L", (long long)r->v, (long long)r->e,
           (long long)r->nseeds, (long long)sn_zfix_len(fix),
           (long long)sn_zfix_rounds(fix), (long long)sn_zfix_probes(fix),
           gc_midflight ? " (gc mid-bracket)" : "");

    sn_zfix_release(fix);
    sn_release(seed);
    sn_release(edges);
}

/* --------------- 7. the E-LKIND guard probes (fork'd) --------------- */

static int elkind_probe(int mode) {
    char tmpl[] = "/tmp/snc_zfix_elkind_XXXXXX";
    int fd = mkstemp(tmpl);
    if (fd < 0) return -1;
    fflush(stdout); /* the children must not replay the parent's buffer */
    pid_t pid = fork();
    if (pid == 0) {
        dup2(fd, 2);
        close(fd);
        switch (mode) {
        case 1: { /* kind confusion: a list handed to zfix_len */
            sn_zfix_len(sn_list_new(0));
            break;
        }
        case 2: { /* tier confusion: the Tier-G stop test on a Tier-L fix */
            void *e = sn_zset_new(0);
            void *s = sn_zset_new(0);
            sn_zset_add(s, 1, 1);
            sn_zfix_quiescent_g(sn_zfix_new_l(e, s));
            break;
        }
        case 3: { /* tier confusion twin: the Tier-L stop test on a Tier-G fix */
            void *e = sn_zset_new(0);
            void *s = sn_zset_new(0);
            sn_zset_add(s, 1, 1);
            sn_zfix_quiescent_l(sn_zfix_new(e, s));
            break;
        }
        case 4: { /* the node universe is int64: a string-keyed edge Z-set */
            void *e = sn_zset_new(1); /* Str-keyed */
            void *s = sn_zset_new(0);
            sn_zfix_new(e, s);
            break;
        }
        default:
            break;
        }
        _exit(0); /* a clean child means the guard failed to fire */
    }
    close(fd);
    int status = 0;
    waitpid(pid, &status, 0);
    FILE *f = fopen(tmpl, "r");
    char buf[4096];
    size_t n = f ? fread(buf, 1, sizeof(buf) - 1, f) : 0;
    if (f) fclose(f);
    unlink(tmpl);
    buf[n] = '\0';
    int died = (status != 0);
    int has_diag = strstr(buf, "[E-LKIND]") != NULL;
    static const char *kw[5] = {"", "expects a Salt-ζ fixpoint engine",
                               "use zfix_quiescent_l", "use zfix_quiescent_g",
                               "node universe is int64"};
    int has_kw = mode >= 1 && mode <= 4 && strstr(buf, kw[mode]) != NULL;
    return died && has_diag && has_kw ? 1 : 0;
}

int main(void) {
    sn_rt_init(0, NULL);

    printf("== Salt ζ: the fixpoint-bracket engine (W2C §3; Thm 4b semi-naïve) ==\n");

    /* --- 1–4: the chain — the ≤ height bound with EQUALITY, and the
       §3.4 amplification: ONE seed fact, 63 rounds, 63 derivations
       (W_μ is the round-sum, not |ΔDB|-bounded). Both tiers. */
    {
        Ref r;
        ref_build_chain(&r, 64);
        ref_finish(&r);
        ck(r.levels, 63, "chain: the reference's longest chain");
        run_config(&r, 0, "chain", 0);
        run_config(&r, 1, "chain", 0);
        /* the semi-naïve vs naive gap, exact on the chain: the bracket
           probes 63 adjacencies total; the naive rescan would visit
           every edge every round: 64 × 63 = 4032. */
        printf("  chain: semi-naïve Σ probes = %lld vs naive (rounds+1)×|E| = %lld (the O(|δ|) vs O(|state|) gap)\n",
               (long long)r.total_probes, (long long)(r.levels + 1) * r.e);
        ref_free(&r);
    }

    /* --- the star: 1 round, 257 facts — height prices depth, not width */
    {
        Ref r;
        ref_build_star(&r, 257);
        ref_finish(&r);
        ck(r.levels, 1, "star: one productive level");
        run_config(&r, 0, "star", 0);
        run_config(&r, 1, "star", 0);
        ref_free(&r);
    }

    /* --- randomized seeded-LCG graphs, both tiers: closure correctness
       vs the reference BFS + the exact per-round accounting */
    {
        Ref r;
        ref_build_random(&r, 500, 2000, 3, 0x9e3779b97f4a7c15ULL);
        ref_finish(&r);
        run_config(&r, 0, "random-a", 0);
        run_config(&r, 1, "random-a", 0);
        ref_free(&r);
    }
    {
        Ref r;
        ref_build_random(&r, 2000, 8000, 5, 0xbf58476d1ce4e5b9ULL);
        ref_finish(&r);
        run_config(&r, 0, "random-b", 0);
        run_config(&r, 1, "random-b", 0);
        ref_free(&r);
    }

    /* --- 6: a collection fired MID-BRACKET must not disturb the
       engine (the tracked-object design's whole point) */
    {
        Ref r;
        ref_build_random(&r, 800, 3000, 2, 0x94d049bb133111ebULL);
        ref_finish(&r);
        run_config(&r, 0, "random-gc", 1);
        run_config(&r, 1, "random-gc", 1);
        ref_free(&r);
    }

    /* --- 7: the E-LKIND guard probes */
    for (int mode = 1; mode <= 4; mode++) {
        int ok = elkind_probe(mode);
        if (ok != 1) {
            fprintf(stderr, "FAIL: E-LKIND probe %d did not fail-fast with its diagnostic\n", mode);
            g_fail++;
        }
    }

    if (g_fail == 0) {
        printf("PASS: Salt ζ fixpoint bracket — closure vs reference BFS, semi-naïve per-round probes (Thm 4b), rounds <= height (equality on the chain), per-tier quiescence (𝒟=0 / F(s) ⊑ s), GC mid-bracket, E-LKIND guards (W2C §3.3–§3.5)\n");
        return 0;
    }
    fprintf(stderr, "%d check(s) failed\n", g_fail);
    return 1;
}
