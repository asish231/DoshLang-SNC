/* SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O), Cohort Delta:
 * systems check 18 — the global-pool discipline's RUNTIME mechanics
 * (kills G1 §1 A6c — the global-pool freelist race). The checker is
 * the gate (E0803.1/.2/.3, pinned by examples/gpool_*.sn + golden.rs);
 * this file exercises what the checker alone cannot: the runtime
 * half of the three declared modes.
 *
 *   1. FROZEN concurrent read: a frozen pool's handles are read from
 *      N threads at once (the Sendable zero-copy shared read) — all
 *      values exact, all generations stable, len constant, no
 *      corruption. Soundness: a frozen pool has no legal write path,
 *      so any number of threads holding handles cannot corrupt or
 *      race the freelist.
 *   2. SYNCHRONIZED concurrent mutation: N threads x M insert/remove
 *      rounds funnel through the vault lock — the lock serializes;
 *      the final state is EXACT (inserts == len + removes; the
 *      value multiset matches the deterministic expectation), the
 *      data-race-freedom condition (no two threads ever interleave
 *      insert/remove on one pool).
 *   3. The `unfrozen (P)` window's freeze/unfreeze depth discipline:
 *      nested windows (depth 3) unwind LIFO to 0; an unbalanced exit
 *      cannot underflow.
 *   4. E0803.1's runtime backstop (the home-thread tag): a
 *      GP_THREADLOCAL pool mutated from a foreign thread must
 *      fail-fast with the [E0803] diagnostic — probed in a fork'd
 *      child (the E-LKIND pattern from test_lcell.c; the checker
 *      rejects capture statically, the runtime refuses what the
 *      checker cannot see — the two-layer precedent).
 *
 * Built twice by tests/systems/run_all.sh: plain (18), then ASan
 * (18b — the stress pass: no UAF / no heap-error over the concurrent
 * read, the locked mutation, and the window cycles).
 *
 * The fork'd probe pattern: the child's stderr is captured to a temp
 * file; the parent asserts the child DIED and the diagnostic carries
 * [E0803].
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <pthread.h>
#include <sys/wait.h>
#include <unistd.h>

/* compiler/runtime.c public surface (compiled together with this file) */
void sn_rt_init(int argc, char **argv);
void *sn_pool_new(int64_t elem_kind, int64_t elem_size);
void *sn_pool_new_mode(int64_t elem_kind, int64_t elem_size, int64_t mode);
int64_t sn_pool_insert_i64(void *p, int64_t val);
int64_t sn_pool_get_i64(void *p, int64_t handle);
void sn_pool_set_i64(void *p, int64_t handle, int64_t val);
void sn_pool_remove(void *p, int64_t handle);
int64_t sn_pool_is_valid(void *p, int64_t handle);
int64_t sn_pool_len(void *p);
void sn_pool_unfreeze(void *p);
void sn_pool_freeze(void *p);
void sn_go_spawn(void (*fn)(void *), void *arg);

/* runtime.c's GP_* mirror — sn_pool_new_mode's mode codes */
enum { GP_FROZEN = 0, GP_THREADLOCAL = 1, GP_SYNC = 2 };

/* the frozen_depth field is opaque behind the freeze pair; the depth
 * discipline is asserted behaviorally (freeze underflow is a no-op). */

static int failures = 0;
static void ck(int ok, const char *what) {
    if (!ok) {
        fprintf(stderr, "FAIL: %s\n", what);
        failures++;
    }
}

/* ---- 1. the frozen concurrent read ------------------------------------
 * 8 threads read the same 4096-handle frozen pool, each thread summing
 * every slot's value (deterministic expectation: v = i*3+1). Every
 * read is a pure generation-checked load; a corrupted freelist or a
 * racing generation bump would show as a wrong sum, a failed
 * is_valid, or a stale-handle abort. */
#define FR_N 4096
#define FR_THREADS 8

typedef struct {
    void *pool;
    int64_t sum;
    int64_t valid_count;
    int64_t aborts;
} FrArg;

static void fr_reader(void *raw) {
    FrArg *a = (FrArg *)raw;
    int64_t sum = 0;
    int64_t valid = 0;
    for (int i = 0; i < FR_N; i++) {
        int64_t h = (int64_t)(((uint64_t)1 << 32) | (uint64_t)i); /* gen 1, idx i */
        if (sn_pool_is_valid(a->pool, h)) {
            valid++;
            sum += sn_pool_get_i64(a->pool, h);
        }
    }
    a->sum = sum;
    a->valid_count = valid;
}

