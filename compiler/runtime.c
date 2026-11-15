#ifndef _GNU_SOURCE
#define _GNU_SOURCE /* pthread_getattr_np: report worker stack bounds */
#endif
#ifdef _WIN32
#ifndef _CRT_SECURE_NO_WARNINGS
#define _CRT_SECURE_NO_WARNINGS /* fopen/getenv/strerror are fine for snc */
#endif
#endif
#include <ctype.h>
#include <errno.h>
#include <inttypes.h>
#include <stdint.h>
#include <stddef.h> /* SWARM RABBIT HOLE, D0 gap #2: offsetof for the SnLane layout asserts */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#include <direct.h>
#include <io.h>
#include <windows.h>
#include <process.h>
#include <winsock2.h>
#include <ws2tcpip.h>
#include <intrin.h>
#ifndef F_OK
#define F_OK 0
#endif
#else
#include <arpa/inet.h>
#include <dirent.h>
#include <fcntl.h>
#include <netdb.h>
#include <netinet/in.h>
#include <poll.h>
#include <pthread.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/mman.h>
#if defined(__APPLE__) || defined(__FreeBSD__) || defined(__OpenBSD__) || \
    defined(__NetBSD__) || defined(__DragonFly__)
#include <sys/sysctl.h>
#endif
#include <unistd.h>
#include <sys/wait.h>
#if defined(__APPLE__) || defined(__FreeBSD__) || defined(__OpenBSD__) || \
    defined(__NetBSD__) || defined(__DragonFly__)
#include <sys/event.h>
#include <sys/time.h>
#define SN_HAVE_KQUEUE 1
#endif
#endif

typedef struct Obj {
    int64_t rc;
    int64_t kind;
} Obj;

void sn_gc_register_thread(void *lo, void *hi);
static int gc_stack_bounds(void **lo, void **hi);

#ifdef _WIN32
#define SN_POPEN _popen
#define SN_PCLOSE _pclose
#define SN_NULL_REDIR "2>nul"
#else
#define SN_POPEN popen
#define SN_PCLOSE pclose
#define SN_NULL_REDIR "2>/dev/null"
#endif

enum {
    KIND_STR = 1,
    KIND_LIST = 2,
    KIND_MAP = 3,
    KIND_ERR = 4,
    KIND_CHAN = 5,
    KIND_LOCK = 6,
    KIND_BOX = 7,
    KIND_JSON = 8,
    KIND_CLOS = 9,
    /* SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the SFC
       layouts (the morton list / hilbert map headers). Lane-backed:
       never gc_track'd, ARC'd nowhere (the sn_release no-op below is
       the defense-in-depth backstop). */
    KIND_MORTON = 10,
    KIND_HILBERT = 11,
    KIND_USER = 100
};

typedef struct Str {
    int64_t rc;
    int64_t kind;
    int64_t len;
    char data[];
} Str;

typedef struct List {
    int64_t rc;
    int64_t kind;
    int64_t elem_kind; /* 0=i64, 1=owned ptr (ARC), 2=borrow (ref/mut/ptr; no ARC — Wave 12 Impl-Q) */
    int64_t len;
    int64_t cap;
    int64_t *items;
} List;

/* Thread-local storage macro (used by the list slab's grown-list registry
   below and the lane section; kept with the list code because the
   registry is the earliest user — Wave 12, Impl-Q). */
#ifdef _WIN32
#define SN_TLS __declspec(thread)
#else
#define SN_TLS __thread
#endif

typedef struct Map {
    int64_t rc;
    int64_t kind;
    int64_t key_kind;
    int64_t val_kind;
    int64_t len;
    int64_t cap;
    int64_t hcap;
    int64_t *keys;
    int64_t *vals;
    uint64_t *hashes;
    int64_t *buckets;
} Map;

/* --------------------------------------------------------------------- */
/*  Tier-G Z-set (SWARM RABBIT HOLE, P5 slice 1; W2C §1.1, W4C Repair 1). */
/*  G[X] = the direct sum over the key universe with FINITE SUPPORT:     */
/*  a group-valued multiset { key -> ℤ coefficient }. The state space   */
/*  is the change space (𝒟/ℐ are both Z-sets); negative counts are       */
/*  retraction; retraction is EXACT (the Tier-G capability label under   */
/*  the capability order O ⊑ L ⊑ G). Zero counts are pruned in the      */
/*  canonical form (prune=1) or kept per the flag — the direct sum's    */
/*  finite-support discipline is the pruned form. Shares the O(1)       */
/*  open-addressing scheme of the Map (T1.1): hcap buckets storing      */
/*  entry-index+1, stored hashes, linear probing.                       */
/* --------------------------------------------------------------------- */
typedef struct SnZSet {
    int64_t rc;
    int64_t kind;     /* KIND_ZSET */
    int64_t key_kind; /* 0=i64, 1=Str* (content-equal keys) — Map's convention */
    int64_t prune;    /* 1: zero counts pruned (canonical), 0: kept */
    int64_t len;      /* support size |supp(Z)| */
    int64_t cap;
    int64_t hcap;
    int64_t *keys;
    int64_t *counts;  /* the ℤ coefficient per key */
    uint64_t *hashes;
    int64_t *buckets;
    int64_t total;    /* running Σ counts — sn_zset_total is O(1) */
} SnZSet;

/* --------------------------------------------------------------------- */
/*  SnLCell — the Tier-L join-semilattice cell (SWARM RABBIT HOLE, Wave 9, */
/*  Impl-M, Cohort Delta; W2C §1.1 Tier L; final report §8.1; the         */
/*  capability order O ⊑ L ⊑ G). Where the Z-set is the Tier-G group     */
/*  carrier (state space = change space, ⊖ first-class, retraction       */
/*  exact), the LCell is the semilattice tier's carrier: values only     */
/*  INFLATE, ⊔ is the ONLY combinator, deltas are state-relative         */
/*  fragments via the delta-mutator law m(X) = X ⊔ m^δ(X)                */
/*  (Almeida–Shoker–Baquero JPDC 2018; Enes et al. 2018 — the            */
/*  join-decomposition-optimal delta), and retraction is tombstoned or    */
/*  none, NEVER exact: the Grothendieck completion of the idempotent     */
/*  merge operator is trivial (x⊔x = x ⇒ [x] = 0), so ⊖ does not and     */
/*  cannot exist on this surface — the checked E-LRET rejection.         */
/*                                                                       */
/*  Three carriers, each a join-semilattice BY CONSTRUCTION (W2C's       */
/*  admission rule: carrier verified idempotent+commutative+associative, */
/*  checked structurally — the classification the compiler applies to    */
/*  monotone ⊔-merge aggregates):                                       */
/*   LCELL_MAX  — int high-water mark: state = one int64, ⊔ = max.       */
/*                max(x,x)=x, max(x,y)=max(y,x), max(max(x,y),z)=        */
/*                max(x,max(y,z)); ⊥ = INT64_MIN.                        */
/*   LCELL_GSET — grow-only int set: ⊔ = union. A∪A=A, A∪B=B∪A,          */
/*                (A∪B)∪C=A∪(B∪C); ⊥ = ∅. Membership/insert share the    */
/*                SnZSet O(1) open-addressing scheme (T1.1).              */
/*   LCELL_LWW  — last-writer-wins with a logical clock: ⊔ = the          */
/*                lexicographic max of (clock, value) — higher clock     */
/*                wins, ties break DETERMINISTICALLY by the higher       */
/*                value, so the winner of any bag of offers is           */
/*                order-independent (a total order's max is             */
/*                idempotent, commutative, associative); ⊥ = no offer.   */
/* --------------------------------------------------------------------- */
enum {
    LCELL_MAX  = 0, /* int high-water mark: ⊔ = max */
    LCELL_GSET = 1, /* grow-only int set: ⊔ = union */
    LCELL_LWW  = 2  /* last-writer-wins + logical clock: ⊔ = lexmax(clock, value) */
};

typedef struct SnLCell {
    int64_t rc;
    int64_t kind;      /* KIND_LCELL */
    int64_t carrier;   /* LCELL_MAX / LCELL_GSET / LCELL_LWW */
    int64_t hwm;       /* MAX: the high-water mark (⊥ = INT64_MIN) */
    int64_t lww_set;   /* LWW: 1 once an offer exists (⊥ = no offer) */
    int64_t lww_val;   /* LWW: the winning offer's value */
    int64_t lww_clock; /* LWW: the winning offer's logical clock */
    int64_t len;       /* GSET: |set| */
    int64_t cap;
    int64_t hcap;
    int64_t *elems;    /* GSET: the element array (i64 elements only) */
    uint64_t *hashes;
    int64_t *buckets;
} SnLCell;

typedef struct Err {
    int64_t rc;
    int64_t kind;
    Str *msg;
} Err;

typedef struct BoxI64 {
    int64_t rc;
    int64_t kind;
    int64_t val;
} BoxI64;

typedef struct Chan {
    int64_t rc;
    int64_t kind;
    int64_t closed;
    int64_t len;
    int64_t cap;
    int64_t head;
    int64_t *buf;
#ifdef _WIN32
    CRITICAL_SECTION mu;
    CONDITION_VARIABLE cv;
#else
    pthread_mutex_t mu;
    pthread_cond_t cv;
#endif
} Chan;

/* --------------------------------------------------------------------- */
/*  Linear message envelopes (SWARM RABBIT HOLE: W2-E DIFF-3, amended by   */
/*  W4A repairs 4 & 6c). The amended STRATA §2.5.2 Mode B: a lane-backed   */
/*  payload packs into a self-contained slab the receiver ADOPTS, exactly */
/*  once ("or copies" is deleted — a copied-and-adopted slab double-frees */
/*  and a never-adopted copy leaks; linearity is the only sound           */
/*  discipline). The slab is a malloc'd detached buffer — NEVER a lane,    */
/*  NEVER gc_track'd — so the atomic header below is the only ownership   */
/*  record.                                                               */
/* --------------------------------------------------------------------- */
enum {
    SN_ENV_PACKED   = 0,  /* sender built the slab; not yet queued          */
    SN_ENV_QUEUED   = 1,  /* published to a channel queue (release@send)   */
    SN_ENV_RECEIVED = 2,  /* popped by a receiver; not yet adopted/dropped */
    SN_ENV_ADOPTED  = 3,  /* terminal: receiver adopted the bytes onto its */
                          /* own lane/frame; the slab was freed           */
    SN_ENV_FREED    = 4   /* terminal: freed without adoption (drop /      */
                          /* failed send / task-group drain)               */
};

typedef struct SnMsgEnvelope {
    /* W4A repair 4: the CAS on `state` is the single linearization point
       between a racing receiver and the task-group join/abort drain. */
    int64_t state;   /* atomic; SN_ENV_* above */
    int64_t refs;    /* atomic; holders: channel queue, task-group registry,
                        receiver — the HEADER is freed at 0 */
    void *slab;      /* detached malloc'd payload (never a lane, never GC-tracked) */
    int64_t len;     /* payload bytes */
    void *chan;      /* the Chan this envelope was queued on (W4A repair 4
                        residual: the supervisor reaches every channel a
                        group's tasks sent to through the registry) */
    struct SnMsgEnvelope *next_group; /* intrusive task-group registry link */
} SnMsgEnvelope;

/* Portable atomics via compiler builtins (clang on every supported target,
   including the Windows cross builds — build.ps1/Makefile pin clang).
   Memory orders per W4A repair 6c: release-store at send, acquire-load at
   adopt, acquire-release CAS on every state transition (seq_cst subsumes
   acq_rel; acq_rel is the specified minimum). */
static int64_t sn_atomic_load_acq(int64_t *p) {
    return __atomic_load_n(p, __ATOMIC_ACQUIRE);
}
static void sn_atomic_store_rel(int64_t *p, int64_t v) {
    __atomic_store_n(p, v, __ATOMIC_RELEASE);
}
static int sn_atomic_cas_acqrel(int64_t *p, int64_t expected, int64_t desired) {
    int64_t e = expected;
    return __atomic_compare_exchange_n(p, &e, desired, 0,
                                        __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE);
}
static void sn_atomic_retain(int64_t *p) {
    __atomic_fetch_add(p, 1, __ATOMIC_ACQ_REL);
}
/* Returns 1 when this release dropped the count to zero (caller frees). */
static int sn_atomic_release_is_last(int64_t *p) {
    return __atomic_fetch_sub(p, 1, __ATOMIC_ACQ_REL) == 1;
}


typedef struct Lock {
    int64_t rc;
    int64_t kind;
#ifdef _WIN32
    CRITICAL_SECTION mu;
#else
    pthread_mutex_t mu;
#endif
} Lock;

static int64_t g_live_spawns = 0;
#ifdef _WIN32
static CRITICAL_SECTION g_spawn_mu;
static CONDITION_VARIABLE g_spawn_cv;
static int g_rt_ready = 0;
#else
static pthread_mutex_t g_spawn_mu = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t g_spawn_cv = PTHREAD_COND_INITIALIZER;
#endif

static int g_argc = 0;
static char **g_argv = NULL;

#ifdef _WIN32
/* Forward declarations: the real definitions live with the channel/scheduler
   sections below, but sn_rt_init() (also below) initializes them at startup.
   Tentative definitions coalesce, so this is legal C on MSVC and clang. */
static CRITICAL_SECTION g_evt_mu;
static CONDITION_VARIABLE g_evt_cv;
static CRITICAL_SECTION g_go_mu;
static CONDITION_VARIABLE g_go_cv;
static CRITICAL_SECTION g_go_evt_mu;
static CONDITION_VARIABLE g_go_evt_cv;
/* Reactor lock: POSIX uses static initializers, so Windows must init here
   in sn_rt_init -- rx_stop() locks unconditionally even when the reactor
   never started. Tentative definitions coalesce with the later ones. */
static CRITICAL_SECTION g_rx_mu;
static CONDITION_VARIABLE g_rx_cv;

/* Portable current-thread identity for GC thread tracking. */
static void *sn_thread_self(void) {
    return (void *)(uintptr_t)GetCurrentThreadId();
}
#else
static void *sn_thread_self(void) {
    return (void *)pthread_self();
}
#endif


/* ---- endianness / byte swapping (network-order / host conversion) ---- */
static uint16_t sn_swap16(uint16_t v) {
    return (uint16_t)((v >> 8) | (v << 8));
}
static uint32_t sn_swap32(uint32_t v) {
    v = ((v >> 8) & 0x00FF00FFu) | ((v << 8) & 0xFF00FF00u);
    return (v >> 16) | (v << 16);
}
static uint64_t sn_swap64(uint64_t v) {
    v = ((v >> 8)  & 0x00FF00FF00FF00FFULL) | ((v << 8)  & 0xFF00FF00FF00FF00ULL);
    v = ((v >> 16) & 0x0000FFFF0000FFFFULL) | ((v << 16) & 0xFFFF0000FFFF0000ULL);
    return (v >> 32) | (v << 32);
}
#if defined(__BYTE_ORDER__) && __BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__
#define SN_IS_LE 1
#else
#define SN_IS_LE 0
#endif
int64_t sn_hton16(int64_t v) { return SN_IS_LE ? (int64_t)sn_swap16((uint16_t)v) : v; }
int64_t sn_ntoh16(int64_t v) { return SN_IS_LE ? (int64_t)sn_swap16((uint16_t)v) : v; }
int64_t sn_hton32(int64_t v) { return SN_IS_LE ? (int64_t)sn_swap32((uint32_t)v) : v; }
int64_t sn_ntoh32(int64_t v) { return SN_IS_LE ? (int64_t)sn_swap32((uint32_t)v) : v; }
int64_t sn_hton64(int64_t v) { return SN_IS_LE ? (int64_t)sn_swap64((uint64_t)v) : v; }
int64_t sn_ntoh64(int64_t v) { return SN_IS_LE ? (int64_t)sn_swap64((uint64_t)v) : v; }
int64_t sn_swap16_i64(int64_t v) { return (int64_t)sn_swap16((uint16_t)v); }
int64_t sn_swap32_i64(int64_t v) { return (int64_t)sn_swap32((uint32_t)v); }
int64_t sn_swap64_i64(int64_t v) { return (int64_t)sn_swap64((uint64_t)v); }
/* SWARM RABBIT HOLE DIFF-6 (W2-E spec): lane poison-fill mode, resolved once
   here — single-threaded at startup — so sn_lane_alloc's fast path never
   races the lazy fallback in sn_lane_poison_on(). Definition lives with the
   lane machinery below sn_lane_ensure(). */
static int sn_lane_poison_on(void);

void sn_rt_init(int argc, char **argv) {
    g_argc = argc;
    g_argv = argv;
    sn_lane_poison_on(); /* DIFF-6: resolve poison mode before any task starts */

#ifdef _WIN32
    if (!g_rt_ready) {
        InitializeCriticalSection(&g_spawn_mu);
        InitializeConditionVariable(&g_spawn_cv);
        InitializeCriticalSection(&g_evt_mu);
        InitializeConditionVariable(&g_evt_cv);
        InitializeCriticalSection(&g_go_mu);
        InitializeConditionVariable(&g_go_cv);
        InitializeCriticalSection(&g_go_evt_mu);
        InitializeConditionVariable(&g_go_evt_cv);
        InitializeCriticalSection(&g_rx_mu);
        InitializeConditionVariable(&g_rx_cv);
        g_rt_ready = 1;
        WSADATA wsa;
        WSAStartup(MAKEWORD(2, 2), &wsa);
    }
#endif
}

void sn_panic(void *msg);
/* Wave 15 (Impl-T): Salt ε's degraded contract assertion. */
void sn_contract_assert(int64_t ok, const char *msg);
void *sn_error_new(void *msg);

static void *xmalloc(size_t n) {
    void *p = calloc(1, n);
    if (!p) {
        fprintf(stderr, "out of memory\n");
        exit(1);
    }
    return p;
}

void sn_retain(void *p) {
    if (!p) return;
    Obj *o = (Obj *)p;
    o->rc++;
}

typedef struct ByteArray {
    int64_t rc;
    int64_t kind;
    int64_t len;
    int64_t cap;
    unsigned char *data;
} ByteArray;

typedef struct Sb {
    int64_t rc;
    int64_t kind;
    int64_t cap;
    int64_t len;
    char *buf;
} Sb;

enum { KIND_BYTEARRAY = 20, KIND_POOL = 21, KIND_STRBUILDER = 22, KIND_ZSET = 23, KIND_LCELL = 24, KIND_ZFIX = 25, KIND_TAPE = 26 };

/* Forward: the Salt-ζ engine's struct is defined with the Tier-L/G
 * engines below, but gc_mark/gc_free/sn_release trace and free it by
 * kind — the same forward-declaration discipline as Clos. */
typedef struct SnZFix SnZFix;

/* The body's join index: CSR adjacency, target b → the sources a with
 * (a,b) ∈ E (the closure body reach(a) :- edge(a,b), reach(b) joins
 * the recursive variable at the edge relation's TARGET column). Full
 * comment with the engine below. */
typedef struct SnZFixAdj {
    int64_t len;    /* distinct targets */
    int64_t cap;
    int64_t hcap;
    int64_t *nodes; /* the distinct targets b */
    int64_t *cnt;   /* per target: in-edge count (fill cursor during build) */
    int64_t *off;   /* per target: start offset into buf (len+1 entries) */
    int64_t *buf;   /* the flat in-neighbor array */
    uint64_t *hashes;
    int64_t *buckets;
    int64_t total;  /* the number of distinct packed edges */
} SnZFixAdj;

/* Salt ζ — the fixpoint-bracket engine (Wave 9, Impl-N; W2C §3). The
 * struct lives here (before gc_mark) so the collector can trace its
 * carriers; the engine, the round loop, and the convergence-theorem
 * commentary live with the Tier-G/Tier-L engines below. */
struct SnZFix {
    int64_t rc;
    int64_t kind;     /* KIND_ZFIX */
    int64_t tier;     /* SN_ZFIX_TIER_G / SN_ZFIX_TIER_L */
    SnZFixAdj adj;    /* the body's join index (target → sources) */
    SnZSet *edges;    /* borrowed ref: the packed-pair edge relation */
    SnZSet *seed;     /* borrowed ref: the base facts */
    SnZSet *state;    /* Tier G: ∫ — the accumulated reach (owned) */
    SnLCell *state_l; /* Tier L: ∫ — the accumulated reach (owned) */
    SnZSet *delta;    /* Tier G: δ₀ — the NEXT round's delta (owned) */
    SnLCell *delta_l; /* Tier L: δ₀ — the mutator-law fragment (owned) */
    int64_t rounds;   /* productive rounds (empty round = the stop test) */
    int64_t probes;   /* Σ adjacency visits — the per-round work observable */
    int64_t done;     /* 1 once a round's delta was empty */
};

/* SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the global-pool
 * discipline (kills G1 §1 A6c — the global-pool freelist race).
 * A pool allocated at global/static scope carries a MODE:
 *
 *   GP_FROZEN      — the default. Read-only from every thread: the
 *                    checker rejects insert/set/remove outside an
 *                    `unfrozen (P) { ... }` window (E0803.3) and
 *                    rejects spawn/chan capture of P's handles inside
 *                    the window (E0803.2). Handles are Sendable
 *                    outside the window: no legal write path exists,
 *                    so no thread can corrupt or race the freelist.
 *   GP_THREADLOCAL — mutations legal on the declaring thread only
 *                    (E0803.1 on capture; the home_thread tag below
 *                    is the runtime backstop — a ThreadConfined
 *                    pool's mutator refuses any other thread).
 *   GP_SYNC        — the vault: every insert/set/remove funnels
 *                    through mu below; handles Sendable everywhere.
 *                    No two threads can ever interleave insert/remove
 *                    on one pool — the data-race-freedom condition.
 *
 * frozen_depth mirrors SnLane's frozen depth for debug-mode
 * enforcement of the window discipline (enter = unfreeze, exit =
 * re-freeze; the structured escapes are lowered to re-freeze, and a
 * panicked program never resumes).
 *
 * Soundness (the DIFF-4 argument, runtime side): a Frozen pool has no
 * legal write path, so any number of threads holding handles cannot
 * corrupt or race the freelist; ThreadConfined handles never leave the
 * owning thread (capture rejected by the checker; the tag refuses any
 * cross-thread op); Synchronized pools serialize every freelist
 * mutation behind mu — no two threads can ever interleave
 * insert/remove on one pool; the `unfrozen` window restores
 * single-thread, structured, capture-blocked mutation. The CHECKER is
 * the gate (E0803.1/.2/.3); this runtime state is the enforcement
 * backstop, mirroring SnLane's frozen_depth (E-LKIND's two-layer
 * precedent: the checker catches what it can see; the runtime
 * fail-fasts on the rest).
 *
 * Layout note: rc/kind stay first (the generic object header every
 * sn_release/sweep path reads); the mode bits are additive and only
 * consulted by the freeze/lock paths below. */
enum { GP_FROZEN = 0, GP_THREADLOCAL = 1, GP_SYNC = 2 };

typedef struct SnPool {
    int64_t rc;
    int64_t kind;
    int64_t elem_kind;
    int64_t elem_size;
    uint32_t len;
    uint32_t cap;
    uint32_t active_count;
    uint32_t free_head;
    uint8_t *slots;
    uint32_t *generations;
    uint32_t *free_next;
    /* DIFF-4 (Impl-O): the mode (GP_*), the window depth (debug
     * enforcement of E0803.2's mutable-window), the vault mutex
     * (GP_SYNC), and the ThreadConfined home thread. */
    int64_t mode;
    int64_t frozen_depth;
    void *home_thread;
#ifdef _WIN32
    CRITICAL_SECTION mu;
#else
    pthread_mutex_t mu;
#endif
} SnPool;

/* --------------------------------------------------------------------- */
/*  Cycle collector                                                       */
/*                                                                       */
/*  Reference counting alone cannot reclaim reference cycles, so long-lived */
/*  object graphs leak. This is a conservative mark-sweep collector over   */
/*  every GC-managed allocation:                                        */
/*                                                                       */
/*    roots   = registered globals + the C stack of every registered thread */
/*    trace   = list items, map keys/values, error messages, closures,      */
/*              and the pointer-shaped fields of blueprint instances       */
/*    sweep   = anything unreachable is freed regardless of its refcount   */
/*                                                                       */
/*  It only runs when no other thread is executing SNlang code, so a stack  */
/*  scan never races with a mutator.                                       */
/* --------------------------------------------------------------------- */

typedef struct Clos Clos; /* defined with the closure runtime below */

/* Every GC-managed allocation, as {ptr, size, marked}. */
typedef struct GcObj {
    void *p;
    int64_t size;
    int64_t marked;
} GcObj;

static GcObj *g_gc_objs;
static int64_t g_gc_count;
static int64_t g_gc_cap;
static int64_t g_gc_threshold = 4096;
static int64_t g_gc_alloc_since;
static int64_t g_gc_running;

/* SNlang threads currently executing user code. */
static int64_t g_active_threads = 1;

static GcObj *g_gc_roots[256];
static int64_t g_gc_nroots;

/* SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): register an immortal
 * global object (a global pool) as a permanent GC root. The pools are
 * gc_track'd by construction (sn_pool_new), but module-scope pools are
 * only referenced from LLVM globals (@sn_g_{name}) — invisible to the
 * precise-root pushes, so an unlucky collection would sweep them.
 * Global pools are module-scoped: they live for the program's
 * duration, which is exactly what a permanent root means. */
void sn_gc_register_global(void *p);

typedef struct GcStackRange {
    void *lo; /* stack base (lowest address) */
    void *hi; /* stack top (highest address) */
    void *owner;
} GcStackRange;

#ifdef _WIN32
static CRITICAL_SECTION g_gc_mu;
static int g_gc_mu_inited = 0;
static void gc_lock(void) {
    if (!g_gc_mu_inited) { InitializeCriticalSection(&g_gc_mu); g_gc_mu_inited = 1; }
    EnterCriticalSection(&g_gc_mu);
}
static void gc_unlock(void) { LeaveCriticalSection(&g_gc_mu); }
#else
static pthread_mutex_t g_gc_mu = PTHREAD_MUTEX_INITIALIZER;
static void gc_lock(void) { pthread_mutex_lock(&g_gc_mu); }
static void gc_unlock(void) { pthread_mutex_unlock(&g_gc_mu); }
#endif

#define GC_MAX_THREADS 64
/* Window above the collecting thread's stack pointer that is scanned for
   roots. 1 MiB comfortably covers a normal SNlang call chain. */
#define GC_SELF_SCAN (1 << 20)
static GcStackRange g_gc_threads[GC_MAX_THREADS];
static int64_t g_gc_nthreads;

static int gc_index_of(const void *p) {
    /* Linear scan: the heap is small enough that this beats a hash table
       for the object counts SNlang programs actually reach. */
    for (int64_t i = 0; i < g_gc_count; i++) {
        if (g_gc_objs[i].p == p) return (int)i;
    }
    return -1;
}

static void gc_track(void *p, int64_t size) {
    if (!p) return;
    gc_lock();
    if (g_gc_count == g_gc_cap) {
        int64_t ncap = g_gc_cap ? g_gc_cap * 2 : 1024;
        g_gc_objs = (GcObj *)realloc(g_gc_objs, (size_t)ncap * sizeof(GcObj));
        g_gc_cap = ncap;
    }
    g_gc_objs[g_gc_count].p = p;
    g_gc_objs[g_gc_count].size = size;
    g_gc_objs[g_gc_count].marked = 0;
    g_gc_count++;
    gc_unlock();
}

static void gc_untrack(void *p) {
    gc_lock();
    int i = gc_index_of(p);
    if (i >= 0) {
        g_gc_objs[i] = g_gc_objs[g_gc_count - 1];
        g_gc_count--;
    }
    gc_unlock();
}

/* Record a GC-managed allocation. */
void sn_gc_collect(void);

/* Track an allocation. Collection is *not* triggered here: an object under
   construction is not yet reachable from any stack slot the collector can see,
   so sweeping mid-construction would free a live object. Safe points call
   sn_gc_poll() instead (loop back edges, scheduler idle, reactor idle). */
static void gc_alloc(void *p, int64_t size) {
    gc_track(p, size);
    g_gc_alloc_since++;
}

/* Exact stack bounds for the calling thread, or 0 if unavailable. */
static int gc_stack_bounds(void **lo, void **hi) {
    void *addr = NULL;
    size_t sz = 0;
#if defined(__APPLE__)
    addr = pthread_get_stackaddr_np(pthread_self());
    sz = pthread_get_stacksize_np(pthread_self());
#elif defined(_WIN32)
    return 0;
#else
    pthread_attr_t attr;
    if (pthread_getattr_np(pthread_self(), &attr) != 0) return 0;
    int rc = pthread_attr_getstack(&attr, &addr, &sz);
    pthread_attr_destroy(&attr);
    if (rc != 0) return 0;
#endif
    if (!addr || sz == 0) return 0;
#if defined(__APPLE__)
    *lo = (char *)addr - sz;
    *hi = addr;
#else
    *lo = addr;
    *hi = (char *)addr + sz;
#endif
    return 1;
}

void sn_gc_register_global(void *p) {
    gc_lock();
    if (g_gc_nroots < 256) {
        g_gc_roots[g_gc_nroots++] = (GcObj *)p;
    }
    gc_unlock();
}

void sn_gc_register_thread(void *lo, void *hi) {
    void *owner = sn_thread_self();
    gc_lock();
    for (int64_t i = 0; i < g_gc_nthreads; i++) {
        if (g_gc_threads[i].owner == owner) {
            g_gc_threads[i].lo = lo;
            g_gc_threads[i].hi = hi;
            gc_unlock();
            return;
        }
    }
    if (g_gc_nthreads < GC_MAX_THREADS) {
        g_gc_threads[g_gc_nthreads].lo = lo;
        g_gc_threads[g_gc_nthreads].hi = hi;
        g_gc_threads[g_gc_nthreads].owner = owner;
        g_gc_nthreads++;
    }
    gc_unlock();
}

static void gc_mark(void *p) {
    if (!p) return;
    int idx = gc_index_of(p);
    if (idx < 0) return;
    if (g_gc_objs[idx].marked) return;
    g_gc_objs[idx].marked = 1;

    Obj *o = (Obj *)p;
    switch (o->kind) {
    case KIND_LIST: {
        List *l = (List *)p;
        if (l->elem_kind == 1) {
            for (int64_t i = 0; i < l->len; i++) gc_mark((void *)(intptr_t)l->items[i]);
        }
        break;
    }
    case KIND_MAP: {
        Map *m = (Map *)p;
        if (m->key_kind == 1) {
            for (int64_t i = 0; i < m->len; i++) gc_mark((void *)(intptr_t)m->keys[i]);
        }
        if (m->val_kind == 1) {
            for (int64_t i = 0; i < m->len; i++) gc_mark((void *)(intptr_t)m->vals[i]);
        }
        break;
    }
    case KIND_ZSET: {
        SnZSet *z = (SnZSet *)p;
        if (z->key_kind == 1) {
            for (int64_t i = 0; i < z->len; i++) gc_mark((void *)(intptr_t)z->keys[i]);
        }
        break;
    }
    case KIND_LCELL: {
        /* Tier-L cell (Wave 9): GSET elements are i64 values, and the
           MAX/LWW carriers hold inline scalars — nothing to trace. */
        break;
    }
    case KIND_ZFIX: {
        /* Salt-ζ fixpoint engine (Wave 9, Impl-N): a first-class
           tracked object — SNlang programs can trigger a collection
           mid-bracket (threshold-based), and the conservative stack
           scan reaches the engine's raw ptr but NOT the carriers
           behind that heap indirection. Tracing the six carrier
           pointers keeps a live bracket's Z-sets/cells alive; an
           abandoned engine is swept with its unreachable carriers. */
        SnZFix *f = (SnZFix *)p;
        gc_mark(f->edges);
        gc_mark(f->seed);
        gc_mark(f->state);
        gc_mark(f->delta);
        gc_mark(f->state_l);
        gc_mark(f->delta_l);
        break;
    }
    case KIND_ERR: {
        Err *e = (Err *)p;
        gc_mark(e->msg);
        break;
    }
    case KIND_JSON: {
        /* tag: 0 null, 1 bool, 2 int, 3 str, 4 arr, 5 obj */
        int64_t *j = (int64_t *)p;
        int64_t tag = j[2];
        if (tag == 3 || tag == 4 || tag == 5) gc_mark((void *)(intptr_t)j[4]);
        break;
    }
    case KIND_CLOS: {
        Clos *c = (Clos *)p;
        /* fn and env are raw code/raw env, not traced. */
        break;
    }
    case KIND_BYTEARRAY:
    case KIND_STRBUILDER: {
        /* data/buf is raw bytes, not traced. */
        break;
    }
    case KIND_POOL: {
        SnPool *pl = (SnPool *)p;
        if (pl->elem_kind == 1) {
            for (uint32_t i = 0; i < pl->len; i++) {
                if ((pl->generations[i] & 1u) != 0) {
                    int64_t raw = *(int64_t *)(pl->slots + (size_t)i * (size_t)pl->elem_size);
                    gc_mark((void *)(intptr_t)raw);
                }
            }
        }
        break;
    }
    default:
        if (o->kind >= KIND_USER) {
            /* Blueprint instance: header is [rc, kind, vptr], then fields.
               Any field that points at a tracked object keeps it alive. */
            int64_t *slots = (int64_t *)p;
            int64_t nslots = g_gc_objs[idx].size / 8;
            for (int64_t i = 3; i < nslots; i++) gc_mark((void *)(intptr_t)slots[i]);
        }
        break;
    }
}

/* Mark any tracked object whose address appears in [lo, hi).
 *
 * A CONSERVATIVE scanner: it deliberately reads arbitrary stack words
 * (any of them may hold a tracked pointer), so under ASan it also
 * reads stack-frame redzones — false positives intrinsic to the
 * design, not bugs. Exempt the scan from ASan; every real object read
 * (gc_mark/gc_free, the engines) stays instrumented. */
#if defined(__has_feature)
#  if __has_feature(address_sanitizer)
#    define SN_NO_ASAN __attribute__((no_sanitize("address")))
#  endif
#endif
#ifndef SN_NO_ASAN
#  define SN_NO_ASAN
#endif
static SN_NO_ASAN void gc_scan_range(void *lo, void *hi) {
    uintptr_t *p = (uintptr_t *)lo;
    uintptr_t *end = (uintptr_t *)hi;
    if (p > end) {
        uintptr_t *t = p;
        p = end;
        end = t;
    }
    for (; p < end; p++) {
        uintptr_t v = *p;
        if (v < 4096) continue;
        int idx = gc_index_of((void *)v);
        if (idx >= 0 && !g_gc_objs[idx].marked) gc_mark((void *)v);
    }
}

/* Frees one object exactly as sn_release would at refcount zero. */
static int list_is_slab_backed(List *l);

static void gc_free(void *p) {
    Obj *o = (Obj *)p;
    switch (o->kind) {
    case KIND_LIST: {
        List *l = (List *)p;
        if (!list_is_slab_backed(l)) free(l->items);
        break;
    }
    case KIND_MAP: {
        Map *m = (Map *)p;
        free(m->keys);
        free(m->vals);
        free(m->hashes);
        free(m->buckets);
        break;
    }
    case KIND_ZSET: {
        SnZSet *z = (SnZSet *)p;
        free(z->keys);
        free(z->counts);
        free(z->hashes);
        free(z->buckets);
        break;
    }
    case KIND_LCELL: {
        SnLCell *c = (SnLCell *)p;
        free(c->elems);
        free(c->hashes);
        free(c->buckets);
        break;
    }
    case KIND_ZFIX: {
        /* Sweep of an abandoned engine: free the CSR adjacency — the
           block itself is freed by the shared free(p) at the end of
           gc_free (this arm must NOT free it: double free). The
           carriers are tracked objects swept on their own (or kept
           alive by other references); their refcounts are the GC's
           business, not the sweep's — the same discipline as
           List/Map children. */
        SnZFix *f = (SnZFix *)p;
        free(f->adj.nodes);
        free(f->adj.cnt);
        free(f->adj.off);
        free(f->adj.buf);
        free(f->adj.hashes);
        free(f->adj.buckets);
        break;
    }
    case KIND_ERR: {
        Err *e = (Err *)p;
        /* the message is traced; freeing it here keeps refcounts honest */
        if (e->msg) {
            Obj *mo = (Obj *)e->msg;
            mo->rc--;
        }
        break;
    }
    case KIND_CHAN: {
        Chan *c = (Chan *)p;
        free(c->buf);
#ifdef _WIN32
        DeleteCriticalSection(&c->mu);
#else
        pthread_mutex_destroy(&c->mu);
        pthread_cond_destroy(&c->cv);
#endif
        break;
    }
    case KIND_LOCK: {
        Lock *l = (Lock *)p;
#ifdef _WIN32
        DeleteCriticalSection(&l->mu);
#else
        pthread_mutex_destroy(&l->mu);
#endif
        break;
    }
    case KIND_BYTEARRAY: {
        ByteArray *b = (ByteArray *)p;
        free(b->data);
        break;
    }
    case KIND_STRBUILDER: {
        Sb *sb = (Sb *)p;
        free(sb->buf);
        break;
    }
    case KIND_POOL: {
        SnPool *pl = (SnPool *)p;
        free(pl->slots);
        free(pl->generations);
        free(pl->free_next);
        break;
    }
    case KIND_JSON:
    case KIND_CLOS:
    case KIND_STR:
    default:
        break;
    }
    free(p);
}

/* Runs a full collection. Safe only when g_active_threads == 1. */
void sn_gc_collect(void) {
    gc_lock();
    if (g_gc_running || g_active_threads > 1 || g_gc_count == 0) {
        gc_unlock();
        return;
    }
    g_gc_running = 1;

    /* Roots for this thread: everything from the current stack pointer up to
       the thread's registered stack top. Locals of every live frame are
       inside that range. */
    volatile char anchor = 0;
    void *sp = (void *)&anchor;
    void *me = sn_thread_self();
    void *top = NULL;
    for (int64_t t = 0; t < g_gc_nthreads; t++) {
        if (g_gc_threads[t].owner == me) {
            top = g_gc_threads[t].hi;
            break;
        }
    }
    for (int64_t i = 0; i < g_gc_count; i++) g_gc_objs[i].marked = 0;

    /* The collecting thread's own frames: a generous bounded window above the
       current stack pointer. Caller frames always sit at higher addresses, so
       this covers every live frame without walking off the stack. */
    void *scan_hi = top ? top : (void *)((char *)sp + GC_SELF_SCAN);
    gc_scan_range(sp, scan_hi);

    int64_t i;
    for (i = 0; i < g_gc_nroots; i++) gc_mark(g_gc_roots[i]);
    /* Other threads registered at creation time have exact bounds. This is
       only safe because the GC refuses to run while any mutator is active. */
    for (i = 0; i < g_gc_nthreads; i++) {
        if (g_gc_threads[i].owner != me) {
            gc_scan_range(g_gc_threads[i].lo, g_gc_threads[i].hi);
        }
    }

    /* Sweep: compact the table, freeing everything unmarked. */
    int64_t w = 0;
    for (i = 0; i < g_gc_count; i++) {
        GcObj *g = &g_gc_objs[i];
        if (g->marked) {
            if (w != i) g_gc_objs[w] = *g;
            w++;
        } else {
            gc_free(g->p);
        }
    }
    g_gc_count = w;

    g_gc_running = 0;
    gc_unlock();
}

/* Precise roots: the compiler pushes the value of every GC-visible local that
   is live at the collection point, then calls sn_gc_collect_roots(). Pushing
   keeps the runtime free of any per-call-site allocation. */
static void *g_gc_pending[256];
static int64_t g_gc_npending;

void sn_gc_root_push(void *p) {
    if (g_gc_npending < 256) g_gc_pending[g_gc_npending++] = p;
}

/* Precise collection: the compiler passes the addresses of every GC-visible
   local that is live at this point, so reachability is exact rather than
   inferred from a stack scan. */
void sn_gc_collect_roots(void) {
    gc_lock();
    void **roots = g_gc_pending;
    int64_t n = g_gc_npending;
    g_gc_npending = 0;
    if (g_gc_running || g_active_threads > 1) {
        gc_unlock();
        return;
    }
    g_gc_running = 1;
    for (int64_t i = 0; i < g_gc_count; i++) g_gc_objs[i].marked = 0;
    for (int64_t i = 0; i < g_gc_nroots; i++) gc_mark(g_gc_roots[i]);
    /* The compiler-supplied roots are authoritative; no stack scan is needed,
       and avoiding one keeps the collector from walking off the stack. */
    for (int64_t i = 0; i < n; i++) gc_mark(roots[i]);

    int64_t w = 0;
    for (int64_t i = 0; i < g_gc_count; i++) {
        GcObj *g = &g_gc_objs[i];
        if (g->marked) {
            if (w != i) g_gc_objs[w] = *g;
            w++;
        } else {
            gc_free(g->p);
        }
    }
    g_gc_count = w;
    g_gc_running = 0;
    gc_unlock();
}

/* Called from a safe point: no object is half-built, and every live value is
   either in a stack slot or reachable from a root. */
void sn_gc_poll(void) {
    if (g_gc_alloc_since >= g_gc_threshold) {
        g_gc_alloc_since = 0;
        sn_gc_collect();
    }
}

/* Diagnostics for tests and the debugger. */
int64_t sn_gc_object_count(void) { return g_gc_count; }
void sn_gc_set_threshold(int64_t n) { g_gc_threshold = n > 0 ? n : 4096; }
void sn_gc_enter(void) {
    gc_lock();
    g_active_threads++;
    gc_unlock();
}
void sn_gc_leave(void) {
    gc_lock();
    g_active_threads--;
    void *owner = sn_thread_self();
    for (int64_t i = 0; i < g_gc_nthreads; i++) {
        if (g_gc_threads[i].owner == owner) {
            g_gc_threads[i] = g_gc_threads[g_gc_nthreads - 1];
            g_gc_nthreads--;
            break;
        }
    }
    gc_unlock();
}

void sn_release(void *p) {
    if (!p) return;
    Obj *o = (Obj *)p;
    /* SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the
       lane-backed SFC layouts never participate in ARC — the lane
       owns the bytes and reclaims them at its rewind. A stray release
       (an alias the checker's store rules did not see) is a NO-OP
       here, not a free of lane memory; the checker's E-GC-IN-LANE
       store rejections are the primary line. Wave 16 (Impl-U): the
       tape joins the no-op — the chronicle header is lane-backed
       (the hilbert precedent) and dies at the lane's rewind. */
    if (o->kind == KIND_MORTON || o->kind == KIND_HILBERT || o->kind == KIND_TAPE) return;
    o->rc--;
    if (o->rc > 0) return;
    gc_untrack(p);
    switch (o->kind) {
    case KIND_LIST: {
        List *l = (List *)p;
        /* elem_kind == 1 = owned handles only (the pointee law, Wave 12
           Impl-Q): borrow-kind (2) elements are slot/lane addresses —
           releasing them would corrupt the target local's value. */
        if (l->elem_kind == 1) {
            for (int64_t i = 0; i < l->len; i++) sn_release((void *)(intptr_t)l->items[i]);
        }
        if (!list_is_slab_backed(l)) free(l->items);
        free(l);
        break;
    }
    case KIND_MAP: {
        Map *m = (Map *)p;
        if (m->key_kind == 1) {
            for (int64_t i = 0; i < m->len; i++) sn_release((void *)(intptr_t)m->keys[i]);
        }
        if (m->val_kind == 1) {
            for (int64_t i = 0; i < m->len; i++) sn_release((void *)(intptr_t)m->vals[i]);
        }
        free(m->keys);
        free(m->vals);
        free(m->hashes);
        free(m->buckets);
        free(m);
        break;
    }
    case KIND_ZSET: {
        SnZSet *z = (SnZSet *)p;
        if (z->key_kind == 1) {
            for (int64_t i = 0; i < z->len; i++) sn_release((void *)(intptr_t)z->keys[i]);
        }
        free(z->keys);
        free(z->counts);
        free(z->hashes);
        free(z->buckets);
        free(z);
        break;
    }
    case KIND_LCELL: {
        SnLCell *c = (SnLCell *)p;
        free(c->elems);
        free(c->hashes);
        free(c->buckets);
        free(c);
        break;
    }
    case KIND_ZFIX: {
        /* The explicit-release path: drop the borrowed edge/seed refs
           and the owned carriers, then the CSR adjacency and the
           block. (The sweep path frees only the arrays — children are
           swept on their own there; here the rc discipline owns them.) */
        SnZFix *f = (SnZFix *)p;
        sn_release(f->edges);
        sn_release(f->seed);
        sn_release(f->state);
        sn_release(f->delta);
        sn_release(f->state_l);
        sn_release(f->delta_l);
        free(f->adj.nodes);
        free(f->adj.cnt);
        free(f->adj.off);
        free(f->adj.buf);
        free(f->adj.hashes);
        free(f->adj.buckets);
        free(f);
        break;
    }
    case KIND_ERR: {
        Err *e = (Err *)p;
        sn_release(e->msg);
        free(e);
        break;
    }
    case KIND_CHAN: {
        Chan *c = (Chan *)p;
        free(c->buf);
#ifdef _WIN32
        DeleteCriticalSection(&c->mu);
#else
        pthread_mutex_destroy(&c->mu);
        pthread_cond_destroy(&c->cv);
#endif
        free(c);
        break;
    }
    case KIND_LOCK: {
        Lock *l = (Lock *)p;
#ifdef _WIN32
        DeleteCriticalSection(&l->mu);
#else
        pthread_mutex_destroy(&l->mu);
#endif
        free(l);
        break;
    }
    case KIND_JSON: {
        /* tag: 0 null, 1 bool, 2 int, 3 str, 4 arr, 5 obj */
        int64_t *j = (int64_t *)p;
        int64_t tag = j[2];
        void *pval = (void *)(intptr_t)j[4];
        if (tag == 3 || tag == 4 || tag == 5) sn_release(pval);
        free(p);
        break;
    }
    case KIND_BYTEARRAY: {
        ByteArray *b = (ByteArray *)p;
        free(b->data);
        free(p);
        break;
    }
    case KIND_STRBUILDER: {
        Sb *sb = (Sb *)p;
        free(sb->buf);
        free(p);
        break;
    }
    case KIND_POOL: {
        SnPool *pl = (SnPool *)p;
        if (pl->elem_kind == 1) {
            for (uint32_t i = 0; i < pl->len; i++) {
                if ((pl->generations[i] & 1u) != 0) {
                    int64_t raw = *(int64_t *)(pl->slots + (size_t)i * (size_t)pl->elem_size);
                    sn_release((void *)(intptr_t)raw);
                }
            }
        }
        free(pl->slots);
        free(pl->generations);
        free(pl->free_next);
        free(p);
        break;
    }
    case KIND_CLOS:
        free(p);
        break;
    default:
        free(p);
        break;
    }
}

void *sn_str_new(const char *s, int64_t len) {
    Str *st = (Str *)xmalloc(sizeof(Str) + (size_t)len + 1);
    st->rc = 1;
    st->kind = KIND_STR;
    st->len = len;
    if (s && len) memcpy(st->data, s, (size_t)len);
    st->data[len] = 0;
    gc_alloc(st, (int64_t)(sizeof(Str) + (size_t)len + 1));
    return st;
}

void *sn_str_from_cstr(const char *s) {
    if (!s) s = "";
    return sn_str_new(s, (int64_t)strlen(s));
}

int64_t sn_str_len(void *p) {
    if (!p) return 0;
    return ((Str *)p)->len;
}

const char *sn_str_cstr(void *p) {
    if (!p) return "";
    return ((Str *)p)->data;
}

void *sn_str_concat(void *a, void *b) {
    int64_t la = sn_str_len(a), lb = sn_str_len(b);
    Str *st = (Str *)xmalloc(sizeof(Str) + (size_t)(la + lb) + 1);
    st->rc = 1;
    st->kind = KIND_STR;
    st->len = la + lb;
    memcpy(st->data, sn_str_cstr(a), (size_t)la);
    memcpy(st->data + la, sn_str_cstr(b), (size_t)lb);
    st->data[la + lb] = 0;
    return st;
}

int64_t sn_str_eq(void *a, void *b) {
    int64_t la = sn_str_len(a), lb = sn_str_len(b);
    if (la != lb) return 0;
    return memcmp(sn_str_cstr(a), sn_str_cstr(b), (size_t)la) == 0;
}

int64_t sn_enum_eq(void *a, void *b) {
    if (a == b) return 1;
    if (!a || !b) return 0;
    int64_t *sa = (int64_t *)a;
    int64_t *sb = (int64_t *)b;
    if (sa[2] != sb[2]) return 0;
    int idx_a = gc_index_of(a);
    int idx_b = gc_index_of(b);
    if (idx_a < 0 || idx_b < 0) return 0;
    if (g_gc_objs[idx_a].size != g_gc_objs[idx_b].size) return 0;
    int64_t nslots = g_gc_objs[idx_a].size / 8;
    for (int64_t i = 3; i < nslots; i++) {
        if (sa[i] != sb[i]) {
            int ia = gc_index_of((void *)(intptr_t)sa[i]);
            int ib = gc_index_of((void *)(intptr_t)sb[i]);
            if (ia >= 0 && ib >= 0) {
                int64_t *ha = (int64_t *)(intptr_t)sa[i];
                int64_t *hb = (int64_t *)(intptr_t)sb[i];
                if (ha[1] == KIND_STR && hb[1] == KIND_STR) {
                    if (sn_str_eq((void *)ha, (void *)hb)) continue;
                }
            }
            return 0;
        }
    }
    return 1;
}

void *sn_str_slice(void *p, int64_t start, int64_t end) {
    int64_t n = sn_str_len(p);
    if (start < 0) start = 0;
    if (end > n) end = n;
    if (end < start) end = start;
    return sn_str_new(sn_str_cstr(p) + start, end - start);
}

int64_t sn_str_contains(void *p, void *sub) {
    const char *a = sn_str_cstr(p);
    const char *b = sn_str_cstr(sub);
    return strstr(a, b) != NULL;
}

void *sn_list_new(int64_t elem_kind);
void sn_list_push_ptr(void *l, void *v);

void *sn_str_split(void *p, void *sep) {
    List *out = (List *)sn_list_new(1);
    const char *s = sn_str_cstr(p);
    const char *d = sn_str_cstr(sep);
    int64_t sl = sn_str_len(sep);
    if (sl == 0) {
        sn_list_push_ptr(out, sn_str_from_cstr(s));
        return out;
    }
    const char *cur = s;
    while (1) {
        const char *found = strstr(cur, d);
        if (!found) {
            sn_list_push_ptr(out, sn_str_from_cstr(cur));
            break;
        }
        sn_list_push_ptr(out, sn_str_new(cur, (int64_t)(found - cur)));
        cur = found + sl;
    }
    return out;
}

void *sn_str_replace(void *p, void *a, void *b) {
    const char *s = sn_str_cstr(p);
    const char *from = sn_str_cstr(a);
    const char *to = sn_str_cstr(b);
    int64_t fl = sn_str_len(a);
    if (fl == 0) return sn_str_from_cstr(s);
    size_t cap = (size_t)sn_str_len(p) + 16;
    char *buf = (char *)xmalloc(cap);
    size_t n = 0;
    const char *cur = s;
    while (*cur) {
        if (strncmp(cur, from, (size_t)fl) == 0) {
            size_t tl = strlen(to);
            if (n + tl + 1 > cap) {
                cap = (n + tl + 1) * 2;
                buf = (char *)realloc(buf, cap);
            }
            memcpy(buf + n, to, tl);
            n += tl;
            cur += fl;
        } else {
            if (n + 2 > cap) {
                cap *= 2;
                buf = (char *)realloc(buf, cap);
            }
            buf[n++] = *cur++;
        }
    }
    buf[n] = 0;
    void *r = sn_str_new(buf, (int64_t)n);
    free(buf);
    return r;
}

void *sn_str_upper(void *p) {
    int64_t n = sn_str_len(p);
    Str *st = (Str *)sn_str_new(sn_str_cstr(p), n);
    for (int64_t i = 0; i < n; i++) st->data[i] = (char)toupper((unsigned char)st->data[i]);
    return st;
}

void *sn_str_lower(void *p) {
    int64_t n = sn_str_len(p);
    Str *st = (Str *)sn_str_new(sn_str_cstr(p), n);
    for (int64_t i = 0; i < n; i++) st->data[i] = (char)tolower((unsigned char)st->data[i]);
    return st;
}

void *sn_str_from_i64(int64_t v) {
    char buf[32];
    snprintf(buf, sizeof(buf), "%" PRId64, v);
    return sn_str_from_cstr(buf);
}

void *sn_str_from_bool(int64_t v) { return sn_str_from_cstr(v ? "true" : "false"); }

void *sn_str_from_dec(int64_t scaled, int64_t scale) {
    char buf[64];
    int64_t neg = scaled < 0;
    int64_t mag = neg ? -scaled : scaled;
    int64_t pow = 1;
    for (int64_t i = 0; i < scale; i++) pow *= 10;
    int64_t ip = mag / pow;
    int64_t fp = mag % pow;
    if (scale <= 0) {
        snprintf(buf, sizeof(buf), "%s%" PRId64, neg ? "-" : "", mag);
    } else {
        snprintf(buf, sizeof(buf), "%s%" PRId64 ".%0*" PRId64, neg ? "-" : "", ip, (int)scale, fp);
    }
    return sn_str_from_cstr(buf);
}

int64_t sn_i64_from_str(void *p) { return strtoll(sn_str_cstr(p), NULL, 10); }
double sn_f64_from_str(void *p) { return strtod(sn_str_cstr(p), NULL); }
void *sn_str_from_f64(double v) {
    char buf[64];
    snprintf(buf, sizeof(buf), "%.14g", v);
    return sn_str_from_cstr(buf);
}
void sn_print_f64(double v) { printf("%.14g\n", v); }
void sn_printn_f64(double v) { printf("%.14g", v); }

void sn_print_i64(int64_t v) { printf("%" PRId64 "\n", v); }
void sn_printn_i64(int64_t v) { printf("%" PRId64, v); }
void sn_print_bool(int64_t v) { printf("%s\n", v ? "true" : "false"); }
void sn_printn_bool(int64_t v) { printf("%s", v ? "true" : "false"); }
void sn_print_str(void *p) { printf("%s\n", sn_str_cstr(p)); }
void sn_printn_str(void *p) { printf("%s", sn_str_cstr(p)); }
void sn_print_none(void) { printf("none\n"); }
void sn_printn_none(void) { printf("none"); }

/* Non-moving virtual-memory backing for List so references into a growing
   list remain valid across arbitrary .push() calls (Goose / SN-Strata invariant). */
#define SN_LIST_SLAB_BYTES (256ULL * 1024ULL * 1024ULL)
static uint8_t *g_list_slab_base = NULL;
static uint8_t *g_list_slab_top = NULL;
static uint8_t *g_list_slab_limit = NULL;

static int64_t *list_alloc_backing(size_t initial_cap) {
    /* Each list gets a 512 KB virtual-address window (65,536 elements) from the
       256 MB non-moving slab if available; OS pages are committed lazily on touch,
       so idle capacity uses 0 bytes of physical RAM and elements NEVER move on growth
       up to 65,536 elements. */
    size_t window_elems = 65536;
    size_t window_bytes = window_elems * sizeof(int64_t);
#ifndef _WIN32
    if (!g_list_slab_base) {
        uint8_t *mem = (uint8_t *)mmap(NULL, (size_t)SN_LIST_SLAB_BYTES,
                                       PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
        if (mem != MAP_FAILED) {
            g_list_slab_base = mem;
            g_list_slab_top = mem;
            g_list_slab_limit = mem + SN_LIST_SLAB_BYTES;
        }
    }
    if (g_list_slab_top && g_list_slab_top + window_bytes <= g_list_slab_limit) {
        int64_t *p = (int64_t *)g_list_slab_top;
        g_list_slab_top += window_bytes;
        return p;
    }
#endif
    return (int64_t *)xmalloc(sizeof(int64_t) * initial_cap);
}

static int list_is_slab_backed(List *l) {
    if (!g_list_slab_base || !l || !l->items) return 0;
    uint8_t *p = (uint8_t *)l->items;
    return p >= g_list_slab_base && p < g_list_slab_limit;
}

/* SWARM RABBIT HOLE — Wave 12 (Impl-Q): the lane-aware items pool —
   closing the caveat Impl-K's Wave-8 benchmark flagged
   (bench/RESULTS_PINNED_LANE.md): the 1-store lane restore reclaims the
   list HEADER (48 B lane bump memory) but did NOT reclaim the GROWN
   items arrays — each list gets one 512 KB slab window; past 65,536
   elements list_grow mallocs a bigger array and abandons the window;
   a lane-backed header has NO release path (lane bytes die at the
   rewind, never free()), so at N=100K per chain ~1 MB leaked per chain.
   W2A §1.5 E-NESTLED grown-array lifecycle: slab window under 65,536
   elements, malloc'd above, freed at the rewind when lane-backed.

   Registry of lane-backed lists: sn_list_new_lane records the header;
   sn_lane_restore walks the registry and frees the items array of every
   header inside the rewound range [watermark, old_top). Heap-backed
   lists (sn_list_new) never enter this registry — sn_release/gc_free
   own their arrays (their !list_is_slab_backed free is the twin of the
   gate in the walk below). */
#define SN_GROWN_LANES 8 /* == SN_LANE_COUNT, asserted beside g_tls_lanes */
typedef struct {
    void **hdrs; /* lane-backed list headers, in bump order */
    int64_t len, cap;
} LaneGrown;
static SN_TLS LaneGrown g_lane_grown[SN_GROWN_LANES];
static SN_TLS int64_t g_lane_items_freed;

static void lane_grown_register(void *header, int64_t lane_id) {
    /* Clamp exactly as sn_lane_ensure does, so the registry lane matches
       the lane the header actually lives on. */
    if (lane_id < 0 || lane_id >= SN_GROWN_LANES) lane_id = 0;
    LaneGrown *g = &g_lane_grown[lane_id];
    if (g->len == g->cap) {
        int64_t ncap = g->cap ? g->cap * 2 : 16;
        void **nh = (void **)realloc(g->hdrs, (size_t)ncap * sizeof(void *));
        if (!nh) {
            fprintf(stderr, "out of memory\n");
            exit(1);
        }
        g->hdrs = nh;
        g->cap = ncap;
    }
    g->hdrs[g->len++] = header;
}

/* The rewind walk: free the grown items arrays of every lane-backed
   list whose header dies at THIS rewind. RECLAIM INVARIANT: a
   lane-backed list's header dies at the rewind that covers it, and the
   SAME rewind frees its grown items array — no ref to lane memory can
   escape (E-GC-IN-LANE), so the header and its array are unreachable at
   the rewind: freeing them there is safe, correct, and free of double
   frees. Registry order is bump order (entries are appended at
   allocation), and a rewind to watermark W kills exactly the entries
   created after W — a SUFFIX of the array, popped from the end. The
   walk is therefore O(#lists freed by THIS rewind), never O(#live
   lane-backed lists): a restore of an empty range touches one entry,
   and a non-LIFO restore to a deep watermark pops its whole covering
   suffix in one pass. Slab windows are NEVER freed — they are carved
   from the shared 256 MB slab, not malloc'd (the list_is_slab_backed
   gate); only the grown arrays are malloc'd, and only those need
   freeing. Heap-backed lists (sn_list_new) never enter this registry —
   sn_release/gc_free own their arrays. */
static void lane_grown_rewind(int64_t lane_id, void *watermark, void *old_top) {
    if (lane_id < 0 || lane_id >= SN_GROWN_LANES) lane_id = 0;
    LaneGrown *g = &g_lane_grown[lane_id];
    while (g->len > 0) {
        void *h = g->hdrs[g->len - 1];
        if ((uint8_t *)h < (uint8_t *)watermark) break; /* below the range: still live */
        g->len--;
        if ((uint8_t *)h < (uint8_t *)old_top) {        /* inside [wm, top): dead */
            List *l = (List *)h;
            if (l->items && !list_is_slab_backed(l)) {
                free(l->items);
                g_lane_items_freed++;
            }
        }
    }
}

/* Test/diagnostic surface for the lane-aware items pool (Wave 12). */
int64_t sn_lane_items_freed(void) { return g_lane_items_freed; }
int64_t sn_lane_lists_live(int64_t lane_id) {
    if (lane_id < 0 || lane_id >= SN_GROWN_LANES) lane_id = 0;
    return g_lane_grown[lane_id].len;
}

void *sn_list_new(int64_t elem_kind) {
    List *l = (List *)xmalloc(sizeof(List));
    l->rc = 1;
    l->kind = KIND_LIST;
    gc_track(l, (int64_t)sizeof(List));
    l->elem_kind = elem_kind;
    l->len = 0;
    l->items = list_alloc_backing(4);
    l->cap = list_is_slab_backed(l) ? 65536 : 4;
    return l;
}

/* SWARM RABBIT HOLE — Wave 8 (Impl-I), DIFF-1 Rule 1b / W2A §1.3 + §1.7 K1:
   the pinned-lane list constructor. The checker's pre-coloring assigns an
   scc_accum accumulator's allocation site to a pinned lane π_j (L2..L6) at
   compile time (W2A §1.3 S6: "allocation sites flowing into scc_accum_params
   positions are colored onto pinned π_j"); the IR lowers the declaration's
   list literal through this twin so the growable's header lives on the
   pinned lane for its whole SCC lifetime. K1: nothing else opens on π_j —
   no scratch scope exit can truncate the accumulator while its SCC is live,
   so the E0802 buried-top hazard is unrepresentable. The header is Stratum-1
   lane bump memory — untracked, never gc_track'd (a lane-interior pointer
   must never reach the collector's free list); it dies with the lane's own
   reset, not free(). The element backing array stays malloc/slab-backed
   (W2A §1.5 E-NESTED: element storage is pool-backed by design). */
void *sn_list_new_lane(int64_t elem_kind, int64_t lane_id) {
    /* Defined in the lane section below (beside sn_lane_save/restore); the
       forward declaration mirrors sn_lane_ensure's at the nursery section. */
    void *sn_lane_alloc(int64_t lane_id, int64_t bytes);
    List *l = (List *)sn_lane_alloc(lane_id, (int64_t)sizeof(List));
    l->rc = 1;
    l->kind = KIND_LIST;
    l->elem_kind = elem_kind;
    l->len = 0;
    l->items = list_alloc_backing(4);
    l->cap = list_is_slab_backed(l) ? 65536 : 4;
    /* Wave 12 (Impl-Q): the header dies at the lane's rewind — register
       it so the rewind can free the grown items array (the lane-aware
       items pool; the reclaim invariant above lane_grown_register). */
    lane_grown_register(l, lane_id);
    return l;
}

static void list_grow(List *l) {
    if (l->len < l->cap) return;
    int64_t old_cap = l->cap;
    l->cap *= 2;
    if (list_is_slab_backed(l)) {
        int64_t *ni = (int64_t *)xmalloc(sizeof(int64_t) * (size_t)l->cap);
        memcpy(ni, l->items, sizeof(int64_t) * (size_t)old_cap);
        l->items = ni;
    } else {
        l->items = (int64_t *)realloc(l->items, sizeof(int64_t) * (size_t)l->cap);
    }
}

void sn_list_push_i64(void *p, int64_t v) {
    List *l = (List *)p;
    list_grow(l);
    l->items[l->len++] = v;
}

void sn_list_push_ptr(void *p, void *v) {
    List *l = (List *)p;
    list_grow(l);
    /* SWARM RABBIT HOLE — Wave 12 (Impl-Q): the pointee retention law,
       closing Impl-J's Wave-8 flag (escape_ref_outer_ok deferred the
       runtime retain to this tier). elem_kind 1 = owned handles: ARC as
       before (list<str>/list<Point>/json elements — the legitimate use).
       elem_kind 2 (and the pre-split 0 of an empty typed literal) =
       borrows: the element is a ref<T>/mut<T>/raw-ptr SLOT or lane
       address, not an object header — retain the POINTEE object (for a
       scalar target there is none), never the slot. The old blind
       sn_retain(v) treated the slot as an Obj header and incremented
       the target local's value: list<ref<int>> of locals was broken
       (x=99 became 100 at the push, and sn_release's element loop
       decremented it back at list death). The borrow discipline owns
       the target's lifetime (E-ESCAPE's region poset proves the target
       outlives the list), so borrows retain nothing. */
    if (l->elem_kind == 1) sn_retain(v);
    l->items[l->len++] = (int64_t)(intptr_t)v;
}

int64_t sn_list_len(void *p) { return p ? ((List *)p)->len : 0; }

static void bounds(List *l, int64_t i) {
    if (!l || i < 0 || i >= l->len) {
        fprintf(stderr, "list index %" PRId64 " out of bounds (len=%" PRId64 ")\n", i, l ? l->len : 0);
        exit(1);
    }
}

int64_t sn_list_get_i64(void *p, int64_t i) {
    List *l = (List *)p;
    bounds(l, i);
    return l->items[i];
}

/* Raw backing store, for APIs that need to read a list as a C array. */
int64_t *sn_list_data(void *p) {
    if (!p) return NULL;
    return ((List *)p)->items;
}

void *sn_list_get_ptr(void *p, int64_t i) {
    List *l = (List *)p;
    bounds(l, i);
    return (void *)(intptr_t)l->items[i];
}

void sn_list_set_i64(void *p, int64_t i, int64_t v) {
    List *l = (List *)p;
    bounds(l, i);
    l->items[i] = v;
}

void sn_list_set_ptr(void *p, int64_t i, void *v) {
    List *l = (List *)p;
    bounds(l, i);
    /* The pointee law's release half (Wave 12, Impl-Q): only owned-kind
       (elem_kind 1) lists ARC their elements — the symmetric fix of the
       push-side retain. Borrow-kind elements are slot/lane addresses;
       releasing them would decrement the target local's value (and, at
       small values, fall through into the kind switch on bytes past the
       slot). sn_release's KIND_LIST arm and sn_list_clear already gate
       on elem_kind == 1. */
    if (l->elem_kind == 1) {
        sn_release((void *)(intptr_t)l->items[i]);
        sn_retain(v);
    }
    l->items[i] = (int64_t)(intptr_t)v;
}

void *sn_list_elem_addr(void *p, int64_t i) {
    List *l = (List *)p;
    bounds(l, i);
    return (void *)&l->items[i];
}

int64_t sn_list_pop_i64(void *p) {
    List *l = (List *)p;
    if (!l || l->len <= 0) {
        fprintf(stderr, "runtime error: pop from empty list\n");
        exit(1);
    }
    l->len--;
    return l->items[l->len];
}

void *sn_list_pop_ptr(void *p) {
    List *l = (List *)p;
    if (!l || l->len <= 0) {
        fprintf(stderr, "runtime error: pop from empty list\n");
        exit(1);
    }
    l->len--;
    return (void *)(intptr_t)l->items[l->len];
}

void sn_list_clear(void *p) {
    if (!p) return;
    List *l = (List *)p;
    if (l->elem_kind == 1) {
        for (int64_t i = 0; i < l->len; i++) {
            sn_release((void *)(intptr_t)l->items[i]);
        }
    }
    l->len = 0;
}

/* Kind-parameterized hash/equality shared by Map and SnZSet (T1.1 O(1)
   machinery; the Z-set slice reuses it verbatim). */
static uint64_t key_hash(int64_t key_kind, int64_t key) {
    uint64_t h;
    if (key_kind == 1 && key != 0) {
        Str *s = (Str *)(intptr_t)key;
        h = 14695981039346656037ULL;
        for (int64_t i = 0; i < s->len; i++) {
            h ^= (uint8_t)s->data[i];
            h *= 1099511628211ULL;
        }
    } else {
        h = (uint64_t)key;
        h = (h ^ (h >> 30)) * 0xbf58476d1ce4e5b9ULL;
        h = (h ^ (h >> 27)) * 0x94d049bb133111ebULL;
        h = h ^ (h >> 31);
    }
    return h | 1ULL;
}

static int key_equal(int64_t key_kind, int64_t a, int64_t b) {
    if (key_kind == 1) return (int)sn_str_eq((void *)(intptr_t)a, (void *)(intptr_t)b);
    return a == b;
}

static uint64_t map_hash_key(Map *m, int64_t key) {
    return key_hash(m->key_kind, key);
}

static int key_eq(Map *m, int64_t a, int64_t b) {
    return key_equal(m->key_kind, a, b);
}

static void map_rehash(Map *m, int64_t new_hcap) {
    int64_t *nb = (int64_t *)calloc((size_t)new_hcap, sizeof(int64_t));
    if (!nb) {
        fprintf(stderr, "out of memory\n");
        exit(1);
    }
    uint64_t mask = (uint64_t)new_hcap - 1;
    for (int64_t i = 0; i < m->len; i++) {
        uint64_t slot = m->hashes[i] & mask;
        while (nb[slot] != 0) {
            slot = (slot + 1) & mask;
        }
        nb[slot] = i + 1;
    }
    free(m->buckets);
    m->buckets = nb;
    m->hcap = new_hcap;
}

static int64_t map_find_index(Map *m, int64_t key, uint64_t h, uint64_t *out_bucket) {
    uint64_t mask = (uint64_t)m->hcap - 1;
    uint64_t slot = h & mask;
    for (;;) {
        int64_t b = m->buckets[slot];
        if (b == 0) {
            if (out_bucket) *out_bucket = slot;
            return -1;
        }
        int64_t idx = b - 1;
        if (m->hashes[idx] == h && key_eq(m, m->keys[idx], key)) {
            if (out_bucket) *out_bucket = slot;
            return idx;
        }
        slot = (slot + 1) & mask;
    }
}

void *sn_map_new(int64_t key_kind, int64_t val_kind) {
    Map *m = (Map *)xmalloc(sizeof(Map));
    m->rc = 1;
    m->kind = KIND_MAP;
    gc_track(m, (int64_t)sizeof(Map));
    m->key_kind = key_kind;
    m->val_kind = val_kind;
    m->len = 0;
    m->cap = 8;
    m->hcap = 16;
    m->keys = (int64_t *)xmalloc(sizeof(int64_t) * 8);
    m->vals = (int64_t *)xmalloc(sizeof(int64_t) * 8);
    m->hashes = (uint64_t *)xmalloc(sizeof(uint64_t) * 8);
    m->buckets = (int64_t *)calloc(16, sizeof(int64_t));
    return m;
}

void sn_map_set(void *p, int64_t key, int64_t val) {
    if (!p) return;
    Map *m = (Map *)p;
    if ((m->len + 1) * 4 >= m->hcap * 3) {
        map_rehash(m, m->hcap * 2);
    }
    uint64_t h = map_hash_key(m, key);
    uint64_t bucket = 0;
    int64_t idx = map_find_index(m, key, h, &bucket);
    if (idx >= 0) {
        if (m->val_kind == 1) {
            sn_release((void *)(intptr_t)m->vals[idx]);
            sn_retain((void *)(intptr_t)val);
        }
        m->vals[idx] = val;
        return;
    }
    if (m->len == m->cap) {
        m->cap *= 2;
        m->keys = (int64_t *)realloc(m->keys, sizeof(int64_t) * (size_t)m->cap);
        m->vals = (int64_t *)realloc(m->vals, sizeof(int64_t) * (size_t)m->cap);
        m->hashes = (uint64_t *)realloc(m->hashes, sizeof(uint64_t) * (size_t)m->cap);
    }
    if (m->key_kind == 1) sn_retain((void *)(intptr_t)key);
    if (m->val_kind == 1) sn_retain((void *)(intptr_t)val);
    int64_t new_idx = m->len;
    m->keys[new_idx] = key;
    m->vals[new_idx] = val;
    m->hashes[new_idx] = h;
    m->buckets[bucket] = new_idx + 1;
    m->len++;
}

int64_t sn_map_has(void *p, int64_t key) {
    if (!p) return 0;
    Map *m = (Map *)p;
    if (m->len == 0) return 0;
    uint64_t h = map_hash_key(m, key);
    return map_find_index(m, key, h, NULL) >= 0 ? 1 : 0;
}

int64_t sn_map_get(void *p, int64_t key) {
    if (!p) return 0;
    Map *m = (Map *)p;
    if (m->len == 0) return 0;
    uint64_t h = map_hash_key(m, key);
    int64_t idx = map_find_index(m, key, h, NULL);
    return idx >= 0 ? m->vals[idx] : 0;
}

int64_t sn_map_len(void *p) { return p ? ((Map *)p)->len : 0; }

void *sn_map_keys(void *p) {
    Map *m = (Map *)p;
    List *l = (List *)sn_list_new(m->key_kind);
    for (int64_t i = 0; i < m->len; i++) {
        if (m->key_kind == 1) sn_list_push_ptr(l, (void *)(intptr_t)m->keys[i]);
        else sn_list_push_i64(l, m->keys[i]);
    }
    return l;
}

void *sn_map_values(void *p) {
    Map *m = (Map *)p;
    List *l = (List *)sn_list_new(m->val_kind);
    for (int64_t i = 0; i < m->len; i++) {
        if (m->val_kind == 1) sn_list_push_ptr(l, (void *)(intptr_t)m->vals[i]);
        else sn_list_push_i64(l, m->vals[i]);
    }
    return l;
}

/* --------------------------------------------------------------------- */
/*  SnZSet — the Tier-G Z-set engine (W2C §1.1: G[X] = direct sum with   */
/*  finite support; negative counts = retraction; zero counts pruned).   */
/* --------------------------------------------------------------------- */
static void zset_rehash(SnZSet *z, int64_t new_hcap) {
    int64_t *nb = (int64_t *)calloc((size_t)new_hcap, sizeof(int64_t));
    if (!nb) {
        fprintf(stderr, "out of memory\n");
        exit(1);
    }
    uint64_t mask = (uint64_t)new_hcap - 1;
    for (int64_t i = 0; i < z->len; i++) {
        uint64_t slot = z->hashes[i] & mask;
        while (nb[slot] != 0) {
            slot = (slot + 1) & mask;
        }
        nb[slot] = i + 1;
    }
    free(z->buckets);
    z->buckets = nb;
    z->hcap = new_hcap;
}

static int64_t zset_find_index(SnZSet *z, int64_t key, uint64_t h, uint64_t *out_bucket) {
    uint64_t mask = (uint64_t)z->hcap - 1;
    uint64_t slot = h & mask;
    for (;;) {
        int64_t b = z->buckets[slot];
        if (b == 0) {
            if (out_bucket) *out_bucket = slot;
            return -1;
        }
        int64_t idx = b - 1;
        if (z->hashes[idx] == h && key_equal(z->key_kind, z->keys[idx], key)) {
            if (out_bucket) *out_bucket = slot;
            return idx;
        }
        slot = (slot + 1) & mask;
    }
}

/* Swap-with-last removal: exactly one bucket holds the last entry's
   index; repoint it at the moved slot. Counts as freed never enter this
   path (the caller prunes only zero coefficients).

   SWARM RABBIT HOLE (Impl-H, Wave 7 — fix to the concurrently landed
   SnZSet, required by the view engine): the original removal left the
   freed entry's bucket pointing at the dead slot, whose stale key/hash
   bytes still matched — zset_find_index could then "find" a zombie
   entry (writes landed in a dead slot, len desynced from the entry
   array, total corrupted, and a follow-up removal could index
   hashes[-1]). The view engine's pending changelog hits this whenever
   one batch touches the same output key repeatedly across a
   zero-crossing. Repair: clear the freed entry's bucket, then run
   linear-probing backward-shift (Algorithm R) over the bucket
   pointers — pull back every entry whose probe chain ran through the
   hole so no chain crosses a 0 slot. No tombstones, no stale
   pointers; every dead array slot is unreferenced. */
static void zset_remove_at(SnZSet *z, int64_t idx) {
    uint64_t mask = (uint64_t)z->hcap - 1;
    /* the freed entry's own bucket slot */
    uint64_t s = z->hashes[idx] & mask;
    while (z->buckets[s] != idx + 1) {
        s = (s + 1) & mask;
    }
    z->buckets[s] = 0;
    /* backward-shift repair (Algorithm R, on the bucket pointers): an
       entry at j may fill the hole at i iff its probe chain home→j ran
       through i, i.e. (j−home) mod m > (i−home) mod m; the scan j
       always advances, the hole i advances only when filled. */
    {
        uint64_t i = s; /* the hole */
        uint64_t j = s; /* the scan  */
        for (;;) {
            j = (j + 1) & mask;
            if (z->buckets[j] == 0) break;
            int64_t e = z->buckets[j] - 1;
            uint64_t home = z->hashes[e] & mask;
            if (((j - home) & mask) > ((i - home) & mask)) {
                z->buckets[i] = e + 1; /* pulled back into the hole */
                z->buckets[j] = 0;
                i = j; /* the vacated slot is the new hole */
            }
        }
    }
    int64_t last = z->len - 1;
    if (idx != last) {
        uint64_t slot = z->hashes[last] & mask;
        while (z->buckets[slot] != last + 1) {
            slot = (slot + 1) & mask;
        }
        z->keys[idx] = z->keys[last];
        z->counts[idx] = z->counts[last];
        z->hashes[idx] = z->hashes[last];
        z->buckets[slot] = idx + 1;
    }
    z->len--;
}

void *sn_zset_new(int64_t key_kind) {
    SnZSet *z = (SnZSet *)xmalloc(sizeof(SnZSet));
    z->rc = 1;
    z->kind = KIND_ZSET;
    gc_track(z, (int64_t)sizeof(SnZSet));
    z->key_kind = key_kind;
    z->prune = 1;
    z->len = 0;
    z->cap = 8;
    z->hcap = 16;
    z->keys = (int64_t *)xmalloc(sizeof(int64_t) * 8);
    z->counts = (int64_t *)xmalloc(sizeof(int64_t) * 8);
    z->hashes = (uint64_t *)xmalloc(sizeof(uint64_t) * 8);
    z->buckets = (int64_t *)calloc(16, sizeof(int64_t));
    z->total = 0;
    return z;
}

void sn_zset_set_prune(void *p, int64_t flag) {
    if (!p) return;
    SnZSet *z = (SnZSet *)p;
    z->prune = flag ? 1 : 0;
}

/* The ⊕ primitive: count[key] += delta. A zeroed coefficient leaves the
   support when pruning is on (the canonical finite-support form). */
void sn_zset_add(void *p, int64_t key, int64_t delta) {
    if (!p || delta == 0) return;
    SnZSet *z = (SnZSet *)p;
    if ((z->len + 1) * 4 >= z->hcap * 3) {
        zset_rehash(z, z->hcap * 2);
    }
    uint64_t h = key_hash(z->key_kind, key);
    uint64_t bucket = 0;
    int64_t idx = zset_find_index(z, key, h, &bucket);
    if (idx >= 0) {
        z->counts[idx] += delta;
        z->total += delta;
        if (z->counts[idx] == 0 && z->prune) {
            if (z->key_kind == 1) sn_release((void *)(intptr_t)z->keys[idx]);
            zset_remove_at(z, idx);
        }
        return;
    }
    if (z->len == z->cap) {
        z->cap *= 2;
        z->keys = (int64_t *)realloc(z->keys, sizeof(int64_t) * (size_t)z->cap);
        z->counts = (int64_t *)realloc(z->counts, sizeof(int64_t) * (size_t)z->cap);
        z->hashes = (uint64_t *)realloc(z->hashes, sizeof(uint64_t) * (size_t)z->cap);
    }
    if (z->key_kind == 1) sn_retain((void *)(intptr_t)key);
    z->keys[z->len] = key;
    z->counts[z->len] = delta;
    z->hashes[z->len] = h;
    z->buckets[bucket] = z->len + 1;
    z->len++;
    z->total += delta;
}

/* EXACT retraction — the Tier-G guarantee (W2C §1.1: deletion = −[x];
   the group inverse makes retraction first-class, O(1)/key). */
void sn_zset_remove(void *p, int64_t key, int64_t delta) {
    sn_zset_add(p, key, -delta);
}

/* Pointwise add of src into dst (the ⊕ of two Z-sets). */
void sn_zset_merge(void *dst, void *src) {
    if (!dst || !src) return;
    /* E-LKIND, G side of the tier-confusion seam (SWARM RABBIT HOLE,
       Wave 9, Impl-M — mirroring E-GC-IN-LANE's two-direction rule): a
       Tier-L SnLCell (or any other object kind) handed to the Tier-G ⊕
       would be type-confused into this SnZSet layout and corrupt
       memory. The surface types are untyped ptrs, so the CHECKER
       cannot see the carrier — the runtime is the only layer that can
       enforce the boundary, fail-fast per the runtime convention. */
    if (((Obj *)dst)->kind != KIND_ZSET || ((Obj *)src)->kind != KIND_ZSET) {
        fprintf(stderr,
                "runtime error: zset_merge expects Tier-G Z-sets (kind=%d), "
                "got kind=%lld and kind=%lld — Tier-G ⊕ and Tier-L ⊔ do not "
                "mix; use matching tiers [E-LKIND]\n",
                KIND_ZSET, (long long)((Obj *)dst)->kind,
                (long long)((Obj *)src)->kind);
        exit(1);
    }
    SnZSet *s = (SnZSet *)src;
    for (int64_t i = 0; i < s->len; i++) {
        sn_zset_add(dst, s->keys[i], s->counts[i]);
    }
}

int64_t sn_zset_count(void *p, int64_t key) {
    if (!p) return 0;
    SnZSet *z = (SnZSet *)p;
    if (z->len == 0) return 0;
    uint64_t h = key_hash(z->key_kind, key);
    int64_t idx = zset_find_index(z, key, h, NULL);
    return idx >= 0 ? z->counts[idx] : 0;
}

int64_t sn_zset_total(void *p) { return p ? ((SnZSet *)p)->total : 0; }
int64_t sn_zset_len(void *p) { return p ? ((SnZSet *)p)->len : 0; }

void *sn_zset_copy(void *p) {
    if (!p) return NULL;
    SnZSet *z = (SnZSet *)p;
    SnZSet *out = (SnZSet *)sn_zset_new(z->key_kind);
    out->prune = z->prune;
    for (int64_t i = 0; i < z->len; i++) {
        sn_zset_add(out, z->keys[i], z->counts[i]);
    }
    return out;
}

/* The per-timestamp 𝒟: dst − prev as a NEW Z-set (pure; both operands
   untouched). Canonical: only changed keys survive (zero differences are
   pruned), so len(δ) = the size of the change, not of the state. */
void *sn_zset_delta(void *dst, void *prev) {
    SnZSet *d = (SnZSet *)dst;
    SnZSet *pv = (SnZSet *)prev;
    SnZSet *out = (SnZSet *)sn_zset_new(d ? d->key_kind : (pv ? pv->key_kind : 0));
    if (d) {
        for (int64_t i = 0; i < d->len; i++) {
            sn_zset_add(out, d->keys[i], d->counts[i]);
        }
    }
    if (pv) {
        for (int64_t i = 0; i < pv->len; i++) {
            sn_zset_add(out, pv->keys[i], -pv->counts[i]);
        }
    }
    return out;
}

/* The ⊕ composition: base + delta as a NEW Z-set (pure). */
void *sn_zset_apply(void *base, void *delta) {
    SnZSet *b = (SnZSet *)base;
    SnZSet *out = (SnZSet *)sn_zset_copy(b);
    sn_zset_merge(out, delta);
    return out;
}

void *sn_zset_keys(void *p) {
    SnZSet *z = (SnZSet *)p;
    List *l = (List *)sn_list_new(z ? z->key_kind : 0);
    if (!z) return l;
    for (int64_t i = 0; i < z->len; i++) {
        if (z->key_kind == 1) sn_list_push_ptr(l, (void *)(intptr_t)z->keys[i]);
        else sn_list_push_i64(l, z->keys[i]);
    }
    return l;
}

/* --------------------------------------------------------------------- */
/*  SnLCell — the Tier-L join-semilattice engine (Wave 9, Impl-M;        */
/*  see the struct comment above for the carrier-by-carrier lattice      */
/*  proofs). NO ⊖ exists anywhere below: inflation and ⊔ only. Exact     */
/*  retraction is the checked compile-time rejection E-LRET (the         */
/*  Grothendieck argument: x⊔x = x ⇒ [x] = 0 — the idempotent merge      */
/*  operator does not group-embed); the runtime never needs to police    */
/*  it because no lowering of a retract exists.                          */
/*                                                                       */
/*  E-LKIND (tier confusion, runtime-enforced): the surface type of a   */
/*  cell is an untyped ptr, so the CHECKER cannot see the carrier —      */
/*  the runtime fail-fasts (fprintf + exit, the runtime.c convention)    */
/*  on kind mismatch (an L op on a non-LCell, and zset_merge's G-side    */
/*  twin above) and on carrier mismatch between two operands.           */
/* --------------------------------------------------------------------- */

void *sn_lcell_new(int64_t carrier); /* forward: lcell_clone builds on it */

static const char *lcell_carrier_name(int64_t carrier) {
    switch (carrier) {
    case LCELL_MAX: return "max";
    case LCELL_GSET: return "gset";
    case LCELL_LWW: return "lww";
    default: return "unknown";
    }
}

/* One operand must be a live LCell. */
static SnLCell *lcell_guard_one(void *p, const char *op) {
    if (!p) return NULL;
    Obj *o = (Obj *)p;
    if (o->kind != KIND_LCELL) {
        fprintf(stderr,
                "runtime error: lcell_%s expects a Tier-L cell (kind=%d), "
                "got kind=%lld — Tier-L ⊔ is only defined on Tier-L cells; "
                "use the zset_* surface for Tier-G objects [E-LKIND]\n",
                op, KIND_LCELL, (long long)o->kind);
        exit(1);
    }
    return (SnLCell *)p;
}

/* Two operands must be live LCells of the SAME carrier — the ⊔ of
   different carriers is undefined (E-LKIND). */
static void lcell_guard_pair(void *a, void *b, const char *op) {
    SnLCell *ca = lcell_guard_one(a, op);
    SnLCell *cb = lcell_guard_one(b, op);
    if (ca && cb && ca->carrier != cb->carrier) {
        fprintf(stderr,
                "runtime error: cannot %s a %s cell with a %s cell — "
                "the ⊔ of different carriers is undefined [E-LKIND]\n",
                op, lcell_carrier_name(ca->carrier),
                lcell_carrier_name(cb->carrier));
        exit(1);
    }
}

/* SN_LATTICE_CHECK=1 (the SN_POISON env convention): per-merge ⊔ law
   self-checks — see sn_lcell_merge. Resolved once, benign race as
   documented at sn_lane_poison_on. */
static int g_lattice_check_state = 0; /* 0 unresolved, 1 on, -1 off */

static int sn_lattice_check_on(void) {
    int s = g_lattice_check_state;
    if (s != 0) return s > 0;
    const char *e = getenv("SN_LATTICE_CHECK");
    s = (e && e[0] == '1' && e[1] == '\0') ? 1 : -1;
    g_lattice_check_state = s;
    return s > 0;
}

/* GSET insert — the SnZSet O(1) find-or-insert machinery, presence
   only (idempotent by construction: an absent element enters the
   support, a present one is a no-op — the union's idempotence law
   executed). */
static void lcell_gset_rehash(SnLCell *c, int64_t new_hcap) {
    int64_t *nb = (int64_t *)calloc((size_t)new_hcap, sizeof(int64_t));
    if (!nb) {
        fprintf(stderr, "out of memory\n");
        exit(1);
    }
    uint64_t mask = (uint64_t)new_hcap - 1;
    for (int64_t i = 0; i < c->len; i++) {
        uint64_t slot = c->hashes[i] & mask;
        while (nb[slot] != 0) {
            slot = (slot + 1) & mask;
        }
        nb[slot] = i + 1;
    }
    free(c->buckets);
    c->buckets = nb;
    c->hcap = new_hcap;
}

static int64_t lcell_gset_find(SnLCell *c, int64_t e, uint64_t h, uint64_t *out_bucket) {
    /* SWARM RABBIT HOLE — Wave 16 (Impl-U, Cohort Delta), the
       flagged runtime fix: an UNALLOCATED table (hcap == 0 — the
       lazy arrays of an empty cell) contains nothing. The find
       previously computed mask = hcap−1 = 0xFFFF… and read
       buckets[slot] at the full hash — a latent OOB that only fires
       when a caller probes an empty-but-live cell, exactly what the
       tape's ⊥-rooted delta chains do (the first fragment is
       against the empty base; the mutator-law round's first record
       exposed it). The insert path never reaches here with
       hcap == 0 (lcell_gset_insert allocates first); every other
       caller asks a membership question whose honest answer on an
       empty table is "absent". */
    if (c->hcap == 0) {
        if (out_bucket) *out_bucket = 0;
        return -1;
    }
    uint64_t mask = (uint64_t)c->hcap - 1;
    uint64_t slot = h & mask;
    for (;;) {
        int64_t b = c->buckets[slot];
        if (b == 0) {
            if (out_bucket) *out_bucket = slot;
            return -1;
        }
        int64_t idx = b - 1;
        if (c->hashes[idx] == h && c->elems[idx] == e) {
            if (out_bucket) *out_bucket = slot;
            return idx;
        }
        slot = (slot + 1) & mask;
    }
}

static void lcell_gset_insert(SnLCell *c, int64_t e) {
    uint64_t h = key_hash(0, e); /* key_kind 0: the i64 mix, Map's convention */
    if (c->hcap == 0) {
        /* lazy arrays: MAX/LWW cells never pay for GSET storage */
        c->cap = 8;
        c->hcap = 16;
        c->elems = (int64_t *)xmalloc(sizeof(int64_t) * 8);
        c->hashes = (uint64_t *)xmalloc(sizeof(uint64_t) * 8);
        c->buckets = (int64_t *)calloc(16, sizeof(int64_t));
    } else if ((c->len + 1) * 4 >= c->hcap * 3) {
        lcell_gset_rehash(c, c->hcap * 2);
    }
    uint64_t bucket = 0;
    if (lcell_gset_find(c, e, h, &bucket) >= 0) return; /* present: no-op */
    if (c->len == c->cap) {
        c->cap *= 2;
        c->elems = (int64_t *)realloc(c->elems, sizeof(int64_t) * (size_t)c->cap);
        c->hashes = (uint64_t *)realloc(c->hashes, sizeof(uint64_t) * (size_t)c->cap);
    }
    c->elems[c->len] = e;
    c->hashes[c->len] = h;
    c->buckets[bucket] = c->len + 1;
    c->len++;
}

/* The ⊔ join core: src joins into dst, per carrier. This is the whole
   Tier-L merge semantics — three carrier laws, one entry point. */
static void lcell_join(SnLCell *dst, SnLCell *src) {
    switch (dst->carrier) {
    case LCELL_MAX:
        /* ⊔ = max: the higher HWM wins; equal or lower is a no-op
           (idempotence executed: x⊔x = x). */
        if (src->hwm > dst->hwm) dst->hwm = src->hwm;
        break;
    case LCELL_GSET:
        /* ⊔ = union: every src element joins dst's support; shared
           elements are no-ops (A∪A = A executed per element). */
        for (int64_t i = 0; i < src->len; i++) {
            lcell_gset_insert(dst, src->elems[i]);
        }
        break;
    case LCELL_LWW: {
        /* ⊔ = the lexicographic max of (clock, value): the higher
           clock wins; on a clock tie the DETERMINISTIC value tiebreak
           (higher value wins) keeps the join commutative. A losing
           offer is a no-op — the winner never depends on merge order. */
        if (!src->lww_set) break;
        if (!dst->lww_set
            || src->lww_clock > dst->lww_clock
            || (src->lww_clock == dst->lww_clock && src->lww_val > dst->lww_val)) {
            dst->lww_set = 1;
            dst->lww_val = src->lww_val;
            dst->lww_clock = src->lww_clock;
        }
        break;
    }
    }
}

/* Structural state equality per carrier — the read-side primitive the
   law checks (and the systems harness) compare with. Same carrier and
   identical state, not object identity. */
int64_t sn_lcell_eq(void *a, void *b) {
    lcell_guard_pair(a, b, "compare");
    SnLCell *x = (SnLCell *)a;
    SnLCell *y = (SnLCell *)b;
    if (!x || !y) return x == y; /* NULL only equals NULL */
    if (x->carrier != y->carrier) return 0;
    switch (x->carrier) {
    case LCELL_MAX:
        return x->hwm == y->hwm;
    case LCELL_GSET: {
        if (x->len != y->len) return 0;
        for (int64_t i = 0; i < x->len; i++) {
            if (lcell_gset_find(y, x->elems[i], key_hash(0, x->elems[i]), NULL) < 0) {
                return 0;
            }
        }
        return 1;
    }
    case LCELL_LWW:
        return x->lww_set == y->lww_set && x->lww_val == y->lww_val
            && x->lww_clock == y->lww_clock;
    }
    return 0;
}

/* A deep copy (a fresh cell joined to x's state) — the law checks and
   the pure ops build on it. */
static SnLCell *lcell_clone(SnLCell *x) {
    SnLCell *out = (SnLCell *)sn_lcell_new(x->carrier);
    lcell_join(out, x);
    return out;
}

void *sn_lcell_new(int64_t carrier) {
    if (carrier < LCELL_MAX || carrier > LCELL_LWW) {
        fprintf(stderr,
                "runtime error: unknown Tier-L carrier %lld (expected 0=max, "
                "1=gset, 2=lww) [E-LKIND]\n",
                (long long)carrier);
        exit(1);
    }
    SnLCell *c = (SnLCell *)xmalloc(sizeof(SnLCell));
    c->rc = 1;
    c->kind = KIND_LCELL;
    gc_track(c, (int64_t)sizeof(SnLCell));
    c->carrier = carrier;
    c->hwm = INT64_MIN; /* ⊥ of max: max(x, ⊥) = x */
    c->lww_set = 0;     /* ⊥ of lww: no offer yet */
    c->lww_val = 0;
    c->lww_clock = 0;
    c->len = 0;
    c->cap = 0;         /* GSET arrays are lazy (see lcell_gset_insert) */
    c->hcap = 0;
    c->elems = NULL;
    c->hashes = NULL;
    c->buckets = NULL;
    return c;
}

/* The monotone inflation — the ONLY mutator on the Tier-L surface
   (the LVar/CRDT vocabulary: values only INFLATE, never decrease).
   Uniform 3-arg shape:
     MAX:  inflate(c, candidate, _) — the HWM rises to max(hwm, candidate)
     GSET: inflate(c, element, _)  — the element joins the support (no-op
                                     if present: idempotent insert)
     LWW:  inflate(c, value, clock) — the offer (value@clock) joins; it
                                     wins iff it beats the current winner
                                     under the (clock, value) total order
   A losing offer is a no-op on every carrier — the join can only grow. */
void sn_lcell_inflate(void *p, int64_t a, int64_t b) {
    SnLCell *c = lcell_guard_one(p, "inflate");
    if (!c) return;
    switch (c->carrier) {
    case LCELL_MAX:
        if (a > c->hwm) c->hwm = a;
        break;
    case LCELL_GSET:
        lcell_gset_insert(c, a);
        break;
    case LCELL_LWW: {
        /* A stack-shaped offer joined through the SAME accept test the
           merge uses — inflate and merge share one ⊔ law by
           construction. lcell_join's LWW arm reads only the lww_* trio
           (the memset keeps the rest honest anyway). */
        SnLCell offer;
        memset(&offer, 0, sizeof(offer));
        offer.kind = KIND_LCELL;
        offer.carrier = LCELL_LWW;
        offer.hwm = INT64_MIN;
        offer.lww_set = 1;
        offer.lww_val = a;
        offer.lww_clock = b;
        lcell_join(c, &offer);
        break;
    }
    }
}

/* The ⊔ join: dst = dst ⊔ src (mutating, the zset_merge shape). */
void sn_lcell_merge(void *dst, void *src) {
    if (!dst || !src) return;
    lcell_guard_pair(dst, src, "merge");
    SnLCell *d = (SnLCell *)dst;
    SnLCell *s = (SnLCell *)src;
    if (sn_lattice_check_on()) {
        /* Per-merge ⊔ law self-checks (env-gated; O(state) each, so
           off by default — the exhaustive randomized harness over
           seeded op sequences is tests/systems/test_lcell.c):
             idempotence   x ⊔ x == x, x = dst as it stands
             commutativity a ⊔ b == b ⊔ a, this exact pair
           Associativity is a three-operand law — a per-merge check
           would need a synthetic third operand the caller never
           joined, so it lives in the systems harness, which asserts
           (a⊔b)⊔c == a⊔(b⊔c) over randomized triples directly. */
        SnLCell *xc = lcell_clone(d);
        lcell_join(xc, d); /* xc = d ⊔ d */
        if (!sn_lcell_eq(xc, d)) {
            fprintf(stderr, "runtime error: lattice law violated: x⊔x != x (idempotence)\n");
            exit(1);
        }
        sn_release(xc);
        SnLCell *ab = lcell_clone(d);
        lcell_join(ab, s); /* ab = a ⊔ b */
        SnLCell *ba = lcell_clone(s);
        lcell_join(ba, d); /* ba = b ⊔ a */
        if (!sn_lcell_eq(ab, ba)) {
            fprintf(stderr, "runtime error: lattice law violated: a⊔b != b⊔a (commutativity)\n");
            exit(1);
        }
        sn_release(ab);
        sn_release(ba);
    }
    lcell_join(d, s);
}

/* The mutator-law delta m^δ (Almeida–Shoker–Baquero JPDC 2018; Enes
   et al. 2018): a NEW cell holding the join-decomposition-optimal
   fragment such that prev ⊔ m^δ(dst, prev) == dst — the elements that
   actually moved, NOT a copy of dst. The law holds under the monotone
   regime prev ⊑ dst (the LVar discipline — Tier-L states only
   inflate, so this is the only regime the mutator law is claimed
   for); outside it the fragment is still the honest "what sits above
   prev", and prev ⊔ fragment = prev ⊔ dst.
     MAX:  the new HWM if it rose, else ⊥ (max is join-irreducible)
     GSET: the elements in dst not in prev
     LWW:  the winning offer, iff the winner changed, else ⊥       */
void *sn_lcell_delta(void *dst, void *prev) {
    lcell_guard_pair(dst, prev, "delta");
    SnLCell *d = (SnLCell *)dst;
    SnLCell *pv = (SnLCell *)prev;
    int64_t carrier = d ? d->carrier : (pv ? pv->carrier : LCELL_MAX);
    SnLCell *out = (SnLCell *)sn_lcell_new(carrier);
    if (!d) return out; /* nothing moved */
    switch (carrier) {
    case LCELL_MAX: {
        int64_t p_hwm = pv ? pv->hwm : INT64_MIN;
        if (d->hwm > p_hwm) out->hwm = d->hwm; /* the join-irreducible fragment */
        break;
    }
    case LCELL_GSET:
        for (int64_t i = 0; i < d->len; i++) {
            if (!pv
                || lcell_gset_find(pv, d->elems[i], key_hash(0, d->elems[i]), NULL) < 0) {
                lcell_gset_insert(out, d->elems[i]);
            }
        }
        break;
    case LCELL_LWW:
        if (d->lww_set
            && (!pv || !pv->lww_set || d->lww_clock != pv->lww_clock
                || d->lww_val != pv->lww_val)) {
            out->lww_set = 1;
            out->lww_val = d->lww_val;
            out->lww_clock = d->lww_clock;
        }
        break;
    }
    return out;
}

/* The read-side of the mutator law: base ⊔ delta as a NEW cell
   (pure, the zset_apply shape). */
void *sn_lcell_apply(void *base, void *delta) {
    lcell_guard_pair(base, delta, "apply");
    SnLCell *b = (SnLCell *)base;
    SnLCell *d = (SnLCell *)delta;
    int64_t carrier = b ? b->carrier : (d ? d->carrier : LCELL_MAX);
    SnLCell *out = (SnLCell *)sn_lcell_new(carrier);
    if (b) lcell_join(out, b);
    if (d) lcell_join(out, d);
    return out;
}

/* ---- readouts, per carrier (the honest-label plumbing) ------------- */

/* The support size: MAX → 1 iff a value was inflated past ⊥; GSET →
   |set|; LWW → 1 iff an offer exists. */
int64_t sn_lcell_len(void *p) {
    SnLCell *c = lcell_guard_one(p, "len");
    if (!c) return 0;
    switch (c->carrier) {
    case LCELL_MAX: return c->hwm != INT64_MIN ? 1 : 0;
    case LCELL_GSET: return c->len;
    case LCELL_LWW: return c->lww_set ? 1 : 0;
    }
    return 0;
}

/* The single-value readout: MAX → the HWM (⊥ = INT64_MIN when empty);
   LWW → the winner's value (0 when no offer — sn_lcell_len carries
   the set flag); GSET → 0, documented: the set carrier has no
   single-value readout — use len/has. */
int64_t sn_lcell_value(void *p) {
    SnLCell *c = lcell_guard_one(p, "value");
    if (!c) return 0;
    switch (c->carrier) {
    case LCELL_MAX: return c->hwm;
    case LCELL_GSET: return 0;
    case LCELL_LWW: return c->lww_set ? c->lww_val : 0;
    }
    return 0;
}

/* Membership in the support: GSET → set membership; MAX → the HWM is
   exactly e; LWW → the winner's value is exactly e. */
int64_t sn_lcell_has(void *p, int64_t e) {
    SnLCell *c = lcell_guard_one(p, "has");
    if (!c) return 0;
    switch (c->carrier) {
    case LCELL_MAX: return c->hwm != INT64_MIN && c->hwm == e;
    case LCELL_GSET:
        if (c->len == 0) return 0;
        return lcell_gset_find(c, e, key_hash(0, e), NULL) >= 0;
    case LCELL_LWW: return c->lww_set && c->lww_val == e;
    }
    return 0;
}

int64_t sn_lcell_carrier(void *p) {
    SnLCell *c = lcell_guard_one(p, "carrier");
    return c ? c->carrier : -1;
}

/* --------------------------------------------------------------------- */
/*  SnWave — the wave region context (SWARM RABBIT HOLE, Wave 13,        */
/*  Impl-R, Cohort Delta; W2B §3 the typed read-mode surface + §4 the    */
/*  wave-seam composition theorem, Thm WS — the campaign's centerpiece   */
/*  made executable).                                                     */
/*                                                                       */
/*  A wave region `wave { ... }` is a DETERMINISTIC OBSERVATION BRACKET   */
/*  over module-scope Tier-L seam cells (W2B §4.1):                       */
/*    entry  — sn_wave_capture registers the seam cells the body uses    */
/*             (the compiler emits one capture per cell, in sorted        */
/*             name order; pointer identity is the slot key);             */
/*    body   — read_W observes the frozen capture κ (the static          */
/*             discipline E0812/R1 makes the cell immutable through     */
/*             the body: fallback merges are legal only program-order-    */
/*             before the wave, and contribs only STAGE); contrib(c,a,b) */
/*             stages the payload pair (a,b) — exactly an lcell_inflate   */
/*             offer — into the per-cell buffer, the contribution        */
/*             multiset 𝒰_W(c);                                           */
/*    exit   — sn_wave_join is J(W), the J2 BATCHED CANONICAL FOLD: per  */
/*             cell, the staged multiset folds into ONE offer cell      */
/*             (any internal order — the carrier's laws make it          */
/*             order/grouping/duplication-independent, Lemma 1) and      */
/*             joins the seam cell through sn_lcell_merge — a single     */
/*             atomic ⊔ per cell per wave. Lemma NI(ii): "contributions   */
/*             ARE atomic ⊔-merges on seam cells — the same event class  */
/*             the tier's theorems quantify over."                       */
/*                                                                       */
/*  Thm WS Part 2 (join confluence): read_J after the block sees         */
/*  κ₀(c) ⊔ ⊔𝒰_W(c), independent of staging order (commutativity),        */
/*  grouping (associativity), duplication (idempotence — the S5          */
/*  at-least-once dividend), and thread count. The fold's correctness    */
/*  IS the Tier-L carrier's checked lattice laws — this section adds no  */
/*  algebra, only the staging buffer and the join discipline W3.         */
/*                                                                       */
/*  SLICE-1 BOUNDARY (the honest scope, W2B §4.7): the body runs ONCE,   */
/*  single-tile — the wave IS the parallelism in the model, and the      */
/*  parallel-tile instantiation (sn_go_spawn per tile + the live-arity    */
/*  barrier, the DBM emit ranges, the (sender,seq) k-way merge) is the   */
/*  documented future form this API is shaped for (the staging buffer   */
/*  is the tile payload interface; the fold already ignores arrival     */
/*  order by construction). No locks: the single-tile join is            */
/*  trivially atomic; the frozen κ is realized by the STATIC rules      */
/*  (nothing can change the cell between capture and J(W)), not by a    */
/*  runtime copy — the copy becomes necessary exactly when tiles run    */
/*  concurrently.                                                        */
/* --------------------------------------------------------------------- */
typedef struct SnWaveSlot {
    SnLCell *cell;  /* the seam cell — pointer identity is the key */
    int64_t n;      /* |𝒰_W(cell)| — staged payloads */
    int64_t cap;
    int64_t *pa;    /* payload a (value/element/HWM offer) */
    int64_t *pb;    /* payload b (the LWW logical clock; 0 elsewhere) */
} SnWaveSlot;

typedef struct SnWave {
    int64_t nslots;
    SnWaveSlot *slots;
} SnWave;

void *sn_wave_new(int64_t nslots) {
    if (nslots < 0) nslots = 0;
    SnWave *w = (SnWave *)xmalloc(sizeof(SnWave));
    w->nslots = nslots;
    w->slots = nslots > 0 ? (SnWaveSlot *)xmalloc(sizeof(SnWaveSlot) * (size_t)nslots) : NULL;
    return w;
}

/* Register seam cell #i. The compiler calls this once per captured
   cell at wave entry (sorted name order — the slot index is internal
   bookkeeping; every lookup is by pointer identity, so indices carry
   no semantic weight). A duplicate cell would fold the same seam
   twice — idempotent by the laws, but it means the compiler's capture
   set was malformed: fail fast (the E-LKIND backstop pattern). */
void sn_wave_capture(void *wave, int64_t i, void *cell) {
    SnWave *w = (SnWave *)wave;
    if (!w || i < 0 || i >= w->nslots || !cell) {
        fprintf(stderr,
                "runtime error: wave capture of slot %lld rejected (wave context "
                "malformed) [E-R1]\n",
                (long long)i);
        exit(1);
    }
    for (int64_t k = 0; k < w->nslots; k++) {
        if (w->slots[k].cell == cell) {
            fprintf(stderr,
                    "runtime error: seam cell %p captured twice by one wave "
                    "(the fold is idempotent, but the capture set is malformed) "
                    "[E-R1]\n",
                    cell);
            exit(1);
        }
    }
    w->slots[i].cell = (SnLCell *)cell;
    w->slots[i].n = 0;
    w->slots[i].cap = 0;
    w->slots[i].pa = NULL;
    w->slots[i].pb = NULL;
}

static SnWaveSlot *wave_slot_of(SnWave *w, void *cell) {
    for (int64_t i = 0; i < w->nslots; i++) {
        if (w->slots[i].cell == cell) return &w->slots[i];
    }
    return NULL;
}

/* read_W — the frozen capture's readout (W2B §3: the deterministic
   wave read mode). The cell must be a captured seam of this wave:
   the checker certifies the mode; this lookup is the runtime backstop
   (a read of an uncaptured cell through the wave read mode is a
   compiler bug or an analysis hole, never a silent fallback). */
int64_t sn_wave_read_w(void *wave, void *cell) {
    SnWave *w = (SnWave *)wave;
    if (!w || !wave_slot_of(w, cell)) {
        fprintf(stderr,
                "runtime error: read_W on a cell not captured by this wave "
                "(the frozen capture is defined only over the wave's seam "
                "cells) [E-R1]\n");
        exit(1);
    }
    return sn_lcell_value(cell);
}

/* contrib — stage the payload pair (a,b) into the cell's buffer
   𝒰_W(cell). STAGING ONLY: no merge happens here (the J1 exit-join
   discipline — contrib→exit→J(W); the J0 immediate merge is not the
   default and is checker-rejected in the body, E0812). Duplicate
   payloads are legal and fold once (⊔-idempotence — the S5
   at-least-once/retry dividend). */
void sn_wave_contrib(void *wave, void *cell, int64_t a, int64_t b) {
    SnWave *w = (SnWave *)wave;
    SnWaveSlot *s = w ? wave_slot_of(w, cell) : NULL;
    if (!s) {
        fprintf(stderr,
                "runtime error: contrib to a cell not captured by this wave "
                "(contributions fold at J(W) only over the wave's seam cells) "
                "[E-R1]\n");
        exit(1);
    }
    if (s->n == s->cap) {
        int64_t ncap = s->cap ? s->cap * 2 : 8;
        int64_t *na = (int64_t *)realloc(s->pa, sizeof(int64_t) * (size_t)ncap);
        int64_t *nb = (int64_t *)realloc(s->pb, sizeof(int64_t) * (size_t)ncap);
        if (!na || !nb) {
            fprintf(stderr, "out of memory\n");
            exit(1);
        }
        s->pa = na;
        s->pb = nb;
        s->cap = ncap;
    }
    s->pa[s->n] = a;
    s->pb[s->n] = b;
    s->n++;
}

/* J(W) — the join (W2B §4.3's J2 batched canonical fold; the join
   discipline W3). Per seam cell: the staged multiset folds into ONE
   offer cell — the internal application order is the staging order
   here, but the fold's RESULT is order-independent by the carrier's
   checked laws (commutativity/associativity; Lemma 1 absorbs
   duplicates) — and the offer joins the seam cell through ONE
   sn_lcell_merge: a single atomic ⊔ event per cell per wave (NI(ii)),
   running the SN_LATTICE_CHECK=1 law self-checks on exactly the
   event the theorem quantifies over. The offer is a fresh ⊥ cell of
   the same carrier; merging ⊥ is the identity, so read-only waves
   (𝒰_W = ∅) are no-ops by construction. */
void sn_wave_join(void *wave) {
    SnWave *w = (SnWave *)wave;
    if (!w) return;
    for (int64_t i = 0; i < w->nslots; i++) {
        SnWaveSlot *s = &w->slots[i];
        if (!s->cell) continue;
        SnLCell *offer = (SnLCell *)sn_lcell_new(s->cell->carrier);
        for (int64_t j = 0; j < s->n; j++) {
            sn_lcell_inflate(offer, s->pa[j], s->pb[j]);
        }
        sn_lcell_merge(s->cell, offer);
        sn_release(offer);
        free(s->pa);
        free(s->pb);
        s->pa = NULL;
        s->pb = NULL;
        s->n = 0;
        s->cap = 0;
    }
    free(w->slots);
    free(w);
}

/* --------------------------------------------------------------------- */
/*  SnMorton / SnHilbert — the SFC layouts (SWARM RABBIT HOLE, Wave 14,    */
/*  Impl-S, Cohort Delta; A2 §2.4's space-filling-geometry decision       */
/*  table, made executable).                                             */
/*                                                                       */
/*  morton list<T, dims=[n1, n2]> — tensor iteration with per-element    */
/*  indexed access (A2 §2.4): ONE contiguous lane reservation (header +  */
/*  the values, commit-on-push — the SnLane machinery), NO directory for  */
/*  fixed-size T, addr = base + interleave(i, j)·8. The code space        */
/*  covers a power-of-two grid EXACTLY (codes [0, d1·d2) ↔ cells), so     */
/*  the O(1)-carry iteration is the integer increment (consecutive        */
/*  cells have consecutive codes — the code IS the linear index) and the  */
/*  curve-order scan is a stride-1 buffer walk (A2 §2.3). Position       */
/*  independence: the values are ints, never absolute pointers —          */
/*  to_bytes/from_bytes is zero-fixup by construction (the link<T, u16>   */
/*  seam needs its O(N) scan for exactly the self-relative bytes this     */
/*  layout does not have). from_lex is the one blocked reshuffle          */
/*  (Chatterjee ICS'99's 2–5%-of-runtime layout price — a dyadic tile    */
/*  at a time, so each tile's scatter lands in one contiguous code run).  */
/*                                                                       */
/*  hilbert map<int, int> — spatial range queries (A2 §2.4): entries     */
/*  sorted by d=2 Hilbert key on a lane (the order IS the index — exact   */
/*  point lookup is a binary search, no auxiliary hash); the box query    */
/*  is the Tropf–Herzog/Lawder–King segment advance over the box's key    */
/*  range. The range is computed EXACTLY from the curve's connectedness   */
/*  (it enters and exits any box on the boundary — consecutive cells     */
/*  are unit-step neighbors — so min/max over the box's boundary cells    */
/*  IS the key range; Θ(perimeter·k) here, the Lawder–King Θ(k) hix       */
/*  descent is the documented slice-2 refinement). The key itself is     */
/*  the Butz 1971 / Skilling 2004 loop — Θ(k), no direct LUT (A2 §2.2;   */
/*  the per-access cost is the honest price C2.9 names). Insert is        */
/*  append-in-key-order O(1); out-of-order is the honest slice-1         */
/*  REFUSAL ([E-SFC-ORDER] — no silent sort; the rebuild-epoch policy     */
/*  is the documented future form, A2 §3's P1/P3 flag).                   */
/*                                                                       */
/*  Lane discipline (E-GC-IN-LANE): both layouts' storage lives on lane 0 */
/*  and is NEVER gc_track'd; sn_release on either kind is a NO-OP (a      */
/*  stray release would free lane bytes as heap — the checker rejects    */
/*  the store sites; the no-op is the runtime backstop). They die at      */
/*  the lane's rewind, exactly like lane-backed list headers.            */
/* --------------------------------------------------------------------- */

/* --- the feature dispatch (A2 §2.2 / C2.8 — load-bearing) ---------------
   The interleave/deinterleave is dispatched ONCE PER PROCESS: BMI2
   PDEP/PEXT on x86-64 (__builtin_cpu_supports("bmi2") — 2 PDEP + or,
   the 3-cycle Haswell+ form), the software shift-or path (~10 ops,
   still ≪ an L2 miss) everywhere else — AArch64 has no BMI2 (this M4
   runs the software path). The knob SN_SFC_FORCE_PATH=soft forces the
   software path (the systems suite's equivalence probe runs the grid
   under both settings and diffs); =bmi2 fails fast off a BMI2 machine
   (executing PDEP without the feature would be an illegal instruction
   — the refusal is the honest answer, not a crash). NEVER use the
   PDEP builtins unconditionally: they do not compile on arm64. */
/* The forward shims: sn_lane_alloc / sn_bytearray_new are defined
   later in the file (the single-TU layout keeps the SFC section
   beside the wave/zset tiers); the local declaration mirrors
   sn_list_new_lane's (the lane section's own precedent). */
static void *sn_lane_alloc_fwd(int64_t lane_id, int64_t bytes) {
    void *sn_lane_alloc(int64_t lane_id, int64_t bytes);
    return sn_lane_alloc(lane_id, bytes);
}

static void *sn_bytearray_new_fwd(int64_t cap) {
    void *sn_bytearray_new(int64_t cap);
    return sn_bytearray_new(cap);
}

#define SN_SFC_EVEN_MASK 0x5555555555555555ULL /* the x (even-bit) spread mask */

static int g_sfc_dispatch = -1; /* -1 unresolved, 0 software, 1 BMI2 */

static int sn_sfc_bmi2_ready(void) {
    if (g_sfc_dispatch >= 0) return g_sfc_dispatch;
    int bmi2 = 0;
#if defined(_WIN32) && (defined(__x86_64__) || defined(_M_X64))
    int cpuInfo[4] = {0};
    __cpuidex(cpuInfo, 7, 0);
    bmi2 = (cpuInfo[1] & (1 << 8)) ? 1 : 0;
#elif (defined(__GNUC__) || defined(__clang__)) && (defined(__x86_64__) || defined(_M_X64))
    __builtin_cpu_init();
    bmi2 = __builtin_cpu_supports("bmi2") ? 1 : 0;
#endif
    const char *e = getenv("SN_SFC_FORCE_PATH");
    if (e) {
        if (strcmp(e, "soft") == 0) {
            bmi2 = 0;
        } else if (strcmp(e, "bmi2") == 0) {
            if (!bmi2) {
                fprintf(stderr,
                        "runtime error: SN_SFC_FORCE_PATH=bmi2 but this machine has "
                        "no BMI2 (AArch64 has no PDEP/PEXT — C2.8; the software "
                        "shift-or path is what runs here) [E-SFC-PATH]\n");
                exit(1);
            }
        }
    }
    g_sfc_dispatch = bmi2;
    return bmi2;
}

/* The test/bench readout: which path the process resolved (0=soft, 1=BMI2). */
int64_t sn_sfc_path(void) {
    return sn_sfc_bmi2_ready();
}

/* Test hook: re-resolve the dispatch. The knob is read at resolution
   time, so the equivalence probes run one pass per setting IN ONE
   PROCESS (setenv + recheck + hash, both settings, compare) — and a
   fork'd child can force a setting its parent never resolved. */
void sn_sfc_force_recheck(void) {
    g_sfc_dispatch = -1;
}

/* The software spread/compact: x's bits to the even positions and back
   (the classic 5-step shift-or; ~10 ops total — A2 §2.2's non-BMI2
   tier, ≪ an L2 miss). */
static uint64_t sn_sfc_part_by1(uint64_t x) {
    x &= 0x00000000FFFFFFFFULL;
    x = (x | (x << 16)) & 0x0000FFFF0000FFFFULL;
    x = (x | (x << 8)) & 0x00FF00FF00FF00FFULL;
    x = (x | (x << 4)) & 0x0F0F0F0F0F0F0F0FULL;
    x = (x | (x << 2)) & 0x3333333333333333ULL;
    x = (x | (x << 1)) & SN_SFC_EVEN_MASK;
    return x;
}

static uint64_t sn_sfc_compact_by1(uint64_t x) {
    x &= SN_SFC_EVEN_MASK;
    x = (x | (x >> 1)) & 0x3333333333333333ULL;
    x = (x | (x >> 2)) & 0x0F0F0F0F0F0F0F0FULL;
    x = (x | (x >> 4)) & 0x00FF00FF00FF00FFULL;
    x = (x | (x >> 8)) & 0x0000FFFF0000FFFFULL;
    x = (x | (x >> 16)) & 0x00000000FFFFFFFFULL;
    return x;
}

static uint64_t sn_morton_interleave_soft(uint64_t x, uint64_t y) {
    return sn_sfc_part_by1(x) | (sn_sfc_part_by1(y) << 1);
}

static void sn_morton_deinterleave_soft(uint64_t m, uint64_t *x, uint64_t *y) {
    *x = sn_sfc_compact_by1(m);
    *y = sn_sfc_compact_by1(m >> 1);
}

#if defined(__x86_64__) || defined(_M_X64)
#include <immintrin.h>
#if defined(__GNUC__) || defined(__clang__)
__attribute__((__target__("bmi2")))
#endif
static uint64_t sn_morton_interleave_bmi2(uint64_t x, uint64_t y) {
    return _pdep_u64(x, SN_SFC_EVEN_MASK)
         | _pdep_u64(y, SN_SFC_EVEN_MASK << 1);
}
#if defined(__GNUC__) || defined(__clang__)
__attribute__((__target__("bmi2")))
#endif
static void sn_morton_deinterleave_bmi2(uint64_t m, uint64_t *x, uint64_t *y) {
    *x = _pext_u64(m, SN_SFC_EVEN_MASK);
    *y = _pext_u64(m, SN_SFC_EVEN_MASK << 1);
}
#endif

static uint64_t sn_morton_interleave(uint64_t x, uint64_t y) {
    if (sn_sfc_bmi2_ready()) {
#if defined(__x86_64__) || defined(_M_X64)
        return sn_morton_interleave_bmi2(x, y);
#endif
    }
    return sn_morton_interleave_soft(x, y);
}

static void sn_morton_deinterleave(uint64_t m, uint64_t *x, uint64_t *y) {
    if (sn_sfc_bmi2_ready()) {
#if defined(__x86_64__) || defined(_M_X64)
        sn_morton_deinterleave_bmi2(m, x, y);
        return;
#endif
    }
    sn_morton_deinterleave_soft(m, x, y);
}

/* --- SnMorton: one contiguous reservation, header + curve-order data --- */

typedef struct SnMorton {
    int64_t rc;    /* unused on the lane (no ARC — see sn_release's no-op) */
    int64_t kind;  /* KIND_MORTON */
    int64_t d1;    /* dim 1 (a power of two) */
    int64_t d2;    /* dim 2 (a power of two) */
    int64_t len;   /* d1·d2 == the code space size (exact — no padding) */
    int64_t data[]; /* the values in curve order: data[interleave(i, j)] */
} SnMorton;

static void sn_morton_die_dims(int64_t d1, int64_t d2) {
    fprintf(stderr,
            "runtime error: morton dims must be equal powers of two (slice 1 maps "
            "the SQUARE grid onto the code space exactly — no padding; unequal "
            "dims want the padded reservation / the hybrid 2^t tile floor, the "
            "slice-2 form), got dims=[%lld, %lld] [E-SFC]\n",
            (long long)d1, (long long)d2);
    exit(1);
}

static void sn_morton_check_dims(int64_t d1, int64_t d2) {
    /* equal powers of two: the code space covers a SQUARE power-of-two
       grid exactly — interleave(d−1, d−1)+1 == d·d. Unequal
       power-of-two dims spread the code space past d1·d2 (the
       high-axis/low-axis corner overflows the reservation), so slice
       1 refuses them (the padded reservation is the slice-2 form). */
    if (d1 <= 0 || d1 > (1ll << 31) || d1 != d2 || (d1 & (d1 - 1)) != 0) {
        sn_morton_die_dims(d1, d2);
    }
}

static void *sn_morton_build(int64_t d1, int64_t d2) {
    sn_morton_check_dims(d1, d2);
    int64_t len = d1 * d2; /* d == 2^a ≤ 2^31 → len = d·d ≤ 2^62, fits i64 */
    /* The byte-budget check BEFORE any multiply: keeps the
       reservation inside the 256 MB lane AND len·8 inside int64 (the
       dims alone cannot — 2^31-squared wants 2^65 bytes). */
    if (len > (256ll * 1024 * 1024 - 64) / 8) {
        fprintf(stderr,
                "runtime error: morton dims [%lld, %lld] need about %lld MB — "
                "over the 256 MB Elastic Lane reservation; use a smaller grid "
                "or an explicit slab [E-SFC]\n",
                (long long)d1, (long long)d2, (long long)(len / 131072));
        exit(1);
    }
    int64_t bytes = (int64_t)sizeof(SnMorton) + len * (int64_t)sizeof(int64_t);
    SnMorton *m = (SnMorton *)sn_lane_alloc_fwd(0, bytes);
    m->rc = 1;
    m->kind = KIND_MORTON;
    m->d1 = d1;
    m->d2 = d2;
    m->len = len;
    /* determinism: fresh lane bytes are stale after any rewind, and a
       read of an unwritten cell must not leak them (the zero fill is
       part of the layout's one-time price — priced in the from_lex
       bench row). */
    memset(m->data, 0, (size_t)len * sizeof(int64_t));
    return m;
}

static SnMorton *sn_morton_of(void *p, const char *op) {
    SnMorton *m = (SnMorton *)p;
    if (!m || m->kind != KIND_MORTON) {
        fprintf(stderr,
                "runtime error: %s operates on a morton list (the lane-backed "
                "SFC header); the value is not one [E-SFC]\n",
                op);
        exit(1);
    }
    return m;
}

void *sn_morton_new(int64_t d1, int64_t d2) {
    return sn_morton_build(d1, d2);
}

/* from_lex — the one blocked reshuffle (A2 §3; the 2–5%-of-runtime
   price, priced in bench/RESULTS_SFC.md). The source is a plain
   row-major list<int> of exactly d1·d2 values; the write walks
   dyadic 8×8 tiles so each tile's scatter lands in one contiguous
   code run (the quadtree block property — a 2^a×2^b dyadic tile's
   codes are contiguous), keeping the reshuffle cache-local on both
   sides. The result is a PERMUTATION: every cell lands at its own
   interleave address, the multiset is preserved. */
void *sn_morton_from_lex(void *src, int64_t d1, int64_t d2) {
    sn_morton_check_dims(d1, d2);
    int64_t n = sn_list_len(src);
    int64_t len = d1 * d2;
    if (n != len) {
        fprintf(stderr,
                "runtime error: morton_from_lex source holds %lld values, dims "
                "[%lld, %lld] need %lld — the lexicographic reshape is exact "
                "[E-SFC]\n",
                (long long)n, (long long)d1, (long long)d2, (long long)len);
        exit(1);
    }
    SnMorton *m = (SnMorton *)sn_morton_build(d1, d2);
    const int64_t T = 8; /* the dyadic tile side (a code run of 64) */
    for (int64_t ti = 0; ti < d1; ti += T) {
        for (int64_t tj = 0; tj < d2; tj += T) {
            int64_t ie = ti + T < d1 ? ti + T : d1;
            int64_t je = tj + T < d2 ? tj + T : d2;
            for (int64_t i = ti; i < ie; i++) {
                for (int64_t j = tj; j < je; j++) {
                    m->data[sn_morton_interleave((uint64_t)i, (uint64_t)j)]
                        = sn_list_get_i64(src, i * d2 + j);
                }
            }
        }
    }
    return m;
}

int64_t sn_morton_get(void *p, int64_t i, int64_t j) {
    SnMorton *m = sn_morton_of(p, "morton_get");
    if (i < 0 || i >= m->d1 || j < 0 || j >= m->d2) {
        fprintf(stderr,
                "runtime error: morton index (%lld, %lld) out of bounds for dims "
                "[%lld, %lld] — SFC addressing is bounds-checked, never wraps "
                "[E-SFC]\n",
                (long long)i, (long long)j, (long long)m->d1, (long long)m->d2);
        exit(1);
    }
    return m->data[sn_morton_interleave((uint64_t)i, (uint64_t)j)];
}

void sn_morton_set(void *p, int64_t i, int64_t j, int64_t v) {
    SnMorton *m = sn_morton_of(p, "morton_set");
    if (i < 0 || i >= m->d1 || j < 0 || j >= m->d2) {
        fprintf(stderr,
                "runtime error: morton index (%lld, %lld) out of bounds for dims "
                "[%lld, %lld] — SFC addressing is bounds-checked, never wraps "
                "[E-SFC]\n",
                (long long)i, (long long)j, (long long)m->d1, (long long)m->d2);
        exit(1);
    }
    m->data[sn_morton_interleave((uint64_t)i, (uint64_t)j)] = v;
}

/* The pure index arithmetic — the feature-dispatched interleave. */
int64_t sn_morton_index(int64_t i, int64_t j) {
    if (i < 0 || i > INT32_MAX || j < 0 || j > INT32_MAX) {
        fprintf(stderr,
                "runtime error: morton coordinates must be in [0, 2^31) — the "
                "interleave spreads two 32-bit axes into one 64-bit code "
                "[E-SFC]\n");
        exit(1);
    }
    return (int64_t)sn_morton_interleave((uint64_t)i, (uint64_t)j);
}

int64_t sn_morton_x(int64_t k) {
    if (k < 0) {
        fprintf(stderr, "runtime error: morton code must be non-negative [E-SFC]\n");
        exit(1);
    }
    uint64_t x, y;
    sn_morton_deinterleave((uint64_t)k, &x, &y);
    return (int64_t)x;
}

int64_t sn_morton_y(int64_t k) {
    if (k < 0) {
        fprintf(stderr, "runtime error: morton code must be non-negative [E-SFC]\n");
        exit(1);
    }
    uint64_t x, y;
    sn_morton_deinterleave((uint64_t)k, &x, &y);
    return (int64_t)y;
}

int64_t sn_morton_len(void *p) {
    return sn_morton_of(p, "morton_len")->len;
}

/* The curve-order read: position k in CODE order — a stride-1 buffer
   walk (A2 §2.3's linear scan; the curve costs nothing at L1 during
   traversal). */
int64_t sn_morton_scan_get(void *p, int64_t k) {
    SnMorton *m = sn_morton_of(p, "morton_scan_get");
    if (k < 0 || k >= m->len) {
        fprintf(stderr,
                "runtime error: morton curve position %lld out of range [0, "
                "%lld) — the curve-order scan is bounds-checked [E-SFC]\n",
                (long long)k, (long long)m->len);
        exit(1);
    }
    return m->data[k];
}

/* to_bytes — the values as a self-describing blob (dims header + the
   raw curve-order bytes). VALUES ONLY: no absolute pointers anywhere
   in the layout, so the round trip is ZERO-FIXUP by construction
   (position independence — what the link<T, u16> seam needs an O(N)
   scan for, this layout gets for free). */
void *sn_morton_to_bytes(void *p) {
    SnMorton *m = sn_morton_of(p, "morton_to_bytes");
    int64_t payload = m->len * (int64_t)sizeof(int64_t);
    ByteArray *b = (ByteArray *)sn_bytearray_new_fwd(24 + payload);
    int64_t hdr[3] = { m->d1, m->d2, m->len };
    memcpy(b->data, hdr, sizeof hdr);
    memcpy(b->data + sizeof hdr, m->data, (size_t)payload);
    b->len = (int64_t)sizeof hdr + payload;
    return b;
}

void *sn_morton_from_bytes(void *bytes, int64_t d1, int64_t d2) {
    ByteArray *b = (ByteArray *)bytes;
    if (!b || b->kind != KIND_BYTEARRAY) {
        fprintf(stderr,
                "runtime error: morton_from_bytes expects a to_bytes blob (a "
                "bytearray) [E-SFC]\n");
        exit(1);
    }
    sn_morton_check_dims(d1, d2);
    if (b->len < 24) {
        fprintf(stderr,
                "runtime error: morton_from_bytes blob holds %lld bytes — too "
                "short for the dims header [E-SFC]\n",
                (long long)b->len);
        exit(1);
    }
    int64_t hdr[3];
    memcpy(hdr, b->data, sizeof hdr);
    if (hdr[0] != d1 || hdr[1] != d2) {
        fprintf(stderr,
                "runtime error: morton_from_bytes dims [%lld, %lld] do not "
                "match the blob's dims [%lld, %lld] — the serialized header "
                "is the source of truth and the type must agree [E-SFC]\n",
                (long long)d1, (long long)d2, (long long)hdr[0], (long long)hdr[1]);
        exit(1);
    }
    if (b->len != 24 + hdr[2] * (int64_t)sizeof(int64_t)) {
        fprintf(stderr,
                "runtime error: morton_from_bytes blob holds %lld value bytes, "
                "the header's dims need %lld — the blob is truncated [E-SFC]\n",
                (long long)(b->len - 24), (long long)(hdr[2] * 8));
        exit(1);
    }
    SnMorton *m = (SnMorton *)sn_morton_build(d1, d2);
    memcpy(m->data, b->data + 24, (size_t)m->len * sizeof(int64_t));
    return m;
}

/* --- SnHilbert: the sorted-key lane (the order IS the index) ------------ */

typedef struct SnHilbert {
    int64_t rc;    /* unused on the lane (no ARC) */
    int64_t kind;  /* KIND_HILBERT */
    int64_t k;     /* the order: the grid side is 2^k */
    int64_t len;   /* entries */
    int64_t cap;
    int64_t *keys;  /* the sorted d=2 Hilbert codes — the order IS the index */
    int64_t *xs;    /* the coords, parallel columns (SoA in one reservation) */
    int64_t *ys;
    int64_t *vals;
    /* The per-query range cache (A2 §2.4: the key is computed ONCE per
       box and amortized over V elements). hilbert_box_first computes
       the box's key range and caches it; hilbert_box_next over the
       SAME box reuses it — a stateless-int iterator whose per-query
       arithmetic is paid once (a different box invalidates the
       cache). Single-threaded by the slice-1 contract. */
    int64_t cx0, cy0, cx1, cy1; /* the cached box (raw, unclipped) */
    int64_t ckmin, ckmax;       /* the cached range */
    int64_t ccached;            /* 1 once a range is cached */
} SnHilbert;

/* The Butz 1971 / Skilling 2004 loop (Θ(k), no direct LUT — A2 §2.2).
   The standard quadrant-digit form: per level, the quadrant digit is
   (3·rx)^ry and the frame rotates (Butz's rotation propagation); the
   base-4 digits of the code ARE the quadrant ranks. */
static void sn_hilbert_rot(int64_t n, int64_t *x, int64_t *y, int rx, int ry) {
    if (ry == 0) {
        if (rx == 1) {
            *x = n - 1 - *x;
            *y = n - 1 - *y;
        }
        int64_t t = *x;
        *x = *y;
        *y = t;
    }
}

static int64_t sn_hilbert_encode(int64_t x, int64_t y, int64_t n) {
    int64_t d = 0;
    for (int64_t s = n / 2; s > 0; s /= 2) {
        int rx = (x & s) > 0;
        int ry = (y & s) > 0;
        d += s * s * ((3 * rx) ^ ry);
        sn_hilbert_rot(n, &x, &y, rx, ry);
    }
    return d;
}

static void sn_hilbert_decode(int64_t n, int64_t d, int64_t *x, int64_t *y) {
    *x = 0;
    *y = 0;
    for (int64_t s = 1, t = d; s < n; s *= 2) {
        int rx = 1 & (t / 2);
        int ry = 1 & (t ^ rx);
        sn_hilbert_rot(s, x, y, rx, ry);
        *x += s * rx;
        *y += s * ry;
        t /= 4;
    }
}

static SnHilbert *sn_hilbert_of(void *p, const char *op) {
    SnHilbert *h = (SnHilbert *)p;
    if (!h || h->kind != KIND_HILBERT) {
        fprintf(stderr,
                "runtime error: %s operates on a hilbert map (the lane-backed "
                "SFC header); the value is not one [E-SFC]\n",
                op);
        exit(1);
    }
    return h;
}

void *sn_hilbert_new(int64_t k) {
    if (k < 1 || k > 31) {
        fprintf(stderr,
                "runtime error: hilbert_new's order k must be in 1..=31 (the "
                "d=2 grid side is 2^k and the code must fit the 62-bit "
                "space), got %lld [E-SFC]\n",
                (long long)k);
        exit(1);
    }
    SnHilbert *h = (SnHilbert *)sn_lane_alloc_fwd(0, (int64_t)sizeof(SnHilbert));
    h->rc = 1;
    h->kind = KIND_HILBERT;
    h->k = k;
    h->len = 0;
    h->cap = 0;
    h->keys = NULL;
    h->xs = NULL;
    h->ys = NULL;
    h->vals = NULL;
    h->ccached = 0;
    return h;
}

/* Growth: the four columns live in ONE contiguous lane reservation,
   re-made at double capacity (the old block is abandoned to the
   rewind — lane bump memory, never freed in place; amortized O(1)
   per insert, the list-growth discipline). */
static void sn_hilbert_grow(SnHilbert *h) {
    if (h->len < h->cap) return;
    int64_t ncap = h->cap ? h->cap * 2 : 16;
    int64_t *base = (int64_t *)sn_lane_alloc_fwd(
        0, ncap * 4 * (int64_t)sizeof(int64_t));
    int64_t *nk = base;
    int64_t *nx = base + ncap;
    int64_t *ny = base + 2 * ncap;
    int64_t *nv = base + 3 * ncap;
    if (h->len > 0) {
        memcpy(nk, h->keys, (size_t)h->len * sizeof(int64_t));
        memcpy(nx, h->xs, (size_t)h->len * sizeof(int64_t));
        memcpy(ny, h->ys, (size_t)h->len * sizeof(int64_t));
        memcpy(nv, h->vals, (size_t)h->len * sizeof(int64_t));
    }
    h->keys = nk;
    h->xs = nx;
    h->ys = ny;
    h->vals = nv;
    h->cap = ncap;
}

void sn_hilbert_insert(void *p, int64_t x, int64_t y, int64_t v) {
    SnHilbert *h = sn_hilbert_of(p, "hilbert_insert");
    int64_t side = 1ll << h->k;
    if (x < 0 || x >= side || y < 0 || y >= side) {
        fprintf(stderr,
                "runtime error: hilbert point (%lld, %lld) out of the 2^%lld "
                "grid — the d=2 key space is bounds-checked [E-SFC]\n",
                (long long)x, (long long)y, (long long)h->k);
        exit(1);
    }
    int64_t key = sn_hilbert_encode(x, y, side);
    if (h->len > 0) {
        int64_t last = h->keys[h->len - 1];
        if (key < last) {
            fprintf(stderr,
                    "runtime error: hilbert insert out of key order (new key "
                    "%lld after last %lld) — the sorted lane IS the index and "
                    "slice 1 does not reorder; insert along the curve or "
                    "await the rebuild-epoch policy (P1/P3) [E-SFC-ORDER]\n",
                    (long long)key, (long long)last);
            exit(1);
        }
        if (key == last) {
            fprintf(stderr,
                    "runtime error: hilbert point (%lld, %lld) already present "
                    "(key %lld) — maps hold one value per point; slice 1 has "
                    "no retraction (await the rebuild-epoch policy, P1/P3) "
                    "[E-SFC-DUP]\n",
                    (long long)x, (long long)y, (long long)key);
            exit(1);
        }
    }
    sn_hilbert_grow(h);
    h->keys[h->len] = key;
    h->xs[h->len] = x;
    h->ys[h->len] = y;
    h->vals[h->len] = v;
    h->len++;
    /* a mutation invalidates any in-flight box iteration's cached
       range (the append never reorders the [0, len) prefix, but the
       iterator's bounds and the range's emptiness are per-query
       facts — recompute on the next query). */
    h->ccached = 0;
}

/* The binary search over the sorted lane (the order IS the index —
   no auxiliary hash; A2 §3). */
static int64_t sn_hilbert_lower_bound(SnHilbert *h, int64_t key) {
    int64_t lo = 0, hi = h->len;
    while (lo < hi) {
        int64_t mid = lo + (hi - lo) / 2;
        if (h->keys[mid] < key) lo = mid + 1;
        else hi = mid;
    }
    return lo;
}

static int64_t sn_hilbert_find(SnHilbert *h, int64_t key) {
    int64_t i = sn_hilbert_lower_bound(h, key);
    if (i < h->len && h->keys[i] == key) return i;
    return -1;
}

int64_t sn_hilbert_get(void *p, int64_t x, int64_t y) {
    SnHilbert *h = sn_hilbert_of(p, "hilbert_get");
    int64_t side = 1ll << h->k;
    if (x < 0 || x >= side || y < 0 || y >= side) {
        fprintf(stderr,
                "runtime error: hilbert point (%lld, %lld) out of the 2^%lld "
                "grid — the d=2 key space is bounds-checked [E-SFC]\n",
                (long long)x, (long long)y, (long long)h->k);
        exit(1);
    }
    int64_t key = sn_hilbert_encode(x, y, side);
    int64_t i = sn_hilbert_find(h, key);
    if (i < 0) {
        fprintf(stderr,
                "runtime error: hilbert point (%lld, %lld) not present — "
                "slice 1 has no optional values on this surface; probe with "
                "hilbert_has [E-SFC-KEYX]\n",
                (long long)x, (long long)y);
        exit(1);
    }
    return h->vals[i];
}

int64_t sn_hilbert_has(void *p, int64_t x, int64_t y) {
    SnHilbert *h = sn_hilbert_of(p, "hilbert_has");
    int64_t side = 1ll << h->k;
    if (x < 0 || x >= side || y < 0 || y >= side) return 0;
    return sn_hilbert_find(h, sn_hilbert_encode(x, y, side)) >= 0 ? 1 : 0;
}

int64_t sn_hilbert_len(void *p) {
    return sn_hilbert_of(p, "hilbert_len")->len;
}

int64_t sn_hilbert_code(int64_t x, int64_t y, int64_t k) {
    if (k < 1 || k > 31) {
        fprintf(stderr,
                "runtime error: hilbert order k must be in 1..=31 [E-SFC]\n");
        exit(1);
    }
    int64_t side = 1ll << k;
    if (x < 0 || x >= side || y < 0 || y >= side) {
        fprintf(stderr,
                "runtime error: hilbert point (%lld, %lld) out of the 2^%lld "
                "grid — the d=2 key space is bounds-checked [E-SFC]\n",
                (long long)x, (long long)y, (long long)k);
        exit(1);
    }
    return sn_hilbert_encode(x, y, side);
}

int64_t sn_hilbert_dx(int64_t c, int64_t k) {
    int64_t side;
    int64_t x, y;
    if (k < 1 || k > 31) {
        fprintf(stderr, "runtime error: hilbert order k must be in 1..=31 [E-SFC]\n");
        exit(1);
    }
    side = 1ll << k;
    if (c < 0 || c >= side * side) {
        fprintf(stderr,
                "runtime error: hilbert code %lld out of range [0, %lld) for "
                "order %lld [E-SFC]\n",
                (long long)c, (long long)(side * side), (long long)k);
        exit(1);
    }
    sn_hilbert_decode(side, c, &x, &y);
    return x;
}

int64_t sn_hilbert_dy(int64_t c, int64_t k) {
    int64_t side;
    int64_t x, y;
    if (k < 1 || k > 31) {
        fprintf(stderr, "runtime error: hilbert order k must be in 1..=31 [E-SFC]\n");
        exit(1);
    }
    side = 1ll << k;
    if (c < 0 || c >= side * side) {
        fprintf(stderr,
                "runtime error: hilbert code %lld out of range [0, %lld) for "
                "order %lld [E-SFC]\n",
                (long long)c, (long long)(side * side), (long long)k);
        exit(1);
    }
    sn_hilbert_decode(side, c, &x, &y);
    return y;
}

static int64_t sn_hilbert_entry_col(SnHilbert *h, int64_t idx, int64_t *col,
                                    const char *op) {
    if (idx < 0 || idx >= h->len) {
        fprintf(stderr,
                "runtime error: hilbert entry index %lld out of range [0, "
                "%lld) — the sorted lane's order is the index [E-SFC]\n",
                (long long)idx, (long long)h->len);
        exit(1);
    }
    return col[idx];
}

int64_t sn_hilbert_entry_key(void *p, int64_t idx) {
    SnHilbert *h = sn_hilbert_of(p, "hilbert_entry_key");
    return sn_hilbert_entry_col(h, idx, h->keys, "hilbert_entry_key");
}

int64_t sn_hilbert_entry_x(void *p, int64_t idx) {
    SnHilbert *h = sn_hilbert_of(p, "hilbert_entry_x");
    return sn_hilbert_entry_col(h, idx, h->xs, "hilbert_entry_x");
}

int64_t sn_hilbert_entry_y(void *p, int64_t idx) {
    SnHilbert *h = sn_hilbert_of(p, "hilbert_entry_y");
    return sn_hilbert_entry_col(h, idx, h->ys, "hilbert_entry_y");
}

int64_t sn_hilbert_entry_val(void *p, int64_t idx) {
    SnHilbert *h = sn_hilbert_of(p, "hilbert_entry_val");
    return sn_hilbert_entry_col(h, idx, h->vals, "hilbert_entry_val");
}

/* The box's key range, EXACT: the curve enters and exits any box on
   its boundary (connectedness — consecutive cells are unit-step
   neighbors), so min/max over the box's boundary cells IS the
   Tropf–Herzog range. Θ(perimeter·k); the Lawder–King Θ(k) hix
   descent is the documented slice-2 refinement. Returns 0 when the
   clipped box is empty. */
static int sn_hilbert_box_range(SnHilbert *h, int64_t x0, int64_t y0,
                                int64_t x1, int64_t y1,
                                int64_t *kmin, int64_t *kmax) {
    int64_t side = 1ll << h->k;
    if (x0 < 0) x0 = 0;
    if (y0 < 0) y0 = 0;
    if (x1 > side - 1) x1 = side - 1;
    if (y1 > side - 1) y1 = side - 1;
    if (x0 > x1 || y0 > y1) return 0;
    int64_t lo = INT64_MAX, hi = INT64_MIN;
    for (int64_t x = x0; x <= x1; x++) {
        int64_t a = sn_hilbert_encode(x, y0, side);
        int64_t b = sn_hilbert_encode(x, y1, side);
        if (a < lo) lo = a;
        if (a > hi) hi = a;
        if (b < lo) lo = b;
        if (b > hi) hi = b;
    }
    for (int64_t y = y0; y <= y1; y++) {
        int64_t a = sn_hilbert_encode(x0, y, side);
        int64_t b = sn_hilbert_encode(x1, y, side);
        if (a < lo) lo = a;
        if (a > hi) hi = a;
        if (b < lo) lo = b;
        if (b > hi) hi = b;
    }
    *kmin = lo;
    *kmax = hi;
    return 1;
}

/* The segment advance: from `idx` (INCLUSIVE), skip entries whose key
   is in range but whose point is outside the box, and stop at the
   first in-box entry — or at the range's end (-1). The entries in
   [kmin, kmax] are the box's segments (A2's S_q/(2d) cluster law:
   the scan touches exactly the range, never the whole lane). */
static int64_t sn_hilbert_box_advance(SnHilbert *h, int64_t x0, int64_t y0,
                                      int64_t x1, int64_t y1, int64_t kmax,
                                      int64_t idx) {
    while (idx < h->len && h->keys[idx] <= kmax) {
        if (h->xs[idx] >= x0 && h->xs[idx] <= x1
            && h->ys[idx] >= y0 && h->ys[idx] <= y1) {
            return idx;
        }
        idx++;
    }
    return -1;
}

int64_t sn_hilbert_box_first(void *p, int64_t x0, int64_t y0,
                             int64_t x1, int64_t y1) {
    SnHilbert *h = sn_hilbert_of(p, "hilbert_box_first");
    int64_t kmin, kmax;
    if (!sn_hilbert_box_range(h, x0, y0, x1, y1, &kmin, &kmax)) return -1;
    if (h->len == 0) return -1;
    /* cache the range for the box_next chain (A2 §2.4: computed once
       per box, amortized over the yielded entries) */
    h->cx0 = x0;
    h->cy0 = y0;
    h->cx1 = x1;
    h->cy1 = y1;
    h->ckmin = kmin;
    h->ckmax = kmax;
    h->ccached = 1;
    return sn_hilbert_box_advance(h, x0, y0, x1, y1, kmax,
                                  sn_hilbert_lower_bound(h, kmin));
}

int64_t sn_hilbert_box_next(void *p, int64_t x0, int64_t y0, int64_t x1,
                            int64_t y1, int64_t idx) {
    SnHilbert *h = sn_hilbert_of(p, "hilbert_box_next");
    int64_t kmax;
    if (h->ccached && h->cx0 == x0 && h->cy0 == y0 && h->cx1 == x1 && h->cy1 == y1) {
        /* the same query's chain: the cached range (the O(1)
           amortized step — no boundary re-encoding per advance). */
        kmax = h->ckmax;
    } else {
        int64_t kmin;
        if (!sn_hilbert_box_range(h, x0, y0, x1, y1, &kmin, &kmax)) return -1;
        h->cx0 = x0;
        h->cy0 = y0;
        h->cx1 = x1;
        h->cy1 = y1;
        h->ckmin = kmin;
        h->ckmax = kmax;
        h->ccached = 1;
    }
    if (idx < 0 || idx >= h->len) {
        fprintf(stderr,
                "runtime error: hilbert box iterator position %lld out of "
                "range [0, %lld) — pass the index hilbert_box_first/next "
                "returned [E-SFC]\n",
                (long long)idx, (long long)h->len);
        exit(1);
    }
    return sn_hilbert_box_advance(h, x0, y0, x1, y1, kmax, idx + 1);
}


/* --------------------------------------------------------------------- */
/*  SnTape — the tape record kinds (SWARM RABBIT HOLE, Wave 16,          */
/*  Impl-U, Cohort Delta; W2C §36–§38 + §48 item 4 — the P5↔P2          */
/*  elevation plan made executable).                                     */
/*                                                                       */
/*  THE KINSHIP, RULED (W2C §36): the tape IS a δ₀-chronicle in        */
/*  SHAPE — discrete impulses against an integrated state, ordered     */
/*  by a clock, replay = re-integration from a checkpoint — and is     */
/*  NOT a calculus in algebra: no group on opaque entries, no ℐ/𝒟     */
/*  pair, no chain rule (§38's E3 is the named slice-2 boundary).      */
/*  The DELTA record kind is the elevation's slice-1 extent: the      */
/*  record carries the (prev → next) change against a LANDED carrier,  */
/*  the tape derives the impulse, CHECKS the tier-parametric law at    */
/*  this effect boundary (the record IS the boundary — the impulse    */
/*  is accepted only if the law holds), and stores the raw pairs.      */
/*                                                                       */
/*  THE THREE RECORD KINDS (a P2-sanctioned record kind, like wave's   */
/*  contrib — §48 item 4):                                               */
/*   TAPE_OPAQUE (0)  — the free-monoid chronicle: a raw int payload,  */
/*                      no algebra, no law (§36's honest baseline;     */
/*                      differentiation vacuous). Refused at replay —  */
/*                      an opaque record in the window has no          */
/*                      integral to contribute.                        */
/*   TAPE_DELTA (1)   — the (prev → next) carrier pair. The impulse    */
/*                      is derived with the carrier's own ops and the  */
/*                      LAW IS CHECKED HERE (§332's tier-parametric   */
/*                      per-operation laws):                           */
/*                        Tier G: d = b ⊖ a (sn_zset_delta — the       */
/*                          exact group ⊖); ILC's a ⊕ (b ⊖ a) = b     */
/*                          verified by reconstruction (sn_zset_apply  */
/*                          compared against b). The impulse stores    */
/*                          as raw (key, count) pairs — E2's          */
/*                          embeddability, LITERAL: the delta records  */
/*                          embed in the ℤ[X] multiplicity carrier.   */
/*                        Tier L: m^δ = sn_lcell_delta (the           */
/*                          join-decomposition-optimal fragment,      */
/*                          Almeida–Shoker–Baquero JPDC 2018); the    */
/*                          mutator law X ⊔ m^δ(X) = m(X) verified     */
/*                          (sn_lcell_apply compared via              */
/*                          sn_lcell_eq). NO ⊖ exists on this path     */
/*                          (the E-LRET precedent — the fragment is   */
/*                          ⊔-world state-relative material).         */
/*                        Tier O: NOTHING is checked because the kind  */
/*                          is REFUSED — no delta-local law at any     */
/*                          price (W2C §1.1; the checker's E-TAPE     */
/*                          arm fires first; this runtime arm is the */
/*                          raw-ptr backstop).                        */
/*   TAPE_CHECKPOINT (2) — the carrier STATE snapshot: the            */
/*                      materialized base; replay re-integrates from  */
/*                      the LAST one (§36's shape).                     */
/*                                                                       */
/*  ONE TAPE, ONE LEDGER (the P2↔wave boundary rule extended to P5,     */
/*  §4's E4): the first delta/checkpoint record fixes the tape's       */
/*  (tier, carrier); a record against a different carrier is a        */
/*  fail-fast [E-TAPE] refusal — the chronicle must stay one linear    */
/*  ledger of one carrier's evolution (the L replay's ⊔-fold is       */
/*  exact re-integration exactly under that discipline).               */
/*                                                                       */
/*  REPLAY (§36: replay = re-integration from the checkpoint):         */
/*   Tier G: the target is reset to the zero state (the group         */
/*     inverse — Tier-G's exact capability), the last checkpoint's     */
/*     snapshot pairs are applied, then every delta impulse in the     */
/*     window. Idempotent by exactness.                                 */
/*   Tier L: the snapshot's offers and every fragment's offers are     */
/*     ⊔-joined into the target (inflate only — the LVar discipline;  */
/*     NO reset exists on this surface). Idempotent by ⊔-idempotence   */
/*     (x⊔x = x — the checked lattice law). The fold into a FRESH ⊥    */
/*     cell is the exact re-integration witness; folding into a       */
/*     stateful cell is target ⊔ integral (the monotone capability).   */
/*                                                                       */
/*  LANE DISCIPLINE (E-GC-IN-LANE): the header, the record directory,  */
/*  and the flat payload buffer live on lane 0 (sn_lane_alloc — the   */
/*  hilbert precedent); the tape is NEVER gc_track'd and sn_release    */
/*  on it is a NO-OP (the runtime backstop above). The records hold   */
/*  RAW INT64 PAIRS ONLY — never GC pointers — so the chronicle is    */
/*  lane-safe by construction and dies at the lane's rewind. The      */
/*  derived d/m^δ objects are gc_track'd heap objects: built, law-    */
/*  checked, flattened onto the lane, and released HERE, inside the  */
/*  record call (the effect boundary) — no GC pointer ever crosses    */
/*  into the chronicle.                                                 */
/* --------------------------------------------------------------------- */

enum {
    TAPE_OPAQUE = 0,     /* the free-monoid chronicle (W2C §36)     */
    TAPE_DELTA = 1,     /* the (prev → next) carrier change         */
    TAPE_CHECKPOINT = 2 /* the carrier state snapshot                */
};
enum {
    TAPE_TIER_NONE = 0, /* nothing classified yet                    */
    TAPE_TIER_G = 1,    /* the ℤ[X] carrier: exact ⊖, ILC law       */
    TAPE_TIER_L = 2     /* the ⊔ carriers: the mutator law, no ⊖    */
};

typedef struct SnTapeRec {
    int64_t kind; /* TAPE_OPAQUE / TAPE_DELTA / TAPE_CHECKPOINT */
    int64_t off;  /* the payload region's start (int64 units)     */
    int64_t len;  /* the payload region's length (int64 units)    */
} SnTapeRec;

typedef struct SnTape {
    int64_t rc;      /* unused on the lane (no ARC) */
    int64_t kind;    /* KIND_TAPE */
    int64_t tier;    /* learned at the first delta/checkpoint (one tape, one ledger) */
    int64_t carrier; /* G: the zset key_kind; L: the lcell carrier */
    int64_t nrec;
    int64_t cap;
    SnTapeRec *recs;   /* lane: the record directory */
    int64_t plen;
    int64_t pcap;
    int64_t *payloads; /* lane: the flat (a, b) pair buffer — the impulses */
    char name[32];    /* the chronicle's label (diagnostics; NUL-terminated) */
} SnTape;

static SnTape *tape_of(void *p, const char *op) {
    SnTape *t = (SnTape *)p;
    if (!t || t->kind != KIND_TAPE) {
        fprintf(stderr,
                "runtime error: %s operates on a tape (the lane-backed "
                "chronicle header); the value is not one [E-TAPE]\n",
                op);
        exit(1);
    }
    return t;
}

/* The lane-bump growth discipline (the hilbert grow precedent): the
   old block is abandoned to the rewind — lane bump memory, never
   freed in place; amortized O(1) per append. */
static void tape_grow_recs(SnTape *t) {
    if (t->nrec < t->cap) return;
    int64_t ncap = t->cap ? t->cap * 2 : 16;
    SnTapeRec *nr = (SnTapeRec *)sn_lane_alloc_fwd(0, ncap * (int64_t)sizeof(SnTapeRec));
    if (t->nrec > 0) {
        memcpy(nr, t->recs, (size_t)t->nrec * sizeof(SnTapeRec));
    }
    t->recs = nr;
    t->cap = ncap;
}

static void tape_grow_payload(SnTape *t, int64_t need) {
    if (t->plen + need <= t->pcap) return;
    int64_t ncap = t->pcap ? t->pcap : 64;
    while (ncap < t->plen + need) ncap *= 2;
    int64_t *np = (int64_t *)sn_lane_alloc_fwd(0, ncap * (int64_t)sizeof(int64_t));
    if (t->plen > 0) {
        memcpy(np, t->payloads, (size_t)t->plen * sizeof(int64_t));
    }
    t->payloads = np;
    t->pcap = ncap;
}

/* The common append: one record header + the raw payload pairs. */
static void tape_append(SnTape *t, int64_t kind, const int64_t *pairs, int64_t npairs) {
    tape_grow_payload(t, npairs * 2);
    SnTapeRec r;
    r.kind = kind;
    r.off = t->plen;
    r.len = npairs * 2;
    for (int64_t i = 0; i < npairs; i++) {
        t->payloads[t->plen + 2 * i] = pairs[2 * i];
        t->payloads[t->plen + 2 * i + 1] = pairs[2 * i + 1];
    }
    t->plen += npairs * 2;
    tape_grow_recs(t);
    t->recs[t->nrec] = r;
    t->nrec++;
}

/* ONE TAPE, ONE LEDGER: the first delta/checkpoint fixes the
   (tier, carrier); any later divergence is the fail-fast refusal. */
static void tape_learn(SnTape *t, int64_t tier, int64_t carrier, const char *op) {
    if (t->tier == TAPE_TIER_NONE) {
        t->tier = tier;
        t->carrier = carrier;
        return;
    }
    if (t->tier != tier || t->carrier != carrier) {
        fprintf(stderr,
                "runtime error: %s on tape '%s': one tape, one ledger — the "
                "tape already chronicles a %s ledger and this record is %s "
                "(the P2↔wave boundary rule extended to P5, W2C §4 E4; keep "
                "one carrier per tape) [E-TAPE]\n",
                op, t->name,
                t->tier == TAPE_TIER_G ? "Tier-G" : "Tier-L",
                tier == TAPE_TIER_G ? "Tier-G" : "Tier-L");
        exit(1);
    }
}

/* Structural Z-set equality (the law check's comparator): every key
   of either side carries the same coefficient on both. */
static int tape_zset_eq(void *a, void *b) {
    SnZSet *x = (SnZSet *)a;
    SnZSet *y = (SnZSet *)b;
    if (sn_zset_total(x) != sn_zset_total(y)) return 0;
    for (int64_t i = 0; i < x->len; i++) {
        if (sn_zset_count(x, x->keys[i]) != sn_zset_count(y, x->keys[i])) return 0;
    }
    for (int64_t i = 0; i < y->len; i++) {
        if (sn_zset_count(y, y->keys[i]) != sn_zset_count(x, y->keys[i])) return 0;
    }
    return 1;
}

/* Flatten a Z-set into raw (key, count) pairs — the ℤ[X] element as
   lane bytes (E2's embeddability, literal). Zero coefficients are
   skipped (the canonical finite-support form). */
static void tape_zset_flatten(SnZSet *z, int64_t **out, int64_t *nout) {
    int64_t cap = z->len > 0 ? z->len : 1;
    int64_t *buf = (int64_t *)xmalloc(sizeof(int64_t) * (size_t)cap * 2);
    int64_t n = 0;
    for (int64_t i = 0; i < z->len; i++) {
        if (z->counts[i] == 0) continue;
        buf[2 * n] = z->keys[i];
        buf[2 * n + 1] = z->counts[i];
        n++;
    }
    *out = buf;
    *nout = n;
}

/* Flatten an LCell's state/fragment into raw (a, b) offer pairs
   (the lcell_inflate payload form — one shape for state and
   fragment: MAX (hwm, 0); GSET (elem, 0)*; LWW (val, clock)). */
static void tape_lcell_flatten(SnLCell *c, int64_t **out, int64_t *nout) {
    int64_t cap = 8;
    if (c->carrier == LCELL_GSET && c->len > cap) cap = c->len;
    int64_t *buf = (int64_t *)xmalloc(sizeof(int64_t) * (size_t)cap * 2);
    int64_t n = 0;
    switch (c->carrier) {
    case LCELL_MAX:
        if (c->hwm != INT64_MIN) {
            buf[0] = c->hwm;
            buf[1] = 0;
            n = 1;
        }
        break;
    case LCELL_GSET:
        for (int64_t i = 0; i < c->len; i++) {
            buf[2 * i] = c->elems[i];
            buf[2 * i + 1] = 0;
        }
        n = c->len;
        break;
    case LCELL_LWW:
        if (c->lww_set) {
            buf[0] = c->lww_val;
            buf[1] = c->lww_clock;
            n = 1;
        }
        break;
    }
    *out = buf;
    *nout = n;
}

void *sn_tape_new(void *name) {
    Str *s = (Str *)name;
    int64_t nlen = 0;
    const char *nd = "";
    if (s && s->kind == KIND_STR && s->len >= 0) {
        nlen = s->len;
        nd = s->data;
    }
    if (nlen > 31) {
        fprintf(stderr,
                "runtime error: tape name longer than 31 bytes (got %lld) — "
                "the label is copied into the lane-backed header "
                "[E-TAPE]\n",
                (long long)nlen);
        exit(1);
    }
    SnTape *t = (SnTape *)sn_lane_alloc_fwd(0, (int64_t)sizeof(SnTape));
    t->rc = 1;
    t->kind = KIND_TAPE;
    t->tier = TAPE_TIER_NONE;
    t->carrier = 0;
    t->nrec = 0;
    t->cap = 0;
    t->recs = NULL;
    t->plen = 0;
    t->pcap = 0;
    t->payloads = NULL;
    memset(t->name, 0, sizeof(t->name));
    for (int64_t i = 0; i < nlen; i++) t->name[i] = nd[i];
    return t;
}

/* The OPAQUE record (kind 0): the int payload, no algebra, no law.
   The i64 ABI (the int payload); the p ABI below carries kinds 1/2. */
void sn_tape_record_i(void *p, int64_t kind, int64_t payload) {
    SnTape *t = tape_of(p, "tape_record");
    if (kind != TAPE_OPAQUE) {
        fprintf(stderr,
                "runtime error: unknown tape record kind %lld — 0=opaque, "
                "1=delta, 2=checkpoint [E-TAPE]\n",
                (long long)kind);
        exit(1);
    }
    int64_t pair[2];
    pair[0] = payload;
    pair[1] = 0;
    tape_append(t, TAPE_OPAQUE, pair, 1);
}

/* The DELTA and CHECKPOINT records (kinds 1/2) — the ptr ABI. THE
   EFFECT BOUNDARY: the impulse is derived with the carrier's own
   ops and the TIER-PARAMETRIC LAW IS CHECKED before the record is
   accepted. */
void sn_tape_record_p(void *p, int64_t kind, void *a, void *b) {
    SnTape *t = tape_of(p, "tape_record");
    if (kind != TAPE_DELTA && kind != TAPE_CHECKPOINT) {
        fprintf(stderr,
                "runtime error: unknown tape record kind %lld — 0=opaque, "
                "1=delta, 2=checkpoint [E-TAPE]\n",
                (long long)kind);
        exit(1);
    }
    Obj *oa = (Obj *)a;
    Obj *ob = (Obj *)b;
    if (kind == TAPE_DELTA) {
        /* The tier-confusion backstop (E-LKIND's seam, the raw-ptr
           case the checker's origin classification cannot see): both
           endpoints must be live carriers of the SAME kind. */
        if (!oa || !ob || (oa->kind != KIND_ZSET && oa->kind != KIND_LCELL)
            || oa->kind != ob->kind) {
            fprintf(stderr,
                    "runtime error: a delta record needs a same-tier "
                    "(prev, next) carrier pair — got kind=%lld and kind=%lld "
                    "(the Tier-G ⊕ and the Tier-L ⊔ do not mix; the checker's "
                    "classification fired first on the classified shapes) "
                    "[E-TAPE]\n",
                    oa ? (long long)oa->kind : -1, ob ? (long long)ob->kind : -1);
            exit(1);
        }
        if (oa->kind == KIND_ZSET) {
            /* ---- Tier G: ILC's a ⊕ (b ⊖ a) = b, checked ---- */
            SnZSet *prev = (SnZSet *)a;
            SnZSet *next = (SnZSet *)b;
            if (prev->key_kind != next->key_kind) {
                fprintf(stderr,
                        "runtime error: a delta record's Z-sets must share the "
                        "key kind (got %lld and %lld) [E-TAPE]\n",
                        (long long)prev->key_kind, (long long)next->key_kind);
                exit(1);
            }
            tape_learn(t, TAPE_TIER_G, prev->key_kind, "tape_record");
            void *d = sn_zset_delta(next, prev); /* b ⊖ a */
            void *chk = sn_zset_apply(prev, d);  /* a ⊕ (b ⊖ a) */
            if (!tape_zset_eq(chk, next)) {
                fprintf(stderr,
                        "runtime error: tape law violated on the recorded "
                        "triple: a ⊕ (b ⊖ a) != b (ILC's change-structure law "
                        "at the effect boundary, W2C §38 E1) [E-TAPE]\n");
                exit(1);
            }
            int64_t *pairs;
            int64_t npairs;
            tape_zset_flatten((SnZSet *)d, &pairs, &npairs);
            tape_append(t, TAPE_DELTA, pairs, npairs);
            free(pairs);
            sn_release(d);
            sn_release(chk);
        } else {
            /* ---- Tier L: the mutator law X ⊔ m^δ(X) = m(X), checked
               (NO ⊖ on this path — the E-LRET precedent; the fragment
               is sn_lcell_delta's join-decomposition-optimal material) ---- */
            lcell_guard_pair(a, b, "tape_record");
            tape_learn(t, TAPE_TIER_L, ((SnLCell *)a)->carrier, "tape_record");
            void *m = sn_lcell_delta(b, a);      /* m^δ(m(X), X)     */
            void *chk = sn_lcell_apply(a, m);    /* X ⊔ m^δ(X)       */
            if (!sn_lcell_eq(chk, b)) {
                fprintf(stderr,
                        "runtime error: tape law violated on the recorded "
                        "pair: X ⊔ m^δ(X) != m(X) (the delta-mutator law at "
                        "the effect boundary, Almeida–Shoker–Baquero JPDC "
                        "2018; W2C §1.1 Tier L) [E-TAPE]\n");
                exit(1);
            }
            int64_t *pairs;
            int64_t npairs;
            tape_lcell_flatten((SnLCell *)m, &pairs, &npairs);
            tape_append(t, TAPE_DELTA, pairs, npairs);
            free(pairs);
            sn_release(m);
            sn_release(chk);
        }
        return;
    }
    /* ---- The CHECKPOINT record: the carrier state snapshot. No law
       to check — the snapshot IS the base; the classification must
       still be a carrier (Tier O has nothing to snapshot). ---- */
    if (!oa || (oa->kind != KIND_ZSET && oa->kind != KIND_LCELL)) {
        fprintf(stderr,
                "runtime error: a checkpoint is a carrier snapshot (a Tier-G "
                "Z-set or a Tier-L cell) — got kind=%lld [E-TAPE]\n",
                oa ? (long long)oa->kind : -1);
        exit(1);
    }
    if (oa->kind == KIND_ZSET) {
        SnZSet *z = (SnZSet *)a;
        tape_learn(t, TAPE_TIER_G, z->key_kind, "tape_record");
        int64_t *pairs;
        int64_t npairs;
        tape_zset_flatten(z, &pairs, &npairs);
        tape_append(t, TAPE_CHECKPOINT, pairs, npairs);
        free(pairs);
    } else {
        lcell_guard_one(a, "tape_record");
        tape_learn(t, TAPE_TIER_L, ((SnLCell *)a)->carrier, "tape_record");
        int64_t *pairs;
        int64_t npairs;
        tape_lcell_flatten((SnLCell *)a, &pairs, &npairs);
        tape_append(t, TAPE_CHECKPOINT, pairs, npairs);
        free(pairs);
    }
}

int64_t sn_tape_len(void *p) {
    SnTape *t = tape_of(p, "tape_len");
    return t->nrec;
}

int64_t sn_tape_rec_kind(void *p, int64_t i) {
    SnTape *t = tape_of(p, "tape_rec_kind");
    if (i < 0 || i >= t->nrec) {
        fprintf(stderr,
                "runtime error: tape record index %lld out of range (the tape "
                "holds %lld records) [E-TAPE]\n",
                (long long)i, (long long)t->nrec);
        exit(1);
    }
    return t->recs[i].kind;
}

int64_t sn_tape_rec_payload(void *p, int64_t i) {
    SnTape *t = tape_of(p, "tape_rec_payload");
    if (i < 0 || i >= t->nrec) {
        fprintf(stderr,
                "runtime error: tape record index %lld out of range (the tape "
                "holds %lld records) [E-TAPE]\n",
                (long long)i, (long long)t->nrec);
        exit(1);
    }
    if (t->recs[i].kind != TAPE_OPAQUE) {
        fprintf(stderr,
                "runtime error: tape_rec_payload reads the OPAQUE payload — "
                "record %lld is kind %lld (a delta/checkpoint record's meaning "
                "is its carrier ops at replay, not a raw int; W2C §36) "
                "[E-TAPE]\n",
                (long long)i, (long long)t->recs[i].kind);
        exit(1);
    }
    return t->payloads[t->recs[i].off];
}

/* tape_replay — replay = re-integration from the last checkpoint
   (W2C §36's shape, executable). Tier G: the target is reset to the
   zero state (the group inverse — Tier-G's exact capability), then
   the base + window impulses apply. Tier L: the base + window
   offers ⊔-join into the target (inflate only — no reset exists on
   the L surface; idempotent by ⊔-idempotence). An OPAQUE record in
   the window is the honest REFUSAL: no algebra, no integral. */
void sn_tape_replay(void *p, void *target) {
    SnTape *t = tape_of(p, "tape_replay");
    Obj *og = (Obj *)target;
    if (!og || (og->kind != KIND_ZSET && og->kind != KIND_LCELL)) {
        fprintf(stderr,
                "runtime error: tape_replay folds into a carrier (a Tier-G "
                "Z-set or a Tier-L cell) — got kind=%lld [E-TAPE]\n",
                og ? (long long)og->kind : -1);
        exit(1);
    }
    /* The one-ledger discipline: a tape that learned a tier replays
       only into that tier's carrier (the raw-ptr backstop of the
       checker's classification). */
    int64_t want = og->kind == KIND_ZSET ? TAPE_TIER_G : TAPE_TIER_L;
    if (t->tier != TAPE_TIER_NONE && t->tier != want) {
        fprintf(stderr,
                "runtime error: tape_replay on tape '%s': one tape, one "
                "ledger — the tape chronicles a %s ledger and the target is a "
                "%s carrier [E-TAPE]\n",
                t->name,
                t->tier == TAPE_TIER_G ? "Tier-G" : "Tier-L",
                want == TAPE_TIER_G ? "Tier-G" : "Tier-L");
        exit(1);
    }
    /* The window: everything after the LAST checkpoint (or the whole
       tape when none exists — the zero state is the base). */
    int64_t c = -1;
    for (int64_t i = 0; i < t->nrec; i++) {
        if (t->recs[i].kind == TAPE_CHECKPOINT) c = i;
    }
    if (og->kind == KIND_ZSET) {
        /* The exact reset: subtract the target's whole support (the
           group inverse; pruned to the zero state). */
        List *keys = (List *)sn_zset_keys(target);
        for (int64_t i = 0; i < keys->len; i++) {
            int64_t k = keys->items[i];
            sn_zset_add(target, k, -sn_zset_count(target, k));
        }
        sn_release(keys);
        if (c >= 0) {
            SnTapeRec *r = &t->recs[c];
            for (int64_t j = 0; j < r->len; j += 2) {
                sn_zset_add(target, t->payloads[r->off + j],
                            t->payloads[r->off + j + 1]);
            }
        }
        for (int64_t i = (c >= 0 ? c + 1 : 0); i < t->nrec; i++) {
            SnTapeRec *r = &t->recs[i];
            if (r->kind == TAPE_DELTA) {
                for (int64_t j = 0; j < r->len; j += 2) {
                    sn_zset_add(target, t->payloads[r->off + j],
                                t->payloads[r->off + j + 1]);
                }
            } else if (r->kind == TAPE_OPAQUE) {
                fprintf(stderr,
                        "runtime error: tape_replay hit an opaque record in "
                        "the replay window — opaque payloads have no algebra "
                        "(W2C §36: a free monoid at best, differentiation "
                        "vacuous) and cannot integrate; keep opaque and delta "
                        "records on separate tapes, or checkpoint before the "
                        "opaque run [E-TAPE]\n");
                exit(1);
            }
        }
    } else {
        /* The ⊔-fold: offers only, never a reset (the LVar
           discipline). A fresh ⊥ cell receives exactly the integral.
           THE TIER ASYMMETRY, honestly refused: Tier-G deltas are
           ABSOLUTE (the group difference — a zero base replays
           them exactly); Tier-L fragments are STATE-RELATIVE (the
           mutator law joins m^δ onto ITS base), so a replay without
           a CHECKPOINT has no base — the fold would silently
           under-approximate, and the honest answer is the refusal. */
        lcell_guard_one(target, "tape_replay");
        if (c < 0) {
            fprintf(stderr,
                    "runtime error: tape_replay on tape '%s': the Tier-L "
                    "fragments are state-relative (the mutator law joins "
                    "m^δ onto its base) and this tape has no checkpoint to "
                    "re-integrate from — record tape_record(t, 2, state) as "
                    "the base first; the Tier-G deltas are absolute and "
                    "replay from the zero state, the Tier-L fragments are "
                    "not [E-TAPE]\n",
                    t->name);
            exit(1);
        }
        SnTapeRec *r = &t->recs[c];
        for (int64_t j = 0; j < r->len; j += 2) {
            sn_lcell_inflate(target, t->payloads[r->off + j],
                             t->payloads[r->off + j + 1]);
        }
        for (int64_t i = c + 1; i < t->nrec; i++) {
            SnTapeRec *rr = &t->recs[i];
            if (rr->kind == TAPE_DELTA) {
                for (int64_t j = 0; j < rr->len; j += 2) {
                    sn_lcell_inflate(target, t->payloads[rr->off + j],
                                     t->payloads[rr->off + j + 1]);
                }
            } else if (rr->kind == TAPE_OPAQUE) {
                fprintf(stderr,
                        "runtime error: tape_replay hit an opaque record in "
                        "the replay window — opaque payloads have no algebra "
                        "(W2C §36: a free monoid at best, differentiation "
                        "vacuous) and cannot integrate; keep opaque and delta "
                        "records on separate tapes, or checkpoint before the "
                        "opaque run [E-TAPE]\n");
                exit(1);
            }
        }
    }
}


/* --------------------------------------------------------------------- */
/*  SnZFix — Salt ζ: the fixpoint-bracket engine (SWARM RABBIT HOLE,     */
/*  Wave 9, Impl-N, Cohort Delta; W2C §3, the δ₀/∫ bracket).             */
/*                                                                       */
/*  The ADMITTED body is TRANSITIVE CLOSURE: reach(a) :- edge(a,b),      */
/*  reach(b) — MONOTONE in the recursive variable (adding reached        */
/*  nodes can only add derived nodes) over a FINITE-HEIGHT per-key       */
/*  carrier: presence {0,1} on the keys of the FINITE edge universe      */
/*  (sets over a finite universe qualify; ℤ-valued recursion through    */
/*  the recursive variable does not — the counting-through-recursion     */
/*  rejection, E-ZADM). The convergence theorem therefore applies       */
/*  (W2C §3.3): the bracket terminates in ≤ height rounds at the LEAST   */
/*  fixpoint (Knaster–Tarski; Cousot–Cousot Pacific J. Math. 82(1)        */
/*  1979). Correctness is conditional on termination (W2C §3.5): an      */
/*  admitted body terminates, and what it terminates at is the least    */
/*  fixpoint; divergent bodies never reach the runtime — refusals are    */
/*  compile-time errors (E-ZADM), never silent divergence.               */
/*                                                                       */
/*  The round loop — the bracket, executed (W2C §3.2):                   */
/*      δ₀ opens the round's delta; T^Δ applies against the accumulated  */
/*      integral ∫; the next round's δ₀ carries only what is newly       */
/*      derived. Each round is SEMI-NAÏVE (Thm 4b: the                  */
/*      incrementalized fixpoint IS classical semi-naïve evaluation —   */
/*      Bancilhon–Ramakrishnan SIGMOD 1986, derived from the calculus): */
/*      derive only from the PREVIOUS round's delta, never the full     */
/*      state. `probes` counts the adjacency visits — each round's      */
/*      work is O(|δ^(r-1)| + Σ indeg over the delta), the per-round     */
/*      term of W_μ = Σ_rounds c·(|δ₀^(r)| + Σ_v |Δv^(r)|) (W2C §3.4:    */
/*      the ROUND-SUM, not bounded by |ΔDB| — one seed can trigger      */
/*      Θ(n) re-derivation rounds).                                     */
/*                                                                       */
/*  The per-tier QUIESCENCE TESTS, both executable (W4C Repair 3's      */
/*  per-tier form):                                                      */
/*    Tier G: 𝒟(∫α) = 0 — the round's delta Z-set is empty (len 0).     */
/*    Tier L: F(s) ⊑ s — the MUTATOR-LAW test: the round's state-       */
/*      relative fragment (sn_lcell_delta; delta(x,x) = ⊥, no          */
/*      inflation) is empty. The tests differ BECAUSE the algebras      */
/*      differ: 𝒟 presupposes ⊖, which does not and cannot exist on     */
/*      Tier L (Grothendieck triviality — E-LRET); the mutator law is   */
/*      the semilattice tier's own honest form of "nothing new".        */
/*                                                                       */
/*  Tier L (sn_zfix_new_l): the same body over a GSET LCell carrier —   */
/*  ⊔ (union) is the join, so the carrier qualifies BY CONSTRUCTION    */
/*  (W2C §3.3's admission rule — the premise Impl-M's SnLCell landed). */
/*  The round's true delta is extracted BY the mutator law itself       */
/*  (lcell_delta between the folded state and its pre-round clone),     */
/*  and quiescence is that fragment being ⊥.                             */
/*                                                                       */
/*  Representation: the edge relation is a Z-set of PACKED PAIRS —      */
/*  key = a·2³² + b with a, b ∈ [0, 2³²) (plain int arithmetic on the   */
/*  surface); coefficient ≠ 0 means present (the AGG_COUNT presence     */
/*  convention). The constructor builds the body's join index — the    */
/*  CSR adjacency target → sources — in one O(|E|) materialization      */
/*  pass (outside every per-round claim, the sn_view_new ℐ precedent).  */
/*                                                                       */
/*  GC: unlike SnView (untracked; its C harness never collects         */
/*  mid-circuit), the ζ engine is a FIRST-CLASS TRACKED OBJECT          */
/*  (KIND_ZFIX): SNlang programs trigger threshold collections that    */
/*  can fire mid-bracket, and the conservative stack scan sees the     */
/*  engine's raw ptr but not the carriers behind the heap               */
/*  indirection — the gc_mark arm traces them (see gc_mark above).     */
/* --------------------------------------------------------------------- */

enum {
    SN_ZFIX_TIER_G = 0, /* reach is a Z-set: presence {0,1} coefficients  */
    SN_ZFIX_TIER_L = 1  /* reach is a GSET LCell: ⊔ = union is the join  */
};

/* --------------------------------------------------------------------- */
/* (The SnZFixAdj / SnZFix structs are defined before gc_mark, above —
 *  the collector traces the engine's carriers by kind. The engine
 *  proper starts here.)                                                   */

static void zfix_adj_rehash(SnZFixAdj *j, int64_t new_hcap) {
    int64_t *nb = (int64_t *)calloc((size_t)new_hcap, sizeof(int64_t));
    if (!nb) {
        fprintf(stderr, "out of memory\n");
        exit(1);
    }
    uint64_t mask = (uint64_t)new_hcap - 1;
    for (int64_t i = 0; i < j->len; i++) {
        uint64_t slot = j->hashes[i] & mask;
        while (nb[slot] != 0) {
            slot = (slot + 1) & mask;
        }
        nb[slot] = i + 1;
    }
    free(j->buckets);
    j->buckets = nb;
    j->hcap = new_hcap;
}

static int64_t zfix_adj_find(SnZFixAdj *j, int64_t b, uint64_t h, uint64_t *out_bucket) {
    uint64_t mask = (uint64_t)j->hcap - 1;
    uint64_t slot = h & mask;
    for (;;) {
        int64_t bk = j->buckets[slot];
        if (bk == 0) {
            if (out_bucket) *out_bucket = slot;
            return -1;
        }
        int64_t idx = bk - 1;
        if (j->hashes[idx] == h && j->nodes[idx] == b) {
            if (out_bucket) *out_bucket = slot;
            return idx;
        }
        slot = (slot + 1) & mask;
    }
}

/* One O(|E|) materialization of the join index from the packed-pair
 * edge Z-set. Two passes over the edge support; cnt doubles as the
 * backward fill cursor (it returns to the true counts). */
static void zfix_adj_build(SnZFixAdj *j, SnZSet *edges) {
    memset(j, 0, sizeof(*j));
    j->cap = 8;
    j->hcap = 16;
    j->nodes = (int64_t *)xmalloc(sizeof(int64_t) * 8);
    j->cnt = (int64_t *)xmalloc(sizeof(int64_t) * 8);
    j->hashes = (uint64_t *)xmalloc(sizeof(uint64_t) * 8);
    j->buckets = (int64_t *)calloc(16, sizeof(int64_t));
    if (!j->buckets) {
        fprintf(stderr, "out of memory\n");
        exit(1);
    }
    if (!edges) return;
    /* pass 1: count in-edges per target (find-or-insert targets) */
    for (int64_t i = 0; i < edges->len; i++) {
        if (edges->counts[i] == 0) continue; /* presence: coefficient ≠ 0 */
        uint64_t u = (uint64_t)edges->keys[i];
        int64_t b = (int64_t)(u & 0xFFFFFFFFULL);
        if ((j->len + 1) * 4 >= j->hcap * 3) {
            zfix_adj_rehash(j, j->hcap * 2);
        }
        uint64_t h = key_hash(0, b);
        uint64_t bucket = 0;
        int64_t idx = zfix_adj_find(j, b, h, &bucket);
        if (idx < 0) {
            if (j->len == j->cap) {
                j->cap *= 2;
                j->nodes = (int64_t *)realloc(j->nodes, sizeof(int64_t) * (size_t)j->cap);
                j->cnt = (int64_t *)realloc(j->cnt, sizeof(int64_t) * (size_t)j->cap);
                j->hashes = (uint64_t *)realloc(j->hashes, sizeof(uint64_t) * (size_t)j->cap);
            }
            j->nodes[j->len] = b;
            j->cnt[j->len] = 0;
            j->hashes[j->len] = h;
            j->buckets[bucket] = j->len + 1;
            idx = j->len;
            j->len++;
        }
        j->cnt[idx]++;
        j->total++;
    }
    /* prefix sums: off[b] .. off[b+1] bounds target b's in-neighbors */
    j->off = (int64_t *)xmalloc(sizeof(int64_t) * (size_t)(j->len + 1));
    int64_t acc = 0;
    for (int64_t i = 0; i < j->len; i++) {
        j->off[i] = acc;
        acc += j->cnt[i];
    }
    j->off[j->len] = acc;
    j->buf = (int64_t *)xmalloc(sizeof(int64_t) * (size_t)(acc > 0 ? acc : 1));
    /* pass 2: fill backwards off cnt (the cursor returns to 0; the
       true count is recovered as off[idx+1] - off[idx]) */
    for (int64_t i = 0; i < edges->len; i++) {
        if (edges->counts[i] == 0) continue;
        uint64_t u = (uint64_t)edges->keys[i];
        int64_t a = (int64_t)(u >> 32);
        int64_t b = (int64_t)(u & 0xFFFFFFFFULL);
        int64_t idx = zfix_adj_find(j, b, key_hash(0, b), NULL);
        j->cnt[idx]--;
        j->buf[j->off[idx] + j->cnt[idx]] = a;
    }
}

/* --------------------------------------------------------------------- */

static SnZFix *zfix_guard(void *p, const char *op) {
    if (!p) return NULL;
    Obj *o = (Obj *)p;
    if (o->kind != KIND_ZFIX) {
        fprintf(stderr,
                "runtime error: zfix_%s expects a Salt-ζ fixpoint engine "
                "(kind=%d), got kind=%lld [E-LKIND]\n",
                op, KIND_ZFIX, (long long)o->kind);
        exit(1);
    }
    return (SnZFix *)p;
}

static SnZFix *zfix_new_tiered(SnZSet *edges, SnZSet *seed, int64_t tier) {
    if ((edges && edges->key_kind != 0) || (seed && seed->key_kind != 0)) {
        fprintf(stderr,
                "runtime error: the ζ bracket's node universe is int64 "
                "(packed-pair edge keys a·2³² + b); a string-keyed Z-set "
                "cannot be an edge relation or a seed [E-LKIND]\n");
        exit(1);
    }
    SnZFix *f = (SnZFix *)xmalloc(sizeof(SnZFix));
    f->rc = 1;
    f->kind = KIND_ZFIX;
    f->tier = tier;
    gc_track(f, (int64_t)sizeof(SnZFix));
    zfix_adj_build(&f->adj, edges);
    f->edges = edges;
    if (edges) sn_retain(f->edges);
    f->seed = seed;
    if (seed) sn_retain(f->seed);
    f->state = NULL;
    f->delta = NULL;
    f->state_l = NULL;
    f->delta_l = NULL;
    f->rounds = 0;
    f->probes = 0;
    f->done = 0;
    if (tier == SN_ZFIX_TIER_G) {
        f->state = (SnZSet *)sn_zset_new(0);
        f->delta = (SnZSet *)sn_zset_new(0);
        if (seed) {
            for (int64_t i = 0; i < seed->len; i++) {
                if (seed->counts[i] == 0) continue; /* presence ≠ 0 */
                if (sn_zset_count(f->state, seed->keys[i]) == 0) {
                    sn_zset_add(f->state, seed->keys[i], 1); /* ∫ */
                    sn_zset_add(f->delta, seed->keys[i], 1); /* δ₀ */
                }
            }
        }
    } else {
        f->state_l = (SnLCell *)sn_lcell_new(LCELL_GSET);
        f->delta_l = (SnLCell *)sn_lcell_new(LCELL_GSET);
        if (seed) {
            for (int64_t i = 0; i < seed->len; i++) {
                if (seed->counts[i] == 0) continue;
                lcell_gset_insert(f->state_l, seed->keys[i]);
                lcell_gset_insert(f->delta_l, seed->keys[i]);
            }
        }
    }
    return f;
}

void *sn_zfix_new(void *edges, void *seed) {
    return zfix_new_tiered((SnZSet *)edges, (SnZSet *)seed, SN_ZFIX_TIER_G);
}

void *sn_zfix_new_l(void *edges, void *seed) {
    return zfix_new_tiered((SnZSet *)edges, (SnZSet *)seed, SN_ZFIX_TIER_L);
}

/* One Tier-G round — semi-naïve (Thm 4b): derive ONLY from the
 * previous round's delta. Returns |δ^(r)| — the Tier-G quiescence
 * test 𝒟(∫α) = 0 is exactly this being 0. */
static int64_t zfix_round_g(SnZFix *f) {
    SnZSet *d = f->delta;
    SnZSet *next = (SnZSet *)sn_zset_new(0);
    for (int64_t i = 0; i < d->len; i++) {
        int64_t b = d->keys[i];
        int64_t idx = zfix_adj_find(&f->adj, b, key_hash(0, b), NULL);
        if (idx < 0) continue; /* no in-edges: nothing derives */
        for (int64_t k = f->adj.off[idx]; k < f->adj.off[idx + 1]; k++) {
            f->probes++; /* one adjacency visit — the counted work unit */
            int64_t a = f->adj.buf[k];
            if (sn_zset_count(f->state, a) == 0) { /* presence: 0→1 only */
                sn_zset_add(f->state, a, 1); /* ∫: the integral fold */
                sn_zset_add(next, a, 1);      /* δ₀ of the next round */
            }
        }
    }
    sn_release(d);
    f->delta = next;
    return next->len;
}

/* One Tier-L round — the same semi-naïve mechanics over the GSET
 * carrier. The round's delta is the STATE-RELATIVE fragment (the
 * mutator law's m^δ): each candidate joins the state by ⊔ ONLY if new
 * (the F(s) ⊑ s guard), so the fragment collects exactly what moved —
 * which is precisely lcell_delta(state_after, state_before), computed
 * membership-guarded in O(|δ|) (the Thm 4b round) instead of by the
 * O(|state|) scan the ⊖-less carrier would otherwise need. Tier L has
 * no ⊖ (Grothendieck triviality — E-LRET), so 𝒟(∫α) = 0 is not the
 * test here; the stop condition is the mutator-law form: the fragment
 * being ⊥ (no inflation ⟺ F(s) ⊑ s ⟺ delta(x,x) = ⊥). */
static int64_t zfix_round_l(SnZFix *f) {
    SnLCell *d = f->delta_l;
    SnLCell *frag = (SnLCell *)sn_lcell_new(LCELL_GSET); /* the round's m^δ */
    for (int64_t i = 0; i < d->len; i++) {
        int64_t b = d->elems[i];
        int64_t idx = zfix_adj_find(&f->adj, b, key_hash(0, b), NULL);
        if (idx < 0) continue;
        for (int64_t k = f->adj.off[idx]; k < f->adj.off[idx + 1]; k++) {
            f->probes++;
            int64_t a = f->adj.buf[k];
            if (!sn_lcell_has(f->state_l, a)) { /* only NEW facts move */
                lcell_gset_insert(f->state_l, a); /* ⊔: the integral fold */
                lcell_gset_insert(frag, a);       /* the state-relative fragment */
            }
        }
    }
    sn_release(d);
    f->delta_l = frag;
    return sn_lcell_len(frag);
}

/* Advance ONE round. Returns the round's derived count (0 = the stop
 * test fired). `rounds` counts PRODUCTIVE rounds only: the terminating
 * empty round is the quiescence observation, not a derivation — so a
 * seed-only graph reports 0 rounds and the ≤ height bound holds with
 * equality on a chain. */
int64_t sn_zfix_round(void *p) {
    SnZFix *f = zfix_guard(p, "round");
    if (!f || f->done) return 0;
    int64_t derived = (f->tier == SN_ZFIX_TIER_G) ? zfix_round_g(f)
                                                  : zfix_round_l(f);
    if (derived > 0) {
        f->rounds++;
    } else {
        f->done = 1;
    }
    return derived;
}

/* The full δ₀/∫ bracket: rounds until quiescence. Returns the
 * productive round count (≤ the carrier height; for closure, exactly
 * the longest derivation chain). */
int64_t sn_zfix_run(void *p) {
    SnZFix *f = zfix_guard(p, "run");
    if (!f) return 0;
    while (!f->done) {
        sn_zfix_round(f);
    }
    return f->rounds;
}

/* -------------------------------- readouts ---------------------------- */

int64_t sn_zfix_rounds(void *p) {
    SnZFix *f = zfix_guard(p, "rounds");
    return f ? f->rounds : 0;
}

int64_t sn_zfix_probes(void *p) {
    SnZFix *f = zfix_guard(p, "probes");
    return f ? f->probes : 0;
}

/* |reach| — the least fixpoint's support size, per tier. */
int64_t sn_zfix_len(void *p) {
    SnZFix *f = zfix_guard(p, "len");
    if (!f) return 0;
    return (f->tier == SN_ZFIX_TIER_G) ? sn_zset_len(f->state)
                                       : sn_lcell_len(f->state_l);
}

int64_t sn_zfix_has(void *p, int64_t node) {
    SnZFix *f = zfix_guard(p, "has");
    if (!f) return 0;
    return (f->tier == SN_ZFIX_TIER_G) ? (sn_zset_count(f->state, node) != 0)
                                       : sn_lcell_has(f->state_l, node);
}

/* A copy of the reach state, owned by the caller (Tier G: a Z-set;
 * Tier L: an LCell clone) — the read-side for tests and checksums. */
void *sn_zfix_state(void *p) {
    SnZFix *f = zfix_guard(p, "state");
    if (!f) return NULL;
    return (f->tier == SN_ZFIX_TIER_G) ? (void *)sn_zset_copy(f->state)
                                       : (void *)lcell_clone(f->state_l);
}

/* ---------------------- the per-tier stop tests ----------------------- */
/* Diagnostic predicates — they do NOT perturb the bracket's probe
 * counter (the bracket's own stop condition is the round's delta
 * size; these are the same tests in their direct whole-state forms). */

/* Tier G: 𝒟(∫α) = 0 — one full-state body step (the naive
 * re-derivation over EVERY state fact) derives an empty delta. */
int64_t sn_zfix_quiescent_g(void *p) {
    SnZFix *f = zfix_guard(p, "quiescent_g");
    if (!f) return 1;
    if (f->tier != SN_ZFIX_TIER_G) {
        fprintf(stderr,
                "runtime error: zfix_quiescent_g is the Tier-G stop test "
                "(𝒟(∫α) = 0); this bracket is Tier-L — use "
                "zfix_quiescent_l [E-LKIND]\n");
        exit(1);
    }
    for (int64_t i = 0; i < f->state->len; i++) {
        int64_t b = f->state->keys[i];
        int64_t idx = zfix_adj_find(&f->adj, b, key_hash(0, b), NULL);
        if (idx < 0) continue;
        for (int64_t k = f->adj.off[idx]; k < f->adj.off[idx + 1]; k++) {
            if (sn_zset_count(f->state, f->adj.buf[k]) == 0) return 0;
        }
    }
    return 1;
}

/* Tier L: F(s) ⊑ s via the MUTATOR LAW — one full-state body step
 * joined into a candidate s ⊔ F(s), then the state-relative fragment
 * delta(s ⊔ F(s), s) must be ⊥ (no inflation). Tier L has no ⊖
 * (Grothendieck triviality — E-LRET), so the 𝒟 form is not
 * expressible there; this is the semilattice tier's own test. */
int64_t sn_zfix_quiescent_l(void *p) {
    SnZFix *f = zfix_guard(p, "quiescent_l");
    if (!f) return 1;
    if (f->tier != SN_ZFIX_TIER_L) {
        fprintf(stderr,
                "runtime error: zfix_quiescent_l is the Tier-L stop test "
                "(F(s) ⊑ s via the mutator law); this bracket is Tier-G — "
                "use zfix_quiescent_g [E-LKIND]\n");
        exit(1);
    }
    SnLCell *cand = lcell_clone(f->state_l); /* s ⊔ F(s), built */
    for (int64_t i = 0; i < f->state_l->len; i++) {
        int64_t b = f->state_l->elems[i];
        int64_t idx = zfix_adj_find(&f->adj, b, key_hash(0, b), NULL);
        if (idx < 0) continue;
        for (int64_t k = f->adj.off[idx]; k < f->adj.off[idx + 1]; k++) {
            lcell_gset_insert(cand, f->adj.buf[k]);
        }
    }
    SnLCell *frag = (SnLCell *)sn_lcell_delta(cand, f->state_l);
    int64_t q = (sn_lcell_len(frag) == 0) ? 1 : 0;
    sn_release(frag);
    sn_release(cand);
    return q;
}

/* The explicit release (the tracked-object discipline: sn_release at
 * rc 0 frees; this alias reads clearly in the C harnesses). */
void sn_zfix_release(void *p) {
    sn_release(p);
}


/* --------------------------------------------------------------------- */
/*  The self-relative link system (SWARM RABBIT HOLE, Wave 11 / DIFF-5,  */
/*  Impl-P, Cohort Delta; kills G1 §1 A2 → S4; STRATA §2.4.6's           */
/*  NoRelLinkCopy closure — wave2_spec_diffs.md DIFF-5).                   */
/*                                                                       */
/*  link<T, u16>: a 2-byte self-relative delta field. A value of the    */
/*  type at address A stores δ = target − A (decoded: target = A + δ);    */
/*  the u16 range bounds |δ| ≤ 32767. The value is position-dependent    */
/*  BY CONSTRUCTION: a byte-copy preserves deltas relative to the        */
/*  SOURCE field addresses, so the checker rejects every by-value copy  */
/*  of a HasSelfLink value that reads stored bytes (E0801 — the          */
/*  compiler's four sites), and this runtime provides the only legal    */
/*  movers:                                                             */
/*                                                                       */
/*   sn_relocate_node  — decode at source (t = addr(f_src) + δ), re-     */
/*     encode at destination (δ' = t − addr(f_dst)), |δ'| ≤ 32767 else   */
/*     the E0801 (relocate) fail-fast. Links targeting INSIDE the moved */
/*     value are fixed to the new interior; links targeting outside     */
/*     keep their absolute target (re-based: δ' = δ − (addr(f_dst) −     */
/*     addr(f_src))).                                                   */
/*                                                                       */
/*   sn_relocate_range — the CONSTANT-SHIFT EXEMPTION, the one bulk      */
/*     path where memcpy is sound without per-link re-encode: with      */
/*     uniform k = dst_base − src_base every intra-range link is delta- */
/*     preserving (target and field both shift by k ⇒ δ unchanged), so  */
/*     the range bulk-copies iff self-contained (the same scan as       */
/*     to_bytes); range-boundary-crossing links re-encode once with     */
/*     δ' = δ − k. Two O(N) passes, never per-link re-encode of the      */
/*     interior.                                                         */
/*                                                                       */
/*   sn_link_to_bytes  — the O(N) SENDER-SIDE self-containedness scan   */
/*     before packing: for every non-null link field at element offset  */
/*     e inside the serialized range, decode t = field_addr + δ and     */
/*     require slice_base ≤ t < slice_end. Cross-slice links (targets   */
/*     in other lists on the same lane — legal to HOLD, illegal to      */
/*     SERIALIZE) → the E0801 (serialize) fail-fast: §4.5's load-time    */
/*     verifier would otherwise receive a guaranteed failure. raw=1 is  */
/*     the bench-only comparator that prices the scan (never sound to  */
/*     call on link-bearing data).                                       */
/*                                                                       */
/*   sn_link_block_new / sn_link_elem / sn_link_set / sn_link_get — the  */
/*     contiguous block carriers (the self-contained slice of the        */
/*     spec's model: n record slots back to back) and the position-     */
/*     checked encode/decode the tests and benchmarks drive. Blocks     */
/*     zero on creation: raw 0 is the NULL link (scans and re-encodes  */
/*     skip nulls). The layout (element size, the ≤4 link-field        */
/*     offsets, their names and target-type labels) is compile-time-    */
/*     constant per call site: the compiler emits it into every call,   */
/*     which is what makes the verbatim diagnostics name the field.     */
/*                                                                       */
/*  SOUNDNESS (DIFF-5's statement, enforced here): every copy of a      */
/*  HasSelfLink value now either constructs it fresh at its final       */
/*  address (sret), re-encodes every delta against the new field        */
/*  address (relocate_*), or is rejected (E0801) — no 2-byte delta is   */
/*  ever interpreted against an address it was not encoded for; the    */
/*  to_bytes scan extends the same invariant across the serialization  */
/*  boundary, making the bulk-copy claim TRUE BY PRECONDITION instead  */
/*  of false by omission.                                                */
/* --------------------------------------------------------------------- */

/* The fixed per-record link-field window the compiler emits (4 slots,
   unused slots carry offset -1 and ""). */
#define SN_LINK_MAX 4

static int sn_link_range_ok(int64_t d) { return d >= -32767 && d <= 32767; }

static int16_t sn_link_raw_at(void *node, int64_t off) {
    int16_t v;
    memcpy(&v, (char *)node + off, sizeof v);
    return v;
}

static void sn_link_store_at(void *node, int64_t off, int64_t d) {
    int16_t v = (int16_t)d;
    memcpy((char *)node + off, &v, sizeof v);
}

/* E0801 (relocate) — the spec's verbatim diagnostic (DIFF-5 (c)); the
   range is written with the unicode minus sign exactly as spec'd. */
static void sn_link_die_relocate(const char *f, const char *src, const char *dst,
                                 int64_t nd, const char *t) {
    fprintf(stderr,
            "E0801 (relocate): cannot relocate '%s' from '%s' to '%s': "
            "the re-encoded delta (%" PRId64 ") for its link target exceeds "
            "the i16 range [\xE2\x88\x92" "32767, +32767]; move the value and "
            "its target closer together, or use 'link<%s in pool, u32>'. "
            "[E0801]\n",
            f, src, dst, nd, t);
    exit(1);
}

/* E0801 (serialize) — the spec's verbatim diagnostic (DIFF-5 (b)). */
static void sn_link_die_serialize(const char *xs, int64_t a, int64_t b,
                                  const char *t, int64_t i, const char *field) {
    fprintf(stderr,
            "E0801 (serialize): cannot serialize '%s[%" PRId64 "..%" PRId64
            "]' to bytes: 'link<%s, u16>' at element %" PRId64 " ('%s') "
            "decodes to an address outside the serialized slice "
            "(cross-slice links cannot survive relocation). Serialize the "
            "enclosing whole list, drop the link, or use 'link<%s in pool, "
            "u32>'. [E0801]\n",
            xs, a, b, t, i, field, t);
    exit(1);
}

/* A contiguous block of n record slots, zeroed: every link field starts
   as the NULL link (raw 0). malloc'd (the lane-backed flavor of the
   same carriers is exercised against these functions directly in the
   systems test via sn_lane_alloc). */
void *sn_link_block_new(int64_t n, int64_t sz) {
    if (n < 0) n = 0;
    if (sz < 1) sz = 1;
    void *p = xmalloc((size_t)(n * sz) + 1);
    memset(p, 0, (size_t)(n * sz));
    return p;
}

/* The address of slot i — the readout primitive (decode comparisons,
   checksums). */
void *sn_link_elem(void *block, int64_t i, int64_t sz) {
    return (char *)block + i * sz;
}

/* Encode: δ = target_addr − field_addr, written AT the field (the only
   checked writer — the field's position is what the delta encodes
   against, so the encode cannot exist without the address). |δ| ≤
   32767 else the same E0801 (relocate) fail-fast: the 2-byte type
   cannot represent the geometry, and a wrapped delta would be a lie. */
void sn_link_set(void *block, int64_t i, int64_t off, void *tblock, int64_t ti,
                 int64_t sz, const char *flabel, const char *tlabel) {
    char *field = (char *)block + i * sz + off;
    char *target = (char *)tblock + ti * sz;
    int64_t d = (int64_t)(target - field);
    if (!sn_link_range_ok(d)) {
        sn_link_die_relocate(flabel, "the field", "the target", d, tlabel);
    }
    sn_link_store_at(field, 0, d);
}

/* Decode: t = field_addr + δ (the i16 sign extension IS the decode);
   the NULL link (raw 0) reads as 0. No dereference ever happens on a
   decoded target on this surface — link_get is the readout. */
int64_t sn_link_get(void *block, int64_t i, int64_t off, int64_t sz) {
    char *field = (char *)block + i * sz + off;
    int16_t raw = sn_link_raw_at(field, 0);
    if (raw == 0) return 0;
    return (int64_t)(field + (int64_t)raw);
}

/* relocate_node: decode every link AT THE SOURCE, byte-copy, re-encode
   AT THE DESTINATION. Interior targets follow the moved value; outside
   targets keep their absolute address. */
void sn_relocate_node(void *src, int64_t si, void *dst, int64_t di, int64_t sz,
                      int64_t nl, int64_t o0, int64_t o1, int64_t o2, int64_t o3,
                      const char *f0, const char *f1, const char *f2, const char *f3,
                      const char *t0, const char *t1, const char *t2, const char *t3,
                      const char *src_label, const char *dst_label) {
    int64_t offs[SN_LINK_MAX] = { o0, o1, o2, o3 };
    const char *names[SN_LINK_MAX] = { f0, f1, f2, f3 };
    const char *targs[SN_LINK_MAX] = { t0, t1, t2, t3 };
    char *s = (char *)src + si * sz;
    char *d = (char *)dst + di * sz;
    /* Pass 1 — decode at source (the deltas are readable only where
       they were encoded). */
    int64_t raw[SN_LINK_MAX];
    for (int j = 0; j < SN_LINK_MAX; j++) {
        raw[j] = (j < nl && offs[j] >= 0) ? (int64_t)sn_link_raw_at(s, offs[j]) : 0;
    }
    memcpy(d, s, (size_t)sz);
    /* Pass 2 — re-encode at destination. */
    for (int j = 0; j < nl && offs[j] >= 0; j++) {
        if (raw[j] == 0) continue; /* the NULL link is position-free */
        char *t = (char *)s + offs[j] + raw[j]; /* field_addr + δ */
        char *t_new;
        if (t >= s && t < s + sz) {
            t_new = d + (t - s); /* interior: fixed to the new interior */
        } else {
            t_new = t;           /* outside: keeps its absolute target */
        }
        int64_t dp = (int64_t)(t_new - (d + offs[j]));
        if (!sn_link_range_ok(dp)) {
            sn_link_die_relocate(names[j], src_label, dst_label, dp, targs[j]);
        }
        sn_link_store_at(d, offs[j], dp);
    }
}

/* relocate_range — the constant-shift bulk exemption. Pass 1 is the
   self-containedness/boundary scan (O(N) in slice bytes): intra-range
   links need NO re-encode (δ unchanged under the uniform shift —
   provably the identity), boundary-crossing links must fit after
   δ' = δ − k. The memcpy is the ONE exempt bulk copy. Pass 2 re-walks
   the (untouched) source to write the boundary re-encodes. */
void sn_relocate_range(void *src, void *dst, int64_t n, int64_t sz,
                       int64_t nl, int64_t o0, int64_t o1, int64_t o2, int64_t o3,
                       const char *f0, const char *f1, const char *f2, const char *f3,
                       const char *t0, const char *t1, const char *t2, const char *t3,
                       const char *src_label, const char *dst_label) {
    int64_t offs[SN_LINK_MAX] = { o0, o1, o2, o3 };
    const char *names[SN_LINK_MAX] = { f0, f1, f2, f3 };
    const char *targs[SN_LINK_MAX] = { t0, t1, t2, t3 };
    if (n <= 0) return;
    int64_t total = n * sz;
    int64_t k = (int64_t)((char *)dst - (char *)src);
    /* Pass 1 — the scan (also the range check for boundary links). */
    for (int64_t e = 0; e < n; e++) {
        char *node = (char *)src + e * sz;
        for (int j = 0; j < nl && offs[j] >= 0; j++) {
            int64_t raw = (int64_t)sn_link_raw_at(node, offs[j]);
            if (raw == 0) continue;
            char *field = node + offs[j];
            char *t = field + raw;
            if (t >= (char *)src && t < (char *)src + total) continue; /* intra-range */
            int64_t dp = raw - k; /* = t − (dst field addr) */
            if (!sn_link_range_ok(dp)) {
                char sl[64], el[64];
                snprintf(sl, sizeof sl, "%s[%" PRId64 "]", src_label, e);
                snprintf(el, sizeof el, "%s[%" PRId64 "]", dst_label, e);
                sn_link_die_relocate(names[j], sl, el, dp, targs[j]);
            }
        }
    }
    /* The exempt bulk copy. */
    memcpy(dst, src, (size_t)total);
    /* Pass 2 — re-encode the boundary-crossing links (the source is
       untouched; walk it again). */
    for (int64_t e = 0; e < n; e++) {
        char *snode = (char *)src + e * sz;
        char *dnode = (char *)dst + e * sz;
        for (int j = 0; j < nl && offs[j] >= 0; j++) {
            int64_t raw = (int64_t)sn_link_raw_at(snode, offs[j]);
            if (raw == 0) continue;
            char *field = snode + offs[j];
            char *t = field + raw;
            if (t >= (char *)src && t < (char *)src + total) continue;
            sn_link_store_at(dnode, offs[j], raw - k);
        }
    }
}

/* to_bytes — the sender-side scan + pack. Slice [a, b) of the block;
   every non-null link must decode INSIDE the serialized slice. The
   returned buffer is exactly the slice bytes (the caller knows the
   length: (b − a) · sz). raw=1 skips the scan: the BENCH-ONLY
   comparator (link_to_bytes_raw) that prices it. */
void *sn_link_to_bytes(void *base, int64_t a, int64_t b, int64_t sz,
                       int64_t nl, int64_t o0, int64_t o1, int64_t o2, int64_t o3,
                       const char *xs_label,
                       const char *f0, const char *f1, const char *f2, const char *f3,
                       const char *t0, const char *t1, const char *t2, const char *t3,
                       int64_t raw) {
    int64_t offs[SN_LINK_MAX] = { o0, o1, o2, o3 };
    const char *names[SN_LINK_MAX] = { f0, f1, f2, f3 };
    const char *targs[SN_LINK_MAX] = { t0, t1, t2, t3 };
    if (a < 0) a = 0;
    if (b < a) b = a;
    char *slice_base = (char *)base + a * sz;
    char *slice_end = (char *)base + b * sz;
    if (!raw) {
        for (int64_t e = a; e < b; e++) {
            char *node = (char *)base + e * sz;
            for (int j = 0; j < nl && offs[j] >= 0; j++) {
                int64_t r = (int64_t)sn_link_raw_at(node, offs[j]);
                if (r == 0) continue;
                char *t = node + offs[j] + r;
                if (t < slice_base || t >= slice_end) {
                    sn_link_die_serialize(xs_label, a, b, targs[j], e, names[j]);
                }
            }
        }
    }
    int64_t bytes = (b - a) * sz;
    void *buf = xmalloc((size_t)(bytes > 0 ? bytes : 1));
    if (bytes > 0) memcpy(buf, slice_base, (size_t)bytes);
    return buf;
}

/* ---------------------------------------------------------------------
 *  SnView — the incremental view engine (Problem-5 second slice).        */
/*  Impl-H, Cohort Delta, Wave 7. Built on Impl-G's SnZSet (Tier G).     */
/*                                                                       */
/*  A view is a dataflow node wrapping an input Z-set (the source), a    */
/*  transformation kind, and a maintained output Z-set with the          */
/*  invariant output = F(input). The first cut covers the honest         */
/*  non-amplifying fragment 𝒞₀ (W2C §2.2): FILTER by a constant         */
/*  predicate on the element key, MAP by a key function, AGGREGATE-SUM   */
/*  per key, AGGREGATE-COUNT (the fixed-key presence aggregate). NOT in  */
/*  this slice, by construction and without an O(|dA|) claim for any of  */
/*  them: joins (⊗ fanout ρ), guarded μ — the Salt ζ fixpoint bracket,  */
/*  W2C §3, is future work — predicates-as-views, and every Tier-L and  */
/*  Tier-O operator (the verified barrier list, W2C §2.4).               */
/*                                                                       */
/*  Delta path — the probe-linear bound made runtime (W2C §2.1):       */
/*  sn_view_apply_delta probes maintained state ONLY at the changed     */
/*  keys, never scanning. Work is counted in hash-map operations        */
/*  (g_sn_view_probes), not wall time: each zs_apply is exactly one     */
/*  find-or-insert (rehash growth is amortized into it), so the linear  */
/*  kinds cost exactly 3 ops per changed key — input fold, output       */
/*  update, pending changelog — independent of |input|. A key a        */
/*  FILTER drops costs 1 op (only the input fold). AGGREGATE-COUNT      */
/*  costs 1 + 2·[presence flip] per changed key (the flip is            */
/*  determined from the old count the single input probe returns).      */
/*                                                                       */
/*  Integrator ownership (DBSP's DB node, made explicit): apply_delta   */
/*  folds dA into the view's OWN input Z-set (the ∫ of its input        */
/*  stream). A downstream view therefore wraps its OWN COPY of the      */
/*  upstream output (sn_zset_copy at wiring time): sharing one object   */
/*  between an upstream output and a downstream input would             */
/*  double-integrate every delta — the upstream apply and the          */
/*  downstream fold would both write it.                                */
/*  sn_view_commit is O(1) and probe-free: a pending swap.               */
/*                                                                       */
/*  Batch/changelog API: sn_view_commit returns 𝒟 of the output stream  */
/*  since the last commit — the runtime form of the chain rule (W2C     */
/*  Thm 2): view₁ → view₂ → view₃, each consuming the previous commit;  */
/*  the downstream batch sizes are the general law's |Δv| terms         */
/*  (W ≤ c_F·(|dA| + Σ_v |Δv|), W2C §2.1 — the 𝒞₀ specialization        */
/*  collapses it to Σ 3·|Δ_in| for this fragment).                     */
/*                                                                       */
/*  Quiescence (W4C v2.1 repair 3): the tier-G test is 𝒟(∫α) = 0 —     */
/*  an empty commit result; an empty batch performs zero probes. The    */
/*  tier-L test F(s) ⊑ s is future work: no L carriers exist here.     */
/* --------------------------------------------------------------------- */

static int64_t g_sn_view_probes = 0;

void sn_view_probe_reset(void) { g_sn_view_probes = 0; }
int64_t sn_view_probe_count(void) { return g_sn_view_probes; }

/* defined further down (the euclidean-mod helper lives with the math
 * utilities; the fn-taking constructor below sn_view_new) */
int64_t sn_euclid_mod_i64(int64_t a, int64_t b);
void *sn_view_new_fn(int64_t kind, void *input, int64_t p1, int64_t p2,
                     int64_t (*pred)(int64_t), int64_t (*mapfn)(int64_t));

enum {
    SN_VIEW_FILTER    = 1, /* p1: predicate family; p2: family argument   */
    SN_VIEW_MAP       = 2, /* y = p1·k + p2 — a group homomorphism iff    */
                          /* p2 == 0; the delta law holds for any total  */
                          /* key function (ℤ[X]-linear either way)        */
    SN_VIEW_AGG_SUM   = 3, /* group g(k) = k mod p1; sums the weights     */
    SN_VIEW_AGG_COUNT = 4  /* group g(k) = k mod p1; counts present keys */
};

typedef struct SnView {
    int64_t kind;
    SnZSet *input;   /* source (retained ref; mutated only via deltas)  */
    SnZSet *output;  /* maintained output (owned ref): output = F(input) */
    SnZSet *pending; /* output delta since the last commit (owned ref)  */
    int64_t p1;
    int64_t p2;
    int64_t (*pred)(int64_t k);   /* optional C predicate override (FILTER) */
    int64_t (*mapfn)(int64_t k);  /* optional C key-function override (MAP) */
} SnView;

/* One counted hash-map operation: sn_zset_add's find-or-insert, but it
 * returns the OLD count (absent = 0) — AGGREGATE-COUNT needs the
 * presence flip, and every probe the engine makes is routed through
 * here so the work bound is an exact operation count. */
static int64_t zs_apply_counted(SnZSet *z, int64_t key, int64_t delta) {
    g_sn_view_probes++;
    int64_t old = 0;
    if ((z->len + 1) * 4 >= z->hcap * 3) {
        zset_rehash(z, z->hcap * 2);
    }
    uint64_t h = key_hash(z->key_kind, key);
    uint64_t bucket = 0;
    int64_t idx = zset_find_index(z, key, h, &bucket);
    if (idx >= 0) {
        old = z->counts[idx];
        z->counts[idx] += delta;
        z->total += delta;
        if (z->counts[idx] == 0 && z->prune) {
            if (z->key_kind == 1) sn_release((void *)(intptr_t)z->keys[idx]);
            zset_remove_at(z, idx);
        }
        return old;
    }
    if (z->len == z->cap) {
        z->cap *= 2;
        z->keys = (int64_t *)realloc(z->keys, sizeof(int64_t) * (size_t)z->cap);
        z->counts = (int64_t *)realloc(z->counts, sizeof(int64_t) * (size_t)z->cap);
        z->hashes = (uint64_t *)realloc(z->hashes, sizeof(uint64_t) * (size_t)z->cap);
    }
    if (z->key_kind == 1) sn_retain((void *)(intptr_t)key);
    z->keys[z->len] = key;
    z->counts[z->len] = delta;
    z->hashes[z->len] = h;
    z->buckets[bucket] = z->len + 1;
    z->len++;
    z->total += delta;
    return old;
}

/* ----------------------------- the view rules -------------------------- */

static int view_pred(SnView *v, int64_t k) {
    if (v->pred) return v->pred(k) != 0;
    switch (v->p1) {
    case 0: return (k % 2) == 0;                  /* keep even keys      */
    case 1: return (k % 2) != 0;                  /* keep odd keys       */
    case 2: return k >= v->p2;                    /* keep keys ≥ p2      */
    case 3: return k < v->p2;                     /* keep keys < p2      */
    case 4: return (v->p2 > 0) && (sn_euclid_mod_i64(k, v->p2) == 0);
    default: return 0;
    }
}

static int64_t view_keyfn(SnView *v, int64_t k) {
    if (v->mapfn) return v->mapfn(k);
    return v->p1 * k + v->p2; /* group homomorphism iff p2 == 0 */
}

static int64_t view_group(SnView *v, int64_t k) {
    return sn_euclid_mod_i64(k, v->p1); /* ℤ → ℤ/m, the quotient homomorphism */
}

/* ------------------------------ SnView API ----------------------------- */

/* Initial materialization (the ℐ step): ONE full scan of the input at
 * construction — O(|input|), once, outside the per-delta claim. */
void *sn_view_new(int64_t kind, void *input, int64_t p1, int64_t p2) {
    return sn_view_new_fn(kind, input, p1, p2, NULL, NULL);
}

void *sn_view_new_fn(int64_t kind, void *input, int64_t p1, int64_t p2,
                     int64_t (*pred)(int64_t), int64_t (*mapfn)(int64_t)) {
    SnView *v = (SnView *)xmalloc(sizeof(SnView));
    int64_t kk = input ? ((SnZSet *)input)->key_kind : 0;
    v->kind = kind;
    if (input) {
        v->input = (SnZSet *)input;
        sn_retain(v->input); /* the borrowed source ref */
    } else {
        v->input = (SnZSet *)sn_zset_new(kk); /* owned empty source */
    }
    v->output = (SnZSet *)sn_zset_new(kk);
    v->pending = (SnZSet *)sn_zset_new(kk);
    v->p1 = p1;
    v->p2 = p2;
    v->pred = pred;
    v->mapfn = mapfn;

    SnZSet *in = v->input;
    for (int64_t i = 0; i < in->len; i++) {
        int64_t k = in->keys[i];
        int64_t w = in->counts[i];
        switch (kind) {
        case SN_VIEW_FILTER:
            if (view_pred(v, k)) sn_zset_add(v->output, k, w);
            break;
        case SN_VIEW_MAP:
            sn_zset_add(v->output, view_keyfn(v, k), w);
            break;
        case SN_VIEW_AGG_SUM:
            sn_zset_add(v->output, view_group(v, k), w);
            break;
        case SN_VIEW_AGG_COUNT:
            if (w != 0) sn_zset_add(v->output, view_group(v, k), 1);
            break;
        default:
            break;
        }
    }
    return v;
}

/* The delta-application path — the O(|dA|) claim's implementation.
 * dA is a Z-set of changes; each entry (k, d) means count(k) += d.
 * Probes: exactly 3 per entry for the linear kinds (input, output,
 * pending), 1 + 2·[presence flip] for AGGREGATE-COUNT. */
void sn_view_apply_delta(void *view, void *delta) {
    SnView *v = (SnView *)view;
    if (!v || !delta) return;
    SnZSet *dA = (SnZSet *)delta;
    for (int64_t i = 0; i < dA->len; i++) {
        int64_t k = dA->keys[i];
        int64_t d = dA->counts[i];
        switch (v->kind) {
        case SN_VIEW_FILTER:
            zs_apply_counted(v->input, k, d); /* the ∫ fold into the source */
            if (view_pred(v, k)) {
                zs_apply_counted(v->output, k, d);
                zs_apply_counted(v->pending, k, d);
            }
            break;
        case SN_VIEW_MAP: {
            zs_apply_counted(v->input, k, d);
            int64_t y = view_keyfn(v, k);
            zs_apply_counted(v->output, y, d);
            zs_apply_counted(v->pending, y, d);
            break;
        }
        case SN_VIEW_AGG_SUM: {
            zs_apply_counted(v->input, k, d);
            int64_t g = view_group(v, k);
            zs_apply_counted(v->output, g, d);
            zs_apply_counted(v->pending, g, d);
            break;
        }
        case SN_VIEW_AGG_COUNT: {
            int64_t old = zs_apply_counted(v->input, k, d);
            int64_t now = old + d;
            if ((old != 0) != (now != 0)) {
                int64_t c = (now != 0) ? 1 : -1;
                int64_t g = view_group(v, k);
                zs_apply_counted(v->output, g, c);
                zs_apply_counted(v->pending, g, c);
            }
            break;
        }
        default:
            break;
        }
    }
}

/* The batch/changelog API: returns 𝒟(output stream) since the last
 * commit — the runtime chain-rule form; downstream views consume this
 * as their dA. O(1) and probe-free: a pending swap; the caller owns the
 * returned Z-set (release with sn_release). */
void *sn_view_commit(void *view) {
    SnView *v = (SnView *)view;
    if (!v) return sn_zset_new(0);
    SnZSet *d = v->pending;
    v->pending = (SnZSet *)sn_zset_new(v->input->key_kind);
    return d;
}

void *sn_view_input(void *view) { return view ? (void *)((SnView *)view)->input : NULL; }
void *sn_view_output(void *view) { return view ? (void *)((SnView *)view)->output : NULL; }
int64_t sn_view_kind(void *view) { return view ? ((SnView *)view)->kind : 0; }

void sn_view_release(void *view) {
    SnView *v = (SnView *)view;
    if (!v) return;
    sn_release(v->input);
    sn_release(v->output);
    sn_release(v->pending);
    free(v);
}

void *sn_input(void *prompt) {
    if (prompt) {
        fputs(sn_str_cstr(prompt), stdout);
        fflush(stdout);
    }
    char buf[4096];
    if (!fgets(buf, sizeof(buf), stdin)) return sn_str_from_cstr("");
    size_t n = strlen(buf);
    if (n && buf[n - 1] == '\n') buf[--n] = 0;
    return sn_str_new(buf, (int64_t)n);
}

void *sn_file_read(void *path) {
    FILE *f = fopen(sn_str_cstr(path), "rb");
    if (!f) return sn_str_from_cstr("");
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (n < 0) n = 0;
    char *buf = (char *)xmalloc((size_t)n + 1);
    fread(buf, 1, (size_t)n, f);
    buf[n] = 0;
    fclose(f);
    void *s = sn_str_new(buf, n);
    free(buf);
    return s;
}

static void *sn_os_error(void) {
    return sn_error_new(sn_str_from_cstr(strerror(errno)));
}

static int sn_read_file_contents(void *path, void **out_body) {
    FILE *f = fopen(sn_str_cstr(path), "rb");
    if (!f) return -1;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (n < 0) n = 0;
    char *buf = (char *)xmalloc((size_t)n + 1);
    if (n > 0) fread(buf, 1, (size_t)n, f);
    buf[n] = 0;
    fclose(f);
    if (out_body) *out_body = sn_str_new(buf, n);
    free(buf);
    return 0;
}

void sn_file_read_ex(void *path, void **out_body, void **out_err) {
    if (sn_read_file_contents(path, out_body) != 0) {
        if (out_body) *out_body = sn_str_from_cstr("");
        if (out_err) *out_err = sn_os_error();
        return;
    }
    if (out_err) *out_err = NULL;
}

void *sn_file_write_ex(void *path, void *data) {
    FILE *f = fopen(sn_str_cstr(path), "wb");
    if (!f) return sn_os_error();
    fwrite(sn_str_cstr(data), 1, (size_t)sn_str_len(data), f);
    if (ferror(f)) {
        int err = errno;
        fclose(f);
        errno = err;
        return sn_os_error();
    }
    fclose(f);
    return NULL;
}

int64_t sn_file_write(void *path, void *data) {
    return sn_file_write_ex(path, data) == NULL ? 1 : 0;
}

void *sn_file_append(void *path, void *data) {
    FILE *f = fopen(sn_str_cstr(path), "ab");
    if (!f) return sn_os_error();
    fwrite(sn_str_cstr(data), 1, (size_t)sn_str_len(data), f);
    if (ferror(f)) {
        int err = errno;
        fclose(f);
        errno = err;
        return sn_os_error();
    }
    fclose(f);
    return NULL;
}

int64_t sn_file_exists(void *path) {
    const char *p = sn_str_cstr(path);
#ifdef _WIN32
    return _access(p, 0) == 0 ? 1 : 0;
#else
    return access(p, F_OK) == 0 ? 1 : 0;
#endif
}

void *sn_file_delete(void *path) {
    if (remove(sn_str_cstr(path)) != 0) return sn_os_error();
    return NULL;
}

static int sn_copy_file(const char *src, const char *dst) {
#ifdef _WIN32
    if (!CopyFileA(src, dst, 0)) {
        errno = EIO;
        return -1;
    }
    return 0;
#else
    FILE *in = fopen(src, "rb");
    if (!in) return -1;
    FILE *out = fopen(dst, "wb");
    if (!out) {
        fclose(in);
        return -1;
    }
    char buf[8192];
    size_t n;
    while ((n = fread(buf, 1, sizeof(buf), in)) > 0) {
        if (fwrite(buf, 1, n, out) != n) {
            fclose(in);
            fclose(out);
            return -1;
        }
    }
    int err = ferror(in) || ferror(out);
    fclose(in);
    fclose(out);
    return err ? -1 : 0;
#endif
}

void *sn_file_copy(void *src, void *dst) {
    if (sn_copy_file(sn_str_cstr(src), sn_str_cstr(dst)) != 0) return sn_os_error();
    return NULL;
}

void *sn_file_move(void *src, void *dst) {
#ifdef _WIN32
    if (!MoveFileExA(sn_str_cstr(src), sn_str_cstr(dst), MOVEFILE_REPLACE_EXISTING)) {
        return sn_os_error();
    }
    return NULL;
#else
    if (rename(sn_str_cstr(src), sn_str_cstr(dst)) != 0) return sn_os_error();
    return NULL;
#endif
}

void *sn_file_mkdir(void *path) {
    const char *p = sn_str_cstr(path);
#ifdef _WIN32
    if (_mkdir(p) != 0 && errno != EEXIST) return sn_os_error();
#else
    if (mkdir(p, 0755) != 0 && errno != EEXIST) return sn_os_error();
#endif
    return NULL;
}

void *sn_file_rmdir(void *path) {
    const char *p = sn_str_cstr(path);
#ifdef _WIN32
    if (_rmdir(p) != 0) return sn_os_error();
#else
    if (rmdir(p) != 0) return sn_os_error();
#endif
    return NULL;
}

void sn_file_list(void *path, void **out_list, void **out_err) {
#ifdef _WIN32
    char pattern[MAX_PATH];
    const char *p = sn_str_cstr(path);
    snprintf(pattern, sizeof(pattern), "%s\\*", p);
    WIN32_FIND_DATAA fd;
    HANDLE h = FindFirstFileA(pattern, &fd);
    if (h == INVALID_HANDLE_VALUE) {
        if (out_list) *out_list = sn_list_new(1);
        if (out_err) *out_err = sn_os_error();
        return;
    }
    List *l = (List *)sn_list_new(1);
    do {
        if (strcmp(fd.cFileName, ".") == 0 || strcmp(fd.cFileName, "..") == 0) continue;
        sn_list_push_ptr(l, sn_str_from_cstr(fd.cFileName));
    } while (FindNextFileA(h, &fd));
    FindClose(h);
    if (out_list) *out_list = l;
    if (out_err) *out_err = NULL;
#else
    DIR *d = opendir(sn_str_cstr(path));
    if (!d) {
        if (out_list) *out_list = sn_list_new(1);
        if (out_err) *out_err = sn_os_error();
        return;
    }
    List *l = (List *)sn_list_new(1);
    struct dirent *ent;
    while ((ent = readdir(d)) != NULL) {
        if (strcmp(ent->d_name, ".") == 0 || strcmp(ent->d_name, "..") == 0) continue;
        sn_list_push_ptr(l, sn_str_from_cstr(ent->d_name));
    }
    closedir(d);
    if (out_list) *out_list = l;
    if (out_err) *out_err = NULL;
#endif
}

static int sn_is_sep(char c) {
#ifdef _WIN32
    return c == '/' || c == '\\';
#else
    return c == '/';
#endif
}

void *sn_path_join(void *a, void *b) {
    const char *sa = sn_str_cstr(a);
    const char *sb = sn_str_cstr(b);
    size_t la = strlen(sa);
    size_t lb = strlen(sb);
    int need_sep = la > 0 && !sn_is_sep(sa[la - 1]);
    size_t n = la + (need_sep ? 1 : 0) + lb;
    char *buf = (char *)xmalloc(n + 1);
    memcpy(buf, sa, la);
    size_t off = la;
    if (need_sep) buf[off++] = '/';
    memcpy(buf + off, sb, lb);
    buf[n] = 0;
    void *s = sn_str_new(buf, (int64_t)n);
    free(buf);
    return s;
}

static const char *sn_path_last_sep(const char *p) {
    const char *last = NULL;
    for (const char *c = p; *c; c++) {
        if (sn_is_sep(*c)) last = c;
    }
    return last;
}

void *sn_path_base(void *p) {
    const char *s = sn_str_cstr(p);
    const char *slash = sn_path_last_sep(s);
    if (!slash) return sn_str_from_cstr(s);
    return sn_str_from_cstr(slash + 1);
}

void *sn_path_dir(void *p) {
    const char *s = sn_str_cstr(p);
    const char *slash = sn_path_last_sep(s);
    if (!slash) return sn_str_from_cstr(".");
    if (slash == s) return sn_str_from_cstr("/");
    return sn_str_new(s, (int64_t)(slash - s));
}

void *sn_path_ext(void *p) {
    const char *s = sn_str_cstr(p);
    const char *dot = strrchr(s, '.');
    const char *slash = sn_path_last_sep(s);
    if (!dot || (slash && dot < slash)) return sn_str_from_cstr("");
    return sn_str_from_cstr(dot);
}

void *sn_os_getenv(void *name) {
    const char *v = getenv(sn_str_cstr(name));
    if (!v) return NULL;
    return sn_str_from_cstr(v);
}

void sn_os_exit(int64_t code) {
    exit((int)code);
}

void *sn_os_args(void) {
    List *l = (List *)sn_list_new(1);
    for (int i = 0; i < g_argc; i++) {
        sn_list_push_ptr(l, sn_str_from_cstr(g_argv[i]));
    }
    return l;
}

int64_t sn_list_contains_i64(void *p, int64_t v) {
    List *l = (List *)p;
    if (!l || l->elem_kind != 0) return 0;
    for (int64_t i = 0; i < l->len; i++) {
        if (l->items[i] == v) return 1;
    }
    return 0;
}

int64_t sn_list_contains_ptr(void *p, void *v) {
    List *l = (List *)p;
    if (!l || l->elem_kind != 1) return 0;
    for (int64_t i = 0; i < l->len; i++) {
        void *item = (void *)(intptr_t)l->items[i];
        if (sn_str_eq(item, v)) return 1;
    }
    return 0;
}

void *sn_error_new(void *msg) {
    Err *e = (Err *)xmalloc(sizeof(Err));
    e->rc = 1;
    e->kind = KIND_ERR;
    gc_track(e, (int64_t)sizeof(Err));
    e->msg = (Str *)msg;
    sn_retain(msg);
    return e;
}

void *sn_error_msg(void *p) {
    if (!p) return sn_str_from_cstr("");
    sn_retain(((Err *)p)->msg);
    return ((Err *)p)->msg;
}

void sn_panic(void *msg) {
    fprintf(stderr, "panic: %s\n", sn_str_cstr(msg));
    exit(1);
}

/* SWARM RABBIT HOLE — Wave 15 (Impl-T, Cohort Delta): Salt ε — the
 * numeric contract layer's degraded runtime assertion (the boundary
 * rule's fail-fast; the E-LKIND/E-SFC convention: fprintf + exit(1)).
 * The DBM engine is compile-time only — this is the ONLY contract
 * artifact the runtime ever sees: a condition the compile-time engine
 * could not discharge (tier L/D) or could never discharge (tier N),
 * lowered at the boundary (the call site / the fn entry for requires,
 * the fn exit for ensures). The message carries the atom text and the
 * typed classification ([E-0702]). */
void sn_contract_assert(int64_t ok, const char *msg) {
    if (!ok) {
        fprintf(stderr, "runtime error: %s\n", msg);
        exit(1);
    }
}

/* --------------------------------------------------------------------- */
/*  Manual-allocation guards                                              */
/*                                                                       */
/*  `alloc` hands out raw memory whose lifetime the programmer manages.   */
/*  A poisoned-header scheme catches the two mistakes that otherwise turn   */
/*  into silent corruption in a long-running server:                      */
/*                                                                       */
/*    double free  - the freed block's magic is overwritten, so the second */
/*                   free is reported and ignored instead of corrupting    */
/*                   the allocator                                        */
/*    use after free - reading the block first checks the magic, so the     */
/*                   read is reported instead of returning garbage         */
/*                                                                       */
/*  Guarded allocations are [magic, size, pad.. payload]; the returned     */
/*  pointer is the payload, so C libraries see an ordinary buffer.          */
/* --------------------------------------------------------------------- */

#define SN_ALLOC_MAGIC_LIVE 0x534E4C49465545ULL  /* "SNLIvE" */
#define SN_ALLOC_MAGIC_DEAD 0x534E4C49464444ULL  /* "SNLiDD" */
typedef struct AllocHeader {
    uint64_t magic;
    int64_t size;
    /* second copy of the magic, so a one-word overwrite is still caught */
    uint64_t guard;
    /* padding so the payload starts on a 16-byte boundary */
    uint64_t pad;
} AllocHeader;

/* Payload offset. Must match sizeof(AllocHeader) exactly, or the first bytes
   of the payload would overlap the guard word. */
#define SN_ALLOC_HEADER ((int64_t)sizeof(AllocHeader))

/* Freed blocks are quarantined rather than returned to malloc, so reading a
   freed block's header is safe and use-after-free can be reported instead of
   corrupting the allocator. The oldest entry is really freed once the
   quarantine is full. */
#define SN_QUARANTINE 64
static void *g_quarantine[SN_QUARANTINE];
static int64_t g_quarantine_n;
static int64_t g_quarantine_next;

static int alloc_guards_on = 1;
static int64_t g_alloc_live;
static int64_t g_alloc_freed;

void sn_alloc_guard(int on) { alloc_guards_on = on ? 1 : 0; }

int64_t sn_alloc_live_count(void) { return g_alloc_live; }
int64_t sn_alloc_freed_count(void) { return g_alloc_freed; }

static int is_guarded(void *p) {
    if (!p) return 0;
    AllocHeader *h = (AllocHeader *)((char *)p - SN_ALLOC_HEADER);
    return h->magic == SN_ALLOC_MAGIC_LIVE || h->magic == SN_ALLOC_MAGIC_DEAD;
}

void *sn_alloc(int64_t n) {
    if (n < 0) n = 0;
    if (n == 0) n = 1;
    if (!alloc_guards_on) {
        return xmalloc((size_t)n);
    }
    AllocHeader *h = (AllocHeader *)xmalloc(sizeof(AllocHeader) + (size_t)n);
    h->magic = SN_ALLOC_MAGIC_LIVE;
    h->size = n;
    h->guard = SN_ALLOC_MAGIC_LIVE;
    g_alloc_live++;
    return (char *)h + SN_ALLOC_HEADER;
}

/* Returns 0 on success, -1 when the block was already freed. */
int64_t sn_free_checked(void *p) {
    if (!p) return 0;
    if (!is_guarded(p)) {
        free(p);
        return 0;
    }
    AllocHeader *h = (AllocHeader *)((char *)p - SN_ALLOC_HEADER);
    if (h->magic == SN_ALLOC_MAGIC_DEAD || h->guard == SN_ALLOC_MAGIC_DEAD) {
        g_alloc_freed++;
        fprintf(stderr, "runtime error: double free of %p\n", p);
        return -1;
    }
    if (h->magic != SN_ALLOC_MAGIC_LIVE || h->guard != SN_ALLOC_MAGIC_LIVE) {
        /* Header damaged: refuse to free, we cannot tell what it is. */
        fprintf(stderr, "runtime error: free of corrupted block %p\n", p);
        return -1;
    }
    h->magic = SN_ALLOC_MAGIC_DEAD;
    h->guard = SN_ALLOC_MAGIC_DEAD;
    g_alloc_live--;
    /* Quarantine: keep the header readable, hand the memory back only once the
       quarantine is full. */
    if (g_quarantine_n < SN_QUARANTINE) {
        g_quarantine[g_quarantine_n++] = h;
    } else {
        free(g_quarantine[g_quarantine_next]);
        g_quarantine[g_quarantine_next] = h;
        g_quarantine_next = (g_quarantine_next + 1) % SN_QUARANTINE;
    }
    return 0;
}

void sn_free(void *p) {
    if (!p) return;
    sn_free_checked(p);
}

/* Releases everything still quarantined at shutdown. */
static void alloc_quarantine_flush(void) {
    int64_t i;
    for (i = 0; i < g_quarantine_n; i++) free(g_quarantine[i]);
    g_quarantine_n = 0;
    g_quarantine_next = 0;
}

/* Detects reads of a freed block. Returns 0 when the block is live. */
int64_t sn_check_live(void *p) {
    if (!p) return 0;
    if (!is_guarded(p)) return 1;
    AllocHeader *h = (AllocHeader *)((char *)p - SN_ALLOC_HEADER);
    if (h->magic != SN_ALLOC_MAGIC_LIVE || h->guard != SN_ALLOC_MAGIC_LIVE) {
        fprintf(stderr, "runtime error: use after free of %p\n", p);
        return 0;
    }
    return 1;
}

/* Live byte count of a guarded block, or -1 if not guarded. */
int64_t sn_alloc_size(void *p) {
    if (!p || !is_guarded(p)) return -1;
    return ((AllocHeader *)((char *)p - SN_ALLOC_HEADER))->size;
}

void *sn_box_i64(int64_t v) {
    BoxI64 *b = (BoxI64 *)xmalloc(sizeof(BoxI64));
    b->rc = 1;
    b->kind = KIND_BOX;
    b->val = v;
    return b;
}

int64_t sn_unbox_i64(void *p) {
    if (!p) return 0;
    return ((BoxI64 *)p)->val;
}

int64_t sn_pow_i64(int64_t base, int64_t exp) {
    if (exp < 0) return 0;
    int64_t r = 1;
    while (exp) {
        if (exp & 1) r *= base;
        base *= base;
        exp >>= 1;
    }
    return r;
}

int64_t sn_div_i64(int64_t a, int64_t b) {
    if (b == 0) {
        fprintf(stderr, "division by zero\n");
        exit(1);
    }
    return a / b;
}

int64_t sn_mod_i64(int64_t a, int64_t b) {
    if (b == 0) {
        fprintf(stderr, "division by zero\n");
        exit(1);
    }
    return a % b;
}

void *sn_obj_new(int64_t size, int64_t type_id, void *vptr) {
    if (size < 24) size = 24;
    int64_t *p = (int64_t *)xmalloc((size_t)size);
    p[0] = 1;
    p[1] = KIND_USER + type_id;
    p[2] = (int64_t)(intptr_t)vptr;
    gc_alloc(p, size);
    return p;
}

void *sn_chan_new(void) {
    Chan *c = (Chan *)xmalloc(sizeof(Chan));
    c->rc = 1;
    c->kind = KIND_CHAN;
    gc_track(c, (int64_t)sizeof(Chan));
    c->closed = 0;
    c->len = 0;
    c->cap = 8;
    c->head = 0;
    c->buf = (int64_t *)xmalloc(sizeof(int64_t) * 8);
#ifdef _WIN32
    InitializeCriticalSection(&c->mu);
    InitializeConditionVariable(&c->cv);
#else
    pthread_mutex_init(&c->mu, NULL);
    pthread_cond_init(&c->cv, NULL);
#endif
    return c;
}

void sn_chan_notify(void);

int64_t sn_chan_send(void *p, int64_t v) {
    Chan *c = (Chan *)p;
    if (!c) return 0;
#ifdef _WIN32
    EnterCriticalSection(&c->mu);
#else
    pthread_mutex_lock(&c->mu);
#endif
    if (c->closed) {
#ifdef _WIN32
        LeaveCriticalSection(&c->mu);
#else
        pthread_mutex_unlock(&c->mu);
#endif
        return 0;
    }
    if (c->len == c->cap) {
        int64_t *nb = (int64_t *)xmalloc(sizeof(int64_t) * (size_t)c->cap * 2);
        for (int64_t i = 0; i < c->len; i++) nb[i] = c->buf[(c->head + i) % c->cap];
        free(c->buf);
        c->buf = nb;
        c->head = 0;
        c->cap *= 2;
    }
    c->buf[(c->head + c->len) % c->cap] = v;
    c->len++;
#ifdef _WIN32
    WakeConditionVariable(&c->cv);
    LeaveCriticalSection(&c->mu);
#else
    pthread_cond_signal(&c->cv);
    pthread_mutex_unlock(&c->mu);
#endif
    sn_chan_notify();
    return 1;
}

int64_t sn_chan_recv_ok(void *p, int64_t *out_val) {
    Chan *c = (Chan *)p;
    if (!c) {
        if (out_val) *out_val = 0;
        return 0;
    }
#ifdef _WIN32
    EnterCriticalSection(&c->mu);
    while (c->len == 0 && !c->closed) SleepConditionVariableCS(&c->cv, &c->mu, INFINITE);
#else
    pthread_mutex_lock(&c->mu);
    while (c->len == 0 && !c->closed) pthread_cond_wait(&c->cv, &c->mu);
#endif
    if (c->len == 0) {
#ifdef _WIN32
        LeaveCriticalSection(&c->mu);
#else
        pthread_mutex_unlock(&c->mu);
#endif
        if (out_val) *out_val = 0;
        return 0;
    }
    int64_t v = c->buf[c->head];
    c->head = (c->head + 1) % c->cap;
    c->len--;
#ifdef _WIN32
    LeaveCriticalSection(&c->mu);
#else
    pthread_mutex_unlock(&c->mu);
#endif
    if (out_val) *out_val = v;
    return 1;
}

int64_t sn_chan_recv(void *p) {
    int64_t v = 0;
    sn_chan_recv_ok(p, &v);
    return v;
}

void sn_chan_close(void *p) {
    Chan *c = (Chan *)p;
#ifdef _WIN32
    EnterCriticalSection(&c->mu);
    c->closed = 1;
    WakeAllConditionVariable(&c->cv);
    LeaveCriticalSection(&c->mu);
#else
    pthread_mutex_lock(&c->mu);
    c->closed = 1;
    pthread_cond_broadcast(&c->cv);
    pthread_mutex_unlock(&c->mu);
#endif
    sn_chan_notify();
}

int64_t sn_chan_open(void *p) { return p && !((Chan *)p)->closed; }

void *sn_lock_new(void) {
    Lock *l = (Lock *)xmalloc(sizeof(Lock));
    l->rc = 1;
    l->kind = KIND_LOCK;
    gc_track(l, (int64_t)sizeof(Lock));
#ifdef _WIN32
    InitializeCriticalSection(&l->mu);
#else
    pthread_mutex_init(&l->mu, NULL);
#endif
    return l;
}

void sn_lock_acq(void *p) {
    Lock *l = (Lock *)p;
#ifdef _WIN32
    EnterCriticalSection(&l->mu);
#else
    pthread_mutex_lock(&l->mu);
#endif
}

void sn_lock_rel(void *p) {
    Lock *l = (Lock *)p;
#ifdef _WIN32
    LeaveCriticalSection(&l->mu);
#else
    pthread_mutex_unlock(&l->mu);
#endif
}

/* --------------------------------------------------------------------- */
/*  Structured Concurrency Nursery Runtime                               */
/* --------------------------------------------------------------------- */
/* SWARM RABBIT HOLE, D0 gap #5 (Impl-F, Wave 6): the nursery child-lane
   carve. A nursery child's scratch is a ν lane — one of the child's own
   thread's TLS lanes, reserved by pointing SnLane.parent (Impl-A's field)
   at the nursery's parent_lane and registered on the intrusive list below.
   sn_lane is defined in the lane section at the bottom of this file; loan
   bookkeeping only ever holds pointers to it, so the incomplete type is
   enough here. The loan/reserve/drain implementations live with the lane
   machinery (they touch g_tls_lanes), forward-declared below. */
struct SnLane;
typedef struct SnLoan {
    struct SnLane *lane;   /* the loaned ν lane (in the child's TLS array) */
    struct SnLoan *next;   /* intrusive LIFO registry link */
} SnLoan;
#ifdef _WIN32
typedef struct SnNursery {
    CRITICAL_SECTION mu;
    CONDITION_VARIABLE cv;
    int64_t active_tasks;
    /* SWARM RABBIT HOLE DIFF-3 / W4A repair 4: the supervisor's registry
       anchor — every envelope a task of this group sent but no receiver has
       adopted yet sits on this intrusive list; sn_nursery_wait drains it
       (queue-drain rule) so a join frees exactly the un-adopted slabs. */
    SnMsgEnvelope *env_head;
    /* SWARM RABBIT HOLE, D0 gap #5 (Impl-F, Wave 6): the child-lane carve.
       parent_lane = the spawning thread's lane 0, the ⊑ₗ anchor every
       loaned ν lane's SnLane.parent points at (W2A §1.1: parent ⊑ₗ ν_child);
       loan_head = the intrusive registry of ν lanes loaned to this group's
       children — sn_nursery_wait drains it with the 1-store reset. */
    struct SnLane *parent_lane;
    SnLoan *loan_head;
} SnNursery;
#else
typedef struct SnNursery {
    pthread_mutex_t mu;
    pthread_cond_t cv;
    int64_t active_tasks;
    /* SWARM RABBIT HOLE DIFF-3 / W4A repair 4: the supervisor's registry
       anchor — every envelope a task of this group sent but no receiver has
       adopted yet sits on this intrusive list; sn_nursery_wait drains it
       (queue-drain rule) so a join frees exactly the un-adopted slabs. */
    SnMsgEnvelope *env_head;
    /* SWARM RABBIT HOLE, D0 gap #5 (Impl-F, Wave 6): the child-lane carve.
       parent_lane = the spawning thread's lane 0, the ⊑ₗ anchor every
       loaned ν lane's SnLane.parent points at (W2A §1.1: parent ⊑ₗ ν_child);
       loan_head = the intrusive registry of ν lanes loaned to this group's
       children — sn_nursery_wait drains it with the 1-store reset. */
    struct SnLane *parent_lane;
    SnLoan *loan_head;
} SnNursery;
#endif
static void sn_nursery_loan_drain(SnNursery *n); /* lane section below */
void sn_lane_loan_task_end(void);                /* lane section below */
static struct SnLane *sn_lane_ensure(int64_t lane_id); /* lane section below */

void *sn_nursery_new(void) {
    SnNursery *n = (SnNursery *)xmalloc(sizeof(SnNursery));
    n->active_tasks = 0;
    n->env_head = NULL; /* DIFF-3: registry starts empty */
    n->loan_head = NULL; /* D0 gap #5: ν registry starts empty */
    /* D0 gap #5: the spawning thread's lane 0 is the ⊑ₗ anchor. Ensuring it
       here (on the parent thread) materializes this thread's TLS lane and
       pins the pointer the children's loans will point at. */
    n->parent_lane = sn_lane_ensure(0);
#ifdef _WIN32
    InitializeCriticalSection(&n->mu);
    InitializeConditionVariable(&n->cv);
#else
    pthread_mutex_init(&n->mu, NULL);
    pthread_cond_init(&n->cv, NULL);
#endif
    return n;
}

void sn_nursery_enter_task(void *nursery) {
    if (!nursery) return;
    SnNursery *n = (SnNursery *)nursery;
#ifdef _WIN32
    EnterCriticalSection(&n->mu);
    n->active_tasks++;
    LeaveCriticalSection(&n->mu);
#else
    pthread_mutex_lock(&n->mu);
    n->active_tasks++;
    pthread_mutex_unlock(&n->mu);
#endif
}

void sn_nursery_leave_task(void *nursery) {
    /* D0 gap #5 (Impl-F): a nursery child ends with this call (the compiled
       child emits it at task end). Its ν loan permission dies with the task —
       a later task scheduled on this worker thread must not be able to
       allocate on the still-loaned lane; the join owns it until the reset.
       The loan itself stays registered until sn_nursery_wait drains it. */
    sn_lane_loan_task_end();
    if (!nursery) return;
    SnNursery *n = (SnNursery *)nursery;
#ifdef _WIN32
    EnterCriticalSection(&n->mu);
    n->active_tasks--;
    if (n->active_tasks == 0) {
        WakeAllConditionVariable(&n->cv);
    }
    LeaveCriticalSection(&n->mu);
#else
    pthread_mutex_lock(&n->mu);
    n->active_tasks--;
    if (n->active_tasks == 0) {
        pthread_cond_broadcast(&n->cv);
    }
    pthread_mutex_unlock(&n->mu);
#endif
}

/* --------------------------------------------------------------------- */
/*  Linear message envelopes — DIFF-3 runtime slice (W4A repairs 4 & 6c) */
/*                                                                        */
/*  State machine (extends STRATA §2.5.3 to envelopes, W2-E DIFF-3):      */
/*    sn_msg_pack()        : Uninit -> Packed                             */
/*    sn_msg_send(g,c,e)   : Packed -> Queued  (release-store, W4A 6c)    */
/*    sn_msg_recv(c)       : Queued -> Received (acquire CAS = HB edge)   */
/*    sn_msg_adopt(e,dst)  : Received -> Adopted (bytes land on the       */
/*                           receiver's own lane/frame; slab freed;       */
/*                           exactly once)                                */
/*    sn_msg_drop(e)       : Received -> Freed                            */
/*    group join/abort     : Queued -> Freed (CAS under the queue lock,    */
/*                           W4A repair 4 queue-drain rule)               */
/*                                                                        */
/*  A single holder owns the Live state at every point; the atomic `refs` */
/*  count guards only the HEADER against the one race the CAS cannot      */
/*  order: a receiver that popped the pointer but has not yet CAS'd while */
/*  the drain frees the slab (the failed-CAS receiver then releases the    */
/*  header exactly once itself).                                          */
/* --------------------------------------------------------------------- */

static void sn_env_retain(SnMsgEnvelope *env) {
    if (env) sn_atomic_retain(&env->refs);
}

static void sn_env_release(SnMsgEnvelope *env) {
    if (env && sn_atomic_release_is_last(&env->refs)) free(env);
}

void *sn_msg_pack(int64_t len) {
    SnMsgEnvelope *env = (SnMsgEnvelope *)xmalloc(sizeof(SnMsgEnvelope));
    env->state = SN_ENV_PACKED;
    env->refs = 1; /* the packer's reference; sn_msg_send consumes it */
    env->len = len > 0 ? len : 0;
    env->chan = NULL;
    env->next_group = NULL;
    /* DIFF-3: malloc'd detached buffer — NEVER a lane, NEVER gc_track'd.
       NOT zeroed: zeroing is exactly the O(n) cost DIFF-6 drops; the sender
       writes the payload before send's release-store publishes it. */
    env->slab = NULL;
    if (env->len > 0) {
        env->slab = malloc((size_t)env->len);
        if (!env->slab) {
            fprintf(stderr, "out of memory\n");
            exit(1);
        }
    }
    return env;
}

/* DIFF-3: sender-side payload surface. The sender writes the slab bytes
   through this pointer BEFORE sn_msg_send's release-store publishes them
   (W4A repair 6c: the release at send orders the initialization writes
   before the state publication). Read-only accessors for diagnostics. */
void *sn_msg_slab(void *envp) {
    SnMsgEnvelope *env = (SnMsgEnvelope *)envp;
    return env ? env->slab : NULL;
}

int64_t sn_msg_len(void *envp) {
    SnMsgEnvelope *env = (SnMsgEnvelope *)envp;
    return env ? env->len : 0;
}

int64_t sn_msg_send(void *group, void *chan, void *envp) {
    SnMsgEnvelope *env = (SnMsgEnvelope *)envp;
    if (!env) return 0;
    env->chan = chan;
    /* DIFF-3 Rule 3b: register with the supervisor (the structured spawn
       scope's task group) BEFORE publishing, so the envelope can never be
       invisible to the join-time drain. The registry reference keeps the
       header alive until the group's join releases it. */
    if (group) {
        SnNursery *n = (SnNursery *)group;
#ifdef _WIN32
        EnterCriticalSection(&n->mu);
        env->next_group = n->env_head;
        n->env_head = env;
        LeaveCriticalSection(&n->mu);
#else
        pthread_mutex_lock(&n->mu);
        env->next_group = n->env_head;
        n->env_head = env;
        pthread_mutex_unlock(&n->mu);
#endif
        sn_env_retain(env); /* the registry's reference */
    }
    /* W4A repair 6c: release-store the header AFTER all slab writes at
       pack/send — the release publishes the sender's initialization; the
       receiver's acquire at recv/adopt forms the HB edge (the send-publish /
       adopt-observe pair plays for slab transfer the role join() plays for
       task results). */
    sn_atomic_store_rel(&env->state, SN_ENV_QUEUED);
    int64_t ok = sn_chan_send(chan, (int64_t)(intptr_t)env);
    if (ok) {
        sn_env_retain(env); /* the queue's reference (travels with the value) */
    } else {
        /* The value never entered a queue, so no receiver can exist: the
           send consumed the envelope (DIFF-3 linearity) — retire it here;
           the registry (if any) releases its header at group join. */
        (void)sn_atomic_cas_acqrel(&env->state, SN_ENV_QUEUED, SN_ENV_FREED);
        free(env->slab);
        env->slab = NULL;
    }
    sn_env_release(env); /* the sender's reference: send consumes it */
    return ok;
}

int64_t sn_msg_recv(void *chan) {
    int64_t v = 0;
    if (!sn_chan_recv_ok(chan, &v) || v == 0) return 0;
    SnMsgEnvelope *env = (SnMsgEnvelope *)(intptr_t)v;
    /* The queue's reference is now ours (it traveled with the popped value).
       W4A repair 6c: acquire-load the header before any adopt-side read of
       the slab bytes; the CAS (Queued -> Received) is the single
       linearization point against the drain's Queued -> Freed (repair 4):
       a racing drain's CAS fails, so the receiver never sees a freed slab. */
    int64_t st = sn_atomic_load_acq(&env->state);
    if (st != SN_ENV_QUEUED ||
        !sn_atomic_cas_acqrel(&env->state, SN_ENV_QUEUED, SN_ENV_RECEIVED)) {
        /* The group's join-time drain freed the slab while it sat queued;
           the header is spent — release the popped reference. */
        sn_env_release(env);
        return 0;
    }
    return (int64_t)(intptr_t)env;
}

int64_t sn_msg_adopt(void *envp, void *dst, int64_t cap) {
    SnMsgEnvelope *env = (SnMsgEnvelope *)envp;
    if (!env) return 0;
    /* W4A repair 6c: acquire-load before any slab read. Adopt is exactly-once
       (DIFF-3): re-use of a Consumed envelope is E0806 at compile time; at
       runtime it aborts rather than alias (double-adopt = cross-thread UAF,
       the G1 A4b hole this machine closes). */
    int64_t st = sn_atomic_load_acq(&env->state);
    if (st != SN_ENV_RECEIVED ||
        !sn_atomic_cas_acqrel(&env->state, SN_ENV_RECEIVED, SN_ENV_ADOPTED)) {
        fprintf(stderr, "runtime error: adopt of a non-Received message envelope\n");
        exit(1);
    }
    int64_t n = env->len;
    if (cap >= 0 && n > cap) n = cap;
    if (n > 0 && dst && env->slab) memcpy(dst, env->slab, (size_t)n);
    /* DIFF-3: the bytes re-enter Stratum 0/1 at the receiver — the slab is
       freed and the envelope is spent. */
    free(env->slab);
    env->slab = NULL;
    env->len = 0;
    sn_env_release(env); /* the receiver's reference */
    return n;
}

void sn_msg_drop(void *envp) {
    SnMsgEnvelope *env = (SnMsgEnvelope *)envp;
    if (!env) return;
    /* DIFF-3 env.drop(): explicit discard of a Received envelope; the slab
       is freed and the envelope is spent. Any use of a Consumed envelope is
       E0806 at compile time; at runtime it aborts rather than double-free
       (mirrors sn_msg_adopt's exactly-once guard). */
    if (!sn_atomic_cas_acqrel(&env->state, SN_ENV_RECEIVED, SN_ENV_FREED)) {
        fprintf(stderr, "runtime error: drop of a non-Received message envelope\n");
        exit(1);
    }
    free(env->slab);
    env->slab = NULL;
    env->len = 0;
    sn_env_release(env);
}

static void sn_chan_mu_lock(Chan *c) {
#ifdef _WIN32
    EnterCriticalSection(&c->mu);
#else
    pthread_mutex_lock(&c->mu);
#endif
}

static void sn_chan_mu_unlock(Chan *c) {
#ifdef _WIN32
    LeaveCriticalSection(&c->mu);
#else
    pthread_mutex_unlock(&c->mu);
#endif
}

/* W4A repair 4 (queue-drain rule): at task-group join — and, in a later
   slice, abort — free every QUEUED (sent-but-unreceived) slab by CASing the
   header Queued -> Freed, then dequeue the buffered value UNDER THE QUEUE
   LOCK so no late receiver pops a pointer to a spent envelope. The CAS is
   the single linearization point: a racing receiver's Queued -> Received CAS
   fails and the receiver never touches the freed slab. The registry's
   env->chan reaches every channel this group's tasks sent to (W4A repair 4
   residual: the supervisor holds the channel list through the registry). */
static void sn_env_drain_group(SnNursery *n) {
    SnMsgEnvelope *env = n->env_head;
    n->env_head = NULL;
    while (env) {
        SnMsgEnvelope *next = env->next_group;
        env->next_group = NULL;
        int64_t st = sn_atomic_load_acq(&env->state);
        if (st == SN_ENV_QUEUED &&
            sn_atomic_cas_acqrel(&env->state, SN_ENV_QUEUED, SN_ENV_FREED)) {
            /* Won the race: no receiver adopted it. Free the slab... */
            free(env->slab);
            env->slab = NULL;
            /* ...and, under the queue lock, remove the buffered value so the
               queue no longer references the spent header. If the value is
               already gone, a receiver popped it and holds the queue's
               reference — its CAS on the FREED state fails and it releases
               the header itself; the refcount frees the header exactly once
               either way. */
            Chan *c = (Chan *)env->chan;
            if (c) {
                int64_t want = (int64_t)(intptr_t)env;
                int found = 0;
                sn_chan_mu_lock(c);
                for (int64_t i = 0; i < c->len; i++) {
                    if (c->buf[(c->head + i) % c->cap] == want) {
                        for (int64_t j = i; j + 1 < c->len; j++) {
                            c->buf[(c->head + j) % c->cap] =
                                c->buf[(c->head + j + 1) % c->cap];
                        }
                        c->len--;
                        found = 1;
                        break;
                    }
                }
                sn_chan_mu_unlock(c);
                if (found) sn_env_release(env); /* the queue's reference */
            }
        }
        sn_env_release(env); /* the registry's reference */
        env = next;
    }
}

void sn_nursery_wait(void *nursery) {
    if (!nursery) return;
    SnNursery *n = (SnNursery *)nursery;
#ifdef _WIN32
    EnterCriticalSection(&n->mu);
    while (n->active_tasks > 0) {
        SleepConditionVariableCS(&n->cv, &n->mu, INFINITE);
    }
    LeaveCriticalSection(&n->mu);
#else
    pthread_mutex_lock(&n->mu);
    while (n->active_tasks > 0) {
        pthread_cond_wait(&n->cv, &n->mu);
    }
    pthread_mutex_unlock(&n->mu);
#endif
    /* DIFF-3 Rule 3b / W4A repair 4 (panic-safe join): before the group dies,
       drain the registry — free every QUEUED (sent-but-unreceived) slab via
       the CAS header under the queue lock. All of this group's tasks have
       left above, so there are no concurrent registrants. */
    sn_env_drain_group(n);
    /* SWARM RABBIT HOLE, D0 gap #5 (Impl-F, Wave 6) / W2A §1.1, §2.2 A.1
       (C-JOIN, SR3): the join resets every ν lane this group's children
       loaned — one store per lane (top = base), LIFO over the registry,
       then each lane returns to its thread's free set. All of the group's
       tasks have left above, so no child can still be allocating; the
       child's ENTIRE lane allocation reclaims in O(1) (W2A Theorem 1(iii):
       the single-store pop license). Panic-safe by the same argument as
       the envelope drain: the walk runs on every path that reaches the
       join, after the last leave_task. */
    sn_nursery_loan_drain(n);
#ifdef _WIN32
    DeleteCriticalSection(&n->mu);
    free(n);
#else
    pthread_mutex_destroy(&n->mu);
    pthread_cond_destroy(&n->cv);
    free(n);
#endif
}

typedef struct SpawnJob {
    void (*fn)(void *);
    void *arg;
} SpawnJob;

#ifdef _WIN32
static unsigned __stdcall spawn_tramp(void *p) {
#else
/* Records the stack a to-be-created thread will get, so its frames count as
   GC roots once it registers itself. */
static pthread_attr_t *g_spawn_attr;
static void sn_spawn_set_stack_attr(pthread_attr_t *attr) {
    g_spawn_attr = attr;
}

static void *spawn_tramp(void *p) {
#endif
    SpawnJob *j = (SpawnJob *)p;
    char anchor = 0;
    {
        void *lo = NULL, *hi = NULL;
        if (gc_stack_bounds(&lo, &hi)) {
            sn_gc_register_thread(lo, hi);
        } else {
#ifndef _WIN32
            /* POSIX only: g_spawn_attr records the about-to-be-created thread's
               stack so its frames count as GC roots. Windows threads created via
               _beginthreadex fall back to the anchor window below. */
            if (g_spawn_attr) {
                void *addr = NULL;
                size_t sz = 0;
                if (pthread_attr_getstack(g_spawn_attr, &addr, &sz) == 0 && addr && sz) {
                    lo = addr;
                    hi = (char *)addr + sz;
                }
            }
#endif
            if (lo && hi) {
                sn_gc_register_thread(lo, hi);
            } else {
                sn_gc_register_thread((char *)&anchor + (1 << 20), &anchor);
            }
        }
    }
    sn_gc_enter();
    j->fn(j->arg);
    sn_gc_leave();
    free(j);
#ifdef _WIN32
    EnterCriticalSection(&g_spawn_mu);
    g_live_spawns--;
    WakeConditionVariable(&g_spawn_cv);
    LeaveCriticalSection(&g_spawn_mu);
    return 0;
#else
    pthread_mutex_lock(&g_spawn_mu);
    g_live_spawns--;
    pthread_cond_signal(&g_spawn_cv);
    pthread_mutex_unlock(&g_spawn_mu);
    return NULL;
#endif
}

void sn_spawn(void (*fn)(void *), void *arg) {
    SpawnJob *j = (SpawnJob *)xmalloc(sizeof(SpawnJob));
    j->fn = fn;
    j->arg = arg;
#ifdef _WIN32
    EnterCriticalSection(&g_spawn_mu);
    g_live_spawns++;
    LeaveCriticalSection(&g_spawn_mu);
    uintptr_t th = _beginthreadex(NULL, 0, spawn_tramp, j, 0, NULL);
    if (th) CloseHandle((HANDLE)th);
#else
    pthread_mutex_lock(&g_spawn_mu);
    g_live_spawns++;
    pthread_mutex_unlock(&g_spawn_mu);
    pthread_t t;
    {
        pthread_attr_t attr;
        if (pthread_attr_init(&attr) == 0) {
            pthread_attr_setstacksize(&attr, 1024 * 1024);
            sn_spawn_set_stack_attr(&attr);
            pthread_create(&t, &attr, spawn_tramp, j);
            pthread_attr_destroy(&attr);
        } else {
            pthread_create(&t, NULL, spawn_tramp, j);
        }
    }
    pthread_detach(t);
#endif
}

static int g_go_pending = 0;
#ifdef _WIN32
static CRITICAL_SECTION g_go_mu;
static CONDITION_VARIABLE g_go_cv;
#else
static pthread_mutex_t g_go_mu = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t g_go_cv = PTHREAD_COND_INITIALIZER;
#endif

static void rx_stop(void);
static void alloc_quarantine_flush(void);

/* ------------------------------------------------------------------ */
/* Line coverage counters. `snc test --coverage` compiles with counters */
/* compiled in; each distinct source line gets one slot, allocated by   */
/* the compiler and passed here as its id. Counts are dumped to the file */
/* named by $SN_COVERAGE_OUT at shutdown.                              */
/* ------------------------------------------------------------------ */

#define SN_COV_SLOTS 262144
static uint64_t *g_cov = NULL;
static size_t g_cov_used = 0;

void sn_cov_hit(int64_t slot) {
    if (!g_cov) {
        g_cov = (uint64_t *)calloc(SN_COV_SLOTS, sizeof(uint64_t));
        if (!g_cov) return;
    }
    if (slot < 0 || (size_t)slot >= SN_COV_SLOTS) return;
    g_cov[slot]++;
    size_t end = (size_t)slot + 1;
    if (end > g_cov_used) g_cov_used = end;
}

static void cov_dump(void) {
    if (!g_cov || g_cov_used == 0) return;
    const char *out = getenv("SN_COVERAGE_OUT");
    if (!out || !*out) return;
    FILE *f = fopen(out, "a");
    if (!f) return;
    for (size_t i = 0; i < g_cov_used; i++) {
        if (g_cov[i]) fprintf(f, "%zu %llu\n", i, (unsigned long long)g_cov[i]);
    }
    fclose(f);
    free(g_cov);
    g_cov = NULL;
    g_cov_used = 0;
}

void sn_rt_shutdown(void) {
#ifdef _WIN32
    EnterCriticalSection(&g_spawn_mu);
    while (g_live_spawns > 0) SleepConditionVariableCS(&g_spawn_cv, &g_spawn_mu, INFINITE);
    LeaveCriticalSection(&g_spawn_mu);
#else
    pthread_mutex_lock(&g_spawn_mu);
    while (g_live_spawns > 0) pthread_cond_wait(&g_spawn_cv, &g_spawn_mu);
    pthread_mutex_unlock(&g_spawn_mu);
    pthread_mutex_lock(&g_go_mu);
    while (g_go_pending > 0) pthread_cond_wait(&g_go_cv, &g_go_mu);
    pthread_mutex_unlock(&g_go_mu);
#endif
    /* Let the reactor finish in-flight timers/sockets, then stop it. */
    rx_stop();
    alloc_quarantine_flush();
    cov_dump();
}

typedef struct Clos {
    int64_t rc;
    int64_t kind;
    void *fn;
    void *env;
} Clos;

void *sn_clos_new(void *fn, void *env) {
    Clos *c = (Clos *)xmalloc(sizeof(Clos));
    c->rc = 1;
    c->kind = KIND_CLOS;
    gc_track(c, (int64_t)sizeof(Clos));
    c->fn = fn;
    c->env = env;
    return c;
}

void *sn_clos_fn(void *p) { return p ? ((Clos *)p)->fn : NULL; }
void *sn_clos_env(void *p) { return p ? ((Clos *)p)->env : NULL; }

static void *sn_clos_call1(void *clos, void *arg) {
    Clos *c = (Clos *)clos;
    if (!c || !c->fn) return sn_str_from_cstr("");
    void *(*fn)(void *, void *) = (void *(*)(void *, void *))(uintptr_t)c->fn;
    return fn(c->env, arg);
}

/* JSON: [rc, kind, tag, ival, pval] */
enum { J_NULL = 0, J_BOOL = 1, J_INT = 2, J_STR = 3, J_ARR = 4, J_OBJ = 5 };

static void *json_new(int64_t tag, int64_t ival, void *pval) {
    int64_t *j = (int64_t *)xmalloc(sizeof(int64_t) * 5);
    j[0] = 1;
    j[1] = KIND_JSON;
    j[2] = tag;
    j[3] = ival;
    j[4] = (int64_t)(intptr_t)pval;
    return j;
}

static int64_t json_tag(void *p) { return p ? ((int64_t *)p)[2] : J_NULL; }
static int64_t json_ival(void *p) { return p ? ((int64_t *)p)[3] : 0; }
static void *json_pval(void *p) {
    return p ? (void *)(intptr_t)((int64_t *)p)[4] : NULL;
}

typedef struct JParse {
    const char *s;
    size_t n;
    size_t i;
    const char *err;
} JParse;

static void jskip(JParse *p) {
    while (p->i < p->n && (p->s[p->i] == ' ' || p->s[p->i] == '\n' || p->s[p->i] == '\r' || p->s[p->i] == '\t'))
        p->i++;
}

static void *jparse_value(JParse *p);

static int jhex(char c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    return -1;
}

static void *jparse_string(JParse *p) {
    if (p->i >= p->n || p->s[p->i] != '"') {
        p->err = "expected string";
        return NULL;
    }
    p->i++;
    size_t cap = 32, n = 0;
    char *buf = (char *)xmalloc(cap);
    while (p->i < p->n && p->s[p->i] != '"') {
        char c = p->s[p->i++];
        if (c == '\\') {
            if (p->i >= p->n) break;
            char e = p->s[p->i++];
            switch (e) {
            case 'n': c = '\n'; break;
            case 't': c = '\t'; break;
            case 'r': c = '\r'; break;
            case '"': c = '"'; break;
            case '\\': c = '\\'; break;
            case 'u': {
                int h = 0;
                for (int k = 0; k < 4 && p->i < p->n; k++) {
                    int d = jhex(p->s[p->i++]);
                    if (d < 0) break;
                    h = (h << 4) | d;
                }
                c = (char)(h < 128 ? h : '?');
                break;
            }
            default: c = e; break;
            }
        }
        if (n + 1 >= cap) {
            cap *= 2;
            buf = (char *)realloc(buf, cap);
        }
        buf[n++] = c;
    }
    if (p->i < p->n && p->s[p->i] == '"') p->i++;
    else {
        p->err = "unterminated string";
        free(buf);
        return NULL;
    }
    void *st = sn_str_new(buf, (int64_t)n);
    free(buf);
    return json_new(J_STR, 0, st);
}

static void *jparse_number(JParse *p) {
    int neg = 0;
    if (p->i < p->n && p->s[p->i] == '-') {
        neg = 1;
        p->i++;
    }
    int64_t v = 0;
    if (p->i >= p->n || p->s[p->i] < '0' || p->s[p->i] > '9') {
        p->err = "invalid number";
        return NULL;
    }
    while (p->i < p->n && p->s[p->i] >= '0' && p->s[p->i] <= '9') {
        v = v * 10 + (p->s[p->i] - '0');
        p->i++;
    }
    if (p->i < p->n && p->s[p->i] == '.') {
        p->i++;
        while (p->i < p->n && p->s[p->i] >= '0' && p->s[p->i] <= '9') p->i++;
    }
    if (neg) v = -v;
    return json_new(J_INT, v, NULL);
}

static void *jparse_array(JParse *p) {
    p->i++;
    List *l = (List *)sn_list_new(1);
    jskip(p);
    if (p->i < p->n && p->s[p->i] == ']') {
        p->i++;
        return json_new(J_ARR, 0, l);
    }
    while (1) {
        void *v = jparse_value(p);
        if (p->err) return NULL;
        sn_list_push_ptr(l, v);
        sn_release(v);
        jskip(p);
        if (p->i < p->n && p->s[p->i] == ',') {
            p->i++;
            jskip(p);
            continue;
        }
        if (p->i < p->n && p->s[p->i] == ']') {
            p->i++;
            break;
        }
        p->err = "expected , or ]";
        return NULL;
    }
    return json_new(J_ARR, 0, l);
}

static void *jparse_object(JParse *p) {
    p->i++;
    Map *m = (Map *)sn_map_new(1, 1);
    jskip(p);
    if (p->i < p->n && p->s[p->i] == '}') {
        p->i++;
        return json_new(J_OBJ, 0, m);
    }
    while (1) {
        jskip(p);
        void *ks = jparse_string(p);
        if (p->err || !ks) return NULL;
        void *kstr = json_pval(ks);
        jskip(p);
        if (p->i >= p->n || p->s[p->i] != ':') {
            p->err = "expected :";
            return NULL;
        }
        p->i++;
        void *v = jparse_value(p);
        if (p->err) return NULL;
        sn_map_set(m, (int64_t)(intptr_t)kstr, (int64_t)(intptr_t)v);
        sn_release(ks);
        sn_release(v);
        jskip(p);
        if (p->i < p->n && p->s[p->i] == ',') {
            p->i++;
            continue;
        }
        if (p->i < p->n && p->s[p->i] == '}') {
            p->i++;
            break;
        }
        p->err = "expected , or }";
        return NULL;
    }
    return json_new(J_OBJ, 0, m);
}

static void *jparse_value(JParse *p) {
    jskip(p);
    if (p->i >= p->n) {
        p->err = "unexpected end";
        return NULL;
    }
    char c = p->s[p->i];
    if (c == '"') return jparse_string(p);
    if (c == '{') return jparse_object(p);
    if (c == '[') return jparse_array(p);
    if (c == '-' || (c >= '0' && c <= '9')) return jparse_number(p);
    if (p->i + 4 <= p->n && strncmp(p->s + p->i, "true", 4) == 0) {
        p->i += 4;
        return json_new(J_BOOL, 1, NULL);
    }
    if (p->i + 5 <= p->n && strncmp(p->s + p->i, "false", 5) == 0) {
        p->i += 5;
        return json_new(J_BOOL, 0, NULL);
    }
    if (p->i + 4 <= p->n && strncmp(p->s + p->i, "null", 4) == 0) {
        p->i += 4;
        return json_new(J_NULL, 0, NULL);
    }
    p->err = "invalid json";
    return NULL;
}

void sn_json_parse(void *src, void **out_json, void **out_err) {
    const char *s = sn_str_cstr(src);
    JParse p = {s, strlen(s), 0, NULL};
    void *v = jparse_value(&p);
    jskip(&p);
    if (!p.err && p.i != p.n) p.err = "trailing data";
    if (p.err) {
        if (out_json) *out_json = json_new(J_NULL, 0, NULL);
        if (out_err) *out_err = sn_error_new(sn_str_from_cstr(p.err));
        return;
    }
    if (out_json) *out_json = v;
    if (out_err) *out_err = NULL;
}

static void json_encode_into(void *p, char **buf, size_t *n, size_t *cap);

static void jappend(char **buf, size_t *n, size_t *cap, const char *s, size_t ln) {
    if (*n + ln + 1 > *cap) {
        *cap = (*n + ln + 1) * 2;
        *buf = (char *)realloc(*buf, *cap);
    }
    memcpy(*buf + *n, s, ln);
    *n += ln;
    (*buf)[*n] = 0;
}

static void json_encode_into(void *p, char **buf, size_t *n, size_t *cap) {
    int64_t tag = json_tag(p);
    char tmp[64];
    if (tag == J_NULL) {
        jappend(buf, n, cap, "null", 4);
    } else if (tag == J_BOOL) {
        if (json_ival(p)) jappend(buf, n, cap, "true", 4);
        else jappend(buf, n, cap, "false", 5);
    } else if (tag == J_INT) {
        snprintf(tmp, sizeof(tmp), "%" PRId64, json_ival(p));
        jappend(buf, n, cap, tmp, strlen(tmp));
    } else if (tag == J_STR) {
        const char *s = sn_str_cstr(json_pval(p));
        jappend(buf, n, cap, "\"", 1);
        for (; *s; s++) {
            if (*s == '"' || *s == '\\') {
                char e[2] = {'\\', *s};
                jappend(buf, n, cap, e, 2);
            } else if (*s == '\n') jappend(buf, n, cap, "\\n", 2);
            else {
                jappend(buf, n, cap, s, 1);
            }
        }
        jappend(buf, n, cap, "\"", 1);
    } else if (tag == J_ARR) {
        List *l = (List *)json_pval(p);
        jappend(buf, n, cap, "[", 1);
        for (int64_t i = 0; l && i < l->len; i++) {
            if (i) jappend(buf, n, cap, ",", 1);
            json_encode_into((void *)(intptr_t)l->items[i], buf, n, cap);
        }
        jappend(buf, n, cap, "]", 1);
    } else if (tag == J_OBJ) {
        Map *m = (Map *)json_pval(p);
        jappend(buf, n, cap, "{", 1);
        for (int64_t i = 0; m && i < m->len; i++) {
            if (i) jappend(buf, n, cap, ",", 1);
            jappend(buf, n, cap, "\"", 1);
            const char *ks = sn_str_cstr((void *)(intptr_t)m->keys[i]);
            jappend(buf, n, cap, ks, strlen(ks));
            jappend(buf, n, cap, "\":", 2);
            json_encode_into((void *)(intptr_t)m->vals[i], buf, n, cap);
        }
        jappend(buf, n, cap, "}", 1);
    }
}

void *sn_json_encode(void *p) {
    size_t cap = 64, n = 0;
    char *buf = (char *)xmalloc(cap);
    buf[0] = 0;
    json_encode_into(p, &buf, &n, &cap);
    void *s = sn_str_new(buf, (int64_t)n);
    free(buf);
    return s;
}

void *sn_json_as_str(void *p) {
    if (json_tag(p) == J_STR) {
        void *s = json_pval(p);
        sn_retain(s);
        return s;
    }
    return sn_json_encode(p);
}

int64_t sn_json_as_int(void *p) { return json_tag(p) == J_INT ? json_ival(p) : 0; }
int64_t sn_json_as_bool(void *p) {
    if (json_tag(p) == J_BOOL) return json_ival(p);
    if (json_tag(p) == J_INT) return json_ival(p) != 0;
    return 0;
}
int64_t sn_json_is_null(void *p) { return !p || json_tag(p) == J_NULL; }
int64_t sn_json_is_object(void *p) { return json_tag(p) == J_OBJ; }
int64_t sn_json_is_list(void *p) { return json_tag(p) == J_ARR; }
int64_t sn_json_len(void *p) {
    if (json_tag(p) == J_ARR) return sn_list_len(json_pval(p));
    if (json_tag(p) == J_OBJ) return sn_map_len(json_pval(p));
    if (json_tag(p) == J_STR) return sn_str_len(json_pval(p));
    return 0;
}

void *sn_json_get(void *p, void *key) {
    if (json_tag(p) != J_OBJ) return json_new(J_NULL, 0, NULL);
    Map *m = (Map *)json_pval(p);
    if (!sn_map_has(m, (int64_t)(intptr_t)key)) return json_new(J_NULL, 0, NULL);
    void *v = (void *)(intptr_t)sn_map_get(m, (int64_t)(intptr_t)key);
    sn_retain(v);
    return v;
}

void *sn_json_at(void *p, int64_t i) {
    if (json_tag(p) != J_ARR) return json_new(J_NULL, 0, NULL);
    void *v = sn_list_get_ptr(json_pval(p), i);
    sn_retain(v);
    return v;
}

void *sn_json_keys(void *p) {
    if (json_tag(p) != J_OBJ) return sn_list_new(1);
    return sn_map_keys(json_pval(p));
}

int64_t sn_time_now_ms(void) {
#ifdef _WIN32
    FILETIME ft;
    GetSystemTimeAsFileTime(&ft);
    uint64_t t = ((uint64_t)ft.dwHighDateTime << 32) | ft.dwLowDateTime;
    return (int64_t)(t / 10000ULL - 11644473600000ULL);
#else
    struct timeval tv;
    gettimeofday(&tv, NULL);
    return (int64_t)tv.tv_sec * 1000 + tv.tv_usec / 1000;
#endif
}

void sn_time_sleep_ms(int64_t ms) {
    if (ms < 0) ms = 0;
#ifdef _WIN32
    Sleep((DWORD)ms);
#else
    usleep((useconds_t)(ms * 1000));
#endif
}

void *sn_time_format(int64_t ms) {
    time_t sec = (time_t)(ms / 1000);
    struct tm tmv;
#ifdef _WIN32
    gmtime_s(&tmv, &sec);
#else
    gmtime_r(&sec, &tmv);
#endif
    char buf[64];
    snprintf(buf, sizeof(buf), "%04d-%02d-%02dT%02d:%02d:%02dZ",
             tmv.tm_year + 1900, tmv.tm_mon + 1, tmv.tm_mday,
             tmv.tm_hour, tmv.tm_min, tmv.tm_sec);
    return sn_str_from_cstr(buf);
}

#ifdef _WIN32
typedef SOCKET sn_sock;
#define SN_INVALID INVALID_SOCKET
#define sn_closesocket closesocket
#else
typedef int sn_sock;
#define SN_INVALID -1
#define sn_closesocket close
#endif

static int parse_url(const char *url, char *host, size_t hostn, int *port, char *path, size_t pathn, int *is_https) {
    const char *p = url;
    *is_https = 0;
    if (strncmp(p, "http://", 7) == 0) p += 7;
    else if (strncmp(p, "https://", 8) == 0) { p += 8; *is_https = 1; *port = 443; }
    const char *slash = strchr(p, '/');
    const char *colon = strchr(p, ':');
    if (colon && (!slash || colon < slash)) {
        size_t hl = (size_t)(colon - p);
        if (hl >= hostn) hl = hostn - 1;
        memcpy(host, p, hl);
        host[hl] = 0;
        *port = atoi(colon + 1);
        p = slash ? slash : "";
    } else {
        size_t hl = slash ? (size_t)(slash - p) : strlen(p);
        if (hl >= hostn) hl = hostn - 1;
        memcpy(host, p, hl);
        host[hl] = 0;
        if (!*is_https) *port = 80;
        p = slash ? slash : "/";
    }
    if (!p || !*p) p = "/";
    strncpy(path, p, pathn - 1);
    path[pathn - 1] = 0;
    return 0;
}

static int sn_http_timeout_secs(void) {
    const char *e = getenv("SN_HTTP_TIMEOUT");
    if (!e || !*e) return 30;
    int t = atoi(e);
    return t > 0 ? t : 30;
}

static int sn_http_insecure(void) {
    const char *e = getenv("SN_HTTP_INSECURE");
    return e && (e[0] == '1' || e[0] == 't' || e[0] == 'T' || e[0] == 'y' || e[0] == 'Y');
}

static const char *sn_http_user_agent(void) {
    const char *e = getenv("SN_HTTP_USER_AGENT");
    return (e && *e) ? e : "snlang/0.2";
}

static void sn_http_set_sock_timeout(sn_sock fd, int secs) {
    if (secs <= 0) return;
#ifdef _WIN32
    DWORD ms = (DWORD)secs * 1000u;
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, (const char *)&ms, sizeof(ms));
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, (const char *)&ms, sizeof(ms));
#else
    struct timeval tv;
    tv.tv_sec = secs;
    tv.tv_usec = 0;
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv));
#endif
}

/* Escape single quotes for a POSIX single-quoted shell string. */
static void sn_shell_single_quote(char *dst, size_t dstn, const char *src) {
    size_t j = 0;
    if (j + 1 < dstn) dst[j++] = '\'';
    for (; *src && j + 5 < dstn; src++) {
        if (*src == '\'') {
            /* '\'' */
            dst[j++] = '\'';
            dst[j++] = '\\';
            dst[j++] = '\'';
            dst[j++] = '\'';
        } else {
            dst[j++] = *src;
        }
    }
    if (j + 1 < dstn) dst[j++] = '\'';
    dst[j] = 0;
}

static void http_via_curl(const char *method, const char *url, const char *body, char **out_body, void **out_err) {
    char qurl[4096], qbody[4096], qua[256], cmd[9216];
    sn_shell_single_quote(qurl, sizeof(qurl), url);
    sn_shell_single_quote(qua, sizeof(qua), sn_http_user_agent());
    int timeout = sn_http_timeout_secs();
    const char *insecure = sn_http_insecure() ? " -k" : "";
    int rc;
    if (body && body[0]) {
        sn_shell_single_quote(qbody, sizeof(qbody), body);
        snprintf(cmd, sizeof(cmd),
                 "curl -sS -L --max-time %d -A %s%s -X %s -H 'Content-Type: application/octet-stream' -d %s %s %s",
                 timeout, qua, insecure, method, qbody, qurl, SN_NULL_REDIR);
    } else {
        snprintf(cmd, sizeof(cmd),
                 "curl -sS -L --max-time %d -A %s%s -X %s %s %s",
                 timeout, qua, insecure, method, qurl, SN_NULL_REDIR);
    }
    FILE *fp = SN_POPEN(cmd, "r");
    if (!fp) {
        if (out_body) *out_body = sn_str_from_cstr("");
        if (out_err) *out_err = sn_error_new(sn_str_from_cstr("curl failed"));
        return;
    }
    size_t cap = 4096, n = 0;
    char *buf = (char *)xmalloc(cap);
    int c;
    while ((c = fgetc(fp)) != EOF) {
        if (n + 2 > cap) { cap *= 2; buf = (char *)realloc(buf, cap); }
        buf[n++] = (char)c;
    }
    buf[n] = 0;
    rc = SN_PCLOSE(fp);
    if (rc != 0 && n == 0) {
        if (out_body) *out_body = sn_str_from_cstr("");
        if (out_err) *out_err = sn_error_new(sn_str_from_cstr("curl failed (check TLS/certs; set SN_HTTP_INSECURE=1 only if needed)"));
        free(buf);
        return;
    }
    if (out_body) *out_body = sn_str_from_cstr(buf);
    if (out_err) *out_err = NULL;
    free(buf);
}

void sn_os_system(void *cmd, int64_t *out_code, void **out_out, void **out_err) {
    if (!cmd) {
        if (out_code) *out_code = -1;
        if (out_out) *out_out = sn_str_from_cstr("");
        if (out_err) *out_err = sn_error_new(sn_str_from_cstr("null command"));
        return;
    }
    const char *c = sn_str_cstr(cmd);
    size_t clen = strlen(c);
    char *full_cmd = (char *)xmalloc(clen + 16);
    snprintf(full_cmd, clen + 16, "%s 2>&1", c);
#ifdef _WIN32
    FILE *fp = _popen(full_cmd, "r");
#else
    FILE *fp = popen(full_cmd, "r");
#endif
    free(full_cmd);
    if (!fp) {
        if (out_code) *out_code = -1;
        if (out_out) *out_out = sn_str_from_cstr("");
        if (out_err) *out_err = sn_error_new(sn_str_from_cstr("failed to start process"));
        return;
    }
    size_t cap = 4096, n = 0;
    char *buf = (char *)xmalloc(cap);
    int ch;
    while ((ch = fgetc(fp)) != EOF) {
        if (n + 2 > cap) { cap *= 2; buf = (char *)realloc(buf, cap); }
        buf[n++] = (char)ch;
    }
    buf[n] = 0;
#ifdef _WIN32
    int status = _pclose(fp);
    int exit_code = status;
#else
    int status = pclose(fp);
    int exit_code = WIFEXITED(status) ? WEXITSTATUS(status) : status;
#endif
    if (out_code) *out_code = (int64_t)exit_code;
    if (out_out) *out_out = sn_str_from_cstr(buf);
    if (out_err) *out_err = NULL;
    free(buf);
}

void sn_os_exec(void *cmd, void *args, int64_t *out_code, void **out_out, void **out_err) {
    if (!cmd) {
        if (out_code) *out_code = -1;
        if (out_out) *out_out = sn_str_from_cstr("");
        if (out_err) *out_err = sn_error_new(sn_str_from_cstr("null command"));
        return;
    }
    const char *c = sn_str_cstr(cmd);
    size_t total_len = strlen(c) * 4 + 32;
    int64_t argc = args ? sn_list_len(args) : 0;
    for (int64_t i = 0; i < argc; i++) {
        void *arg_str = sn_list_get_ptr(args, i);
        if (arg_str) {
            total_len += strlen(sn_str_cstr(arg_str)) * 4 + 8;
        }
    }
    char *full_cmd = (char *)xmalloc(total_len);
    char *qpart = (char *)xmalloc(total_len);
    sn_shell_single_quote(qpart, total_len, c);
    size_t pos = strlen(qpart);
    memcpy(full_cmd, qpart, pos);

    for (int64_t i = 0; i < argc; i++) {
        void *arg_str = sn_list_get_ptr(args, i);
        if (arg_str) {
            sn_shell_single_quote(qpart, total_len, sn_str_cstr(arg_str));
            size_t qlen = strlen(qpart);
            full_cmd[pos++] = ' ';
            memcpy(full_cmd + pos, qpart, qlen);
            pos += qlen;
        }
    }
    full_cmd[pos] = 0;
    free(qpart);

    void *cmd_obj = sn_str_from_cstr(full_cmd);
    free(full_cmd);
    sn_os_system(cmd_obj, out_code, out_out, out_err);
}

void sn_http_request(void *method, void *url, void *body, void **out_body, void **out_err) {
    const char *urlc = sn_str_cstr(url);
    int is_https = 0;
    if (strncmp(urlc, "https://", 8) == 0) {
        http_via_curl(sn_str_cstr(method), urlc, body ? sn_str_cstr(body) : "", (char **)out_body, out_err);
        return;
    }
    char host[256], path[1024];
    int port = 80;
    parse_url(urlc, host, sizeof(host), &port, path, sizeof(path), &is_https);
    (void)is_https;
    struct addrinfo hints, *res = NULL;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    char portstr[16];
    snprintf(portstr, sizeof(portstr), "%d", port);
    if (getaddrinfo(host, portstr, &hints, &res) != 0) {
        if (out_body) *out_body = sn_str_from_cstr("");
        if (out_err) *out_err = sn_error_new(sn_str_from_cstr("DNS lookup failed"));
        return;
    }
    sn_sock fd = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    if (fd == SN_INVALID || connect(fd, res->ai_addr, (int)res->ai_addrlen) != 0) {
        if (fd != SN_INVALID) sn_closesocket(fd);
        freeaddrinfo(res);
        if (out_body) *out_body = sn_str_from_cstr("");
        if (out_err) *out_err = sn_error_new(sn_str_from_cstr("connection failed"));
        return;
    }
    freeaddrinfo(res);
    sn_http_set_sock_timeout(fd, sn_http_timeout_secs());
    const char *meth = sn_str_cstr(method);
    const char *bd = body ? sn_str_cstr(body) : "";
    int64_t blen = body ? sn_str_len(body) : 0;
    const char *ua = sn_http_user_agent();
    char hdr[2048];
    if (blen > 0) {
        snprintf(hdr, sizeof(hdr),
                 "%s %s HTTP/1.1\r\nHost: %s\r\nUser-Agent: %s\r\nContent-Length: %" PRId64 "\r\nConnection: close\r\n\r\n",
                 meth, path, host, ua, blen);
    } else {
        snprintf(hdr, sizeof(hdr),
                 "%s %s HTTP/1.1\r\nHost: %s\r\nUser-Agent: %s\r\nConnection: close\r\n\r\n",
                 meth, path, host, ua);
    }
#ifdef _WIN32
    send(fd, hdr, (int)strlen(hdr), 0);
    if (blen) send(fd, bd, (int)blen, 0);
#else
    send(fd, hdr, strlen(hdr), 0);
    if (blen) send(fd, bd, (size_t)blen, 0);
#endif
    size_t cap = 4096, n = 0;
    char *buf = (char *)xmalloc(cap);
    for (;;) {
        if (n + 1024 > cap) {
            cap *= 2;
            buf = (char *)realloc(buf, cap);
        }
#ifdef _WIN32
        int r = recv(fd, buf + n, (int)(cap - n - 1), 0);
#else
        ssize_t r = recv(fd, buf + n, cap - n - 1, 0);
#endif
        if (r <= 0) break;
        n += (size_t)r;
    }
    buf[n] = 0;
    sn_closesocket(fd);
    char *bodyp = strstr(buf, "\r\n\r\n");
    const char *payload = bodyp ? bodyp + 4 : buf;
    int status = 0;
    if (strncmp(buf, "HTTP/", 5) == 0) {
        char *sp = strchr(buf, ' ');
        if (sp) status = atoi(sp + 1);
    }
    if (out_body) *out_body = sn_str_from_cstr(payload);
    if (out_err) {
        if (status >= 400) {
            char em[64];
            snprintf(em, sizeof(em), "HTTP %d", status);
            *out_err = sn_error_new(sn_str_from_cstr(em));
        } else {
            *out_err = NULL;
        }
    }
    free(buf);
}

void *sn_http_serve_once(int64_t port, void *handler) {
#ifdef _WIN32
    sn_sock fd = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
#else
    sn_sock fd = socket(AF_INET, SOCK_STREAM, 0);
#endif
    if (fd == SN_INVALID) return sn_error_new(sn_str_from_cstr("socket failed"));
    int opt = 1;
#ifdef _WIN32
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, (const char *)&opt, sizeof(opt));
#else
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &opt, sizeof(opt));
#endif
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    addr.sin_port = htons((uint16_t)port);
    if (bind(fd, (struct sockaddr *)&addr, sizeof(addr)) != 0) {
        sn_closesocket(fd);
        return sn_error_new(sn_str_from_cstr("bind failed"));
    }
    if (listen(fd, 1) != 0) {
        sn_closesocket(fd);
        return sn_error_new(sn_str_from_cstr("listen failed"));
    }
    sn_sock cfd = accept(fd, NULL, NULL);
    sn_closesocket(fd);
    if (cfd == SN_INVALID) return sn_error_new(sn_str_from_cstr("accept failed"));
    char req[8192];
    size_t n = 0;
    while (n < sizeof(req) - 1) {
#ifdef _WIN32
        int r = recv(cfd, req + n, (int)(sizeof(req) - 1 - n), 0);
#else
        ssize_t r = recv(cfd, req + n, sizeof(req) - 1 - n, 0);
#endif
        if (r <= 0) break;
        n += (size_t)r;
        req[n] = 0;
        if (strstr(req, "\r\n\r\n")) break;
    }
    req[n] = 0;
    char path[1024] = "/";
    char *sp1 = strchr(req, ' ');
    if (sp1) {
        sp1++;
        char *sp2 = strchr(sp1, ' ');
        size_t pl = sp2 ? (size_t)(sp2 - sp1) : strlen(sp1);
        if (pl >= sizeof(path)) pl = sizeof(path) - 1;
        memcpy(path, sp1, pl);
        path[pl] = 0;
    }
    void *resp = sn_clos_call1(handler, sn_str_from_cstr(path));
    const char *body = sn_str_cstr(resp);
    int64_t bl = sn_str_len(resp);
    char hdr[256];
    snprintf(hdr, sizeof(hdr),
             "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: %" PRId64 "\r\nConnection: close\r\n\r\n",
             bl);
#ifdef _WIN32
    send(cfd, hdr, (int)strlen(hdr), 0);
    send(cfd, body, (int)bl, 0);
#else
    send(cfd, hdr, strlen(hdr), 0);
    send(cfd, body, (size_t)bl, 0);
#endif
    sn_closesocket(cfd);
    return NULL;
}

int64_t sn_obj_type_id(void *p) {
    if (!p) return 0;
    int64_t *o = (int64_t *)p;
    return o[1] - KIND_USER;
}

void sn_record_copy(void *dst, void *src, int64_t n) {
    if (dst && src && n > 0) memcpy(dst, src, (size_t)n);
}

enum { ANY_INT = 1, ANY_STR = 2, ANY_BOOL = 3 };

typedef struct Any {
    int64_t rc;
    int64_t kind;
    int64_t tag;
    int64_t data;
    void *ptr;
} Any;

void *sn_any_box_i64(int64_t v) {
    Any *a = (Any *)xmalloc(sizeof(Any));
    a->rc = 1;
    a->kind = 10;
    a->tag = ANY_INT;
    a->data = v;
    a->ptr = NULL;
    return a;
}

int64_t sn_any_as_int(void *p) {
    if (!p) return 0;
    Any *a = (Any *)p;
    if (a->tag == ANY_INT) return a->data;
    if (a->tag == ANY_BOOL) return a->data;
    return 0;
}

void *sn_any_as_str(void *p) {
    if (!p) return sn_str_from_cstr("");
    Any *a = (Any *)p;
    if (a->tag == ANY_STR && a->ptr) return a->ptr;
    return sn_str_from_cstr("");
}

int64_t sn_any_as_bool(void *p) {
    if (!p) return 0;
    Any *a = (Any *)p;
    if (a->tag == ANY_BOOL) return a->data;
    return 0;
}

void *sn_any_type_name(void *p) {
    if (!p) return sn_str_from_cstr("none");
    Any *a = (Any *)p;
    switch (a->tag) {
    case ANY_INT: return sn_str_from_cstr("int");
    case ANY_STR: return sn_str_from_cstr("str");
    case ANY_BOOL: return sn_str_from_cstr("bool");
    default: return sn_str_from_cstr("unknown");
    }
}


/* --------------------------------------------------------------------- */
/*  Bumpable event counter: every chan send / close broadcasts on it      */
/*  so sn_chan_select can sleep until *something* changes.                */
/* --------------------------------------------------------------------- */
static int64_t g_evt_seq = 0;
#ifdef _WIN32
static CRITICAL_SECTION g_evt_mu;
static CONDITION_VARIABLE g_evt_cv;
#else
static pthread_mutex_t g_evt_mu = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t g_evt_cv = PTHREAD_COND_INITIALIZER;
#endif

void sn_chan_notify(void) {
#ifdef _WIN32
    EnterCriticalSection(&g_evt_mu);
    g_evt_seq++;
    WakeAllConditionVariable(&g_evt_cv);
    LeaveCriticalSection(&g_evt_mu);
#else
    pthread_mutex_lock(&g_evt_mu);
    g_evt_seq++;
    pthread_cond_broadcast(&g_evt_cv);
    pthread_mutex_unlock(&g_evt_mu);
#endif
}

/* --------------------------------------------------------------------- */
/*  sn_chan_select: wait until ONE of the given channels has a message    */
/*  (recv-ready).  Params: n = number of channels, chans = array of       */
/*  channel pointers, timeout_ms = -1 (infinite), 0 (poll), >0 (ms).     */
/*  Returns the 0-based index of the first ready channel, or -1.          */
/* --------------------------------------------------------------------- */
int64_t sn_chan_select(int64_t n, void **chans, int64_t timeout_ms) {
    if (n <= 0 || !chans) return -1;
#ifdef _WIN32
    int64_t elapsed = 0;
    for (;;) {
        for (int64_t i = 0; i < n; i++) {
            Chan *c = (Chan *)chans[i];
            if (!c) continue;
            EnterCriticalSection(&c->mu);
            int ready = c->len > 0;
            LeaveCriticalSection(&c->mu);
            if (ready) return i;
        }
        if (timeout_ms == 0) return -1;
        if (timeout_ms > 0 && elapsed >= timeout_ms) return -1;
        Sleep(1);
        elapsed++;
    }
#else
    int64_t deadline = timeout_ms >= 0 ? sn_time_now_ms() + timeout_ms : -1;
    for (;;) {
        for (int64_t i = 0; i < n; i++) {
            Chan *c = (Chan *)chans[i];
            if (!c) continue;
            pthread_mutex_lock(&c->mu);
            int ready = c->len > 0 || c->closed;
            pthread_mutex_unlock(&c->mu);
            if (ready) return i;
        }
        if (timeout_ms == 0) return -1;
        if (deadline != -1 && sn_time_now_ms() >= deadline) return -1;

        /* Sleep until a send/close bumps the event counter or the deadline
           passes. This is a real condition-variable wait, not a poll. */
        pthread_mutex_lock(&g_evt_mu);
        uint64_t seen = (uint64_t)g_evt_seq;
        if ((uint64_t)g_evt_seq == seen) {
            if (deadline < 0) {
                pthread_cond_wait(&g_evt_cv, &g_evt_mu);
            } else {
                struct timespec ts;
                clock_gettime(CLOCK_REALTIME, &ts);
                int64_t left = deadline - sn_time_now_ms();
                if (left < 0) left = 0;
                ts.tv_sec += left / 1000;
                ts.tv_nsec += (left % 1000) * 1000000L;
                if (ts.tv_nsec >= 1000000000L) {
                    ts.tv_sec++;
                    ts.tv_nsec -= 1000000000L;
                }
                pthread_cond_timedwait(&g_evt_cv, &g_evt_mu, &ts);
            }
        }
        pthread_mutex_unlock(&g_evt_mu);
    }
#endif
}

/* --------------------------------------------------------------------- */
/*  Futures: async work is queued onto the scheduler and finished from    */
/*  a pool thread; await parks the caller on a condvar (no busy spin).    */
/* --------------------------------------------------------------------- */
typedef struct Future {
    int64_t rc;
    int64_t kind;
    int64_t done;
    int64_t result;
    void *data;
#ifdef _WIN32
    CRITICAL_SECTION mu;
    CONDITION_VARIABLE cv;
#else
    pthread_mutex_t mu;
    pthread_cond_t cv;
#endif
} Future;

static Future *future_new(void) {
    Future *f = (Future *)xmalloc(sizeof(Future));
    f->rc = 1;
    f->kind = 11;
    f->done = 0;
    f->result = 0;
    f->data = NULL;
#ifdef _WIN32
    InitializeCriticalSection(&f->mu);
    InitializeConditionVariable(&f->cv);
#else
    pthread_mutex_init(&f->mu, NULL);
    pthread_cond_init(&f->cv, NULL);
#endif
    return f;
}

static void future_complete(Future *f, int64_t result, void *data) {
    if (!f) return;
#ifdef _WIN32
    EnterCriticalSection(&f->mu);
    f->result = result;
    f->data = data;
    f->done = 1;
    WakeAllConditionVariable(&f->cv);
    LeaveCriticalSection(&f->mu);
#else
    pthread_mutex_lock(&f->mu);
    f->result = result;
    f->data = data;
    f->done = 1;
    pthread_cond_broadcast(&f->cv);
    pthread_mutex_unlock(&f->mu);
#endif
}

/* --------------------------------------------------------------------- */
/*  Event loop reactor (replaces thread-blocking futures)                 */
/*                                                                       */
/*  Timers and socket readiness are owned by a single reactor thread that */
/*  blocks in kqueue/epoll/poll — never in `sleep()` or `recv()`. Futures */
/*  are completed by the reactor, so creating one never blocks a worker.  */
/* --------------------------------------------------------------------- */

typedef enum {
    RX_TIMER = 1,
    RX_READ,
    RX_WRITE,
    RX_CONNECT
} RxKind;

typedef struct RxOp {
    int kind;
    int fd;
    void (*fire)(struct RxOp *op, int revents);
    void *arg;
    /* HTTP state machine */
    void *f;
    void *body;
    size_t body_len;
    size_t body_cap;
    size_t sent;
    /* Platform filter this fd is currently armed with (0 = not registered).
       Retargeting a socket must delete the old knote, otherwise a stale
       filter keeps firing the wrong state machine. */
    int reg_filter;
    /* Position within a multi-fd wait, used to report which fd fired. */
    int slot;
    /* Set once the op has been handed to the reactor's deferred-free list,
       so a second retire cannot push it twice. */
    int retired;
} RxOp;

typedef struct RxTimer {
    int64_t deadline_ms;
    RxOp *op;
    struct RxTimer *next;
} RxTimer;

#define RX_MAX_OPS 256

static RxOp *g_rx_ops[RX_MAX_OPS];
static int g_rx_del_fd[RX_MAX_OPS];
static int g_rx_del_filter[RX_MAX_OPS];
static int g_rx_ndel;
static RxTimer *g_rx_timers;
static int g_rx_nops;
static int g_rx_started;
static int g_rx_stopping;
static int64_t g_rx_stopping_at;
#ifdef SN_HAVE_KQUEUE
static int g_rx_kq = -1;
#endif

#ifdef _WIN32
static CRITICAL_SECTION g_rx_mu;
static CONDITION_VARIABLE g_rx_cv;
static HANDLE g_rx_thread;
static WSAPOLLFD g_rx_poll[RX_MAX_OPS + 1];
static int64_t g_rx_next_deadline;
static void *g_rx_next_op;
#else
static pthread_mutex_t g_rx_mu = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t g_rx_cv = PTHREAD_COND_INITIALIZER;
static pthread_t g_rx_thread;
#endif

static void rx_lock(void) {
#ifdef _WIN32
    EnterCriticalSection(&g_rx_mu);
#else
    pthread_mutex_lock(&g_rx_mu);
#endif
}
static void rx_unlock(void) {
#ifdef _WIN32
    LeaveCriticalSection(&g_rx_mu);
#else
    pthread_mutex_unlock(&g_rx_mu);
#endif
}
static void rx_wake(void) {
#ifdef _WIN32
    WakeConditionVariable(&g_rx_cv);
#else
    pthread_cond_broadcast(&g_rx_cv);
#endif
}

static void rx_free_op(RxOp *op) {
    if (!op) return;
    if (op->body) free(op->body);
    free(op);
}

/* Register `op` on `fd` for readiness. Caller holds g_rx_mu. */
static int rx_set(int fd, RxOp *op) {
    if (fd < 0 || g_rx_nops >= RX_MAX_OPS) return 0;
    int i;
    for (i = 0; i < RX_MAX_OPS; i++) {
        if (g_rx_ops[i] == NULL) {
            g_rx_ops[i] = op;
            op->fd = fd;
            op->reg_filter = (op->kind == RX_READ) ? 1 : 2; /* READ / WRITE */
            g_rx_nops++;
            return 1;
        }
    }
    return 0;
}

/* Liveness test for dispatch: a callback may free its own op, so the batch
   scan must not touch ops that are no longer registered. */
static int rx_is_registered(const RxOp *op) {
    int i;
    for (i = 0; i < RX_MAX_OPS; i++) {
        if (g_rx_ops[i] == op) return 1;
    }
    return 0;
}

/* Remove `op` from the reactor without touching the fd. Queues the old
   knote for deletion so a stale filter cannot fire into freed memory. */
static int rx_unregister(RxOp *op) {
    int i;
    for (i = 0; i < RX_MAX_OPS; i++) {
        if (g_rx_ops[i] == op) {
            g_rx_ops[i] = NULL;
            g_rx_nops--;
            if (op->reg_filter && g_rx_ndel < RX_MAX_OPS) {
                g_rx_del_fd[g_rx_ndel] = op->fd;
                g_rx_del_filter[g_rx_ndel] = op->reg_filter;
                g_rx_ndel++;
            }
            op->reg_filter = 0;
            return 1;
        }
    }
    return 0;
}

/* Unregister and close the fd. */
static void rx_drop(RxOp *op) {
    if (!rx_unregister(op)) return;
    if (op->fd >= 0) {
#ifdef _WIN32
        closesocket((SOCKET)op->fd);
#else
        close(op->fd);
#endif
        op->fd = -1;
    }
}

/* Switch `op` from one readiness filter to another, keeping the same fd. */
static int rx_retarget(RxOp *op, int kind) {
    int fd = op->fd;
    rx_unregister(op);
    op->kind = kind;
    op->fd = fd;
    return rx_set(fd, op);
}

static void rx_add_timer(RxOp *op, int64_t delay_ms) {
    RxTimer *t = (RxTimer *)xmalloc(sizeof(RxTimer));
    RxTimer **pp = &g_rx_timers;
    t->deadline_ms = sn_time_now_ms() + delay_ms;
    t->op = op;
    t->next = NULL;
    while (*pp && (*pp)->deadline_ms <= t->deadline_ms) pp = &(*pp)->next;
    t->next = *pp;
    *pp = t;
}

/* Remove a timer that lost the race, so it cannot fire on a dead stack. */
static void rx_cancel_timer(RxOp *op) {
    RxTimer **pp = &g_rx_timers;
    while (*pp) {
        if ((*pp)->op == op) {
            RxTimer *t = *pp;
            *pp = t->next;
            free(t);
            return;
        }
        pp = &(*pp)->next;
    }
}

/* Fire due timers. Caller holds g_rx_mu. Returns 1 if any fired. */
static int rx_run_timers(void) {
    int fired = 0;
    int64_t now = sn_time_now_ms();
    while (g_rx_timers && g_rx_timers->deadline_ms <= now) {
        RxTimer *t = g_rx_timers;
        RxOp *op = t->op;
        g_rx_timers = t->next;
        free(t);
        /* Mark retired *before* dispatching: a callback is allowed to free
           its own op, so the loop must not touch it afterwards. */
        op->retired = 1;
        if (op->fire) op->fire(op, 0);
        fired = 1;
    }
    return fired;
}

static int64_t rx_timeout_ms(void) {
    if (!g_rx_timers) return -1;
    int64_t d = g_rx_timers->deadline_ms - sn_time_now_ms();
    return d < 0 ? 0 : d;
}

/* Non-blocking socket helpers (POSIX / Winsock). */
static void rx_set_nonblock(int fd) {
#ifdef _WIN32
    u_long on = 1;
    ioctlsocket((SOCKET)fd, FIONBIO, &on);
#else
    int fl = fcntl(fd, F_GETFL, 0);
    if (fl >= 0) fcntl(fd, F_SETFL, fl | O_NONBLOCK);
#endif
}

typedef void (*RxFire)(RxOp *op, int revents);

#ifdef _WIN32
static void *rx_thread_main(void *unused) {
    (void)unused;
    /* g_rx_mu/g_rx_cv are initialized once in sn_rt_init. */
    for (;;) {
        int64_t tmo;
        rx_lock();
        if (g_rx_stopping) {
            rx_unlock();
            break;
        }
        rx_run_timers();
        tmo = rx_timeout_ms();
        rx_unlock();
        int nfds = 0;
        rx_lock();
        int i;
        for (i = 0; i < RX_MAX_OPS; i++) {
            if (g_rx_ops[i]) {
                g_rx_poll[nfds].fd = (SOCKET)g_rx_ops[i]->fd;
                g_rx_poll[nfds].events =
                    (g_rx_ops[i]->kind == RX_READ) ? POLLRDNORM : POLLWRNORM;
                g_rx_poll[nfds].revents = 0;
                nfds++;
            }
        }
        rx_unlock();
        if (!nfds && tmo < 0) {
            Sleep(50);
            continue;
        }
        WSAPoll(g_rx_poll, (ULONG)nfds, tmo < 0 ? 50 : (int)tmo);
        rx_lock();
        int k = 0;
        for (i = 0; i < RX_MAX_OPS; i++) {
            if (g_rx_ops[i]) {
                if (g_rx_poll[k].revents && g_rx_ops[i]->fire)
                    g_rx_ops[i]->fire(g_rx_ops[i], 1);
                k++;
            }
        }
        rx_run_timers();
        rx_unlock();
    }
    return NULL;
}
#else
static void *rx_thread_main(void *unused) {
    (void)unused;
    for (;;) {
        int nfds = 0;
        int64_t tmo;
        RxOp *snap[RX_MAX_OPS];
        int i;

        rx_lock();
        if (g_rx_stopping && g_rx_nops == 0 && !g_rx_timers) {
            rx_unlock();
            break;
        }
        rx_run_timers();
        /* Drain knotes for fds that were unregistered since the last wait. */
        for (i = 0; i < g_rx_ndel; i++) {
#ifdef SN_HAVE_KQUEUE
            if (g_rx_kq >= 0) {
                struct kevent del;
                EV_SET(&del, (uintptr_t)g_rx_del_fd[i],
                       g_rx_del_filter[i] == 1 ? EVFILT_READ : EVFILT_WRITE,
                       EV_DELETE, 0, 0, NULL);
                kevent(g_rx_kq, &del, 1, NULL, 0, NULL);
            }
#else
            (void)g_rx_del_filter[i];
#endif
        }
        g_rx_ndel = 0;
        tmo = rx_timeout_ms();
        if (g_rx_stopping && g_rx_stopping_at > 0) {
            int64_t left = g_rx_stopping_at - sn_time_now_ms();
            if (left < 0) left = 0;
            if (tmo < 0 || left < tmo) tmo = left;
        }
        for (i = 0; i < RX_MAX_OPS; i++) {
            if (g_rx_ops[i]) {
                snap[nfds] = g_rx_ops[i];
                nfds++;
            }
        }
        rx_unlock();

        if (!nfds && tmo < 0) {
            /* Nothing registered and nothing scheduled: park briefly so the
               reactor does not spin, but stay responsive to new work. */
            struct timespec ts = {0, 20 * 1000 * 1000};
            nanosleep(&ts, NULL);
            continue;
        }
        if (tmo < 0) tmo = 50; /* cap so registration/stop wake us promptly */

#ifdef SN_HAVE_KQUEUE
        if (g_rx_kq < 0) g_rx_kq = kqueue();
        struct kevent evs[RX_MAX_OPS], outs[RX_MAX_OPS];
        int ne = 0;
        for (i = 0; i < nfds; i++) {
            struct kevent *e = &evs[ne++];
            int16_t f = snap[i]->kind == RX_READ ? EVFILT_READ : EVFILT_WRITE;
            /* Re-arm idempotently: EV_ADD updates the existing knote. */
            EV_SET(e, (uintptr_t)snap[i]->fd, f, EV_ADD | EV_ENABLE, 0, 0,
                   (void *)snap[i]);
        }
        struct timespec ts;
        ts.tv_sec = tmo / 1000;
        ts.tv_nsec = (tmo % 1000) * 1000000L;
        int n = kevent(g_rx_kq, evs, ne, outs, RX_MAX_OPS, &ts);
        if (n < 0 && (errno == EINTR || errno == EAGAIN)) n = 0;
        if (n > 0) {
            rx_lock();
            for (i = 0; i < n; i++) {
                RxOp *op = (RxOp *)outs[i].udata;
                if (!op || !rx_is_registered(op) || !op->fire) continue;
                /* Ignore events for a filter the op is no longer armed with. */
                if (outs[i].filter !=
                    (op->kind == RX_READ ? EVFILT_READ : EVFILT_WRITE))
                    continue;
                op->fire(op, 1);
            }
            rx_run_timers();
            rx_unlock();
        } else if (n < 0) {
            struct timespec t2 = {0, 1000 * 1000};
            nanosleep(&t2, NULL);
        }
#else /* !SN_HAVE_KQUEUE */
        struct pollfd pfds[RX_MAX_OPS];
        RxOp *pmap[RX_MAX_OPS];
        for (i = 0; i < nfds; i++) {
            pfds[i].fd = snap[i]->fd;
            pfds[i].events =
                (short)(snap[i]->kind == RX_READ ? POLLIN : POLLOUT);
            pfds[i].revents = 0;
            pmap[i] = snap[i];
        }
        int n = poll(pfds, (nfds_t)nfds, (int)tmo);
        if (n > 0) {
            rx_lock();
            for (i = 0; i < nfds; i++) {
                if (!pfds[i].revents || !rx_is_registered(pmap[i]) || !pmap[i]->fire)
                    continue;
                short want = (short)(pmap[i]->kind == RX_READ ? POLLIN : POLLOUT);
                if (!(pfds[i].revents & want)) continue;
                pmap[i]->fire(pmap[i], 1);
            }
            rx_run_timers();
            rx_unlock();
        }
#endif
    }
    /* Final timer drain so shutdown-time timers still complete. */
    rx_lock();
    while (g_rx_timers) {
        RxTimer *t = g_rx_timers;
        RxOp *op = t->op;
        g_rx_timers = t->next;
        free(t);
        op->retired = 1;
        if (op->fire) op->fire(op, 0);
    }
    while (g_rx_timers) {
        RxTimer *t = g_rx_timers;
        free(t->op);
        free(t);
        g_rx_timers = t->next;
    }
    rx_unlock();
    return NULL;
}
#endif

#ifdef _WIN32
/* CreateThread needs DWORD WINAPI (*)(LPVOID); the reactor body is shared
   with the pthread variant, so adapt it here (defined after both variants). */
static DWORD WINAPI rx_thread_win(LPVOID unused) {
    rx_thread_main(unused);
    return 0;
}
#endif

static void rx_start(void) {
    rx_lock();
    if (g_rx_started) {
        rx_unlock();
        return;
    }
    g_rx_started = 1;
    g_rx_stopping = 0;
#ifdef _WIN32
    InitializeCriticalSection(&g_rx_mu);
    InitializeConditionVariable(&g_rx_cv);
    g_rx_thread = CreateThread(NULL, 0, rx_thread_win, NULL, 0, NULL);
#else
    pthread_create(&g_rx_thread, NULL, rx_thread_main, NULL);
#endif
    rx_unlock();
}

static void rx_stop(void) {
    rx_lock();
    if (!g_rx_started || g_rx_stopping) {
        rx_unlock();
        return;
    }
    g_rx_stopping = 1;
    g_rx_stopping_at = sn_time_now_ms() + 2000;
    rx_wake();
    rx_unlock();
#ifndef _WIN32
    pthread_join(g_rx_thread, NULL);
#else
    WaitForSingleObject(g_rx_thread, 3000);
    CloseHandle(g_rx_thread);
#endif
    rx_lock();
    g_rx_started = 0;
    int i;
    for (i = 0; i < RX_MAX_OPS; i++) {
        if (g_rx_ops[i]) {
#ifndef _WIN32
            close(g_rx_ops[i]->fd);
#endif
            rx_free_op(g_rx_ops[i]);
            g_rx_ops[i] = NULL;
        }
        g_rx_nops = 0;
    }
    g_rx_timers = NULL;
    rx_unlock();
}

void sn_go_spawn(void (*fn)(void *), void *arg);

/* --- timers: no thread sleeps, the reactor fires the deadline ---------- */

static void rx_timer_fire(RxOp *op, int revents) {
    (void)revents;
    Future *f = (Future *)op->f;
    /* `await sleep_async(ms)` yields the requested delay, matching the
       pre-reactor behaviour of sn_time_sleep_ms(ms). */
    future_complete(f, op->sent, NULL);
}

void *sn_async_sleep(int64_t ms) {
    Future *f = future_new();
    rx_start();
    RxOp *op = (RxOp *)xmalloc(sizeof(RxOp));
    memset(op, 0, sizeof(RxOp));
    op->kind = RX_TIMER;
    op->fd = -1;
    op->f = f;
    op->sent = (size_t)(ms < 0 ? 0 : ms);
    op->fire = rx_timer_fire;
    rx_lock();
    rx_add_timer(op, ms < 0 ? 0 : ms);
    rx_unlock();
    rx_wake();
    return f;
}

/* --- non-blocking HTTP: connect/write/read driven by reactor readiness -- */

typedef struct RxHttp {
    Future *f;
    char *host;
    int port;
    char *path;
    char *buf;
    size_t len, cap, sent;
    size_t header_end;
    int failed;
} RxHttp;

/* Parse "http://host[:port]/path". Returns 0 on success. */
static int rx_parse_url(const char *url, char **host, int *port, char **path) {
    const char *p = url;
    if (strncmp(p, "http://", 7) == 0) p += 7;
    else if (strncmp(p, "https://", 8) == 0) return -1; /* no TLS in reactor */
    const char *slash = strchr(p, '/');
    const char *hostend = slash ? slash : p + strlen(p);
    const char *colon = memchr(p, ':', (size_t)(hostend - p));
    int hl = colon ? (int)(colon - p) : (int)(hostend - p);
    if (hl <= 0) return -1;
    char *h = (char *)xmalloc((size_t)hl + 1);
    memcpy(h, p, (size_t)hl);
    h[hl] = 0;
    *host = h;
    *port = colon ? atoi(colon + 1) : 80;
    if (*port <= 0) *port = 80;
    /* Copy the path: it points into the caller's URL string, which this op
       does not own. */
    const char *pp = slash ? slash : "/";
    size_t pl = strlen(pp);
    char *pc = (char *)xmalloc(pl + 1);
    memcpy(pc, pp, pl + 1);
    *path = pc;
    return 0;
}

static void rx_http_free(RxHttp *h) {
    if (!h) return;
    free(h->host);
    free(h->path);
    free(h->buf);
    free(h);
}

static int rx_would_block(void) {
#ifdef _WIN32
    int e = WSAGetLastError();
    return e == WSAEWOULDBLOCK || e == WSAEINPROGRESS;
#else
    return errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR;
#endif
}

static void rx_http_got_headers(RxOp *op, RxHttp *h) {
    /* Strip the status line and headers, keeping only the body. */
    h->header_end = 0;
    size_t i;
    for (i = 0; i + 3 < h->len; i++) {
        if (h->buf[i] == '\r' && h->buf[i + 1] == '\n' && h->buf[i + 2] == '\r' &&
            h->buf[i + 3] == '\n') {
            h->header_end = i + 4;
            break;
        }
    }
    if (h->header_end == 0) return; /* headers still arriving */
    if (h->header_end < h->len) {
        memmove(h->buf, h->buf + h->header_end, h->len - h->header_end);
        h->len -= h->header_end;
    } else {
        h->len = 0;
    }
    h->buf[h->len] = 0;
}

static void rx_http_finish(RxOp *op, RxHttp *h) {
    Future *f = (Future *)h->f;
    void *body = h->len ? sn_str_new(h->buf, (int64_t)h->len) : NULL;
    future_complete(f, h->len ? 1 : 0, body);
    rx_drop(op);
    rx_http_free(h);
}

static void rx_http_fail(RxOp *op, RxHttp *h) {
    Future *f = (Future *)h->f;
    void *err = sn_str_from_cstr("async http: request failed");
    future_complete(f, 0, err);
    rx_drop(op);
    rx_http_free(h);
}

static void rx_http_on_read(RxOp *op, int revents) {
    (void)revents;
    RxHttp *h = (RxHttp *)op->arg;
    char tmp[4096];

    if (op->kind == RX_READ) {
        int64_t n = (int64_t)recv(op->fd, tmp, sizeof(tmp), 0);
        if (n < 0) {
            if (rx_would_block()) return; /* spurious wake, try again */
            rx_http_fail(op, h);
            return;
        }
        if (n == 0) { /* peer closed */
            if (!h->header_end) rx_http_got_headers(op, h);
            rx_http_finish(op, h);
            return;
        }
        if (h->len + (size_t)n + 1 > h->cap) {
            size_t ncap = h->cap ? h->cap * 2 : 8192;
            while (ncap < h->len + (size_t)n + 1) ncap *= 2;
            h->buf = (char *)realloc(h->buf, ncap);
            h->cap = ncap;
        }
        memcpy(h->buf + h->len, tmp, (size_t)n);
        h->len += (size_t)n;
        h->buf[h->len] = 0;
        if (!h->header_end) rx_http_got_headers(op, h);
        return;
    }

    /* write path */
    int64_t w = (int64_t)send(op->fd, h->buf + h->sent, h->len - h->sent, 0);
    if (w < 0) {
        if (rx_would_block()) return;
        rx_http_fail(op, h);
        return;
    }
    h->sent += (size_t)w;
    if (h->sent < h->len) return; /* more to write */
    /* Request fully sent: recycle the buffer for the response. */
    h->len = 0;
    h->sent = 0;
    h->header_end = 0;
    h->buf[0] = 0;
    if (!rx_retarget(op, RX_READ)) rx_http_fail(op, h);
}

static void rx_http_on_connect(RxOp *op, int revents) {
    (void)revents;
    RxHttp *h = (RxHttp *)op->arg;
    int soerr = 0;
    socklen_t slen = sizeof(soerr);
    if (getsockopt(op->fd, SOL_SOCKET, SO_ERROR, (char *)&soerr, &slen) != 0 ||
        soerr != 0) {
        rx_http_fail(op, h);
        return;
    }
    int req = snprintf(h->buf, h->cap,
                       "GET %s HTTP/1.1\r\nHost: %s:%d\r\n"
                       "User-Agent: snlang-reactor\r\nConnection: close\r\n\r\n",
                       h->path, h->host, h->port);
    h->len = (size_t)(req > 0 ? req : 0);
    h->sent = 0;
    /* Connected: hand the op to the IO state machine, still watching WRITE
       so the next readiness event performs the send. */
    op->fire = rx_http_on_read;
    if (!rx_retarget(op, RX_WRITE)) rx_http_fail(op, h);
}

void *sn_async_http_get(void *url) {
    Future *f = future_new();
    const char *raw = sn_str_cstr(url);
    RxHttp *h = (RxHttp *)xmalloc(sizeof(RxHttp));
    memset(h, 0, sizeof(RxHttp));
    h->f = f;
    if (!raw || rx_parse_url(raw, &h->host, &h->port, &h->path) != 0) {
        rx_http_free(h);
        future_complete(f, 0, sn_str_from_cstr("async http: bad url"));
        return f;
    }
    char hostbuf[256];
    snprintf(hostbuf, sizeof(hostbuf), "%s", h->host);

    rx_start();
    RxOp *op = (RxOp *)xmalloc(sizeof(RxOp));
    memset(op, 0, sizeof(RxOp));
    op->kind = RX_CONNECT;
    op->arg = h;
    op->fire = rx_http_on_connect;

    struct addrinfo hints, *res = NULL, *ai;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    char portstr[16];
    snprintf(portstr, sizeof(portstr), "%d", h->port);
    if (getaddrinfo(hostbuf, portstr, &hints, &res) != 0 || !res) {
        rx_http_free(h);
        rx_free_op(op);
        future_complete(f, 0, sn_str_from_cstr("async http: resolve failed"));
        return f;
    }
    int fd = -1;
    for (ai = res; ai; ai = ai->ai_next) {
        fd = (int)socket(ai->ai_family, ai->ai_socktype, ai->ai_protocol);
        if (fd < 0) continue;
        rx_set_nonblock(fd);
        int r = connect(fd, ai->ai_addr, (socklen_t)ai->ai_addrlen);
        if (r == 0 || errno == EINPROGRESS || errno == EWOULDBLOCK) break;
        sn_closesocket(fd);
        fd = -1;
    }
    freeaddrinfo(res);
    if (fd < 0) {
        rx_http_free(h);
        rx_free_op(op);
        future_complete(f, 0, sn_str_from_cstr("async http: connect failed"));
        return f;
    }

    /* Request buffer is owned by the op so rx_http_free stays simple. */
    h->cap = 1024;
    h->buf = (char *)xmalloc(h->cap);
    h->buf[0] = 0;

    rx_lock();
    if (!rx_set(fd, op)) {
        rx_unlock();
        sn_closesocket(fd);
        rx_http_free(h);
        rx_free_op(op);
        future_complete(f, 0, sn_str_from_cstr("async http: reactor full"));
        return f;
    }
    rx_unlock();
    rx_wake();
    return f;
}

/* Payload attached to a completed future (e.g. an async HTTP body). */
void *sn_future_data(void *p) {
    Future *f = (Future *)p;
    if (!f) return NULL;
    return f->data;
}

/* Block only the calling thread for `ms`, using a reactor timer. Used by the
   socket helpers that must tolerate EAGAIN without burning CPU. */

/* --------------------------------------------------------------------- */
/*  Readiness waits: park the caller until the reactor sees fd readiness   */
/* --------------------------------------------------------------------- */

int64_t sn_future_await(void *p);

/* Wait state lives on the caller's stack: the future always completes
   before rx_wait returns, so the callbacks can never outlive it. */
typedef struct RxWait {
    Future *f;
    int ready;
    RxOp *fdop;
    RxOp *topop;
} RxWait;

static void rx_wait_fire(RxOp *op, int revents) {
    (void)revents;
    RxWait *w = (RxWait *)op->arg;
    RxOp *dead = w->topop;
    w->ready = 1;
    /* Callbacks run on the reactor thread with g_rx_mu already held, so the
       timer list can be edited directly. */
    rx_cancel_timer(dead);
    rx_unregister(op); /* g_rx_mu is held by the reactor here */
    future_complete(w->f, 1, NULL);
    /* Both ops are unregistered now, so neither can be picked up by a later
       snapshot or a queued readiness event. */
    rx_free_op(op);
    rx_free_op(dead);
}

static void rx_wait_timeout(RxOp *op, int revents) {
    (void)revents;
    RxWait *w = (RxWait *)op->arg;
    RxOp *dead = w->fdop;
    w->ready = 0;
    rx_unregister(dead); /* g_rx_mu is held by the reactor here */
    future_complete(w->f, 0, NULL);
    rx_free_op(op);
    rx_free_op(dead);
}

/* Returns 1 when `fd` became ready, 0 on timeout, -1 on error. */
static int rx_wait(int fd, int kind, int64_t timeout_ms) {
    rx_start();
    RxWait w;
    w.f = future_new();
    w.ready = 0;

    RxOp *op = (RxOp *)xmalloc(sizeof(RxOp));
    memset(op, 0, sizeof(RxOp));
    op->kind = kind;
    op->arg = &w;
    op->fire = rx_wait_fire;
    w.fdop = op;

    RxOp *top = (RxOp *)xmalloc(sizeof(RxOp));
    memset(top, 0, sizeof(RxOp));
    top->kind = RX_TIMER;
    top->fd = -1;
    top->f = w.f;
    top->arg = &w;
    top->fire = rx_wait_timeout;
    w.topop = top;

    rx_lock();
    if (!rx_set(fd, op)) {
        rx_unlock();
        rx_free_op(op);
        rx_free_op(top);
        return -1;
    }
    rx_add_timer(top, timeout_ms < 0 ? 2000000000LL : timeout_ms);
    rx_unlock();
    rx_wake();

    sn_future_await(w.f);
    return w.ready ? 1 : 0;
}

/* --------------------------------------------------------------------- */
/*  Multi-fd select: the real "select over sockets" primitive             */
/* --------------------------------------------------------------------- */

typedef struct RxMany {
    Future *f;
    int count;
    int ready_index;
    RxOp **ops;
    RxOp *top;
} RxMany;

static void rx_many_release(RxMany *m) {
    int i;
    for (i = 0; i < m->count; i++) {
        rx_unregister(m->ops[i]);
        rx_free_op(m->ops[i]);
    }
    m->count = 0;
    rx_cancel_timer(m->top);
    rx_free_op(m->top);
}

static void rx_many_fire(RxOp *op, int revents) {
    (void)revents;
    RxMany *m = (RxMany *)op->arg;
    m->ready_index = op->slot;
    rx_many_release(m);
    future_complete(m->f, 1, NULL);
}

static void rx_many_timeout(RxOp *op, int revents) {
    (void)revents;
    RxMany *m = (RxMany *)op->arg;
    m->ready_index = -1;
    rx_many_release(m);
    future_complete(m->f, 0, NULL);
}

/* Wait for the first of `fds` to become readable. Returns its index, or -1
   on timeout / error. Uses one reactor wait, so an idle select costs no CPU
   and occupies no thread beyond the caller. */
static int rx_wait_many(int *fds, int n, int64_t timeout_ms) {
    if (n <= 0) return -1;
    rx_start();
    RxMany m;
    m.f = future_new();
    m.count = 0;
    m.ready_index = -1;
    m.ops = (RxOp **)xmalloc(sizeof(RxOp *) * (size_t)n);

    for (int i = 0; i < n; i++) {
        if (fds[i] < 0) continue;
        RxOp *op = (RxOp *)xmalloc(sizeof(RxOp));
        memset(op, 0, sizeof(RxOp));
        op->kind = RX_READ;
        op->fd = fds[i];
        op->arg = &m;
        op->fire = rx_many_fire;
        op->slot = m.count;
        m.ops[m.count++] = op;
    }
    if (m.count == 0) {
        free(m.ops);
        return -1;
    }

    m.top = (RxOp *)xmalloc(sizeof(RxOp));
    memset(m.top, 0, sizeof(RxOp));
    m.top->kind = RX_TIMER;
    m.top->fd = -1;
    m.top->f = m.f;
    m.top->arg = &m;
    m.top->fire = rx_many_timeout;

    rx_lock();
    int registered = 0;
    for (int i = 0; i < m.count; i++) {
        if (rx_set(m.ops[i]->fd, m.ops[i])) {
            registered++;
        } else {
            m.ops[i]->fd = -1; /* mark unusable */
        }
    }
    if (registered == 0) {
        rx_unlock();
        rx_many_release(&m);
        free(m.ops);
        return -1;
    }
    rx_add_timer(m.top, timeout_ms < 0 ? 2000000000LL : timeout_ms);
    rx_unlock();
    rx_wake();

    sn_future_await(m.f);
    free(m.ops);
    return m.ready_index;
}

static int rx_wait_readable(int fd, int64_t timeout_ms) {
    if (fd < 0) return -1;
    return rx_wait(fd, RX_READ, timeout_ms);
}

static int rx_wait_writable(int fd, int64_t timeout_ms) {
    if (fd < 0) return -1;
    return rx_wait(fd, RX_WRITE, timeout_ms);
}

/* --------------------------------------------------------------------- */
/*  Non-blocking TCP sockets, driven by the same reactor                   */
/* --------------------------------------------------------------------- */

/* A listening socket waits for readability meaning "a peer connected". */
int64_t sn_tcp_listen(int64_t port) {
#ifdef _WIN32
    int fd = (int)socket(AF_INET, SOCK_STREAM, 0);
    if (fd == INVALID_SOCKET) return -1;
#else
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) return -1;
#endif
    int one = 1;
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, (const char *)&one, sizeof(one));
    struct sockaddr_in a;
    memset(&a, 0, sizeof(a));
    a.sin_family = AF_INET;
    a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    a.sin_port = htons((unsigned short)port);
    if (bind(fd, (struct sockaddr *)&a, sizeof(a)) != 0) {
#ifdef _WIN32
        closesocket((SOCKET)fd);
#else
        close(fd);
#endif
        return -1;
    }
    if (listen(fd, 64) != 0) {
#ifdef _WIN32
        closesocket((SOCKET)fd);
#else
        close(fd);
#endif
        return -1;
    }
    rx_set_nonblock(fd);
    return (int64_t)fd;
}

/* Bind to all interfaces (server workloads). */
int64_t sn_tcp_listen_any(int64_t port) {
#ifdef _WIN32
    int fd = (int)socket(AF_INET, SOCK_STREAM, 0);
    if (fd == INVALID_SOCKET) return -1;
#else
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) return -1;
#endif
    int one = 1;
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, (const char *)&one, sizeof(one));
    struct sockaddr_in a;
    memset(&a, 0, sizeof(a));
    a.sin_family = AF_INET;
    a.sin_addr.s_addr = htonl(INADDR_ANY);
    a.sin_port = htons((unsigned short)port);
    if (bind(fd, (struct sockaddr *)&a, sizeof(a)) != 0 ||
        listen(fd, 128) != 0) {
#ifdef _WIN32
        closesocket((SOCKET)fd);
#else
        close(fd);
#endif
        return -1;
    }
    rx_set_nonblock(fd);
    return (int64_t)fd;
}

/* Actual bound port (useful when listening on port 0). */
int64_t sn_tcp_local_port(int64_t fd) {
    struct sockaddr_in a;
    socklen_t l = sizeof(a);
    if (getsockname((int)fd, (struct sockaddr *)&a, &l) != 0) return -1;
    return (int64_t)ntohs(a.sin_port);
}

/* Connect with the reactor: returns fd once the handshake finishes, or -1.
   Blocks only this caller, never a scheduler worker or the reactor thread. */
int64_t sn_tcp_connect(const char *host, int64_t port, int64_t timeout_ms) {
    struct addrinfo hints, *res = NULL, *ai;
    char portstr[16];
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    snprintf(portstr, sizeof(portstr), "%lld", (long long)port);
    if (getaddrinfo(host, portstr, &hints, &res) != 0 || !res) return -1;
    int fd = -1;
    for (ai = res; ai; ai = ai->ai_next) {
        fd = (int)socket(ai->ai_family, ai->ai_socktype, ai->ai_protocol);
        if (fd < 0) continue;
        rx_set_nonblock(fd);
        int r = connect(fd, ai->ai_addr, (socklen_t)ai->ai_addrlen);
        if (r == 0) break;
        if (errno == EINPROGRESS || errno == EWOULDBLOCK) {
            /* Wait for writability using a reactor timer so we do not spin. */
            /* Park until the reactor reports the socket writable, which is
               exactly when a non-blocking connect has finished. */
            rx_wait_writable(fd, timeout_ms < 0 ? 30000 : timeout_ms);
            int soerr = 0;
            socklen_t sl = sizeof(soerr);
            int connected = getsockopt(fd, SOL_SOCKET, SO_ERROR, (char *)&soerr, &sl) == 0 &&
                            soerr == 0;
            if (connected) break;
#ifdef _WIN32
            closesocket((SOCKET)fd);
#else
            close(fd);
#endif
            fd = -1;
            continue;
        }
#ifdef _WIN32
        closesocket((SOCKET)fd);
#else
        close(fd);
#endif
        fd = -1;
    }
    freeaddrinfo(res);
    if (fd < 0) return -1;
    rx_set_nonblock(fd);
    return (int64_t)fd;
}

/* Accept one pending connection. Returns fd, or -1 when none is queued. */
int64_t sn_tcp_accept(int64_t lfd) {
    int fd = (int)accept((int)lfd, NULL, NULL);
    if (fd < 0) return -1;
    rx_set_nonblock(fd);
    return (int64_t)fd;
}

/* Blocking-style read that only waits for this caller. */
int64_t sn_tcp_read(int64_t fd) {
    char tmp[8192];
    int retries = 0;
    for (;;) {
        int64_t n = (int64_t)recv((int)fd, tmp, sizeof(tmp), 0);
        if (n > 0) return (int64_t)sn_str_new(tmp, n);
        if (n == 0) return 0; /* orderly shutdown */
        if (errno == EINTR) continue;
        if (errno == EAGAIN || errno == EWOULDBLOCK) {
            int r = rx_wait_readable((int)fd, 1000);
            if (r < 0) return -1;
            if (r == 0) {
                retries++;
                if (retries >= 30) return -1; /* 30s read deadline */
                continue;
            }
            retries = 0;
            continue;
        }
        return -1;
    }
}

int64_t sn_tcp_write(int64_t fd, const char *data, int64_t len) {
    int64_t sent = 0;
    while (sent < len) {
        int64_t n = (int64_t)send((int)fd, data + sent, (size_t)(len - sent), 0);
        if (n > 0) {
            sent += n;
            continue;
        }
        if (n < 0 && errno == EINTR) continue;
        if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
            if (rx_wait_writable((int)fd, 1000) < 0) return sent > 0 ? sent : -1;
            continue;
        }
        return sent > 0 ? sent : -1;
    }
    return sent;
}

void sn_tcp_close(int64_t fd) {
    if (fd < 0) return;
#ifdef _WIN32
    closesocket((SOCKET)fd);
#else
    close(fd);
#endif
}

/* Wait until one of `fds` is readable. Returns its index or -1 on timeout.
   Uses a single reactor timer for the deadline, so an idle wait costs no
   CPU and no thread. */
int64_t sn_select_read(int64_t n, int64_t *fds, int64_t timeout_ms) {
    if (n <= 0 || !fds) return -1;
    int live[RX_MAX_OPS];
    int64_t orig[RX_MAX_OPS];
    int cnt = 0;
    for (int64_t i = 0; i < n && cnt < RX_MAX_OPS; i++) {
        if (fds[i] < 0) continue;
        live[cnt] = (int)fds[i];
        orig[cnt] = i;
        cnt++;
    }
    if (cnt == 0) return -1;
    int idx = rx_wait_many(live, cnt, timeout_ms);
    if (idx < 0) return -1;
    return orig[idx];
}

/* --------------------------------------------------------------------- */
/*  Hashing / crypto primitives                                           */
/* --------------------------------------------------------------------- */

static uint32_t rotr32(uint32_t x, int n) { return (x >> n) | (x << (32 - n)); }
static uint32_t rotl32(uint32_t x, int n) { return (x << n) | (x >> (32 - n)); }

/* ---- SHA-256 (FIPS 180-4) ---- */

static const uint32_t K256[64] = {
    0x428a2f98u, 0x71374491u, 0xb5c0fbcfu, 0xe9b5dba5u, 0x3956c25bu, 0x59f111f1u,
    0x923f82a4u, 0xab1c5ed5u, 0xd807aa98u, 0x12835b01u, 0x243185beu, 0x550c7dc3u,
    0x72be5d74u, 0x80deb1feu, 0x9bdc06a7u, 0xc19bf174u, 0xe49b69c1u, 0xefbe4786u,
    0x0fc19dc6u, 0x240ca1ccu, 0x2de92c6fu, 0x4a7484aau, 0x5cb0a9dcu, 0x76f988dau,
    0x983e5152u, 0xa831c66du, 0xb00327c8u, 0xbf597fc7u, 0xc6e00bf3u, 0xd5a79147u,
    0x06ca6351u, 0x14292967u, 0x27b70a85u, 0x2e1b2138u, 0x4d2c6dfcu, 0x53380d13u,
    0x650a7354u, 0x766a0abbu, 0x81c2c92eu, 0x92722c85u, 0xa2bfe8a1u, 0xa81a664bu,
    0xc24b8b70u, 0xc76c51a3u, 0xd192e819u, 0xd6990624u, 0xf40e3585u, 0x106aa070u,
    0x19a4c116u, 0x1e376c08u, 0x2748774cu, 0x34b0bcb5u, 0x391c0cb3u, 0x4ed8aa4au,
    0x5b9cca4fu, 0x682e6ff3u, 0x748f82eeu, 0x78a5636fu, 0x84c87814u, 0x8cc70208u,
    0x90befffau, 0xa4506cebu, 0xbef9a3f7u, 0xc67178f2u};

void sn_sha256(const void *data, int64_t len, unsigned char out[32]) {
    uint32_t h[8] = {0x6a09e667u, 0xbb67ae85u, 0x3c6ef372u, 0xa54ff53au,
                     0x510e527fu, 0x9b05688cu, 0x1f83d9abu, 0x5be0cd19u};
    const unsigned char *p = (const unsigned char *)data;
    int64_t total = len + 1;
    int64_t padded = ((total + 8) / 64 + 1) * 64;
    unsigned char *msg = (unsigned char *)xmalloc((size_t)padded);
    memcpy(msg, p, (size_t)len);
    msg[len] = 0x80;
    memset(msg + len + 1, 0, (size_t)(padded - len - 1 - 8));
    uint64_t bits = (uint64_t)len * 8;
    for (int i = 0; i < 8; i++) msg[padded - 1 - i] = (unsigned char)(bits >> (8 * i));

    for (int64_t off = 0; off < padded; off += 64) {
        uint32_t w[64];
        for (int i = 0; i < 16; i++) {
            w[i] = ((uint32_t)msg[off + i * 4] << 24) |
                   ((uint32_t)msg[off + i * 4 + 1] << 16) |
                   ((uint32_t)msg[off + i * 4 + 2] << 8) |
                   ((uint32_t)msg[off + i * 4 + 3]);
        }
        for (int i = 16; i < 64; i++) {
            uint32_t s0 = rotr32(w[i - 15], 7) ^ rotr32(w[i - 15], 18) ^ (w[i - 15] >> 3);
            uint32_t s1 = rotr32(w[i - 2], 17) ^ rotr32(w[i - 2], 19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16] + s0 + w[i - 7] + s1;
        }
        uint32_t a = h[0], b = h[1], c = h[2], d = h[3];
        uint32_t e = h[4], f = h[5], g = h[6], hh = h[7];
        for (int i = 0; i < 64; i++) {
            uint32_t S1 = rotr32(e, 6) ^ rotr32(e, 11) ^ rotr32(e, 25);
            uint32_t ch = (e & f) ^ ((~e) & g);
            uint32_t t1 = hh + S1 + ch + K256[i] + w[i];
            /* Sigma0 is defined with rotations to the *right*. */
            uint32_t S0 = rotr32(a, 2) ^ rotr32(a, 13) ^ rotr32(a, 22);
            uint32_t maj = (a & b) ^ (a & c) ^ (b & c);
            uint32_t t2 = S0 + maj;
            hh = g; g = f; f = e; e = d + t1;
            d = c; c = b; b = a; a = t1 + t2;
        }
        h[0] += a; h[1] += b; h[2] += c; h[3] += d;
        h[4] += e; h[5] += f; h[6] += g; h[7] += hh;
    }
    free(msg);
    for (int i = 0; i < 8; i++) {
        out[i * 4] = (unsigned char)(h[i] >> 24);
        out[i * 4 + 1] = (unsigned char)(h[i] >> 16);
        out[i * 4 + 2] = (unsigned char)(h[i] >> 8);
        out[i * 4 + 3] = (unsigned char)h[i];
    }
}

/* ---- MD5 (RFC 1321) ---- */

static const uint32_t MD5_K[64] = {
    0xd76aa478u, 0xe8c7b756u, 0x242070dbu, 0xc1bdceeeu, 0xf57c0fafu, 0x4787c62au,
    0xa8304613u, 0xfd469501u, 0x698098d8u, 0x8b44f7afu, 0xffff5bb1u, 0x895cd7beu,
    0x6b901122u, 0xfd987193u, 0xa679438eu, 0x49b40821u, 0xf61e2562u, 0xc040b340u,
    0x265e5a51u, 0xe9b6c7aau, 0xd62f105du, 0x02441453u, 0xd8a1e681u, 0xe7d3fbc8u,
    0x21e1cde6u, 0xc33707d6u, 0xf4d50d87u, 0x455a14edu, 0xa9e3e905u, 0xfcefa3f8u,
    0x676f02d9u, 0x8d2a4c8au, 0xfffa3942u, 0x8771f681u, 0x6d9d6122u, 0xfde5380cu,
    0xa4beea44u, 0x4bdecfa9u, 0xf6bb4b60u, 0xbebfbc70u, 0x289b7ec6u, 0xeaa127fau,
    0xd4ef3085u, 0x04881d05u, 0xd9d4d039u, 0xe6db99e5u, 0x1fa27cf8u, 0xc4ac5665u,
    0xf4292244u, 0x432aff97u, 0xab9423a7u, 0xfc93a039u, 0x655b59c3u, 0x8f0ccc92u,
    0xffeff47du, 0x85845dd1u, 0x6fa87e4fu, 0xfe2ce6e0u, 0xa3014314u, 0x4e0811a1u,
    0xf7537e82u, 0xbd3af235u, 0x2ad7d2bbu, 0xeb86d391u};
static const int MD5_R[64] = {7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22,
                             5, 9,  14, 20, 5, 9,  14, 20, 5, 9,  14, 20, 5, 9,  14, 20,
                             4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
                             6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21};

void sn_md5(const void *data, int64_t len, unsigned char out[16]) {
    uint32_t h[4] = {0x67452301u, 0xefcdab89u, 0x98badcfeu, 0x10325476u};
    const unsigned char *p = (const unsigned char *)data;
    int64_t total = len + 1;
    int64_t padded = ((total + 8) / 64 + 1) * 64;
    unsigned char *msg = (unsigned char *)xmalloc((size_t)padded);
    memcpy(msg, p, (size_t)len);
    msg[len] = 0x80;
    memset(msg + len + 1, 0, (size_t)(padded - len - 1 - 8));
    uint64_t bits = (uint64_t)len * 8;
    for (int i = 0; i < 8; i++) msg[padded - 8 + i] = (unsigned char)(bits >> (8 * i));

    for (int64_t off = 0; off < padded; off += 64) {
        uint32_t m[16];
        for (int i = 0; i < 16; i++) {
            m[i] = (uint32_t)msg[off + i * 4] |
                   ((uint32_t)msg[off + i * 4 + 1] << 8) |
                   ((uint32_t)msg[off + i * 4 + 2] << 16) |
                   ((uint32_t)msg[off + i * 4 + 3] << 24);
        }
        uint32_t a = h[0], b = h[1], c = h[2], d = h[3];
        for (int i = 0; i < 64; i++) {
            uint32_t f;
            int g;
            if (i < 16) { f = (b & c) | (~b & d); g = i; }
            else if (i < 32) { f = (d & b) | (~d & c); g = (5 * i + 1) % 16; }
            else if (i < 48) { f = b ^ c ^ d; g = (3 * i + 5) % 16; }
            else { f = c ^ (b | ~d); g = (7 * i) % 16; }
            uint32_t tmp = d;
            d = c;
            c = b;
            uint32_t x = a + f + MD5_K[i] + m[g];
            b = b + rotl32(x, MD5_R[i]);
            a = tmp;
        }
        h[0] += a; h[1] += b; h[2] += c; h[3] += d;
    }
    free(msg);
    for (int i = 0; i < 4; i++) {
        out[i * 4] = (unsigned char)h[i];
        out[i * 4 + 1] = (unsigned char)(h[i] >> 8);
        out[i * 4 + 2] = (unsigned char)(h[i] >> 16);
        out[i * 4 + 3] = (unsigned char)(h[i] >> 24);
    }
}

/* ---- SHA-512 (FIPS 180-4), 64-bit words ---- */

static const uint64_t K512[80] = {
    0x428a2f98d728ae22ULL, 0x7137449123ef65cdULL, 0xb5c0fbcfec4d3b2fULL, 0xe9b5dba58189dbbcULL,
    0x3956c25bf348b538ULL, 0x59f111f1b605d019ULL, 0x923f82a4af194f9bULL, 0xab1c5ed5da6d8118ULL,
    0xd807aa98a3030242ULL, 0x12835b0145706fbeULL, 0x243185be4ee4b28cULL, 0x550c7dc3d5ffb4e2ULL,
    0x72be5d74f27b896fULL, 0x80deb1fe3b1696b1ULL, 0x9bdc06a725c71235ULL, 0xc19bf174cf692694ULL,
    0xe49b69c19ef14ad2ULL, 0xefbe4786384f25e3ULL, 0x0fc19dc68b8cd5b5ULL, 0x240ca1cc77ac9c65ULL,
    0x2de92c6f592b0275ULL, 0x4a7484aa6ea6e483ULL, 0x5cb0a9dcbd41fbd4ULL, 0x76f988da831153b5ULL,
    0x983e5152ee66dfabULL, 0xa831c66d2db43210ULL, 0xb00327c898fb213fULL, 0xbf597fc7beef0ee4ULL,
    0xc6e00bf33da88fc2ULL, 0xd5a79147930aa725ULL, 0x06ca6351e003826fULL, 0x142929670a0e6e70ULL,
    0x27b70a8546d22ffcULL, 0x2e1b21385c26c926ULL, 0x4d2c6dfc5ac42aedULL, 0x53380d139d95b3dfULL,
    0x650a73548baf63deULL, 0x766a0abb3c77b2a8ULL, 0x81c2c92e47edaee6ULL, 0x92722c851482353bULL,
    0xa2bfe8a14cf10364ULL, 0xa81a664bbc423001ULL, 0xc24b8b70d0f89791ULL, 0xc76c51a30654be30ULL,
    0xd192e819d6ef5218ULL, 0xd69906245565a910ULL, 0xf40e35855771202aULL, 0x106aa07032bbd1b8ULL,
    0x19a4c116b8d2d0c8ULL, 0x1e376c085141ab53ULL, 0x2748774cdf8eeb99ULL, 0x34b0bcb5e19b48a8ULL,
    0x391c0cb3c5c95a63ULL, 0x4ed8aa4ae3418acbULL, 0x5b9cca4f7763e373ULL, 0x682e6ff3d6b2b8a3ULL,
    0x748f82ee5defb2fcULL, 0x78a5636f43172f60ULL, 0x84c87814a1f0ab72ULL, 0x8cc702081a6439ecULL,
    0x90befffa23631e28ULL, 0xa4506cebde82bde9ULL, 0xbef9a3f7b2c67915ULL, 0xc67178f2e372532bULL,
    0xca273eceea26619cULL, 0xd186b8c721c0c207ULL, 0xeada7dd6cde0eb1eULL, 0xf57d4f7fee6ed178ULL,
    0x06f067aa72176fbaULL, 0x0a637dc5a2c898a6ULL, 0x113f9804bef90daeULL, 0x1b710b35131c471bULL,
    0x28db77f523047d84ULL, 0x32caab7b40c72493ULL, 0x3c9ebe0a15c9bebcULL, 0x431d67c49c100d4cULL,
    0x4cc5d4becb3e42b6ULL, 0x597f299cfc657e2aULL, 0x5fcb6fab3ad6faecULL, 0x6c44198c4a475817ULL};

static uint64_t rotr64(uint64_t x, int n) { return (x >> n) | (x << (64 - n)); }

void sn_sha512(const void *data, int64_t len, unsigned char out[64]) {
    uint64_t h[8] = {0x6a09e667f3bcc908ULL, 0xbb67ae8584caa73bULL, 0x3c6ef372fe94f82bULL,
                     0xa54ff53a5f1d36f1ULL, 0x510e527fade682d1ULL, 0x9b05688c2b3e6c1fULL,
                     0x1f83d9abfb41bd6bULL, 0x5be0cd19137e2179ULL};
    const unsigned char *p = (const unsigned char *)data;
    int64_t total = len + 17;
    int64_t padded = ((total + 127) / 128) * 128;
    unsigned char *msg = (unsigned char *)xmalloc((size_t)padded);
    memcpy(msg, p, (size_t)len);
    msg[len] = 0x80;
    memset(msg + len + 1, 0, (size_t)(padded - len - 1 - 16));
    uint64_t bits = (uint64_t)len * 8;
    for (int i = 0; i < 8; i++) msg[padded - 1 - i] = (unsigned char)(bits >> (8 * i));

    for (int64_t off = 0; off < padded; off += 128) {
        uint64_t w[80];
        for (int i = 0; i < 16; i++) {
            uint64_t v = 0;
            for (int j = 0; j < 8; j++) v = (v << 8) | msg[off + i * 8 + j];
            w[i] = v;
        }
        for (int i = 16; i < 80; i++) {
            uint64_t s0 = rotr64(w[i - 15], 1) ^ rotr64(w[i - 15], 8) ^ (w[i - 15] >> 7);
            uint64_t s1 = rotr64(w[i - 2], 19) ^ rotr64(w[i - 2], 61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16] + s0 + w[i - 7] + s1;
        }
        uint64_t a = h[0], b = h[1], c = h[2], d = h[3];
        uint64_t e = h[4], f = h[5], g = h[6], hh = h[7];
        for (int i = 0; i < 80; i++) {
            uint64_t S1 = rotr64(e, 14) ^ rotr64(e, 18) ^ rotr64(e, 41);
            uint64_t ch = (e & f) ^ ((~e) & g);
            uint64_t t1 = hh + S1 + ch + K512[i] + w[i];
            uint64_t S0 = rotr64(a, 28) ^ rotr64(a, 34) ^ rotr64(a, 39);
            uint64_t maj = (a & b) ^ (a & c) ^ (b & c);
            uint64_t t2 = S0 + maj;
            hh = g; g = f; f = e; e = d + t1;
            d = c; c = b; b = a; a = t1 + t2;
        }
        h[0] += a; h[1] += b; h[2] += c; h[3] += d;
        h[4] += e; h[5] += f; h[6] += g; h[7] += hh;
    }
    free(msg);
    for (int i = 0; i < 8; i++) {
        for (int j = 0; j < 8; j++) out[i * 8 + j] = (unsigned char)(h[i] >> (56 - 8 * j));
    }
}

/* ---- HMAC (RFC 2104) ---- */

void sn_hmac_sha256(const void *key, int64_t keylen, const void *msg, int64_t msglen,
                    unsigned char out[32]) {
    unsigned char k[64];
    memset(k, 0, sizeof(k));
    if (keylen > 64) {
        sn_sha256(key, keylen, k);
    } else {
        memcpy(k, key, (size_t)keylen);
    }
    unsigned char ipad[64], opad[64];
    for (int i = 0; i < 64; i++) {
        ipad[i] = (unsigned char)(k[i] ^ 0x36);
        opad[i] = (unsigned char)(k[i] ^ 0x5c);
    }
    int64_t ilen = 64 + msglen;
    unsigned char *inner = (unsigned char *)xmalloc((size_t)ilen);
    memcpy(inner, ipad, 64);
    memcpy(inner + 64, msg, (size_t)msglen);
    unsigned char ih[32];
    sn_sha256(inner, ilen, ih);
    free(inner);
    unsigned char *outer = (unsigned char *)xmalloc(96);
    memcpy(outer, opad, 64);
    memcpy(outer + 64, ih, 32);
    sn_sha256(outer, 96, out);
    free(outer);
}

/* ---- CRC-32 (IEEE) ---- */

int64_t sn_crc32(const void *data, int64_t len) {
    static uint32_t table[256];
    static int init = 0;
    if (!init) {
        for (uint32_t i = 0; i < 256; i++) {
            uint32_t c = i;
            for (int k = 0; k < 8; k++) c = (c & 1) ? (0xedb88320u ^ (c >> 1)) : (c >> 1);
            table[i] = c;
        }
        init = 1;
    }
    const unsigned char *p = (const unsigned char *)data;
    uint32_t crc = 0xffffffffu;
    for (int64_t i = 0; i < len; i++) crc = table[(crc ^ p[i]) & 0xff] ^ (crc >> 8);
    return (int64_t)(crc ^ 0xffffffffu);
}

int64_t sn_crc32_update(int64_t crc, const void *data, int64_t len) {
    static uint32_t table[256];
    static int init = 0;
    if (!init) {
        for (uint32_t i = 0; i < 256; i++) {
            uint32_t c = i;
            for (int k = 0; k < 8; k++) c = (c & 1) ? (0xedb88320u ^ (c >> 1)) : (c >> 1);
            table[i] = c;
        }
        init = 1;
    }
    const unsigned char *p = (const unsigned char *)data;
    uint32_t c = (uint32_t)crc ^ 0xffffffffu;
    for (int64_t i = 0; i < len; i++) c = table[(c ^ p[i]) & 0xff] ^ (c >> 8);
    return (int64_t)(c ^ 0xffffffffu);
}

/* ---- Constant-time compare + secure zero ---- */

int64_t sn_consttime_eq(const void *a, const void *b, int64_t len) {
    const unsigned char *x = (const unsigned char *)a;
    const unsigned char *y = (const unsigned char *)b;
    unsigned char d = 0;
    for (int64_t i = 0; i < len; i++) d |= (unsigned char)(x[i] ^ y[i]);
    return d == 0;
}

void sn_secure_zero(void *p, int64_t len) {
    volatile unsigned char *q = (volatile unsigned char *)p;
    for (int64_t i = 0; i < len; i++) q[i] = 0;
}

/* --------------------------------------------------------------------- */
/*  bytearray: a growable mutable byte buffer                             */
/* --------------------------------------------------------------------- */

void *sn_bytearray_new(int64_t cap) {
    ByteArray *b = (ByteArray *)xmalloc(sizeof(ByteArray));
    b->rc = 1;
    b->kind = KIND_BYTEARRAY;
    gc_track(b, (int64_t)sizeof(ByteArray));
    b->len = 0;
    b->cap = cap > 0 ? cap : 8;
    b->data = (unsigned char *)xmalloc((size_t)b->cap);
    return b;
}

static void ba_reserve(ByteArray *b, int64_t need) {
    if (need <= b->cap) return;
    int64_t ncap = b->cap ? b->cap : 8;
    while (ncap < need) ncap *= 2;
    b->data = (unsigned char *)realloc(b->data, (size_t)ncap);
    b->cap = ncap;
}

int64_t sn_bytearray_len(void *p) {
    if (!p) return 0;
    return ((ByteArray *)p)->len;
}

void sn_bytearray_reserve(void *p, int64_t n) {
    if (p) ba_reserve((ByteArray *)p, n);
}

int64_t sn_bytearray_push(void *p, int64_t byte) {
    ByteArray *b = (ByteArray *)p;
    ba_reserve(b, b->len + 1);
    b->data[b->len++] = (unsigned char)(byte & 0xff);
    return b->len;
}

int64_t sn_bytearray_get(void *p, int64_t i) {
    ByteArray *b = (ByteArray *)p;
    if (!b || i < 0 || i >= b->len) return -1;
    return (int64_t)b->data[i];
}

void sn_bytearray_set(void *p, int64_t i, int64_t v) {
    ByteArray *b = (ByteArray *)p;
    if (!b || i < 0 || i >= b->len) return;
    b->data[i] = (unsigned char)(v & 0xff);
}

void sn_bytearray_append_bytes(void *p, const void *bytes, int64_t n) {
    ByteArray *b = (ByteArray *)p;
    if (!b || !bytes) return;
    ba_reserve(b, b->len + n);
    memcpy(b->data + b->len, bytes, (size_t)n);
    b->len += n;
}

void sn_bytearray_append_str(void *p, void *s) {
    if (!p || !s) return;
    sn_bytearray_append_bytes(p, sn_str_cstr(s), sn_str_len(s));
}

/* Copy `n` bytes out; returns a fresh bytearray. */
void *sn_bytearray_slice(void *p, int64_t start, int64_t n) {
    ByteArray *b = (ByteArray *)p;
    if (!b) return NULL;
    if (start < 0) start = 0;
    if (start > b->len) start = b->len;
    if (n < 0 || start + n > b->len) n = b->len - start;
    void *out = sn_bytearray_new(n > 0 ? n : 1);
    sn_bytearray_append_bytes(out, b->data + start, n);
    return out;
}

int64_t sn_bytearray_write_at(void *p, int64_t off, const void *bytes, int64_t n) {
    ByteArray *b = (ByteArray *)p;
    if (!b || !bytes) return -1;
    ba_reserve(b, off + n);
    if (off > b->len) memset(b->data + b->len, 0, (size_t)(off - b->len));
    memcpy(b->data + off, bytes, (size_t)n);
    if (off + n > b->len) b->len = off + n;
    return n;
}

int64_t sn_bytearray_find(void *p, int64_t byte, int64_t from) {
    ByteArray *b = (ByteArray *)p;
    if (!b) return -1;
    for (int64_t i = from < 0 ? 0 : from; i < b->len; i++) {
        if ((int64_t)b->data[i] == (byte & 0xff)) return i;
    }
    return -1;
}

int64_t sn_bytearray_rfind(void *p, int64_t byte) {
    ByteArray *b = (ByteArray *)p;
    if (!b) return -1;
    for (int64_t i = b->len - 1; i >= 0; i--) {
        if ((int64_t)b->data[i] == (byte & 0xff)) return i;
    }
    return -1;
}

/* Set the length, growing (zero-filled) if needed. Used when a C routine
   fills a caller-provided buffer. */
void sn_bytearray_set_len(void *p, int64_t n) {
    ByteArray *b = (ByteArray *)p;
    if (!b || n < 0) return;
    ba_reserve(b, n);
    if (n > b->len) memset(b->data + b->len, 0, (size_t)(n - b->len));
    b->len = n;
}

void sn_bytearray_truncate(void *p, int64_t n) {
    ByteArray *b = (ByteArray *)p;
    if (!b) return;
    if (n < 0) n = 0;
    if (n < b->len) b->len = n;
}

void sn_bytearray_clear(void *p) {
    ByteArray *b = (ByteArray *)p;
    if (b) b->len = 0;
}

/* Raw view of the buffer, for FFI calls that want to read bytes. */
const unsigned char *sn_bytearray_data(void *p) {
    if (!p) return NULL;
    return ((ByteArray *)p)->data;
}

void sn_bytearray_fill(void *p, int64_t byte, int64_t n) {
    ByteArray *b = (ByteArray *)p;
    if (!b) return;
    ba_reserve(b, n);
    memset(b->data, (int)(byte & 0xff), (size_t)n);
    b->len = n;
}

/* Digest helpers that own the buffer length, so a caller-provided bytearray
   ends up with exactly the digest bytes. */
static void *ba_for_digest(void *p, int64_t need) {
    if (!p) return NULL;
    ByteArray *b = (ByteArray *)p;
    ba_reserve(b, need);
    return b->data;
}

static void ba_digest_done(void *p, int64_t n) {
    if (!p) return;
    ((ByteArray *)p)->len = n;
}

void sn_bytearray_sha256(const char *data, int64_t len, void *ba) {
    unsigned char *out = (unsigned char *)ba_for_digest(ba, 32);
    if (!out) return;
    sn_sha256(data, len, out);
    ba_digest_done(ba, 32);
}

void sn_bytearray_md5(const char *data, int64_t len, void *ba) {
    unsigned char *out = (unsigned char *)ba_for_digest(ba, 16);
    if (!out) return;
    sn_md5(data, len, out);
    ba_digest_done(ba, 16);
}

void sn_bytearray_sha512(const char *data, int64_t len, void *ba) {
    unsigned char *out = (unsigned char *)ba_for_digest(ba, 64);
    if (!out) return;
    sn_sha512(data, len, out);
    ba_digest_done(ba, 64);
}

void sn_bytearray_hmac_sha256(const char *key, int64_t keylen, const char *msg,
                              int64_t msglen, void *ba) {
    unsigned char *out = (unsigned char *)ba_for_digest(ba, 32);
    if (!out) return;
    sn_hmac_sha256(key, keylen, msg, msglen, out);
    ba_digest_done(ba, 32);
}

/* --------------------------------------------------------------------- */
/*  StringBuilder: high-performance native string builder                */
/* --------------------------------------------------------------------- */

static void sb_reserve(Sb *sb, int64_t need) {
    if (need <= sb->cap) return;
    int64_t ncap = sb->cap > 0 ? sb->cap : 64;
    while (ncap < need) ncap *= 2;
    sb->buf = (char *)realloc(sb->buf, (size_t)ncap);
    if (!sb->buf) {
        fprintf(stderr, "out of memory\n");
        exit(1);
    }
    sb->cap = ncap;
}

void *sn_sb_new(int64_t cap) {
    Sb *sb = (Sb *)xmalloc(sizeof(Sb));
    sb->rc = 1;
    sb->kind = KIND_STRBUILDER;
    int64_t c = cap > 0 ? cap : 64;
    sb->cap = c;
    sb->len = 0;
    sb->buf = (char *)xmalloc((size_t)c);
    gc_track(sb, (int64_t)sizeof(Sb));
    return sb;
}

void sn_sb_append_cstr(void *sb_ptr, const char *s, int64_t len) {
    if (!sb_ptr || !s || len <= 0) return;
    Sb *sb = (Sb *)sb_ptr;
    sb_reserve(sb, sb->len + len);
    memcpy(sb->buf + sb->len, s, (size_t)len);
    sb->len += len;
}

void sn_sb_append_str(void *sb_ptr, void *str_ptr) {
    if (!sb_ptr || !str_ptr) return;
    const char *s = sn_str_cstr(str_ptr);
    int64_t len = sn_str_len(str_ptr);
    sn_sb_append_cstr(sb_ptr, s, len);
}

void sn_sb_append_i64(void *sb_ptr, int64_t val) {
    if (!sb_ptr) return;
    Sb *sb = (Sb *)sb_ptr;
    sb_reserve(sb, sb->len + 32);
    int n = snprintf(sb->buf + sb->len, 32, "%" PRId64, val);
    if (n > 0) sb->len += n;
}

void sn_sb_append_byte(void *sb_ptr, int64_t b) {
    if (!sb_ptr) return;
    Sb *sb = (Sb *)sb_ptr;
    sb_reserve(sb, sb->len + 1);
    sb->buf[sb->len++] = (char)(b & 0xff);
}

void *sn_sb_to_str(void *sb_ptr) {
    if (!sb_ptr) return sn_str_from_cstr("");
    Sb *sb = (Sb *)sb_ptr;
    return sn_str_new(sb->buf, sb->len);
}

int64_t sn_sb_len(void *sb_ptr) {
    if (!sb_ptr) return 0;
    return ((Sb *)sb_ptr)->len;
}

void sn_sb_clear(void *sb_ptr) {
    if (!sb_ptr) return;
    Sb *sb = (Sb *)sb_ptr;
    sb->len = 0;
}

/* --------------------------------------------------------------------- */
/*  Raw pointer access, for FFI out-parameters and buffers                */
/* --------------------------------------------------------------------- */

/* All of these are deliberately unchecked: `ptr` is the escape hatch for
   C interop, and bounds checking it would only slow down honest callers. */

int64_t sn_ptr_load(void *p, int64_t offset) {
    if (!p) return 0;
    int64_t v;
    memcpy(&v, (const char *)p + offset, sizeof(v));
    return v;
}

void sn_ptr_store(void *p, int64_t offset, int64_t value) {
    if (!p) return;
    memcpy((char *)p + offset, &value, sizeof(value));
}

int64_t sn_ptr_load_byte(void *p, int64_t offset) {
    if (!p) return 0;
    return (int64_t)(*(const unsigned char *)((const char *)p + offset));
}

void sn_ptr_store_byte(void *p, int64_t offset, int64_t value) {
    if (!p) return;
    *(unsigned char *)((char *)p + offset) = (unsigned char)(value & 0xff);
}

double sn_ptr_load_f64(void *p, int64_t offset) {
    if (!p) return 0.0;
    double v;
    memcpy(&v, (const char *)p + offset, sizeof(v));
    return v;
}

void sn_ptr_store_f64(void *p, int64_t offset, double value) {
    if (!p) return;
    memcpy((char *)p + offset, &value, sizeof(value));
}

/* Read a NUL-terminated C string out of foreign memory. */
void *sn_ptr_load_str(void *p, int64_t offset) {
    if (!p) return sn_str_from_cstr("");
    return sn_str_from_cstr((const char *)p + offset);
}

void sn_ptr_store_str(void *p, int64_t offset, const char *s, int64_t len) {
    if (!p || !s) return;
    memcpy((char *)p + offset, s, (size_t)len);
}

void *sn_ptr_alloc(int64_t n) {
    return sn_alloc(n);
}

void sn_ptr_free(void *p) {
    sn_free(p);
}

int64_t sn_future_await(void *p) {
    Future *f = (Future *)p;
    if (!f) return 0;
#ifdef _WIN32
    EnterCriticalSection(&f->mu);
    while (!f->done) SleepConditionVariableCS(&f->cv, &f->mu, INFINITE);
    LeaveCriticalSection(&f->mu);
#else
    pthread_mutex_lock(&f->mu);
    while (!f->done) pthread_cond_wait(&f->cv, &f->mu);
    pthread_mutex_unlock(&f->mu);
#endif
    if (f->data) {
        int64_t len = sn_str_len(f->data);
        return len > 0 ? len : f->result;
    }
    return f->result;
}


/* --------------------------------------------------------------------- */
/*  Work-stealing goroutine scheduler (replaces fixed GO_POOL = 4 queue)   */
/* --------------------------------------------------------------------- */
#define GO_MAX_WORKERS 64
#define GO_QUEUE 256

static int sn_nproc(void) {
#ifdef _WIN32
    SYSTEM_INFO si;
    GetSystemInfo(&si);
    int n = (int)si.dwNumberOfProcessors;
    return n > 0 ? n : 1;
#elif defined(__APPLE__)
    int n = 1;
    size_t len = sizeof(n);
    if (sysctlbyname("hw.ncpu", &n, &len, NULL, 0) != 0 || n <= 0) n = 1;
    return n;
#else
    long n = sysconf(_SC_NPROCESSORS_ONLN);
    return n > 0 ? (int)n : 1;
#endif
}

#ifndef _WIN32 /* POSIX work-stealing pool; on Windows sn_go_spawn falls back to sn_spawn */
typedef struct GoDeque {
    void (*fn[GO_QUEUE])(void *);
    void *arg[GO_QUEUE];
    int head, tail, count;
    pthread_mutex_t mu;
    pthread_cond_t cv;
} GoDeque;
#endif /* _WIN32: no work-stealing pool; sn_go_spawn falls back to sn_spawn */

#ifndef _WIN32
static GoDeque g_deques[GO_MAX_WORKERS];
static pthread_t g_go_workers[GO_MAX_WORKERS];
static int g_go_nworkers = 0;
static int g_go_started = 0;
static int g_go_rr = 0;
#endif


/* global wake line: go_spawn broadcasts so workers re-scan their deques */
#ifdef _WIN32
static CRITICAL_SECTION g_go_evt_mu;
static CONDITION_VARIABLE g_go_evt_cv;
#else
static pthread_mutex_t g_go_evt_mu = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t g_go_evt_cv = PTHREAD_COND_INITIALIZER;
#endif

static void go_evt_wake(void) {
#ifdef _WIN32
    EnterCriticalSection(&g_go_evt_mu);
    WakeAllConditionVariable(&g_go_evt_cv);
    LeaveCriticalSection(&g_go_evt_mu);
#else
    pthread_mutex_lock(&g_go_evt_mu);
    pthread_cond_broadcast(&g_go_evt_cv);
    pthread_mutex_unlock(&g_go_evt_mu);
#endif
}

#ifndef _WIN32 /* POSIX pool internals; Windows uses sn_spawn directly */
static int deque_push(GoDeque *d, void (*fn)(void *), void *arg) {
    pthread_mutex_lock(&d->mu);
    if (d->count >= GO_QUEUE) { pthread_mutex_unlock(&d->mu); return 0; }
    d->fn[d->tail] = fn;
    d->arg[d->tail] = arg;
    d->tail = (d->tail + 1) % GO_QUEUE;
    d->count++;
    pthread_cond_signal(&d->cv);
    pthread_mutex_unlock(&d->mu);
    return 1;
}

static int deque_pop(GoDeque *d, void (**pfn)(void *), void **parg) {
    pthread_mutex_lock(&d->mu);
    if (d->count == 0) { pthread_mutex_unlock(&d->mu); return 0; }
    *pfn = d->fn[d->head];
    *parg = d->arg[d->head];
    d->head = (d->head + 1) % GO_QUEUE;
    d->count--;
    pthread_mutex_unlock(&d->mu);
    return 1;
}

static void *go_worker(void *p) {
    int i = (int)(intptr_t)p;
    char anchor = 0;
    void *lo = NULL, *hi = NULL;
    int has_bounds = gc_stack_bounds(&lo, &hi);

    for (;;) {
        void (*fn)(void *) = NULL;
        void *arg = NULL;
        int got = deque_pop(&g_deques[i], &fn, &arg);
        if (!got) {
            /* steal from the other workers' queues (oldest-first) */
            for (int k = 1; k < g_go_nworkers; k++) {
                int v = (i + k) % g_go_nworkers;
                if (deque_pop(&g_deques[v], &fn, &arg)) { got = 1; break; }
            }
        }
        if (got) {
            if (has_bounds) {
                sn_gc_register_thread(lo, hi);
            } else {
                sn_gc_register_thread((char *)&anchor + (1 << 20), &anchor);
            }
            sn_gc_enter();
            fn(arg);
            sn_gc_leave();
            pthread_mutex_lock(&g_go_mu);
            g_go_pending--;
            if (g_go_pending == 0) pthread_cond_broadcast(&g_go_cv);
            pthread_mutex_unlock(&g_go_mu);
            continue;
        }
        /* all deques empty: sleep briefly on the wake line, twice per slice */
        pthread_mutex_lock(&g_go_evt_mu);
        struct timespec ts;
        clock_gettime(CLOCK_REALTIME, &ts);
        ts.tv_nsec += 2000000; /* 2ms idle slice */
        if (ts.tv_nsec >= 1000000000) { ts.tv_sec++; ts.tv_nsec -= 1000000000; }
        pthread_cond_timedwait(&g_go_evt_cv, &g_go_evt_mu, &ts);
        pthread_mutex_unlock(&g_go_evt_mu);
    }
    return NULL;
}

static pthread_mutex_t g_go_init_mu = PTHREAD_MUTEX_INITIALIZER;
static void go_pool_init(void) {
    pthread_mutex_lock(&g_go_init_mu);
    if (g_go_started) { pthread_mutex_unlock(&g_go_init_mu); return; }
    int n = sn_nproc();
    if (n > GO_MAX_WORKERS) n = GO_MAX_WORKERS;
    if (n < 1) n = 1;
    g_go_nworkers = n;
    for (int i = 0; i < n; i++) {
        pthread_mutex_init(&g_deques[i].mu, NULL);
        pthread_cond_init(&g_deques[i].cv, NULL);
        g_deques[i].head = g_deques[i].tail = g_deques[i].count = 0;
    }
    for (int i = 0; i < n; i++) {
        pthread_create(&g_go_workers[i], NULL, go_worker, (void *)(intptr_t)i);
        pthread_detach(g_go_workers[i]);
    }
    g_go_started = 1;
    pthread_mutex_unlock(&g_go_init_mu);
}
#endif /* POSIX pool internals */

void sn_go_spawn(void (*fn)(void *), void *arg) {
#ifndef _WIN32
    go_pool_init();
    /* track the task so shutdown can drain */
    pthread_mutex_lock(&g_go_mu);
    g_go_pending++;
    pthread_mutex_unlock(&g_go_mu);
    /* round-robin submit across worker queues */
    unsigned int u = (unsigned int)__sync_fetch_and_add(&g_go_rr, 1);
    int idx = (int)(u % (unsigned int)g_go_nworkers);
    if (!deque_push(&g_deques[idx], fn, arg)) {
        /* that deque is full; try every worker before giving up */
        int done = 0;
        for (int k = 0; k < g_go_nworkers; k++) {
            if (deque_push(&g_deques[k], fn, arg)) { done = 1; break; }
        }
        if (!done) {
            /* fall back to a thread-per-task so no work is lost */
            pthread_mutex_lock(&g_go_mu);
            g_go_pending--;
            pthread_mutex_unlock(&g_go_mu);
            sn_spawn(fn, arg);
            return;
        }
    }
    go_evt_wake();
#else
    sn_spawn(fn, arg);
#endif
}

/* --------------------------------------------------------------------- */
/*  SN-Strata Elastic Lanes, Generational Slot Vaults & Multi-Frame RF   */
/* --------------------------------------------------------------------- */

#define SN_LANE_COUNT 8
#define SN_LANE_RESERVE_BYTES (256ULL * 1024ULL * 1024ULL) /* 256 MB virtual reservation per lane */
#define SN_LANE_COMMIT_STEP   (64ULL * 1024ULL)

/* --------------------------------------------------------------------- */
/*  Lane poison-fill mode (DIFF-6 / T-FIBR, W4A repair 2):                 */
/*  Release builds hand out UNWRITTEN (garbage) bytes — the bump is O(1); */
/*  the E0807 full-initialization-before-read checker obligation is what  */
/*  makes that safe. Debug builds poison-fill with 0xAA so any checker     */
/*  miss crashes immediately in CI instead of flowing a garbage list      */
/*  header into bounds arithmetic (silent type confusion). Enabled by     */
/*  -DSN_DEBUG / -DSN_DEBUG_POISON at compile time, or SN_POISON=1 at     */
/*  runtime (resolved once in sn_rt_init, before threads start).          */
/* --------------------------------------------------------------------- */
#if defined(SN_DEBUG) || defined(SN_DEBUG_POISON)
#define SN_LANE_POISON_FORCED 1
#endif

static int g_lane_poison_state = 0; /* 0 unresolved, 1 on, -1 off */

static int sn_lane_poison_on(void) {
#ifdef SN_LANE_POISON_FORCED
    return 1;
#else
    int s = __atomic_load_n(&g_lane_poison_state, __ATOMIC_RELAXED);
    if (s != 0) return s > 0;
    const char *e = getenv("SN_POISON");
    s = (e && e[0] == '1' && e[1] == '\0') ? 1 : -1;
    /* Benign race: every racer stores the same value; sn_rt_init resolves
       the flag single-threaded at startup before this can race in practice. */
    __atomic_store_n(&g_lane_poison_state, s, __ATOMIC_RELAXED);
    return s > 0;
#endif
}

typedef struct SnLane {
    uint8_t *base;
    uint8_t *top;
    uint8_t *committed;
    uint8_t *limit;
    uint32_t initialized;
    uint32_t frozen_depth;
    /* SWARM RABBIT HOLE, D0 gap #1 (first half): lane hierarchy for nursery
       sub-lanes. NULL for thread-root lanes (these g_tls_lanes entries); a
       child nursery lane points at its parent, enabling the O(1) task-group
       join reset and the DIFF-3 adopt path's re-anchoring of received bytes
       onto the receiving frame. Child-lane create/destroy carving is a later
       slice; this slice only plants the field. */
    struct SnLane *parent;
} SnLane;

/* SWARM RABBIT HOLE, D0 gap #2 (Impl-B, Wave 5): the lane array is exported
   (no longer `static`) so the compiler's inline bump/reset fast path can
   gep/load/store `top` directly from LLVM IR (llvm.rs declares @g_tls_lanes
   as `[8 x %sn_lane_t]`, mirroring SnLane). The IR hardcodes the gep field
   indices; these asserts turn any future SnLane layout change into a C
   compile error instead of silent IR corruption. */
SN_TLS SnLane g_tls_lanes[SN_LANE_COUNT];
_Static_assert(SN_LANE_COUNT == 8, "llvm.rs mirror: @g_tls_lanes is [8 x %sn_lane_t]");
_Static_assert(offsetof(SnLane, top) == 8, "llvm.rs gep field 1 (top) must stay at offset 8");
_Static_assert(offsetof(SnLane, limit) == 24, "llvm.rs gep field 3 (limit) must stay at offset 24");
_Static_assert(offsetof(SnLane, initialized) == 32, "llvm.rs gep field 4 (initialized) must stay at offset 32");
_Static_assert(offsetof(SnLane, frozen_depth) == 36, "llvm.rs gep field 5 (frozen_depth) must stay at offset 36");
_Static_assert(sizeof(SnLane) == 48, "llvm.rs lane stride (sizeof %sn_lane_t) must stay 48");
/* Wave 12 (Impl-Q): the lane-aware items pool's registry is sized
   SN_GROWN_LANES at the list section (before SN_LANE_COUNT is defined);
   this ties the two constants together. */
_Static_assert(SN_LANE_COUNT == SN_GROWN_LANES, "lane items registry size must mirror SN_LANE_COUNT");
/* D0 gap #5 (Impl-F, Wave 6): the ν lane the CURRENT task of this thread
   loaned from its nursery (-1 = none). sn_lane_alloc's loaned-lane backstop
   and sn_nursery_loan_lane both consult it; it lives here (not in the carve
   section) because sn_lane_alloc below needs it. */
static SN_TLS int64_t g_tls_loan_lane = -1;

static SnLane *sn_lane_ensure(int64_t lane_id) {
    if (lane_id < 0 || lane_id >= SN_LANE_COUNT) lane_id = 0;
    SnLane *l = &g_tls_lanes[lane_id];
    if (!l->initialized) {
        size_t page_sz;
#ifdef _WIN32
        /* W2F §6: Windows has no POSIX sysconf; keep the 64 KB
           reserve-granularity guard (the MEM_RESERVE tail is the real
           guard on this path). */
        page_sz = 65536;
#else
        /* W2F §6: the POSIX guard must be one OS page — 16 KB on Apple
           Silicon, 4 KB on Linux; the old hardcoded 64 KB was 4 pages on
           macOS and 16 on Linux. */
        long psz = sysconf(_SC_PAGESIZE);
        page_sz = psz > 0 ? (size_t)psz : 4096;
#endif
        size_t total = (size_t)SN_LANE_RESERVE_BYTES + page_sz;
#ifdef _WIN32
        uint8_t *mem = (uint8_t *)VirtualAlloc(NULL, total, MEM_RESERVE, PAGE_NOACCESS);
        if (!mem) { fprintf(stderr, "fatal: SnLane VirtualAlloc failed\n"); exit(1); }
        VirtualAlloc(mem, SN_LANE_COMMIT_STEP, MEM_COMMIT, PAGE_READWRITE);
        l->committed = mem + SN_LANE_COMMIT_STEP;
#else
        uint8_t *mem = (uint8_t *)mmap(NULL, total, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
        if (mem == MAP_FAILED) { fprintf(stderr, "fatal: SnLane mmap failed\n"); exit(1); }
        mprotect(mem + SN_LANE_RESERVE_BYTES, page_sz, PROT_NONE);
        l->committed = mem + SN_LANE_RESERVE_BYTES;
#endif
        l->base = mem;
        l->top = mem;
        l->limit = mem + SN_LANE_RESERVE_BYTES;
        l->initialized = 1;
        l->frozen_depth = 0;
        l->parent = NULL; /* thread-root lane (D0 gap #1): no parent */
    }
    return l;
}

void *sn_lane_save(int64_t lane_id) {
    SnLane *l = sn_lane_ensure(lane_id);
    return (void *)l->top;
}

/* FIBR poison helper (DIFF-6 / T-FIBR, W4A repair 2): fill [from, to) of the
   lane with 0xAA when poison mode is on; no-op in release builds unless
   SN_POISON=1. The rewind path calls this over the freed range so rewinds
   re-poison — a written byte is never trusted across a rewind (the written
   sets die with the lane-write epoch, tied to S2's invalidation lattice). */
void sn_lane_poison(int64_t lane_id, void *from, void *to) {
    if (!sn_lane_poison_on()) return;
    SnLane *l = sn_lane_ensure(lane_id);
    uint8_t *lo = (uint8_t *)from;
    uint8_t *hi = (uint8_t *)to;
    if (!lo || !hi || hi < lo) return;
    if (lo < l->base || hi > l->limit) return;
    memset(lo, 0xAA, (size_t)(hi - lo));
}

void sn_lane_restore(int64_t lane_id, void *watermark) {
    SnLane *l = sn_lane_ensure(lane_id);
    if (l->frozen_depth > 0) {
        fprintf(stderr, "runtime error: cannot rewind frozen Elastic Lane\n");
        exit(1);
    }
    if (watermark && (uint8_t *)watermark >= l->base && (uint8_t *)watermark <= l->limit) {
        /* Wave 12 (Impl-Q) — the lane-aware items pool: free the grown
           items arrays of every lane-backed list header inside the
           rewound range BEFORE the poison fill overwrites the headers
           (the walk reads header->items). The header dies at this
           rewind (no ref to lane memory can escape — E-GC-IN-LANE), so
           its array is unreachable here: the reclaim invariant
           documented at lane_grown_rewind. Registry walk cost:
           O(#lists this rewind frees) — a suffix pop, not a scan. */
        lane_grown_rewind(lane_id, watermark, l->top);
        /* T-FIBR "rewind re-poisons" (W4A repair 2): in debug/poison builds
           the freed range [watermark, old_top) is re-filled with 0xAA before
           the top store, so no byte below the new top is ever trusted as
           written. */
        if (sn_lane_poison_on()) {
            sn_lane_poison(lane_id, watermark, (void *)l->top);
        }
        l->top = (uint8_t *)watermark;
    }
}

void *sn_lane_alloc(int64_t lane_id, int64_t bytes) {
    SnLane *l = sn_lane_ensure(lane_id);
    if (l->frozen_depth > 0) {
        fprintf(stderr, "runtime error: cannot allocate on frozen Elastic Lane\n");
        exit(1);
    }
    /* D0 gap #5 (Impl-F): a lane loaned to a nursery (parent != NULL) is
       exclusively the loaning child's until the join resets it. Allocating
       on a foreign loaned lane would put live bytes on a lane the join
       wipes with its 1-store reset (UAF). The compile-time rule (the
       spawn-safety walk rejecting raw lane_alloc on foreign lanes — later
       slice) is the primary gate; this fail-fast is the runtime backstop.
       Lane 0 is never loaned (sn_nursery_loan_lane skips it), so the
       compiler's region/frozen lowering and the inline IR fast path
       (hardcoded lane 0) never hit this. Out-of-range ids clamp to lane 0
       before this check and are likewise unaffected. */
    if (__atomic_load_n(&l->parent, __ATOMIC_ACQUIRE) != NULL &&
        lane_id != g_tls_loan_lane) {
        fprintf(stderr, "runtime error: lane %lld is loaned to a nursery task group; cannot allocate on a foreign loaned lane\n",
                (long long)lane_id);
        exit(1);
    }
    if (bytes < 0) bytes = 0;
    uintptr_t cur = (uintptr_t)l->top;
    uintptr_t aligned = (cur + 7u) & ~(uintptr_t)7u;
    uint8_t *next = (uint8_t *)(aligned + (size_t)bytes);
#ifdef _WIN32
    if (next > l->committed) {
        if (next > l->limit) { fprintf(stderr, "fatal: SnLane overflow\n"); exit(1); }
        size_t need = ((size_t)(next - l->committed) + SN_LANE_COMMIT_STEP - 1) & ~(SN_LANE_COMMIT_STEP - 1);
        VirtualAlloc(l->committed, need, MEM_COMMIT, PAGE_READWRITE);
        l->committed += need;
    }
#else
    if (next > l->limit) {
        fprintf(stderr, "fatal: SnLane overflow\n");
        exit(1);
    }
#endif
    /* DIFF-6 (W2-E spec, Option A): the bump is O(1) — release builds do NOT
       zero (the E0807 full-initialization-before-read checker obligation
       makes unwritten bytes safe to hand out; the only deleted line in the
       wave was the unconditional memset that was here). Debug/poison builds
       fill 0xAA so any checker miss crashes in CI instead of producing a
       garbage list header that could flow into bounds arithmetic. The limit
       check above stays as-is: POSIX is check-based today (W2F §3 — no
       SIGSEGV trap handler in this slice). */
    if (bytes > 0 && sn_lane_poison_on()) {
        memset((void *)aligned, 0xAA, (size_t)bytes);
    }
    l->top = next;
    return (void *)aligned;
}

/* DIFF-6 opt-in zeroing twin: same commit/overflow logic as sn_lane_alloc,
   but the caller receives zeroed bytes (interop / first-touch-dependent
   code; mirrors bumpalo's alloc_zeroed split so the benchmark plan's
   zeroed-vs-raw rows are both expressible). The region is fully-written at
   allocation, so the E0807 checker treats it as covered (the compiler-side
   intrinsic wiring is a later slice). `align` rounds up to a power of two;
   align <= 8 behaves exactly like sn_lane_alloc. */
void *sn_lane_alloc_zeroed(int64_t lane_id, int64_t bytes, int64_t align) {
    SnLane *l = sn_lane_ensure(lane_id);
    if (l->frozen_depth > 0) {
        fprintf(stderr, "runtime error: cannot allocate on frozen Elastic Lane\n");
        exit(1);
    }
    /* D0 gap #5 (Impl-F): same loaned-ν exclusivity backstop as
       sn_lane_alloc — see the comment there. */
    if (__atomic_load_n(&l->parent, __ATOMIC_ACQUIRE) != NULL &&
        lane_id != g_tls_loan_lane) {
        fprintf(stderr, "runtime error: lane %lld is loaned to a nursery task group; cannot allocate on a foreign loaned lane\n",
                (long long)lane_id);
        exit(1);
    }
    if (bytes < 0) bytes = 0;
    int64_t a = 8;
    if (align > 8 && align <= 65536) {
        while (a < align) a <<= 1;
    }
    uintptr_t cur = (uintptr_t)l->top;
    uintptr_t aligned = (cur + (uintptr_t)a - 1u) & ~((uintptr_t)a - 1u);
    uint8_t *next = (uint8_t *)(aligned + (size_t)bytes);
#ifdef _WIN32
    if (next > l->committed) {
        if (next > l->limit) { fprintf(stderr, "fatal: SnLane overflow\n"); exit(1); }
        size_t need = ((size_t)(next - l->committed) + SN_LANE_COMMIT_STEP - 1) & ~(SN_LANE_COMMIT_STEP - 1);
        VirtualAlloc(l->committed, need, MEM_COMMIT, PAGE_READWRITE);
        l->committed += need;
    }
#else
    if (next > l->limit) { /* W2F §3: POSIX stays check-based (no trap handler) */
        fprintf(stderr, "fatal: SnLane overflow\n");
        exit(1);
    }
#endif
    if (bytes > 0) memset((void *)aligned, 0, (size_t)bytes);
    l->top = next;
    return (void *)aligned;
}

int64_t sn_lane_used(int64_t lane_id) {
    SnLane *l = sn_lane_ensure(lane_id);
    return (int64_t)(l->top - l->base);
}

void sn_lane_freeze(int64_t lane_id) {
    SnLane *l = sn_lane_ensure(lane_id);
    l->frozen_depth++;
}

void sn_lane_unfreeze(int64_t lane_id) {
    SnLane *l = sn_lane_ensure(lane_id);
    if (l->frozen_depth > 0) l->frozen_depth--;
}

/* --------------------------------------------------------------------- */
/*  SWARM RABBIT HOLE, D0 gap #5 (Impl-F, Wave 6): the nursery            */
/*  child-lane carve — ν loan lanes reclaimed O(1) at join.               */
/*                                                                        */
/*  DESIGN DECISION (loaned-lane first cut vs. the full sub-lane):        */
/*  The full design (W2A §1.1's ν generator; D0 gap-table row 5) carves a  */
/*  per-task sub-lane out of the PARENT's lane memory and makes           */
/*  nursery_wait a single store of the parent's watermark. That needs a   */
/*  per-task lane set outside the fixed 8-slot TLS array (the v3 lane-set  */
/*  extension) plus a child-side allocation API the compiler can target —  */
/*  compiler wiring this slice deliberately excludes. The honest first    */
/*  cut on the existing array: the child's ν is one of ITS OWN thread's    */
/*  TLS lanes (1..7; lane 0 stays the compiler's region/frozen lane),     */
/*  reserved by pointing SnLane.parent (Impl-A's field) at the nursery's   */
/*  parent_lane — the loaned mark IS the parent link — and registered in   */
/*  the nursery's intrusive registry. The child allocates through the      */
/*  existing sn_lane_alloc(ν, ·); the join resets each loaned lane with    */
/*  ONE store (top = base) and returns it to the thread's free set        */
/*  (parent = NULL). Per-lane O(1), zero per-allocation work — W2A §1.1   */
/*  "spawn parent ⊑ₗ ν_child; join resets ν O(1)", §1.4 "fresh lane,      */
/*  empty stack" (the loan's top = base), §2.2 A.1 C-SPAWN/C-JOIN + SR3   */
/*  (LIFO residual closes, panic-safe join), Theorem 1(iii) (1 store).    */
/*                                                                        */
/*  Limitation (documented): 7 loanable lanes per worker thread; the      */
/*  8th concurrent child on one worker gets ν = -1 and falls back to the  */
/*  existing behavior (its allocations go to its own thread's stack       */
/*  region/heap and are not join-reclaimed). The pinned/DPS lane classes  */
/*  (L2..L7, STRATA §2) share these lanes with loans in the full design — */
/*  resolved by the v3 lane-set extension this slice defers.              */
/* --------------------------------------------------------------------- */
static int64_t g_join_lanes_reset = 0;      /* stats: ν lanes reset at joins */
static int64_t g_join_bytes_reclaimed = 0; /* stats: ν bytes reclaimed at joins */
static int64_t g_loan_fallbacks = 0;       /* stats: loans refused (all busy) */

/* Child-thread entry (a nursery child calls this when it starts; the
   compiler wiring — an ir.rs spawn-prologue call — is a later slice; the
   systems C test drives it directly, mimicking the compiled shape).
   Reserves one ν lane on the CALLING thread: marks it loaned by pointing
   parent at the nursery's anchor, registers it with the nursery
   (mutex-protected push), returns the lane id. Returns -1 when every
   loanable lane is busy. */
int64_t sn_nursery_loan_lane(void *nursery) {
    SnNursery *n = (SnNursery *)nursery;
    if (g_tls_loan_lane >= 0) return g_tls_loan_lane; /* idempotent per task */
    if (!n || !n->parent_lane) {
        __atomic_add_fetch(&g_loan_fallbacks, 1, __ATOMIC_RELAXED);
        return -1;
    }
    for (int64_t k = 1; k < (int64_t)SN_LANE_COUNT; k++) {
        /* Lane 0 is excluded: the compiler's region/frozen lowering and the
           inline IR fast path are hardcoded to it; a loaned lane 0 would be
           wiped by a join while a later task's region blocks allocate on it. */
        SnLane *l = sn_lane_ensure(k);
        if (__atomic_load_n(&l->parent, __ATOMIC_ACQUIRE) != NULL) continue;
        /* W2A §1.4: fresh lane, empty stack — reset the watermark. Any bytes
           above base are un-reclaimed garbage from finished tasks (the leak
           this slice closes); no live task can hold them: one task runs at a
           time per worker thread, and lane pointers cannot cross threads. */
        l->top = l->base;
        /* The loaned mark IS the parent link (Impl-A's field): ν_child's ⊑ₗ
           anchor is the nursery's parent_lane. Release-store: publishes the
           top = base reset to the next loan of this same lane. */
        __atomic_store_n(&l->parent, n->parent_lane, __ATOMIC_RELEASE);
        SnLoan *node = (SnLoan *)xmalloc(sizeof(SnLoan));
        node->lane = l;
#ifdef _WIN32
        EnterCriticalSection(&n->mu);
        node->next = n->loan_head;
        n->loan_head = node;
        LeaveCriticalSection(&n->mu);
#else
        pthread_mutex_lock(&n->mu);
        node->next = n->loan_head;
        n->loan_head = node;
        pthread_mutex_unlock(&n->mu);
#endif
        g_tls_loan_lane = k;
        return k;
    }
    __atomic_add_fetch(&g_loan_fallbacks, 1, __ATOMIC_RELAXED);
    return -1; /* all loanable lanes busy: fall back to existing behavior */
}

/* Task end (called from sn_nursery_leave_task): the ν permission dies with
   the task. The loan itself stays registered until the join resets it. */
void sn_lane_loan_task_end(void) {
    g_tls_loan_lane = -1;
}

/* The join's O(1) reset (called from sn_nursery_wait after the envelope
   drain; every task of the group has left, so no concurrent loaner or
   allocator exists — the same argument that makes the envelope drain
   safe). SR3 (W2A §1.6/§2.2 A.1 C-JOIN): residual closes LIFO — the
   registry is a push-down, so popping it closes ν lanes innermost-first.
   Per lane: 1 store (top = base, Theorem 1(iii)), then the release-store
   returning it to its thread's free set. */
static void sn_nursery_loan_drain(SnNursery *n) {
    int64_t lanes = 0;
    int64_t bytes = 0;
    SnLoan *node = n->loan_head;
    n->loan_head = NULL;
    while (node) {
        SnLoan *next = node->next;
        SnLane *l = node->lane;
        bytes += (int64_t)(l->top - l->base);
        l->top = l->base; /* THE 1-STORE RESET: the child's whole allocation reclaims */
        __atomic_store_n(&l->parent, (struct SnLane *)NULL, __ATOMIC_RELEASE);
        free(node);
        lanes++;
        node = next;
    }
    if (lanes > 0) {
        __atomic_add_fetch(&g_join_lanes_reset, lanes, __ATOMIC_RELAXED);
        __atomic_add_fetch(&g_join_bytes_reclaimed, bytes, __ATOMIC_RELAXED);
    }
}

/* Test/diagnostic surface for the carve (systems tests; later slices may
   keep or retire it): 0 = ν lanes reset at joins, 1 = ν bytes reclaimed at
   joins, 2 = loans refused (all loanable lanes busy). */
int64_t sn_nursery_join_stats(int64_t what) {
    switch (what) {
    case 0: return __atomic_load_n(&g_join_lanes_reset, __ATOMIC_RELAXED);
    case 1: return __atomic_load_n(&g_join_bytes_reclaimed, __ATOMIC_RELAXED);
    case 2: return __atomic_load_n(&g_loan_fallbacks, __ATOMIC_RELAXED);
    default: return 0;
    }
}

/* Generational Slot Vault (SnPool) */
void *sn_pool_new(int64_t elem_kind, int64_t elem_size) {
    if (elem_size < 8) elem_size = 8;
    SnPool *pl = (SnPool *)xmalloc(sizeof(SnPool));
    pl->rc = 1;
    pl->kind = KIND_POOL;
    gc_track(pl, (int64_t)sizeof(SnPool));
    pl->elem_kind = elem_kind;
    pl->elem_size = elem_size;
    pl->len = 0;
    pl->cap = 16;
    pl->active_count = 0;
    pl->free_head = 0xFFFFFFFFu;
    pl->slots = (uint8_t *)calloc(16, (size_t)elem_size);
    pl->generations = (uint32_t *)calloc(16, sizeof(uint32_t));
    pl->free_next = (uint32_t *)calloc(16, sizeof(uint32_t));
    /* SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the default
     * surface is the frozen global pool (GP_FROZEN) — a plain
     * `pool_new` from the declaring thread, unboxed. */
    pl->mode = GP_FROZEN;
    pl->frozen_depth = 0;
    pl->home_thread = sn_thread_self();
#ifndef _WIN32
    pthread_mutex_init(&pl->mu, NULL);
#else
    InitializeCriticalSection(&pl->mu);
#endif
    return pl;
}

/* SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the mode-aware
 * construction — `pool<T> thread_local` / `pool<T> synchronized`
 * initializers lower here. GP_FROZEN keeps sn_pool_new (the default).
 * The GP_SYNC pool's lock is initialized once here and lives for the
 * pool's lifetime; the GP_THREADLOCAL pool stamps its declaring
 * thread (the E0803.1 backstop). */
void *sn_pool_new_mode(int64_t elem_kind, int64_t elem_size, int64_t mode) {
    void *p = sn_pool_new(elem_kind, elem_size);
    SnPool *pl = (SnPool *)p;
    pl->mode = mode;
    return p;
}

/* DIFF-4 (Impl-O): the `unfrozen (P) { ... }` window's runtime
 * discipline — SnLane's freeze/unfreeze twin on the pool. The
 * CHECKER is the gate (E0803.2/.3); frozen_depth is the debug-mode
 * enforcement of the window shape (nestable, LIFO, per-pool). */
void sn_pool_unfreeze(void *p) {
    if (!p) return;
    SnPool *pl = (SnPool *)p;
    pl->frozen_depth += 1;
}

void sn_pool_freeze(void *p) {
    if (!p) return;
    SnPool *pl = (SnPool *)p;
    if (pl->frozen_depth > 0) pl->frozen_depth -= 1;
}

/* DIFF-4 (Impl-O): the Synchronized vault's per-op lock pair and the
 * home-thread guard. The lock follows the runtime's existing
 * convention (the Lock object's pthread_mutex, runtime.c's
 * "thread_*" pattern); the home-thread guard is the E0803.1 backstop
 * — the checker rejects capture statically, and a ThreadConfined
 * pool mutated from any other thread fail-fasts loudly (the E-LKIND
 * two-layer precedent: checker first, runtime backstop for what the
 * checker cannot see). */
static void sn_pool_lock(SnPool *pl) {
#ifdef _WIN32
    EnterCriticalSection(&pl->mu);
#else
    pthread_mutex_lock(&pl->mu);
#endif
}

static void sn_pool_unlock(SnPool *pl) {
#ifdef _WIN32
    LeaveCriticalSection(&pl->mu);
#else
    pthread_mutex_unlock(&pl->mu);
#endif
}

/* sn_pool_maybe_lock/unlock: the per-op funnel — a GP_SYNC pool takes
 * its lock around every insert/set/remove; every other mode is a
 * no-op (Frozen pools cannot reach a mutation outside the window by
 * the checker; ThreadConfined pools carry the home-thread guard). */
static void sn_pool_maybe_lock(SnPool *pl) {
    if (pl->mode == GP_SYNC) sn_pool_lock(pl);
}

static void sn_pool_maybe_unlock(SnPool *pl) {
    if (pl->mode == GP_SYNC) sn_pool_unlock(pl);
}

static void sn_pool_thread_guard(SnPool *pl, const char *op) {
    if (pl->mode == GP_THREADLOCAL && pl->home_thread != sn_thread_self()) {
        fprintf(stderr,
                "runtime error [E0803.1]: pool is 'thread_local' — %s from a "
                "non-declaring thread (handles carry a home-thread tag and never "
                "leave the declaring thread) [E0803]\n",
                op);
        exit(1);
    }
}

static void sn_pool_grow(SnPool *pl) {
    uint32_t old_cap = pl->cap;
    uint32_t new_cap = old_cap * 2;
    pl->slots = (uint8_t *)realloc(pl->slots, (size_t)new_cap * (size_t)pl->elem_size);
    memset(pl->slots + (size_t)old_cap * (size_t)pl->elem_size, 0, (size_t)old_cap * (size_t)pl->elem_size);
    pl->generations = (uint32_t *)realloc(pl->generations, (size_t)new_cap * sizeof(uint32_t));
    memset(pl->generations + old_cap, 0, (size_t)old_cap * sizeof(uint32_t));
    pl->free_next = (uint32_t *)realloc(pl->free_next, (size_t)new_cap * sizeof(uint32_t));
    memset(pl->free_next + old_cap, 0, (size_t)old_cap * sizeof(uint32_t));
    pl->cap = new_cap;
}

static uint32_t sn_pool_alloc_slot(SnPool *pl, uint32_t *out_gen) {
    uint32_t idx;
    if (pl->free_head != 0xFFFFFFFFu) {
        idx = pl->free_head;
        pl->free_head = pl->free_next[idx];
        /* Bump even (free) generation to odd (live) generation */
        pl->generations[idx] += 1u;
    } else {
        if (pl->len == pl->cap) sn_pool_grow(pl);
        idx = pl->len++;
        pl->generations[idx] = 1u; /* First live generation is 1 */
    }
    pl->active_count++;
    *out_gen = pl->generations[idx];
    return idx;
}

int64_t sn_pool_insert_i64(void *p, int64_t val) {
    if (!p) return 0;
    SnPool *pl = (SnPool *)p;
    /* SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the mutator
     * funnel — GP_SYNC pools serialize the freelist mutation behind
     * the vault lock (no two threads can interleave insert/remove);
     * GP_THREADLOCAL pools refuse any non-declaring thread (the
     * E0803.1 backstop). */
    sn_pool_maybe_lock(pl);
    sn_pool_thread_guard(pl, "insert");
    uint32_t gen = 0;
    uint32_t idx = sn_pool_alloc_slot(pl, &gen);
    if (pl->elem_kind == 1) sn_retain((void *)(intptr_t)val);
    *(int64_t *)(pl->slots + (size_t)idx * (size_t)pl->elem_size) = val;
    sn_pool_maybe_unlock(pl);
    uint64_t h = ((uint64_t)gen << 32) | (uint64_t)idx;
    return (int64_t)h;
}

int64_t sn_pool_insert_rec(void *p, void *src, int64_t sz) {
    if (!p) return 0;
    SnPool *pl = (SnPool *)p;
    sn_pool_maybe_lock(pl);
    sn_pool_thread_guard(pl, "insert");
    uint32_t gen = 0;
    uint32_t idx = sn_pool_alloc_slot(pl, &gen);
    size_t copy_sz = (size_t)(sz <= pl->elem_size ? sz : pl->elem_size);
    if (src && copy_sz > 0) {
        memcpy(pl->slots + (size_t)idx * (size_t)pl->elem_size, src, copy_sz);
    }
    sn_pool_maybe_unlock(pl);
    uint64_t h = ((uint64_t)gen << 32) | (uint64_t)idx;
    return (int64_t)h;
}

int64_t sn_pool_is_valid(void *p, int64_t handle) {
    if (!p || handle == 0) return 0;
    SnPool *pl = (SnPool *)p;
    uint64_t u = (uint64_t)handle;
    uint32_t idx = (uint32_t)(u & 0xFFFFFFFFu);
    uint32_t gen = (uint32_t)(u >> 32);
    if ((gen & 1u) == 0 || idx >= pl->len) return 0;
    return pl->generations[idx] == gen ? 1 : 0;
}

static uint8_t *sn_pool_checked_ptr(SnPool *pl, int64_t handle, const char *op) {
    if (!pl) {
        fprintf(stderr, "runtime error: null pool in %s\n", op);
        exit(1);
    }
    uint64_t u = (uint64_t)handle;
    uint32_t idx = (uint32_t)(u & 0xFFFFFFFFu);
    uint32_t gen = (uint32_t)(u >> 32);
    if ((gen & 1u) == 0 || idx >= pl->len || pl->generations[idx] != gen) {
        fprintf(stderr, "runtime error: stale or invalid pool handle in %s (index=%u, expected_gen=%u, actual_gen=%u)\n",
                op, idx, gen, idx < pl->len ? pl->generations[idx] : 0u);
        exit(1);
    }
    return pl->slots + (size_t)idx * (size_t)pl->elem_size;
}

int64_t sn_pool_get_i64(void *p, int64_t handle) {
    SnPool *pl = (SnPool *)p;
    uint8_t *slot = sn_pool_checked_ptr(pl, handle, "pool.get");
    return *(int64_t *)slot;
}

void sn_pool_set_i64(void *p, int64_t handle, int64_t val) {
    SnPool *pl = (SnPool *)p;
    /* DIFF-4 (Impl-O): the mutator funnel — see sn_pool_insert_i64.
     * The lock spans the whole op: the checked_ptr read of the
     * generation array and the slot store are one atomic unit for
     * GP_SYNC pools (a concurrent remove could otherwise recycle the
     * slot between the check and the store — the A6c race in
     * miniature). */
    sn_pool_maybe_lock(pl);
    sn_pool_thread_guard(pl, "set");
    uint8_t *slot = sn_pool_checked_ptr(pl, handle, "pool.set");
    if (pl->elem_kind == 1) {
        int64_t old = *(int64_t *)slot;
        sn_release((void *)(intptr_t)old);
        sn_retain((void *)(intptr_t)val);
    }
    *(int64_t *)slot = val;
    sn_pool_maybe_unlock(pl);
}

void *sn_pool_get_ptr(void *p, int64_t handle) {
    SnPool *pl = (SnPool *)p;
    return (void *)sn_pool_checked_ptr(pl, handle, "pool.get_ref");
}

void sn_pool_remove(void *p, int64_t handle) {
    SnPool *pl = (SnPool *)p;
    /* DIFF-4 (Impl-O): the mutator funnel — see sn_pool_insert_i64.
     * remove is the freelist mutation itself (free_head push), so
     * GP_SYNC serialization is the data-race-freedom condition
     * here: no two threads can ever interleave insert/remove. */
    sn_pool_maybe_lock(pl);
    sn_pool_thread_guard(pl, "remove");
    uint8_t *slot = sn_pool_checked_ptr(pl, handle, "pool.remove");
    uint32_t idx = (uint32_t)((uint64_t)handle & 0xFFFFFFFFu);
    if (pl->elem_kind == 1) {
        int64_t old = *(int64_t *)slot;
        sn_release((void *)(intptr_t)old);
    }
    memset(slot, 0, (size_t)pl->elem_size);
    /* Advance odd live generation to even freed generation */
    pl->generations[idx] += 1u;
    pl->free_next[idx] = pl->free_head;
    pl->free_head = idx;
    if (pl->active_count > 0) pl->active_count--;
    sn_pool_maybe_unlock(pl);
}

int64_t sn_pool_len(void *p) {
    if (!p) return 0;
    return (int64_t)((SnPool *)p)->active_count;
}

/* Euclidean modulo: result is always in [0, |b|) */
int64_t sn_euclid_mod_i64(int64_t a, int64_t b) {
    if (b == 0) {
        fprintf(stderr, "runtime error: modulo by zero\n");
        exit(1);
    }
    if (a == INT64_MIN && b == -1) return 0;
    int64_t r = a % b;
    if (r < 0) {
        r += (b < 0) ? -b : b;
    }
    return r;
}

/* Multi-frame structured return (`return expr from targetFn`) */
static SN_TLS int64_t g_rf_target_id = 0;
static SN_TLS int64_t g_rf_val0 = 0;
static SN_TLS int64_t g_rf_val1 = 0;

void sn_rf_trigger(int64_t target_id, int64_t v0, int64_t v1) {
    g_rf_target_id = target_id;
    g_rf_val0 = v0;
    g_rf_val1 = v1;
}

int64_t sn_rf_active(void) {
    return g_rf_target_id;
}

int64_t sn_rf_consume_val0(void) {
    int64_t v = g_rf_val0;
    g_rf_target_id = 0;
    g_rf_val0 = 0;
    return v;
}

int64_t sn_rf_consume_val1(void) {
    int64_t v = g_rf_val1;
    g_rf_val1 = 0;
    return v;
}

