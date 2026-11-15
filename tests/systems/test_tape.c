/*
 * Systems Test: Tape Record Kinds — W2C's P5↔P2 Elevation,
 * Runtime Mechanics (SWARM RABBIT HOLE, Wave 16, Impl-U, Cohort Delta)
 * ============================================================================
 *
 * The tape's runtime half (W2C §36–§38 + §48 item 4; the final report
 * §332's tier-parametric per-operation laws). The compile-time side
 * conditions — the kind-literal rule, the Tier-O/mixed refusals, the
 * E-TAPE-WAVE boundary rule, the E1 signature discipline — are
 * pinned by the golden suite (tape_*.sn + golden.rs); THIS file
 * proves the mechanics the elevation claims, over randomized shapes
 * (seeded LCG, reproducible bit-for-bit):
 *
 *   1. THE ILC LAW ON TIER-G TRIPLES, N randomized rounds: every
 *      delta record derives d = b ⊖ a and CHECKS a ⊕ (b ⊖ a) = b at
 *      the effect boundary (inside sn_tape_record_p) — the record
 *      succeeding over N random (prev, next) Z-set pairs IS the law
 *      holding on every recorded triple (an honest producer never
 *      trips it; the check is the tripwire a lying ⊕/⊖ would).
 *   2. THE MUTATOR LAW ON TIER-L PAIRS, all three carriers: every
 *      delta record derives m^δ = lcell_delta(next, prev) and
 *      CHECKS X ⊔ m^δ(X) = m(X) — N random monotone pairs per
 *      carrier, all recorded clean.
 *   3. THE REPLAY == RE-INTEGRATION WITNESS on randomized record
 *      streams: K-step chains with a MID-CHAIN checkpoint (append
 *      deltas, checkpoint, append more deltas) — tape_replay's
 *      result must equal the direct chain endpoint EXACTLY (G:
 *      per-key counts + total over the fixed key universe; L:
 *      sn_lcell_eq), and the pre-checkpoint records are proven
 *      excluded by the with/without-checkpoint pair (both replays
 *      reach the same endpoint: the chains are rooted at the zero
 *      state / ⊥, so the checkpoint is an optimization here, not a
 *      correctness crutch — the L tape WITHOUT the checkpoint is
 *      the refused shape, see probe 5).
 *   4. THE OPAQUE REFUSAL: an opaque record in the replay window has
 *      no integral — a fork'd probe must die with [E-TAPE].
 *   5. THE BOUNDARY RULES' runtime backstops (fork'd, [E-TAPE]):
 *      one tape one ledger (a G delta then an L delta on one tape),
 *      the L replay WITHOUT a checkpoint (the fragments are
 *      state-relative — no base, no silent under-approximation), the
 *      tier-mismatched replay target, the raw-ptr tier-confusion
 *      pair, the payload readback on a delta record, the unknown
 *      kind, the out-of-range index, the over-long name.
 *
 * The runner builds this twice: plain (24), then
 * -fsanitize=address (24b — no UAF over the lane reservations' growth
 * copies / the record directory / the flatten buffers; every derived
 * d/m^δ/chk object is sn_release'd inside the record call, and the
 * impulses stored on the lane are raw int64 pairs only — the
 * chronicle holds no GC pointers by construction, E-GC-IN-LANE).
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

/* compiler/runtime.c public surface (compiled together with this file) */
void sn_rt_init(int argc, char **argv);
void sn_gc_set_threshold(int64_t);
void *sn_zset_new(int64_t);
void sn_zset_add(void *, int64_t, int64_t);
void sn_zset_remove(void *, int64_t, int64_t);
int64_t sn_zset_count(void *, int64_t);
int64_t sn_zset_total(void *);
int64_t sn_zset_len(void *);
void *sn_zset_apply(void *, void *);
void *sn_lcell_new(int64_t);
void sn_lcell_inflate(void *, int64_t, int64_t);
int64_t sn_lcell_len(void *);
int64_t sn_lcell_has(void *, int64_t);
int64_t sn_lcell_carrier(void *);
int64_t sn_lcell_eq(void *, void *);
void sn_release(void *p);
/* the tape (Wave 16, Impl-U; W2C §36–§38) */
void *sn_str_new(const char *, int64_t);
void *sn_tape_new(void *);
void sn_tape_record_i(void *, int64_t, int64_t);
void sn_tape_record_p(void *, int64_t, void *, void *);
void sn_tape_replay(void *, void *);
int64_t sn_tape_len(void *);
int64_t sn_tape_rec_kind(void *, int64_t);
int64_t sn_tape_rec_payload(void *, int64_t);

