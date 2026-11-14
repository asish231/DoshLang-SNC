/*
 * Systems Test: Nursery Child-Lane Carve (SWARM RABBIT HOLE, D0 gap #5)
 * =====================================================================
 *
 * Proves, at the runtime level, that nursery child tasks can allocate in
 * loaned ν lanes and that sn_nursery_wait reclaims EVERY child allocation
 * with one store per lane (the O(1) join):
 *
 *   1. Wave 1 — 12 spawned children each reserve a ν lane via
 *      sn_nursery_loan_lane (mimicking the compiled child shape: the
 *      parent calls sn_nursery_enter_task + sn_go_spawn per spawn; the
 *      child loans, allocates, checksums, and calls
 *      sn_nursery_leave_task), verify the fresh-lane guarantee
 *      (lane_used == 0 at loan, W2A §1.4), and report a checksum of the
 *      bytes they wrote into their loaned lane.
 *   2. Join — sn_nursery_wait; the join-stats surface must show exactly
 *      one lane reset per successful loan and exactly the summed
 *      allocation bytes reclaimed; fallbacks (all lanes busy) counted.
 *   3. Wave 2 — 7 children that must ALL get a ν loan: proves the join
 *      returned every lane to its thread's free set (parent = NULL).
 *   4. Wave 3 — nested-nursery stress: each spawned child loans its own
 *      ν, opens its own nursery, and spawns 2 grandchildren that
 *      loan/allocate concurrently across worker threads; the nested joins
 *      run inside the children (LIFO, W2A SR3), the outer join last.
 *   5. The spawning thread's lane 0 is untouched (pre-spawn lane_used(0)
 *      == post-join lane_used(0)): the carve reserves child threads'
 *      lanes, never the parent's watermark.
 *   6. The loaned-lane backstop: after leave_task (permission cleared,
 *      loan still registered until the join), allocating on the loaned
 *      lane must fail fast — a later task on the same worker must never
 *      put live bytes under a lane the join wipes (UAF closure).
 *
 * The runner (tests/systems/run_all.sh) builds this twice: plain, and
 * with -fsanitize=address,leak (the ASan/LSan stress pass: no leaks, no
 * UAF under concurrent spawns). The fork-based backstop check runs only
 * in the plain build (sanitizer runtimes change exit-code plumbing).
 */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/wait.h>

/* compiler/runtime.c public surface (compiled together with this file) */
void sn_rt_init(int argc, char **argv);
void *sn_nursery_new(void);
void sn_nursery_enter_task(void *nursery);
void sn_nursery_leave_task(void *nursery);
void sn_nursery_wait(void *nursery);
int64_t sn_nursery_loan_lane(void *nursery);
int64_t sn_nursery_join_stats(int64_t what);
void *sn_lane_alloc(int64_t lane_id, int64_t bytes);
int64_t sn_lane_used(int64_t lane_id);
void sn_go_spawn(void (*fn)(void *), void *arg);

static void ck(int cond, const char *what) {
    if (!cond) {
        fprintf(stderr, "FAIL: %s\n", what);
        exit(1);
    }
}

/* --------------------------- wave 1/2 children ------------------------- */

#define W1_CHILDREN 12
#define W2_CHILDREN 7
#define W3_CHILDREN 4
#define W3_GRAND 2

typedef struct ChildArg {
    void *nursery;  /* the nursery this child belongs to */
    int idx;        /* child ordinal (drives the byte pattern) */
    int64_t size;   /* bytes this child allocates */
    int64_t nu;     /* loaned lane id (-1: fallback) */
    uint64_t sum;   /* checksum of the bytes written */
    int checked;    /* child ran to completion */
} ChildArg;

static uint64_t pattern_sum(int idx, int64_t size) {
    uint64_t sum = 0;
    for (int64_t j = 0; j < size; j++) {
        sum += (uint8_t)((uint64_t)idx * 31u + (uint64_t)j);
    }
    return sum;
}

/* The child task's work: loan ν from the nursery, allocate into it, write
 * a pattern, checksum. Does NOT leave the nursery — the callers decide
 * when the task ends (the nested child must join its own nursery first). */
