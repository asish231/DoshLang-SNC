/*
 * Minimal Freestanding Runtime for SNlang
 * Bare-metal mode: zero GC symbols, no POSIX pthreads, minimal footprint.
 */

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include <string.h>
#include <stdio.h>
#include <stdlib.h>

void *sn_str_new(const char *s, int64_t len);
void *sn_str_from_cstr(const char *s);

#define KIND_STR 1
#define KIND_BOX 7

typedef struct Str {
    int64_t rc;
    int64_t kind;
    int64_t len;
    char data[];
} Str;

typedef struct BoxI64 {
    int64_t rc;
    int64_t kind;
    int64_t val;
} BoxI64;

typedef struct BoxF64 {
    int64_t rc;
    int64_t kind;
    double val;
} BoxF64;

/* Allocation / Free */
void *sn_alloc(int64_t sz) {
    if (sz <= 0) sz = 8;
    return calloc(1, (size_t)sz);
}

void sn_free(void *p) {
    if (p) free(p);
}

void *sn_ptr_alloc(int64_t sz) { return sn_alloc(sz); }
void sn_ptr_free(void *p) { sn_free(p); }

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

void *sn_ptr_load_str(void *p, int64_t offset) {
    if (!p) return sn_str_from_cstr("");
    return sn_str_from_cstr((const char *)p + offset);
}

void sn_ptr_store_str(void *p, int64_t offset, const char *s, int64_t len) {
    if (!p || !s) return;
    memcpy((char *)p + offset, s, (size_t)len);
}

void sn_gc_register_global(void *p) { (void)p; }

/* Strings */
void *sn_str_new(const char *s, int64_t len) {
    Str *st = (Str *)malloc(sizeof(Str) + (size_t)len + 1);
    if (!st) return NULL;
    st->rc = 1;
    st->kind = KIND_STR;
    st->len = len;
    if (s && len) memcpy(st->data, s, (size_t)len);
    st->data[len] = 0;
    return st;
}

void *sn_str_from_cstr(const char *s) {
    if (!s) return sn_str_new("", 0);
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
    Str *sa = (Str *)a;
    Str *sb = (Str *)b;
    int64_t la = sa ? sa->len : 0;
    int64_t lb = sb ? sb->len : 0;
    Str *out = (Str *)malloc(sizeof(Str) + (size_t)(la + lb) + 1);
    if (!out) return NULL;
    out->rc = 1;
    out->kind = KIND_STR;
    out->len = la + lb;
    if (sa && la) memcpy(out->data, sa->data, (size_t)la);
    if (sb && lb) memcpy(out->data + la, sb->data, (size_t)lb);
    out->data[la + lb] = 0;
    return out;
}

int64_t sn_str_eq(void *a, void *b) {
    if (a == b) return 1;
    if (!a || !b) return 0;
    Str *sa = (Str *)a;
    Str *sb = (Str *)b;
    if (sa->len != sb->len) return 0;
    return memcmp(sa->data, sb->data, (size_t)sa->len) == 0 ? 1 : 0;
}

void *sn_str_from_i64(int64_t v) {
    char buf[32];
    int n = snprintf(buf, sizeof(buf), "%lld", (long long)v);
    return sn_str_new(buf, n > 0 ? n : 0);
}

void *sn_str_from_bool(int64_t v) {
    return v ? sn_str_from_cstr("true") : sn_str_from_cstr("false");
}

void *sn_str_from_dec(int64_t v, int64_t scale) {
    (void)scale;
    char buf[64];
    int n = snprintf(buf, sizeof(buf), "%lld", (long long)v);
    return sn_str_new(buf, n > 0 ? n : 0);
}

void *sn_str_from_f64(double v) {
    char buf[64];
    int n = snprintf(buf, sizeof(buf), "%g", v);
    return sn_str_new(buf, n > 0 ? n : 0);
}

double sn_f64_from_str(void *p) {
    if (!p) return 0.0;
    return atof(((Str *)p)->data);
}

int64_t sn_i64_from_str(void *p) {
    if (!p) return 0;
    return atoll(((Str *)p)->data);
}