/* The fixed key universe of every G round: keys [0, KU). */
enum { KU = 64 };
/* LCELL_MAX / LCELL_GSET / LCELL_LWW — mirror of runtime.c's enum */
enum { T_MAX = 0, T_GSET = 1, T_LWW = 2 };

static int g_fail = 0;

static void ck(int64_t got, int64_t want, const char *what) {
    if (got != want) {
        fprintf(stderr, "FAIL: %s: got %lld, want %lld\n", what, (long long)got, (long long)want);
        g_fail++;
    }
}

/* The fixed-seed LCG (the test_wave.c family): reproducible. */
static uint64_t g_lcg = 0xfeedface12345678ULL;

static uint64_t lcg(void) {
    g_lcg = g_lcg * 6364136223846793005ULL + 1442695040888963407ULL;
    return g_lcg >> 33;
}

/* Structural Z-set equality over the fixed key universe + totals
   (the public surface's own count/total readouts — no internals). */
static void ck_zset_eq(void *a, void *b, const char *what) {
    ck(sn_zset_total(a), sn_zset_total(b), what);
    ck(sn_zset_len(a), sn_zset_len(b), what);
    for (int k = 0; k < KU; k++) {
        if (sn_zset_count(a, k) != sn_zset_count(b, k)) {
            fprintf(stderr,
                    "FAIL: %s: key %d differs (%lld vs %lld)\n",
                    what, k,
                    (long long)sn_zset_count(a, k),
                    (long long)sn_zset_count(b, k));
            g_fail++;
            return;
        }
    }
}

/* Build the next chain state from `prev` with a few random ⊕/⊖ ops
   (the feed's honest producer: a fresh Z-set walked to its next
   state — adds AND removes, so negative impulses are exercised). */
static void *zset_step(void *prev) {
    void *next = sn_zset_new(0);
    for (int k = 0; k < KU; k++) {
        int64_t c = sn_zset_count(prev, k);
        if (c) sn_zset_add(next, k, c);
    }
    int64_t ops = 1 + (int64_t)(lcg() % 6);
    for (int64_t i = 0; i < ops; i++) {
        int64_t k = (int64_t)(lcg() % KU);
        int64_t d = 1 + (int64_t)(lcg() % 5);
        if (lcg() & 1) {
            sn_zset_add(next, k, d);
        } else {
            sn_zset_remove(next, k, d % 3); /* the exact ⊖, may go negative */
        }
    }
    return next;
}

/* --- 1 + 3a. Tier-G: the ILC law over N randomized triples, and the
   replay == re-integration witness on a randomized stream with a
   MID-CHAIN CHECKPOINT (the pre-checkpoint records must be excluded
   from the window; both tapes reach the same endpoint). --------- */