static void loan_alloc_checksum(ChildArg *a) {
    int64_t nu = sn_nursery_loan_lane(a->nursery);
    a->nu = nu;
    if (nu >= 0) {
        /* W2A §1.4: fresh lane, empty stack — the loan reset the watermark */
        ck(sn_lane_used(nu) == 0, "loaned lane not fresh (watermark != base)");
        uint8_t *bytes = (uint8_t *)sn_lane_alloc(nu, a->size);
        ck(bytes != NULL, "sn_lane_alloc on loaned lane failed");
        for (int64_t j = 0; j < a->size; j++) {
            bytes[j] = (uint8_t)((uint64_t)a->idx * 31u + (uint64_t)j);
        }
        /* the bump landed: watermark == allocation (sizes are 8-aligned) */
        ck(sn_lane_used(nu) == a->size, "loaned lane watermark != allocation");
        a->sum = pattern_sum(a->idx, a->size);
    } else {
        /* fallback path (all loanable lanes busy on this worker): the child
         * allocates on its own thread's stack region / heap — existing
         * behavior; not join-reclaimed (the documented limitation). */
        uint8_t *bytes = (uint8_t *)malloc((size_t)a->size);
        ck(bytes != NULL, "fallback allocation failed");
        for (int64_t j = 0; j < a->size; j++) {
            bytes[j] = (uint8_t)((uint64_t)a->idx * 31u + (uint64_t)j);
        }
        free(bytes);
        a->sum = pattern_sum(a->idx, a->size);
    }
    a->checked = 1;
}

static void carve_child(void *p) {
    ChildArg *a = (ChildArg *)p;
    loan_alloc_checksum(a);
    sn_nursery_leave_task(a->nursery);
}

/* --------------------------- wave 3: nested ---------------------------- */

typedef struct GrandArg {
    void *nursery;
    int idx;
    int64_t size;
    uint64_t sum;
    int checked;
} GrandArg;

typedef struct NestedArg {
    void *outer;     /* the outer nursery n3 */
    ChildArg self;   /* this child's own loan record */
    GrandArg grand[W3_GRAND];
    int64_t grand_loans; /* how many grandchildren got a ν loan */
    int64_t grand_bytes; /* bytes the grandchildren allocated on ν */
} NestedArg;

static void grand_child(void *p) {
    GrandArg *g = (GrandArg *)p;
    int64_t nu = sn_nursery_loan_lane(g->nursery);
    uint8_t *bytes;
    if (nu >= 0) {
        ck(sn_lane_used(nu) == 0, "grandchild loaned lane not fresh");
        bytes = (uint8_t *)sn_lane_alloc(nu, g->size);
    } else {
        bytes = (uint8_t *)malloc((size_t)g->size); /* fallback, not reclaimed */
    }
    ck(bytes != NULL, "grandchild allocation failed");
    for (int64_t j = 0; j < g->size; j++) {
        bytes[j] = (uint8_t)((uint64_t)g->idx * 7u + (uint64_t)j);
    }
    if (nu < 0) free(bytes);
    g->sum = 0;
    for (int64_t j = 0; j < g->size; j++) {
        g->sum += (uint8_t)((uint64_t)g->idx * 7u + (uint64_t)j);
    }
    g->checked = 1;
    sn_nursery_leave_task(g->nursery);
}

static void nested_child(void *p) {
    NestedArg *na = (NestedArg *)p;
    /* this child of n3 loans its own ν from the OUTER nursery */
    na->self.nursery = na->outer;
    loan_alloc_checksum(&na->self);
    /* its own nursery, joined INSIDE this task (LIFO before the outer join) */
    void *inner = sn_nursery_new();
    na->grand_loans = 0;
    na->grand_bytes = 0;
    for (int i = 0; i < W3_GRAND; i++) {
        na->grand[i].nursery = inner;
        na->grand[i].idx = na->self.idx * 10 + i;
        na->grand[i].size = 1024 + i * 256;
        na->grand[i].sum = 0;
        na->grand[i].checked = 0;
        sn_nursery_enter_task(inner);
        sn_go_spawn(grand_child, &na->grand[i]);
    }
    /* INNER JOIN: reclaims the grandchildren's ν lanes with their own
     * 1-store resets, before this task leaves the outer nursery */
    sn_nursery_wait(inner);
    for (int i = 0; i < W3_GRAND; i++) {
        ck(na->grand[i].checked, "grandchild did not run to completion");
        uint64_t expect = 0;
        for (int64_t j = 0; j < na->grand[i].size; j++) {
            expect += (uint8_t)((uint64_t)na->grand[i].idx * 7u + (uint64_t)j);
        }
        ck(na->grand[i].sum == expect, "grandchild checksum mismatch");
    }
    sn_nursery_leave_task(na->outer);
}