/* Printing */
void sn_print_i64(int64_t v) { printf("%lld\n", (long long)v); }
void sn_printn_i64(int64_t v) { printf("%lld", (long long)v); }
void sn_print_bool(int64_t v) { printf("%s\n", v ? "true" : "false"); }
void sn_printn_bool(int64_t v) { printf("%s", v ? "true" : "false"); }
void sn_print_str(void *p) { printf("%s\n", p ? ((Str *)p)->data : ""); }
void sn_printn_str(void *p) { printf("%s", p ? ((Str *)p)->data : ""); }
void sn_print_f64(double v) { printf("%g\n", v); }
void sn_printn_f64(double v) { printf("%g", v); }
void sn_print_none(void) { printf("none\n"); }
void sn_printn_none(void) { printf("none"); }

/* Boxing */
void *sn_box_i64(int64_t v) {
    BoxI64 *b = (BoxI64 *)malloc(sizeof(BoxI64));
    if (!b) return NULL;
    b->rc = 1;
    b->kind = KIND_BOX;
    b->val = v;
    return b;
}

int64_t sn_unbox_i64(void *p) {
    if (!p) return 0;
    return ((BoxI64 *)p)->val;
}

void *sn_box_f64(double v) {
    BoxF64 *b = (BoxF64 *)malloc(sizeof(BoxF64));
    if (!b) return NULL;
    b->rc = 1;
    b->kind = KIND_BOX;
    b->val = v;
    return b;
}

double sn_unbox_f64(void *p) {
    if (!p) return 0.0;
    return ((BoxF64 *)p)->val;
}

/* Math */
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
    if (b == 0) return 0;
    return a / b;
}

int64_t sn_mod_i64(int64_t a, int64_t b) {
    if (b == 0) return 0;
    return a % b;
}

int64_t sn_euclid_mod_i64(int64_t a, int64_t b) {
    if (b == 0) return 0;
    int64_t r = a % b;
    return r < 0 ? (b > 0 ? r + b : r - b) : r;
}

/* Runtime Lifecycle & ARC */
void sn_rt_init(int argc, char **argv) { (void)argc; (void)argv; }
void sn_rt_shutdown(void) {}
void sn_retain(void *p) { (void)p; }
void sn_release(void *p) { (void)p; }

/* Panic and Assertions */
void sn_panic(void *msg) {
    const char *s = msg ? ((Str *)msg)->data : "panic";
    fprintf(stderr, "panic: %s\n", s);
    exit(1);
}

void sn_contract_assert(int64_t cond, void *msg) {
    if (!cond) {
        fprintf(stderr, "contract assertion failed: %s\n", (const char *)msg);
        exit(1);
    }
}

/* Elastic Lanes / Arenas */
#define FREESTANDING_LANE_SIZE (1024 * 1024)
static char g_lane_storage[8][FREESTANDING_LANE_SIZE];
static size_t g_lane_offsets[8] = {0};

void *sn_lane_save(int64_t lane_id) {
    int idx = (int)(lane_id & 7);
    return (void *)(intptr_t)g_lane_offsets[idx];
}

void sn_lane_restore(int64_t lane_id, void *wm) {
    int idx = (int)(lane_id & 7);
    g_lane_offsets[idx] = (size_t)(intptr_t)wm;
}

void *sn_lane_alloc(int64_t lane_id, int64_t sz) {
    int idx = (int)(lane_id & 7);
    size_t aligned_sz = ((size_t)sz + 7) & ~7ULL;
    if (g_lane_offsets[idx] + aligned_sz > FREESTANDING_LANE_SIZE) {
        return NULL;
    }
    void *ptr = &g_lane_storage[idx][g_lane_offsets[idx]];
    g_lane_offsets[idx] += aligned_sz;
    return ptr;
}

int64_t sn_lane_used(int64_t lane_id) {
    int idx = (int)(lane_id & 7);
    return (int64_t)g_lane_offsets[idx];
}

void sn_lane_freeze(int64_t lane_id) { (void)lane_id; }
void sn_lane_unfreeze(int64_t lane_id) { (void)lane_id; }

/* Coverage hook */
void sn_cov_hit(int64_t slot) { (void)slot; }

/* Clean stubs for linking other modules safely */
void *sn_obj_new(int64_t sz, int64_t kind, void *vtable) {
    (void)sz; (void)kind; (void)vtable;
    return NULL;
}
int64_t sn_enum_eq(void *a, void *b) { return a == b ? 1 : 0; }