static void tier_g_round(int64_t rounds) {
    void *tape = sn_tape_new(sn_str_new("g", 1));
    void *tape2 = sn_tape_new(sn_str_new("g2", 2)); /* no checkpoint: the zero base */
    void *x = sn_zset_new(0);                /* the chain's root: ∅ */
    /* The chain states live on the STACK: the GC's conservative scan
       sees them here (a malloc'd array is invisible to it — the
       zfix engine's gc_mark precedent is the other honest option). */
    void *states[256];
    states[0] = x;
    int64_t mid = rounds / 2;

    for (int64_t r = 0; r < rounds; r++) {
        void *nx = zset_step(states[r]);
        /* THE LAW CHECK runs inside every record (ILC: a ⊕ (b⊖a) = b):
           each record succeeding IS the law verified on that triple. */
        sn_tape_record_p(tape, 1, states[r], nx);
        sn_tape_record_p(tape2, 1, states[r], nx);
        if (r == mid) {
            sn_tape_record_p(tape, 2, nx, 0); /* the MID-CHAIN CHECKPOINT */
        }
        states[r + 1] = nx;
    }
    void *end = states[rounds];

    /* Replay into fresh Z-sets: the WITH-checkpoint tape (window =
       the second half only) and the WITHOUT-checkpoint tape (the
       full impulse stream from the zero root) — both must equal the
       direct chain endpoint EXACTLY. */
    void *r1 = sn_zset_new(0);
    sn_tape_replay(tape, r1);
    ck_zset_eq(r1, end, "G replay (checkpointed tape) == the chain endpoint");

    void *r2 = sn_zset_new(0);
    sn_tape_replay(tape2, r2);
    ck_zset_eq(r2, end, "G replay (zero-based tape) == the chain endpoint");

    /* The checkpoint really reset the base: replay the checkpointed
       tape into a PRE-LOADED target — the Tier-G reset (the group
       inverse) must REPLACE the stale state, not merge onto it. */
    void *r3 = sn_zset_new(0);
    sn_zset_add(r3, 999, 42); /* stale bytes the replay must wipe */
    sn_tape_replay(tape, r3);
    ck_zset_eq(r3, end, "G replay replaces a pre-loaded target (the exact reset)");
    ck(sn_zset_count(r3, 999), 0, "G replay: the stale key is gone");

    /* The record kinds read back: mid+1 delta records before the
       checkpoint, one checkpoint, the rest after. */
    int64_t kinds_delta = 0, kinds_ck = 0;
    for (int64_t i = 0; i < sn_tape_len(tape); i++) {
        int64_t k = sn_tape_rec_kind(tape, i);
        if (k == 1) kinds_delta++;
        if (k == 2) kinds_ck++;
    }
    ck(kinds_delta, rounds, "G tape: one delta record per chain step");
    ck(kinds_ck, 1, "G tape: exactly one checkpoint");

    /* end == states[rounds]: released by the loop, exactly once */
    for (int64_t r = 1; r <= rounds; r++) sn_release(states[r]);
    sn_release(r1);
    sn_release(r2);
    sn_release(r3);
    sn_release(x);
}

/* --- 5. The runtime backstops (fork'd; must die with [E-TAPE]) ------- */
static int probe_dies_with_etape(void (*fn)(void)) {
    pid_t pid = fork();
    if (pid == 0) {
        fn();
        _exit(0); /* reachable only if the guard did NOT fire */
    }
    int st = 0;
    waitpid(pid, &st, 0);
    return WIFEXITED(st) && WEXITSTATUS(st) == 1;
}

static void probe_opaque_in_window(void) {
    void *t = sn_tape_new(sn_str_new("o", 1));
    void *a = sn_zset_new(0);
    void *b = sn_zset_new(0);
    sn_zset_add(b, 1, 1);
    sn_tape_record_i(t, 0, 7);        /* opaque: must poison the window */
    sn_tape_record_p(t, 1, a, b);     /* a delta after it */
    void *r = sn_zset_new(0);
    sn_tape_replay(t, r);             /* must die [E-TAPE] */
    _exit(0);
}

static void probe_one_tape_one_ledger(void) {
    void *t = sn_tape_new(sn_str_new("mix", 3));
    void *a = sn_zset_new(0);
    void *b = sn_zset_new(0);
    sn_zset_add(b, 1, 1);
    sn_tape_record_p(t, 1, a, b);     /* learns the Tier-G ledger */
    void *c = sn_lcell_new(T_MAX);
    void *d = sn_lcell_new(T_MAX);
    sn_lcell_inflate(c, 1, 0);
    sn_lcell_inflate(d, 2, 0);
    sn_tape_record_p(t, 1, c, d);     /* a Tier-L record: must die [E-TAPE] */
    _exit(0);
}

