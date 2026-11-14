/*
 * Systems Test: Tier-G Z-Set Delta Collections (SWARM RABBIT HOLE, P5 slice 1)
 * ===========================================================================
 *
 * Proves, at the runtime level, that SnZSet is the W2C §1.1 Tier-G carrier:
 * G[X] = the direct sum over the key universe with finite support (zero
 * counts pruned in the canonical form), negative counts = retraction, and
 * retraction is EXACT — the Tier-G capability label (W4C Repair 1's
 * order O ⊑ L ⊑ G: G is the strongest capability).
 *
 *   1. Add accumulates group coefficients: add(z, k, d) → count[k] += d;
 *      total (Σ counts) is maintained incrementally; len = |support|.
 *   2. Exact retraction: remove(z, k, d) = add(z, k, -d). Add 3, remove 3
 *      → count 0 AND the support is pruned (len shrinks): the zero
 *      coefficient leaves the finite support. O(1)/key, no scan.
 *   3. Retraction past zero stays in ℤ[X]: remove(z, k, 6) with count 4
 *      → count -2 (a legal group coefficient; Tier L could never do this).
 *   4. Str keys: content-equality keys (sn_str_eq), same ops.
 *   5. Merge: pointwise add (the ⊕ of two Z-sets), totals add.
 *   6. Delta + apply — the primitive pair the incremental-view machinery
 *      composes: d = delta(B, A) is the per-timestamp 𝒟 (B − A);
 *      apply(A, d) = A ⊕ d; ILC's law a ⊕ (b ⊖ a) = b holds exactly.
 *   7. Quiescence (W4C Repair 3, Tier-G test): delta(A, A) is the zero
 *      Z-set — 𝒟(∫α) = 0, len 0, total 0.
 *   8. The group law A ⊕ dA − dA = A via primitives only:
 *      neg = delta(∅, dA) = −dA; apply(apply(A, dA), neg) == A.
 *   9. The prune flag: with pruning off, a zeroed count is kept in the
 *      support (count 0, len counts it); the flag is per-Z-set state.
 *
 * The runner (tests/systems/run_all.sh) builds this twice: plain, and with
 * -fsanitize=address (the ASan stress pass: no UAF in the swap-with-last
 * removal, no leaks — every Z-set/list/Str created here is sn_release'd).
 * The macOS `leaks` check pauses on SN_ZSET_LEAK_PAUSE (LSan is not
 * supported on arm64 darwin).
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

/* compiler/runtime.c public surface (compiled together with this file) */
void sn_rt_init(int argc, char **argv);
void *sn_zset_new(int64_t key_kind);
void sn_zset_add(void *z, int64_t key, int64_t delta);
void sn_zset_remove(void *z, int64_t key, int64_t delta);
void sn_zset_merge(void *dst, void *src);
void *sn_zset_delta(void *dst, void *prev);
void *sn_zset_apply(void *base, void *delta);
void *sn_zset_copy(void *z);
int64_t sn_zset_count(void *z, int64_t key);
int64_t sn_zset_total(void *z);
int64_t sn_zset_len(void *z);
void *sn_zset_keys(void *z);
void sn_zset_set_prune(void *z, int64_t flag);
void *sn_str_from_cstr(const char *s);
void *sn_list_new(int64_t elem_kind);
int64_t sn_list_len(void *l);
int64_t sn_list_get_i64(void *l, int64_t i);
void *sn_list_get_ptr(void *l, int64_t i);
void sn_release(void *p);

static int g_fail = 0;

static void ck(int64_t got, int64_t want, const char *what) {
    if (got != want) {
        fprintf(stderr, "FAIL: %s: got %lld, want %lld\n", what, (long long)got, (long long)want);
        g_fail++;
    }
}

