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
    int64_t elem_kind; /* 0=i64, 1=ptr */
    int64_t len;
    int64_t cap;
    int64_t *items;
} List;

typedef struct Map {
    int64_t rc;
    int64_t kind;
    int64_t key_kind;
    int64_t val_kind;
    int64_t len;
    int64_t cap;
    int64_t *keys;
    int64_t *vals;
} Map;

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
void sn_rt_init(int argc, char **argv) {
    g_argc = argc;
    g_argv = argv;

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
        g_rt_ready = 1;
        WSADATA wsa;
        WSAStartup(MAKEWORD(2, 2), &wsa);
    }
#endif
}

void sn_panic(void *msg);
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

enum { KIND_BYTEARRAY = 20 };

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
    *lo = addr;
    *hi = (char *)addr + sz;
    return 1;
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
    case KIND_BYTEARRAY: {
        /* data is raw bytes, not traced. */
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

/* Mark any tracked object whose address appears in [lo, hi). */
static void gc_scan_range(void *lo, void *hi) {
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
static void gc_free(void *p) {
    Obj *o = (Obj *)p;
    switch (o->kind) {
    case KIND_LIST: {
        List *l = (List *)p;
        free(l->items);
        break;
    }
    case KIND_MAP: {
        Map *m = (Map *)p;
        free(m->keys);
        free(m->vals);
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
    /* The collecting thread's own frames: a generous bounded window above the
       current stack pointer. Caller frames always sit at higher addresses, so
       this covers every live frame without walking off the stack. */
    gc_scan_range(sp, (char *)sp + GC_SELF_SCAN);

    for (int64_t i = 0; i < g_gc_count; i++) g_gc_objs[i].marked = 0;

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
    o->rc--;
    if (o->rc > 0) return;
    gc_untrack(p);
    switch (o->kind) {
    case KIND_LIST: {
        List *l = (List *)p;
        if (l->elem_kind == 1) {
            for (int64_t i = 0; i < l->len; i++) sn_release((void *)(intptr_t)l->items[i]);
        }
        free(l->items);
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
        free(m);
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

void *sn_list_new(int64_t elem_kind) {
    List *l = (List *)xmalloc(sizeof(List));
    l->rc = 1;
    l->kind = KIND_LIST;
    gc_track(l, (int64_t)sizeof(List));
    l->elem_kind = elem_kind;
    l->len = 0;
    l->cap = 4;
    l->items = (int64_t *)xmalloc(sizeof(int64_t) * (size_t)l->cap);
    return l;
}

static void list_grow(List *l) {
    if (l->len < l->cap) return;
    l->cap *= 2;
    l->items = (int64_t *)realloc(l->items, sizeof(int64_t) * (size_t)l->cap);
}

void sn_list_push_i64(void *p, int64_t v) {
    List *l = (List *)p;
    list_grow(l);
    l->items[l->len++] = v;
}

void sn_list_push_ptr(void *p, void *v) {
    List *l = (List *)p;
    list_grow(l);
    sn_retain(v);
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
    sn_release((void *)(intptr_t)l->items[i]);
    sn_retain(v);
    l->items[i] = (int64_t)(intptr_t)v;
}

void *sn_map_new(int64_t key_kind, int64_t val_kind) {
    Map *m = (Map *)xmalloc(sizeof(Map));
    m->rc = 1;
    m->kind = KIND_MAP;
    gc_track(m, (int64_t)sizeof(Map));
    m->key_kind = key_kind;
    m->val_kind = val_kind;
    m->len = 0;
    m->cap = 4;
    m->keys = (int64_t *)xmalloc(sizeof(int64_t) * 4);
    m->vals = (int64_t *)xmalloc(sizeof(int64_t) * 4);
    return m;
}

static int key_eq(Map *m, int64_t a, int64_t b) {
    if (m->key_kind == 1) return (int)sn_str_eq((void *)(intptr_t)a, (void *)(intptr_t)b);
    return a == b;
}

void sn_map_set(void *p, int64_t key, int64_t val) {
    if (!p) return;
    Map *m = (Map *)p;
    for (int64_t i = 0; i < m->len; i++) {
        if (key_eq(m, m->keys[i], key)) {
            if (m->val_kind == 1) {
                sn_release((void *)(intptr_t)m->vals[i]);
                sn_retain((void *)(intptr_t)val);
            }
            m->vals[i] = val;
            return;
        }
    }
    if (m->len == m->cap) {
        m->cap *= 2;
        m->keys = (int64_t *)realloc(m->keys, sizeof(int64_t) * (size_t)m->cap);
        m->vals = (int64_t *)realloc(m->vals, sizeof(int64_t) * (size_t)m->cap);
    }
    if (m->key_kind == 1) sn_retain((void *)(intptr_t)key);
    if (m->val_kind == 1) sn_retain((void *)(intptr_t)val);
    m->keys[m->len] = key;
    m->vals[m->len] = val;
    m->len++;
}

int64_t sn_map_has(void *p, int64_t key) {
    if (!p) return 0;
    Map *m = (Map *)p;
    for (int64_t i = 0; i < m->len; i++) if (key_eq(m, m->keys[i], key)) return 1;
    return 0;
}

int64_t sn_map_get(void *p, int64_t key) {
    if (!p) return 0;
    Map *m = (Map *)p;
    for (int64_t i = 0; i < m->len; i++) if (key_eq(m, m->keys[i], key)) return m->vals[i];
    return 0;
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

void sn_chan_send(void *p, int64_t v) {
    Chan *c = (Chan *)p;
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
        fprintf(stderr, "send on closed channel\n");
        exit(1);
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
}

int64_t sn_chan_recv(void *p) {
    Chan *c = (Chan *)p;
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
        fprintf(stderr, "receive on closed empty channel\n");
        exit(1);
    }
    int64_t v = c->buf[c->head];
    c->head = (c->head + 1) % c->cap;
    c->len--;
#ifdef _WIN32
    LeaveCriticalSection(&c->mu);
#else
    pthread_mutex_unlock(&c->mu);
#endif
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
    rx_lock();
    InitializeCriticalSection(&g_rx_mu);
    rx_unlock();
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
    g_rx_thread = CreateThread(NULL, 0, rx_thread_main, NULL, 0, NULL);
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
        close(fd);
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
        close(fd);
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
    /* Register this worker's stack so the collector can treat its frame as a
       root. The GC refuses to run while a mutator is active, so scanning is
       never racy. */
    char anchor = 0;
    {
        void *lo = NULL, *hi = NULL;
        if (gc_stack_bounds(&lo, &hi)) {
            sn_gc_register_thread(lo, hi);
        } else {
            sn_gc_register_thread((char *)&anchor + (1 << 20), &anchor);
        }
    }
    for (;;) {
        void (*fn)(void *) = NULL;
        void *arg = NULL;
        if (deque_pop(&g_deques[i], &fn, &arg)) {
            sn_gc_enter();
            fn(arg);
            sn_gc_leave();
            pthread_mutex_lock(&g_go_mu);
            g_go_pending--;
            if (g_go_pending == 0) pthread_cond_broadcast(&g_go_cv);
            pthread_mutex_unlock(&g_go_mu);
            continue;
        }
        /* steal from the other workers' queues (oldest-first) */
        int stole = 0;
        for (int k = 1; k < g_go_nworkers; k++) {
            int v = (i + k) % g_go_nworkers;
            if (deque_pop(&g_deques[v], &fn, &arg)) { stole = 1; break; }
        }
        if (stole) {
            fn(arg);
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
    int idx = __sync_fetch_and_add(&g_go_rr, 1) % g_go_nworkers;
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