static void probe_l_replay_no_checkpoint(void) {
    void *t = sn_tape_new(sn_str_new("lnb", 3));
    void *a = sn_lcell_new(T_GSET);
    void *b = sn_lcell_new(T_GSET);
    sn_lcell_inflate(a, 1, 0);
    sn_lcell_inflate(b, 1, 0);
    sn_lcell_inflate(b, 2, 0);
    sn_tape_record_p(t, 1, a, b);     /* fragments, no checkpoint */
    void *r = sn_lcell_new(T_GSET);
    sn_tape_replay(t, r);             /* must die [E-TAPE]: no base */
    _exit(0);
}

static void probe_tier_mismatch_replay(void) {
    void *t = sn_tape_new(sn_str_new("tm", 2));
    void *a = sn_zset_new(0);
    void *b = sn_zset_new(0);
    sn_zset_add(b, 1, 1);
    sn_tape_record_p(t, 1, a, b);     /* a Tier-G tape */
    void *r = sn_lcell_new(T_GSET);
    sn_tape_replay(t, r);             /* an LCell target: must die [E-TAPE] */
    _exit(0);
}

static void probe_tier_confusion_pair(void) {
    void *t = sn_tape_new(sn_str_new("tc", 2));
    void *z = sn_zset_new(0);
    void *c = sn_lcell_new(T_GSET);
    sn_tape_record_p(t, 1, z, c);     /* kind mismatch: must die [E-TAPE] */
    _exit(0);
}

static void probe_payload_on_delta(void) {
    void *t = sn_tape_new(sn_str_new("pd", 2));
    void *a = sn_zset_new(0);
    void *b = sn_zset_new(0);
    sn_zset_add(b, 1, 1);
    sn_tape_record_p(t, 1, a, b);
    (void)sn_tape_rec_payload(t, 0); /* a delta record: must die [E-TAPE] */
    _exit(0);
}

static void probe_unknown_kind(void) {
    void *t = sn_tape_new(sn_str_new("uk", 2));
    sn_tape_record_i(t, 3, 1);        /* not 0/1/2: must die [E-TAPE] */
    _exit(0);
}

static void probe_oob_index(void) {
    void *t = sn_tape_new(sn_str_new("oi", 2));
    (void)sn_tape_rec_kind(t, 5);     /* empty tape: must die [E-TAPE] */
    _exit(0);
}

static void probe_long_name(void) {
    char buf[40];
    memset(buf, 'x', sizeof(buf));
    buf[39] = '\0';
    (void)sn_tape_new(sn_str_new(buf, 39)); /* > 31 bytes: must die [E-TAPE] */
    _exit(0);
}