int main(void) {
    sn_rt_init(0, NULL);

    /* --- 1. Add accumulates coefficients (i64 keys) ------------------- */
    void *a = sn_zset_new(0);
    sn_zset_add(a, 10, 1);
    sn_zset_add(a, 10, 2); /* count[10] = 3 */
    sn_zset_add(a, 20, 4);
    sn_zset_add(a, 30, 7);
    ck(sn_zset_count(a, 10), 3, "add accumulates count[10]");
    ck(sn_zset_len(a), 3, "len = support size after 3 distinct adds");
    ck(sn_zset_total(a), 14, "total = sum of coefficients");
    ck(sn_zset_count(a, 99), 0, "absent key counts 0");

    /* --- 2. Exact retraction: add 3, remove 3 → 0 AND pruned ---------- */
    sn_zset_remove(a, 10, 3);
    ck(sn_zset_count(a, 10), 0, "exact retraction: count[10] == 0");
    ck(sn_zset_len(a), 2, "zero coefficient pruned from support");
    ck(sn_zset_total(a), 11, "total tracks the retraction");

    /* --- 3. Retraction past zero: a legal ℤ[X] coefficient ------------ */
    sn_zset_remove(a, 20, 6); /* 4 - 6 = -2 */
    ck(sn_zset_count(a, 20), -2, "retraction past zero: count[20] == -2");
    ck(sn_zset_len(a), 2, "negative coefficient stays in support");
    ck(sn_zset_total(a), 5, "total = 7 + (-2)");

    /* --- 4. Str keys (content equality) -------------------------------- */
    void *s = sn_zset_new(1);
    void *k1 = sn_str_from_cstr("hello");
    void *k2 = sn_str_from_cstr("hello"); /* distinct object, same content */
    void *k3 = sn_str_from_cstr("world");
    sn_zset_add(s, (int64_t)(intptr_t)k1, 2);
    sn_zset_add(s, (int64_t)(intptr_t)k2, 3); /* same key: 2 + 3 */
    sn_zset_add(s, (int64_t)(intptr_t)k3, 5);
    ck(sn_zset_count(s, (int64_t)(intptr_t)k2), 5, "str keys: content-equal keys merge");
    ck(sn_zset_len(s), 2, "str keys: support size 2");
    sn_zset_remove(s, (int64_t)(intptr_t)k1, 5); /* exact retraction via eq content */
    ck(sn_zset_count(s, (int64_t)(intptr_t)k2), 0, "str keys: exact retraction");
    ck(sn_zset_len(s), 1, "str keys: pruned support");
    sn_release(k1);
    sn_release(k2);
    sn_release(k3);

    /* --- 5. Merge: pointwise add (⊕ of two Z-sets) --------------------- */
    void *m1 = sn_zset_new(0);
    void *m2 = sn_zset_new(0);
    sn_zset_add(m1, 1, 2);
    sn_zset_add(m1, 2, 1);
    sn_zset_add(m2, 2, 3);
    sn_zset_add(m2, 3, 5);
    sn_zset_merge(m1, m2);
    ck(sn_zset_count(m1, 1), 2, "merge: untouched key keeps coefficient");
    ck(sn_zset_count(m1, 2), 4, "merge: shared key adds pointwise");
    ck(sn_zset_count(m1, 3), 5, "merge: new key arrives");
    ck(sn_zset_len(m1), 3, "merge: support union");
    ck(sn_zset_total(m1), 11, "merge: totals add (3 + 8)");

    /* --- 6. Delta + apply: the 𝒟/⊕ primitive pair, ILC's law ---------- */
    /* A = {40:2, 50:3}; B = {40:5, 60:4} */
    void *za = sn_zset_new(0);
    void *zb = sn_zset_new(0);
    sn_zset_add(za, 40, 2);
    sn_zset_add(za, 50, 3);
    sn_zset_add(zb, 40, 5);
    sn_zset_add(zb, 60, 4);
    void *d = sn_zset_delta(zb, za); /* B − A = {40:+3, 50:−3, 60:+4} */
    ck(sn_zset_count(d, 40), 3, "delta: positive coefficient");
    ck(sn_zset_count(d, 50), -3, "delta: negative coefficient (retraction)");
    ck(sn_zset_count(d, 60), 4, "delta: inserted key");
    ck(sn_zset_len(d), 3, "delta: support of the change");
    ck(sn_zset_total(d), 4, "delta: total = total(B) − total(A)");
    void *zc = sn_zset_apply(za, d); /* A ⊕ d */
    ck(sn_zset_count(zc, 40), 5, "apply: ILC law a⊕(b⊖a)=b — key 40");
    ck(sn_zset_count(zc, 50), 0, "apply: retracted key pruned");
    ck(sn_zset_count(zc, 60), 4, "apply: inserted key");
    ck(sn_zset_len(zc), 2, "apply: support matches B");
    ck(sn_zset_total(zc), 9, "apply: total matches B");
    /* operands must be untouched (apply/delta are pure) */
    ck(sn_zset_count(za, 40), 2, "apply purity: base unchanged");
    ck(sn_zset_count(zb, 40), 5, "delta purity: dst unchanged");
    sn_release(d);

    /* --- 7. Quiescence: 𝒟(A) = 0 (the Tier-G fixpoint test) ------------ */
    void *q = sn_zset_delta(za, za);
    ck(sn_zset_len(q), 0, "quiescence: delta(A,A) is the zero Z-set");
    ck(sn_zset_total(q), 0, "quiescence: zero total");
    sn_release(q);
    void *q2 = sn_zset_delta(zb, za);
    void *q3 = sn_zset_delta(zb, zc);
    ck(sn_zset_len(q3), 0, "quiescence: delta(B, A⊕(B⊖A)) is zero");
    sn_release(q2);
    sn_release(q3);
    sn_release(zc);

    /* --- 8. The group law A ⊕ dA − dA = A via primitives ---------------- */
    void *da = sn_zset_delta(za, NULL); /* dA = A − ∅ = A */
    void *empty = sn_zset_new(0);
    void *neg = sn_zset_delta(empty, da); /* −dA */
    ck(sn_zset_count(neg, 40), -2, "negation: delta(∅, dA) = −dA");
    void *x1 = sn_zset_apply(za, da); /* A + dA */
    void *x2 = sn_zset_apply(x1, neg); /* A + dA − dA */
    ck(sn_zset_count(x2, 40), 2, "group law: count restored exactly");
    ck(sn_zset_count(x2, 50), 3, "group law: second key restored");
    ck(sn_zset_len(x2), 2, "group law: support restored");
    ck(sn_zset_total(x2), 5, "group law: total restored");
    ck(sn_zset_len(sn_zset_delta(x2, za)), 0, "group law: A + dA − dA == A");
    sn_release(da);
    sn_release(empty);
    sn_release(neg);
    sn_release(x1);
    sn_release(x2);

    /* --- 9. The prune flag: zero counts kept when pruning is off -------- */
    void *pk = sn_zset_new(0);
    sn_zset_add(pk, 7, 3);
    sn_zset_set_prune(pk, 0);
    sn_zset_remove(pk, 7, 3); /* count 0, entry kept */
    ck(sn_zset_count(pk, 7), 0, "prune flag: count is 0");
    ck(sn_zset_len(pk), 1, "prune flag: zero count kept in support");
    sn_zset_set_prune(pk, 1);
    sn_zset_add(pk, 7, 1); /* any further add prunes the zero on flip? no:
                              pruning happens at the moment of zero; the kept
                              zero stays until touched. 1 != 0 so it stays. */
    ck(sn_zset_count(pk, 7), 1, "prune flag: coefficient resumes");
    ck(sn_zset_len(pk), 1, "prune flag: support stable");

    /* --- readout: keys() lists the support ---------------------------- */
    void *ks = sn_zset_keys(za);
    ck(sn_list_len(ks), 2, "keys: support size readout");
    sn_release(ks);

    /* --- cleanup: every object released (the leaks(1) guarantee) ------- */
    sn_release(a);
    sn_release(s);
    sn_release(m1);
    sn_release(m2);
    sn_release(za);
    sn_release(zb);
    sn_release(pk);

    if (g_fail == 0) {
        printf("PASS: tier-G z-set delta collections — exact retraction, "
               "𝒟/⊕ primitives, group law (W2C §1.1)\n");
    } else {
        printf("FAIL: %d z-set delta check(s)\n", g_fail);
        return 1;
    }

    /* SN_ZSET_LEAK_PAUSE=<seconds> sleeps before returning (leaks(1)). */
    {
        const char *pause = getenv("SN_ZSET_LEAK_PAUSE");
        if (pause && pause[0] >= '1' && pause[0] <= '9') {
            int secs = atoi(pause);
            fflush(stdout);
            printf("(paused %ds for leak check)\n", secs);
            fflush(stdout);
            sleep(secs);
        }
    }
    return 0;
}