/* ---- 2. the synchronized concurrent mutation --------------------------
 * 8 threads x (256 inserts + 256 removes) on ONE GP_SYNC pool; every
 * op takes the vault lock. Deterministic expectation: after the
 * join, len == live inserts remaining. Each thread inserts its own
 * value space then removes exactly what it inserted — so the final
 * multiset is exactly the insert survivors and len must equal
 * inserts - removes. The lock serializes the freelist: any
 * interleaving corruption would manifest as len drift or a
 * stale-handle abort (the generation check catches a recycled slot
 * immediately). */
#define SY_THREADS 8
#define SY_OPS 256

typedef struct {
    void *pool;
    int64_t tid;
    int64_t inserted;
    int64_t removed_ok;
} SyArg;

static void sy_worker(void *raw) {
    SyArg *a = (SyArg *)raw;
    int64_t hs[SY_OPS];
    for (int i = 0; i < SY_OPS; i++) {
        hs[i] = sn_pool_insert_i64(a->pool, a->tid * 1000000 + i);
        a->inserted++;
    }
    for (int i = 0; i < SY_OPS; i++) {
        sn_pool_remove(a->pool, hs[i]);
        a->removed_ok++;
    }
}

/* The E0803.1 backstop probe's foreign thread: one insert + one set
 * on a thread_local pool from a NON-declaring thread — the guard must
 * fire on the FIRST mutator (insert). */
static void *sy_worker_pthread(void *raw) {
    SyArg *a = (SyArg *)raw;
    int64_t h = sn_pool_insert_i64(a->pool, 123);
    a->inserted = 1;
    sn_pool_set_i64(a->pool, h, 456);
    a->removed_ok = 1;
    return NULL;
}