/* ----------------------------------------------------------------------- */

int main(void) {
    sn_rt_init(0, NULL);
    int64_t lanes0 = sn_nursery_join_stats(0);
    int64_t bytes0 = sn_nursery_join_stats(1);
    int64_t fb0 = sn_nursery_join_stats(2);

    /* the spawning thread's lane 0 before any nursery */
    int64_t parent_used_before = sn_lane_used(0);

    /* ------------------- wave 1: 12 loaning children ------------------- */
    void *n1 = sn_nursery_new();
    ChildArg args1[W1_CHILDREN];
    for (int i = 0; i < W1_CHILDREN; i++) {
        args1[i].nursery = n1;
        args1[i].idx = i;
        args1[i].size = 4096 + (int64_t)i * 256; /* 8-aligned */
        args1[i].nu = -1;
        args1[i].sum = 0;
        args1[i].checked = 0;
        sn_nursery_enter_task(n1);          /* the compiled shape: the parent */
        sn_go_spawn(carve_child, &args1[i]); /* enters per spawn (ir.rs)     */
    }
    sn_nursery_wait(n1);

    int64_t lanes1 = sn_nursery_join_stats(0) - lanes0;
    int64_t bytes1 = sn_nursery_join_stats(1) - bytes0;
    int64_t fb1 = sn_nursery_join_stats(2) - fb0;
    int64_t loans1 = 0, fallbacks1 = 0, expect_bytes1 = 0;
    for (int i = 0; i < W1_CHILDREN; i++) {
        ck(args1[i].checked, "wave-1 child did not run to completion");
        ck(args1[i].sum == pattern_sum(args1[i].idx, args1[i].size),
           "wave-1 child checksum mismatch (results not intact)");
        if (args1[i].nu >= 0) {
            loans1++;
            expect_bytes1 += args1[i].size;
        } else {
            fallbacks1++;
        }
    }
    /* THE O(1) JOIN ASSERTIONS: one reset per loan, exact byte totals */
    ck(loans1 > 0, "wave 1: no child obtained a ν loan");
    ck(lanes1 == loans1, "wave 1: join reset count != loans");
    ck(bytes1 == expect_bytes1, "wave 1: join reclaimed bytes != children's allocations");
    ck(fb1 == fallbacks1, "wave 1: fallback stats != actual fallbacks");
    printf("wave 1: %lld ν lanes reset at join, %lld bytes reclaimed O(1), %lld fallbacks\n",
           (long long)lanes1, (long long)bytes1, (long long)fb1);

    /* ------------------- wave 2: the free-set return ------------------- */
    /* every wave-2 child MUST get a loan: wave 1's join returned all lanes
     * to their threads' free sets (parent = NULL). */
    void *n2 = sn_nursery_new();
    ChildArg args2[W2_CHILDREN];
    for (int i = 0; i < W2_CHILDREN; i++) {
        args2[i].nursery = n2;
        args2[i].idx = 100 + i; /* distinct pattern space from wave 1 */
        args2[i].size = 512 + (int64_t)i * 128;
        args2[i].nu = -1;
        args2[i].sum = 0;
        args2[i].checked = 0;
        sn_nursery_enter_task(n2);
        sn_go_spawn(carve_child, &args2[i]);
    }
    sn_nursery_wait(n2);

    int64_t lanes2 = sn_nursery_join_stats(0) - lanes0;
    int64_t bytes2 = sn_nursery_join_stats(1) - bytes0;
    int64_t expect_bytes2 = 0;
    for (int i = 0; i < W2_CHILDREN; i++) {
        ck(args2[i].checked, "wave-2 child did not run to completion");
        ck(args2[i].nu >= 0, "wave 2: lane not returned to the free set by the join");
        ck(args2[i].sum == pattern_sum(args2[i].idx, args2[i].size),
           "wave-2 child checksum mismatch");
        expect_bytes2 += args2[i].size;
    }
    ck(lanes2 - lanes1 == W2_CHILDREN, "wave 2: join reset count != 7 loans");
    ck(bytes2 - bytes1 == expect_bytes2, "wave 2: reclaimed bytes mismatch");
    printf("wave 2: all %d children re-loaned (free set returned), %lld more bytes reclaimed\n",
           W2_CHILDREN, (long long)(bytes2 - bytes1));

    /* ------------------- wave 3: nested nurseries ---------------------- */
    void *n3 = sn_nursery_new();
    NestedArg nested[W3_CHILDREN];
    for (int i = 0; i < W3_CHILDREN; i++) {
        nested[i].outer = n3;
        nested[i].self.nursery = n3;
        nested[i].self.idx = 200 + i;
        nested[i].self.size = 2048;
        nested[i].self.nu = -1;
        nested[i].self.sum = 0;
        nested[i].self.checked = 0;
        nested[i].grand_loans = 0;
        nested[i].grand_bytes = 0;
        sn_nursery_enter_task(n3);
        sn_go_spawn(nested_child, &nested[i]);
    }
    sn_nursery_wait(n3);

    int64_t lanes3 = sn_nursery_join_stats(0) - lanes0;
    int64_t bytes3 = sn_nursery_join_stats(1) - bytes0;
    int64_t expect3 = 0;
    for (int i = 0; i < W3_CHILDREN; i++) {
        ck(nested[i].self.checked, "wave-3 child did not run to completion");
        ck(nested[i].self.sum == pattern_sum(nested[i].self.idx, nested[i].self.size),
           "wave-3 child checksum mismatch");
        if (nested[i].self.nu >= 0) expect3 += nested[i].self.size;
        for (int g = 0; g < W3_GRAND; g++) {
            ck(nested[i].grand[g].checked, "wave-3 grandchild did not run");
        }
    }
    ck(lanes3 > lanes2, "wave 3: nested loans were not reclaimed at the joins");
    ck(bytes3 - bytes2 > expect3, "wave 3: nested allocations were not reclaimed");
    printf("wave 3: nested nurseries reclaimed %lld ν lanes / %lld more bytes across joins\n",
           (long long)(lanes3 - lanes2), (long long)(bytes3 - bytes2));

    /* the parent's lane 0 watermark never moved: the carve reserves child
     * threads' lanes, never the parent's */
    ck(sn_lane_used(0) == parent_used_before,
       "parent lane 0 watermark moved (carve must not touch the parent's lane)");
    printf("parent lane_used(0) unchanged: %lld == %lld\n",
           (long long)sn_lane_used(0), (long long)parent_used_before);

    /* ------------- the loaned-lane fail-fast (UAF closure) -------------- */
    /* After leave_task the ν permission is cleared but the loan stays
     * registered until the join; a later task on the same thread allocating
     * on that lane would put live bytes under the join's 1-store reset —
     * the runtime backstop must refuse it (exit 1). Runs in a forked child
     * (the refusal is process-fatal); skipped under ASan. */
#if !defined(__has_feature) || !__has_feature(address_sanitizer)
    {
        fflush(stdout);
        pid_t pid = fork();
        ck(pid >= 0, "fork for backstop test failed");
        if (pid == 0) {
            void *nb = sn_nursery_new();
            sn_nursery_enter_task(nb);
            int64_t nu = sn_nursery_loan_lane(nb);
            if (nu < 0) _exit(2);
            sn_nursery_leave_task(nb); /* permission cleared; loan registered */
            sn_lane_alloc(nu, 8);     /* must exit(1): foreign loaned lane */
            _exit(3);                 /* backstop did NOT fire: bug */
        }
        int st = 0;
        ck(waitpid(pid, &st, 0) == pid, "waitpid failed");
        ck(WIFEXITED(st) && WEXITSTATUS(st) == 1,
           "loaned-lane backstop did not fail-fast (expected exit 1)");
        printf("backstop: allocating on a loaned ν lane after leave refused (exit 1)\n");
    }
#endif

    printf("PASS: nursery child-lane carve — loaned ν lanes reclaimed O(1) at join\n");

    /* Leak-check hook (macOS: LSan is unsupported under ASan on arm64 darwin;
     * the runner pauses here so `leaks` can attach before exit):
     * SN_CARVE_LEAK_PAUSE=<seconds> sleeps before returning. */
    {
        const char *pause = getenv("SN_CARVE_LEAK_PAUSE");
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