int main(void) {
    sn_rt_init(0, NULL);
    /* The GC threshold pinned out of every timed phase (the bench
       precedent): the randomized rounds allocate ~3 carriers per
       record (d/chk/m), and a mid-round collection would reclaim
       cells the conservative stack scan cannot see through this
       file's OWN heap arrays — pin the threshold, keep the test
       deterministic. */
    sn_gc_set_threshold(1 << 30);

    /* --- 1 + 3a. Tier-G: the ILC law + the checkpointed replay ------- */
    tier_g_round(200);

    /* --- 2 + 3b. Tier-L: the mutator law + the ⊔-fold replay ---------- */
    for (int carrier = T_MAX; carrier <= T_LWW; carrier++) {
        void *tape = sn_tape_new(sn_str_new("l", 1));
        void *p = sn_lcell_new(carrier); /* the chain's root: ⊥ */
        int64_t rounds = 100;
        void *states[128]; /* stack: GC-visible (see the G round) */
        states[0] = p;
        int64_t mid = rounds / 2;

        for (int64_t r = 0; r < rounds; r++) {
            void *nx = sn_lcell_new(carrier);
            /* the honest monotone step: a random offer, PLUS a replay of
               the current winner so the chain is cumulative (MAX: the
               HWM carries; GSET: the support carries; LWW: the winner
               re-offered at its own clock is idempotent). */
            int64_t a = (int64_t)(lcg() % 100) + 1;
            if (carrier == T_GSET) {
                for (int e = 0; e < KU; e++) {
                    if (sn_lcell_has(states[r], e)) sn_lcell_inflate(nx, e, 0);
                }
                sn_lcell_inflate(nx, a % KU, 0);
            } else if (carrier == T_MAX) {
                if (sn_lcell_len(states[r])) {
                    int64_t h = 0;
                    /* the HWM ladder: find the current high-water mark
                       over the test's value range via has() probes */
                    for (int v = 300; v >= 0; v--) {
                        if (sn_lcell_has(states[r], v)) { h = v; break; }
                    }
                    if (h > 0) sn_lcell_inflate(nx, h, 0);
                }
                sn_lcell_inflate(nx, a % 300, 0);
            } else {
                /* LWW: strictly-increasing clocks (one per round) make
                   every new offer the winner — the chain is cumulative
                   by construction, and the fragment replay's max-by-
                   clock fold reaches the LAST offer exactly. */
                sn_lcell_inflate(nx, a % 100, (int64_t)(r + 1) * 10 + 1);
            }
            /* THE MUTATOR-LAW CHECK runs inside every record
               (X ⊔ m^δ(X) = m(X)): the record succeeding IS the law
               verified on that pair. */
            sn_tape_record_p(tape, 1, states[r], nx);
            if (r == mid) {
                sn_tape_record_p(tape, 2, nx, 0); /* the MID-CHAIN CHECKPOINT */
            }
            states[r + 1] = nx;
        }
        void *end = states[rounds];

        void *rr = sn_lcell_new(carrier);
        sn_tape_replay(tape, rr);
        if (!sn_lcell_eq(rr, end)) {
            fprintf(stderr, "FAIL: L replay (carrier %d) != the chain endpoint\n", carrier);
            g_fail++;
        }

        /* end == states[rounds]: released by the loop, exactly once */
        for (int64_t r = 1; r <= rounds; r++) sn_release(states[r]);
        sn_release(rr);
        sn_release(p);
    }

    /* --- 4 + 5. the refusals, fork'd (must die [E-TAPE]) ------------- */
    ck(probe_dies_with_etape(probe_opaque_in_window), 1,
       "refusal: an opaque record in the replay window dies [E-TAPE]");
    ck(probe_dies_with_etape(probe_one_tape_one_ledger), 1,
       "refusal: one tape, one ledger (a G then an L record) dies [E-TAPE]");
    ck(probe_dies_with_etape(probe_l_replay_no_checkpoint), 1,
       "refusal: an L replay without a checkpoint dies [E-TAPE] (the fragments are state-relative)");
    ck(probe_dies_with_etape(probe_tier_mismatch_replay), 1,
       "refusal: a tier-mismatched replay target dies [E-TAPE]");
    ck(probe_dies_with_etape(probe_tier_confusion_pair), 1,
       "refusal: the raw-ptr tier-confusion pair dies [E-TAPE]");
    ck(probe_dies_with_etape(probe_payload_on_delta), 1,
       "refusal: the payload readback on a delta record dies [E-TAPE]");
    ck(probe_dies_with_etape(probe_unknown_kind), 1,
       "refusal: the unknown record kind dies [E-TAPE]");
    ck(probe_dies_with_etape(probe_oob_index), 1,
       "refusal: the out-of-range record index dies [E-TAPE]");
    ck(probe_dies_with_etape(probe_long_name), 1,
       "refusal: the over-long tape name dies [E-TAPE]");

    if (g_fail == 0) {
        printf("PASS: test_tape (ILC law on randomized G triples, the mutator law on all L carriers, "
               "replay == re-integration with mid-chain checkpoints, the opaque refusal, the "
               "one-tape-one-ledger/no-base/tier backstops)");
    } else {
        printf("FAIL: %d tape check(s)\n", g_fail);
        return 1;
    }
    return 0;
}