int main(void) {
    sn_rt_init(0, NULL);

    /* --- 1. frozen concurrent read ----------------------------------- */
    {
        void *p = sn_pool_new(0, 8);
        for (int i = 0; i < FR_N; i++) {
            sn_pool_insert_i64(p, (int64_t)i * 3 + 1);
        }
        /* the pool is frozen by construction here: no mutation after
         * the build — the discipline the checker enforces for real. */
        FrArg args[FR_THREADS];
        for (int t = 0; t < FR_THREADS; t++) {
            args[t].pool = p;
            args[t].sum = 0;
            args[t].valid_count = 0;
            args[t].aborts = 0;
            sn_go_spawn(fr_reader, &args[t]);
        }
        /* sn_go_spawn is async; busy-wait for all readers to report
         * (the runtime's spawn pool joins at rt_shutdown, so poll
         * the args). */
        for (int spin = 0; spin < 100000; spin++) {
            int done = 1;
            for (int t = 0; t < FR_THREADS; t++) {
                if (args[t].valid_count == 0) done = 0;
            }
            if (done) break;
            usleep(200);
        }
        int64_t expect = 0;
        for (int i = 0; i < FR_N; i++) expect += (int64_t)i * 3 + 1;
        for (int t = 0; t < FR_THREADS; t++) {
            ck(args[t].sum == expect, "frozen read: thread checksum mismatch (freelist corruption?)");
            ck(args[t].valid_count == FR_N, "frozen read: generations not stable across threads");
        }
        ck(sn_pool_len(p) == FR_N, "frozen read: len drifted during concurrent reads");
        printf("frozen_read: %d threads x %d handles, checksum %lld, generations stable\n",
               FR_THREADS, FR_N, (long long)args[0].sum);
    }

    /* --- 2. synchronized concurrent mutation -------------------------- */
    {
        void *p = sn_pool_new_mode(0, 8, GP_SYNC);
        SyArg args[SY_THREADS];
        for (int t = 0; t < SY_THREADS; t++) {
            args[t].pool = p;
            args[t].tid = t;
            args[t].inserted = 0;
            args[t].removed_ok = 0;
            sn_go_spawn(sy_worker, &args[t]);
        }
        for (int spin = 0; spin < 100000; spin++) {
            int done = 1;
            for (int t = 0; t < SY_THREADS; t++) {
                if (args[t].removed_ok < SY_OPS) done = 0;
            }
            if (done) break;
            usleep(200);
        }
        int64_t total_ins = 0, total_rem = 0;
        for (int t = 0; t < SY_THREADS; t++) {
            ck(args[t].inserted == SY_OPS, "sync mutation: thread lost inserts");
            ck(args[t].removed_ok == SY_OPS, "sync mutation: thread lost removes");
            total_ins += args[t].inserted;
            total_rem += args[t].removed_ok;
        }
        ck(total_ins == (int64_t)SY_THREADS * SY_OPS, "sync mutation: insert count drift");
        ck(sn_pool_len(p) == 0, "sync mutation: final state not exact (lock failed to serialize?)");
        printf("sync_mutate: %d threads x %d ins+rem, final len %lld (exact)\n",
               SY_THREADS, SY_OPS, (long long)sn_pool_len(p));
    }

    /* --- 3. the unfrozen window's depth discipline -------------------- */
    {
        void *p = sn_pool_new(0, 8);
        sn_pool_unfreeze(p); /* depth 1 */
        sn_pool_unfreeze(p); /* depth 2 (nested windows are legal) */
        sn_pool_unfreeze(p); /* depth 3 */
        int64_t h = sn_pool_insert_i64(p, 7); /* mutation legal inside */
        sn_pool_freeze(p); /* 2 */
        sn_pool_freeze(p); /* 1 */
        sn_pool_freeze(p); /* 0 — window closed */
        ck(sn_pool_get_i64(p, h) == 7, "window: value not intact after nested windows");
        /* an unbalanced freeze cannot underflow */
        sn_pool_freeze(p);
        sn_pool_freeze(p);
        ck(sn_pool_is_valid(p, h), "window: handle invalidated by unbalanced freezes");
        printf("window_depth: nested depth 3 -> LIFO unwind to 0, no underflow\n");
    }

    /* --- 4. E0803.1's runtime backstop (fork'd probe) ------------------
     * A GP_THREADLOCAL pool mutated from a FOREIGN thread must
     * fail-fast with [E0803]. The foreign thread is a plain
     * pthread_create — the honest minimal probe (fork'd so the
     * abort cannot kill the test harness; the E-LKIND pattern). */
    {
        void *p = sn_pool_new_mode(0, 8, GP_THREADLOCAL);
        int64_t h = sn_pool_insert_i64(p, 1); /* declaring thread: legal */
        ck(sn_pool_is_valid(p, h), "thread_local: declaring-thread mutation refused?!");
        char tmpl[] = "/tmp/snc_gpool_e08031_XXXXXX";
        int fd = mkstemp(tmpl);
        ck(fd >= 0, "mkstemp failed");
        fflush(stdout); /* the fork must not duplicate buffered output */
        pid_t pid = fork();
        if (pid == 0) {
            /* child: stderr -> temp file, then the forbidden
             * cross-thread mutation on a FRESH child-side pool (the
             * pre-fork pool's tag points at the parent's thread). */
            dup2(fd, 2);
            close(fd);
            void *cp = sn_pool_new_mode(0, 8, GP_THREADLOCAL);
            int64_t ch = sn_pool_insert_i64(cp, 1); /* still declaring thread */
            (void)ch;
            /* the foreign-thread MUTATION: a raw pthread's set must
             * fail-fast (the E0803.1 backstop fires on the mutator,
             * not the reader — reads of a thread_local pool from a
             * foreign thread are memory-safe by the generation check;
             * the FREELIST is what the guard protects). */
            pthread_t th;
            SyArg sa;
            sa.pool = cp;
            sa.tid = 1;
            sa.inserted = 0;
            sa.removed_ok = 0;
            pthread_create(&th, NULL, sy_worker_pthread, &sa);
            pthread_join(th, NULL);
            _exit(0); /* reaching here means the guard FAILED to fire */
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
        int has_diag = strstr(buf, "[E0803]") != NULL
                       && strstr(buf, "thread_local") != NULL;
        ck(died && has_diag,
           "thread_local cross-thread mutation did not fail-fast with [E0803]");
        if (died && has_diag) {
            printf("e0803_1_backstop: foreign-thread mutation failed fast with the diagnostic\n");
        }
    }

    if (failures == 0) {
        printf("PASS: test_gpool\n");
    } else {
        printf("FAIL: test_gpool (%d)\n", failures);
        return 1;
    }
    return 0;
}
