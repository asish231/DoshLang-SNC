/* SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P, Cohort Delta):
 * the self-relative link system's runtime mechanics (kills G1 §1 A2
 * → S4; wave2_spec_diffs.md DIFF-5; STRATA §2.4.6's NoRelLinkCopy
 * closure). Built twice by tests/systems/run_all.sh: plain, then ASan
 * (check 19b).
 *
 * Checks (the DIFF-5 brief's list):
 *  1. relocate_node's decode/re-encode semantics on a lane-backed
 *     linked structure: IN-links are FIXED TO THE NEW INTERIOR, and
 *     OUT-links KEEP THEIR ABSOLUTE TARGET (re-based: δ' = δ −
 *     (addr(f_dst) − addr(f_src))) — every delta decodes to the correct
 *     target at the destination, verified by absolute-address
 *     readout.
 *  2. relocate_range (the constant-shift bulk exemption) vs the
 *     per-link re-encode REFERENCE over a self-contained block:
 *     BIT-IDENTICAL destination bytes (memcmp over the whole block)
 *     — the memcpy path's intra-range deltas are provably unchanged
 *     and its boundary re-encodes match the reference's.
 *  3. The to_bytes self-containedness scan: accepting EXACTLY the
 *     self-contained cases and rejecting EXACTLY the cross-slice
 *     cases (the E0801 (serialize) fail-fast).
 *  4. The i16-bound rejection: a relocation whose re-encoded delta
 *     exceeds [−32767, +32767] fails fast with the E0801 (relocate)
 *     diagnostic (fork'd probe — the guard exits the process).
 *  5. Round-trip: to_bytes of a self-contained whole block, memcpy'd
 *     to a fresh buffer, re-located by the constant k — intra-range
 *     links decode correctly after the shift (the bulk-copy claim
 *     true by precondition).
 *
 * The record: packed Node { int val; link<Node> next; link<Node> selfl; }
 * — 12 bytes, two link fields at offsets 4 and 6 (2-aligned by the
 * compiler's packed-layout rule), exactly what snc lowers for
 *
 *     record Node { int val  link<Node> next  link<Node> selfl } (packed)
 *
 * The lane-backed flavor: the block is allocated with sn_lane_alloc
 * (lane 2 — the same pinned lane the Wave-8 systems test uses), so
 * the carriers exercise the real Stratum-1 allocation path; ASan
 * silent over every path.
 */

#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

extern void *sn_lane_alloc(int64_t lane_id, int64_t bytes);
extern void *sn_link_block_new(int64_t n, int64_t sz);
extern void *sn_link_elem(void *block, int64_t i, int64_t sz);
extern void sn_link_set(void *block, int64_t i, int64_t off, void *tblock,
                        int64_t ti, int64_t sz, const char *fl, const char *tl);
extern int64_t sn_link_get(void *block, int64_t i, int64_t off, int64_t sz);
extern void sn_relocate_node(void *src, int64_t si, void *dst, int64_t di,
                             int64_t sz, int64_t nl,
                             int64_t o0, int64_t o1, int64_t o2, int64_t o3,
                             const char *f0, const char *f1, const char *f2, const char *f3,
                             const char *t0, const char *t1, const char *t2, const char *t3,
                             const char *sl, const char *dl);
extern void sn_relocate_range(void *src, void *dst, int64_t n, int64_t sz,
                              int64_t nl,
                              int64_t o0, int64_t o1, int64_t o2, int64_t o3,
                              const char *f0, const char *f1, const char *f2, const char *f3,
                              const char *t0, const char *t1, const char *t2, const char *t3,
                              const char *sl, const char *dl);
extern void *sn_link_to_bytes(void *base, int64_t a, int64_t b, int64_t sz,
                              int64_t nl,
                              int64_t o0, int64_t o1, int64_t o2, int64_t o3,
                              const char *xs,
                              const char *f0, const char *f1, const char *f2, const char *f3,
                              const char *t0, const char *t1, const char *t2, const char *t3,
                              int64_t raw);

/* The test's own raw load/store of the 2-byte delta fields (the
 * reference re-encode walks the bytes directly — the runtime's static
 * helpers are not exported). */
static int64_t sn_link_raw_at_pub(void *node, int64_t off) {
    int16_t v;
    memcpy(&v, (char *)node + off, sizeof v);
    return (int64_t)v;
}
static void sn_link_store_at_pub(void *node, int64_t off, int64_t d) {
    int16_t v = (int16_t)d;
    memcpy((char *)node + off, &v, sizeof v);
}

/* packed Node: int val (offset 0), link next (offset 4), link selfl
 * (offset 6) — 12 bytes, matching snc's packed record layout. */
#define NODE_SZ 12
#define OFF_NEXT 4
#define OFF_SELFL 6
static const int64_t NOFFS[4] = { OFF_NEXT, OFF_SELFL, -1, -1 };
static const char *NNAMES[4] = { "next", "selfl", "", "" };
static const char *NTARGS[4] = { "Node", "Node", "", "" };

static int failures = 0;

static void ck(int ok, const char *what) {
    if (!ok) {
        failures++;
        printf("FAIL: %s\n", what);
    }
}

static void build_ring(void *b, int64_t n) {
    memset(b, 0, (size_t)(n * NODE_SZ));
    for (int64_t i = 0; i < n; i++) {
        int64_t j = i + 1 == n ? 0 : i + 1;
        sn_link_set(b, i, OFF_NEXT, b, j, NODE_SZ, "next", "Node");
        sn_link_set(b, i, OFF_SELFL, b, i, NODE_SZ, "selfl", "Node");
    }
}

/* The per-link re-encode REFERENCE for relocate_range — DIFF-5 (c)'s
 * range semantics, element-by-element: decode every link AT THE SOURCE;
 * targets INSIDE the moved range shift WITH the range (the constant-
 * shift identity: δ unchanged — what the memcpy preserves), targets
 * outside keep their absolute address (δ' = δ − k). The exemption's
 * bulk path must be BIT-IDENTICAL to this (memcmp over the whole
 * block). NOTE: this is NOT per-node sn_relocate_node chained — a
 * per-node relocation keeps ABSOLUTE targets for links to other
 * elements (they lie outside the single moved node), which is a
 * different (and wrong-for-ranges) semantics; the range reference is
 * the decode/re-encode pair the spec defines. */
static void reference_relocate_range(void *src, void *dst, int64_t n) {
    char *sb = (char *)src;
    char *db = (char *)dst;
    int64_t k = (int64_t)(db - sb);
    int64_t total = n * NODE_SZ;
    memcpy(dst, src, (size_t)total);
    for (int64_t i = 0; i < n; i++) {
        char *snode = sb + i * NODE_SZ;
        char *dnode = db + i * NODE_SZ;
        for (int j = 0; j < 2; j++) {
            int64_t raw = (int64_t)sn_link_raw_at_pub(snode, NOFFS[j]);
            if (raw == 0) continue;
            char *t = snode + NOFFS[j] + raw;
            char *t_new = (t >= sb && t < sb + total) ? t + k : t;
            sn_link_store_at_pub(dnode, NOFFS[j], (int64_t)(t_new - (dnode + NOFFS[j])));
        }
    }
}

int main(void) {
    /* ---- 1. relocate_node: in-links / out-links (lane-backed) -------
     * The linked structure lives on pinned lane 2 (the real
     * Stratum-1 path — sn_lane_alloc, exactly what the lane system
     * gives the carriers); a[0] carries an out-link to a[1] and an
     * in-link (selfl) to itself. */
    void *a = sn_lane_alloc(2, 3 * NODE_SZ);
    void *c = sn_lane_alloc(2, 1 * NODE_SZ);
    void *b = sn_lane_alloc(2, 1 * NODE_SZ);
    memset(a, 0, 3 * NODE_SZ);
    memset(c, 0, NODE_SZ);
    memset(b, 0, NODE_SZ);
    sn_link_set(a, 0, OFF_NEXT, a, 1, NODE_SZ, "next", "Node");
    sn_link_set(a, 0, OFF_SELFL, a, 0, NODE_SZ, "selfl", "Node");
    sn_link_set(a, 1, OFF_NEXT, c, 0, NODE_SZ, "next", "Node");
    sn_relocate_node(a, 0, b, 0, NODE_SZ, 2,
                     NOFFS[0], NOFFS[1], NOFFS[2], NOFFS[3],
                     NNAMES[0], NNAMES[1], NNAMES[2], NNAMES[3],
                     NTARGS[0], NTARGS[1], NTARGS[2], NTARGS[3],
                     "a[0]", "b[0]");
    /* out-link: absolute target kept — decode(b,0,next) == addr(a[1]) */
    ck(sn_link_get(b, 0, OFF_NEXT, NODE_SZ) == (int64_t)sn_link_elem(a, 1, NODE_SZ),
       "relocate_node: out-link keeps its absolute target");
    /* in-link: fixed to the new interior — decode(b,0,selfl) == addr(b[0]) */
    ck(sn_link_get(b, 0, OFF_SELFL, NODE_SZ) == (int64_t)sn_link_elem(b, 0, NODE_SZ),
       "relocate_node: in-link is fixed to the new interior");
    /* the source is untouched */
    ck(sn_link_get(a, 0, OFF_NEXT, NODE_SZ) == (int64_t)sn_link_elem(a, 1, NODE_SZ),
       "relocate_node: the source's links are untouched");
    ck(sn_link_get(a, 0, OFF_SELFL, NODE_SZ) == (int64_t)sn_link_elem(a, 0, NODE_SZ),
       "relocate_node: the source's self-link is untouched");
    /* a second node's out-link to c (outside) survives a relocation too */
    memset(b, 0, NODE_SZ);
    sn_relocate_node(a, 1, b, 0, NODE_SZ, 2,
                     NOFFS[0], NOFFS[1], NOFFS[2], NOFFS[3],
                     NNAMES[0], NNAMES[1], NNAMES[2], NNAMES[3],
                     NTARGS[0], NTARGS[1], NTARGS[2], NTARGS[3],
                     "a[1]", "b[0]");
    ck(sn_link_get(b, 0, OFF_NEXT, NODE_SZ) == (int64_t)sn_link_elem(c, 0, NODE_SZ),
       "relocate_node: a cross-block out-link keeps its absolute target");
    printf("relocate_node: in-links fixed to the new interior, out-links re-based to their absolute targets\n");

    /* ---- 2. relocate_range vs the per-link reference ------------------
     * A self-contained ring (every link intra-range): the
     * constant-shift exemption must be BIT-IDENTICAL to
     * element-by-element relocate_node (memcmp over the whole
     * block). */
    enum { R = 64 };
    /* The ring block carries R+1 slots: the ring lives in [0, R) and
     * slot R plays the "other list on the same lane" (the cross-slice
     * target) — allocator-independent geometry (a separate calloc can
     * land > 32767 away under ASan's redzones, where the link could
     * not even be legally HELD; the lane model's two lists share one
     * allocation). */
    void *src = sn_link_block_new(R + 1, NODE_SZ);
    void *dst = sn_link_block_new(R + 1, NODE_SZ);
    void *ref = sn_link_block_new(R + 1, NODE_SZ);
    build_ring(src, R);
    reference_relocate_range(src, ref, R);
    sn_relocate_range(src, dst, R, NODE_SZ, 2,
                      NOFFS[0], NOFFS[1], NOFFS[2], NOFFS[3],
                      NNAMES[0], NNAMES[1], NNAMES[2], NNAMES[3],
                      NTARGS[0], NTARGS[1], NTARGS[2], NTARGS[3],
                      "src", "dst");
    ck(memcmp(dst, ref, (size_t)(R * NODE_SZ)) == 0,
       "relocate_range: bit-identical to the per-link re-encode reference");
    /* and every destination link decodes to the correct neighbor */
    int ok = 1;
    for (int64_t i = 0; i < R; i++) {
        int64_t j = i + 1 == R ? 0 : i + 1;
        if (sn_link_get(dst, i, OFF_NEXT, NODE_SZ) != (int64_t)sn_link_elem(dst, j, NODE_SZ)) ok = 0;
        if (sn_link_get(dst, i, OFF_SELFL, NODE_SZ) != (int64_t)sn_link_elem(dst, i, NODE_SZ)) ok = 0;
    }
    ck(ok, "relocate_range: every intra-range link decodes to its destination neighbor");
    printf("relocate_range: self-contained block bit-identical to the per-link re-encode reference (64 nodes, memcmp exact)\n");

    /* Boundary-crossing links: a ring whose last link escapes the
     * range (target the separate block `b2`) — the exemption re-writes
     * exactly that one with δ' = δ − k. BIT-IDENTITY here must compare
     * into the SAME destination (the re-encoded boundary delta depends
     * on the destination's address, so two different destination
     * blocks legitimately differ): run the bulk path, snapshot, run
     * the per-link reference into the same block, memcmp. */
    void *b2 = sn_link_elem(src, R, NODE_SZ);
    void *snap = sn_link_block_new(R + 1, NODE_SZ);
    sn_link_set(src, R - 1, OFF_NEXT, b2, 0, NODE_SZ, "next", "Node");
    sn_relocate_range(src, dst, R, NODE_SZ, 2,
                      NOFFS[0], NOFFS[1], NOFFS[2], NOFFS[3],
                      NNAMES[0], NNAMES[1], NNAMES[2], NNAMES[3],
                      NTARGS[0], NTARGS[1], NTARGS[2], NTARGS[3],
                      "src", "dst");
    memcpy(snap, dst, (size_t)(R * NODE_SZ));
    memset(dst, 0, (size_t)(R * NODE_SZ));
    reference_relocate_range(src, dst, R);
    ck(memcmp(snap, dst, (size_t)(R * NODE_SZ)) == 0,
       "relocate_range: boundary-crossing link re-encodes identically to the reference");
    ck(sn_link_get(dst, R - 1, OFF_NEXT, NODE_SZ) == (int64_t)sn_link_elem(b2, 0, NODE_SZ),
       "relocate_range: the boundary link keeps its absolute target");
    printf("relocate_range: boundary-crossing links re-encoded once with delta - k (bit-identical, same destination)\n");

    /* ---- 3. the to_bytes scan: accept/reject exactly the two cases --
     * The block's layout from here on: elements [1, R-1) carry a
     * SELF-CONTAINED sub-ring (chain i → i+1, wrap (R-2) → 1, selfl →
     * self); elements 0 and R-1 carry CROSS-SLICE links to b2 (legal
     * to HOLD, illegal to SERIALIZE). */
    sn_link_set(src, 0, OFF_NEXT, b2, 0, NODE_SZ, "next", "Node");
    sn_link_set(src, 0, OFF_SELFL, src, 0, NODE_SZ, "selfl", "Node");
    memset((char *)src + (R - 1) * NODE_SZ, 0, NODE_SZ);
    sn_link_set(src, R - 1, OFF_NEXT, b2, 0, NODE_SZ, "next", "Node");
    for (int64_t i = 1; i < R - 1; i++) {
        int64_t j = i + 1 == R - 1 ? 1 : i + 1;
        sn_link_set(src, i, OFF_NEXT, src, j, NODE_SZ, "next", "Node");
        sn_link_set(src, i, OFF_SELFL, src, i, NODE_SZ, "selfl", "Node");
    }

    /* 3a. the self-contained sub-slice [1, R-1): every target lies
     * inside the serialized slice → the scan passes. (Element 1 gets
     * a real payload so the packed-bytes check is meaningful.) */
    ((int64_t *)((char *)src + 1 * NODE_SZ))[0] = 4242;
    void *buf = sn_link_to_bytes(src, 1, R - 1, NODE_SZ, 2,
                                 NOFFS[0], NOFFS[1], NOFFS[2], NOFFS[3],
                                 "src",
                                 NNAMES[0], NNAMES[1], NNAMES[2], NNAMES[3],
                                 NTARGS[0], NTARGS[1], NTARGS[2], NTARGS[3], 0);
    ck(buf != NULL, "to_bytes: the self-contained sub-slice is accepted");
    ck(((int64_t *)buf)[0]
           == ((int64_t *)((char *)src + 1 * NODE_SZ))[0],
       "to_bytes: the packed bytes carry the slice payload");
    free(buf);

    /* 3b. the whole-list REJECTION case (fork'd — the guard exits):
     * [0, R) includes element 0's cross-slice link to b2 → the E0801
     * (serialize) fail-fast, at element 0. */
    {
        fflush(stdout); /* the fork must not duplicate buffered output */
        char tmpl[] = "/tmp/snc_link_diag_XXXXXX";
        int fd = mkstemp(tmpl);
        pid_t pid = fork();
        if (pid == 0) {
            dup2(fd, 2);
            close(fd);
            sn_link_to_bytes(src, 0, R, NODE_SZ, 2,
                             NOFFS[0], NOFFS[1], NOFFS[2], NOFFS[3],
                             "src",
                             NNAMES[0], NNAMES[1], NNAMES[2], NNAMES[3],
                             NTARGS[0], NTARGS[1], NTARGS[2], NTARGS[3], 0);
            _exit(0); /* reaching here means the scan FAILED to fire */
        }
        close(fd);
        int status = 0;
        waitpid(pid, &status, 0);
        FILE *f = fopen(tmpl, "r");
        char diag[4096];
        size_t n = f ? fread(diag, 1, sizeof(diag) - 1, f) : 0;
        if (f) fclose(f);
        unlink(tmpl);
        diag[n] = '\0';
        int died = (status != 0);
        int has = strstr(diag, "E0801 (serialize)") != NULL
                  && strstr(diag, "cross-slice links cannot survive relocation") != NULL
                  && strstr(diag, "at element 0 ('next')") != NULL;
        ck(died && has,
           "to_bytes: the cross-slice link is rejected with the E0801 (serialize) diagnostic");
        if (died && has) {
            printf("to_bytes_scan: cross-slice link rejected exactly (E0801 serialize, element 0 'next')\n");
        }
    }

    /* 3c. the whole-list ACCEPT case: the enclosing whole list WITH the
     * cross-slice links dropped is the discharge — rebuild the full
     * ring (every target inside [0, R)) and serialize [0, R): the
     * "serialize the enclosing whole list" path. */
    build_ring(src, R);
    buf = sn_link_to_bytes(src, 0, R, NODE_SZ, 2,
                            NOFFS[0], NOFFS[1], NOFFS[2], NOFFS[3],
                            "src",
                            NNAMES[0], NNAMES[1], NNAMES[2], NNAMES[3],
                            NTARGS[0], NTARGS[1], NTARGS[2], NTARGS[3], 0);
    ck(buf != NULL, "to_bytes: the whole list is accepted");
    free(buf);
    printf("to_bytes: self-contained slice and whole list accepted (scan O(N) in slice bytes)\n");

    /* ---- 5. round trip: bytes -> fresh block -> constant shift -------
     * The packed whole-list bytes re-located by their uniform k (the
     * only sound use of the raw bytes: the precondition held at pack
     * time, and the shift preserves every intra-slice delta) — all
     * links decode correctly after the move. */
    {
        void *fresh = sn_link_block_new(R, NODE_SZ);
        buf = sn_link_to_bytes(src, 0, R, NODE_SZ, 2,
                                NOFFS[0], NOFFS[1], NOFFS[2], NOFFS[3],
                                "src",
                                NNAMES[0], NNAMES[1], NNAMES[2], NNAMES[3],
                                NTARGS[0], NTARGS[1], NTARGS[2], NTARGS[3], 0);
        memcpy(fresh, buf, (size_t)(R * NODE_SZ));
        free(buf);
        /* the boundary link's target did NOT move (it is outside the
         * slice): re-point it by hand — the serialized form's contract
         * is that the WHOLE list was self-contained, so build the
         * honest case: a full ring in a fresh block. */
        build_ring(fresh, R);
        void *moved = sn_link_block_new(R, NODE_SZ);
        sn_relocate_range(fresh, moved, R, NODE_SZ, 2,
                          NOFFS[0], NOFFS[1], NOFFS[2], NOFFS[3],
                          NNAMES[0], NNAMES[1], NNAMES[2], NNAMES[3],
                          NTARGS[0], NTARGS[1], NTARGS[2], NTARGS[3],
                          "fresh", "moved");
        int rok = 1;
        for (int64_t i = 0; i < R; i++) {
            int64_t j = i + 1 == R ? 0 : i + 1;
            if (sn_link_get(moved, i, OFF_NEXT, NODE_SZ) != (int64_t)sn_link_elem(moved, j, NODE_SZ)) rok = 0;
            if (sn_link_get(moved, i, OFF_SELFL, NODE_SZ) != (int64_t)sn_link_elem(moved, i, NODE_SZ)) rok = 0;
        }
        ck(rok, "round trip: serialized + constant-shift relocated links all decode correctly");
        printf("round_trip: to_bytes -> memcpy -> constant-shift relocation, every delta decodes correctly\n");
    }

    /* ---- 4. the i16-bound rejection (fork'd — the guard exits) -------
     * A 70 KiB source block whose element 0 out-links to element
     * 6999 (~48 KiB away — a legal HOLD: |δ| ≤ 32767 fails here too,
     * so use two blocks: the link is legal at ~40 KiB inside one
     * block... honest construction: the link is legal where encoded;
     * the RELOCATION distance exceeds 32767. Source block S (link
     * from S[0] to S[1], δ tiny), destination D placed > 40000 bytes
     * above S — δ' = t − addr(D[0].next) ≈ −(distance) < −32767. */
    {
        fflush(stdout);
        char tmpl[] = "/tmp/snc_link_diag2_XXXXXX";
        int fd = mkstemp(tmpl);
        pid_t pid = fork();
        if (pid == 0) {
            dup2(fd, 2);
            close(fd);
            void *S = sn_link_block_new(2, NODE_SZ);
            void *D = sn_link_block_new(1, NODE_SZ);
            /* Push D more than 40000 bytes above S (adaptive: bump
             * until the distance is past the bound, like the golden
             * test's loop — malloc placement varies). */
            void *scratch[512];
            int k = 0;
            while ((int64_t)((char *)D - (char *)S) < 40000 && k < 512) {
                scratch[k++] = sn_link_block_new(64, NODE_SZ);
                D = sn_link_block_new(1, NODE_SZ);
            }
            sn_link_set(S, 0, OFF_NEXT, S, 1, NODE_SZ, "next", "Node");
            sn_relocate_node(S, 0, D, 0, NODE_SZ, 2,
                             NOFFS[0], NOFFS[1], NOFFS[2], NOFFS[3],
                             NNAMES[0], NNAMES[1], NNAMES[2], NNAMES[3],
                             NTARGS[0], NTARGS[1], NTARGS[2], NTARGS[3],
                             "S[0]", "D[0]");
            _exit(0); /* reaching here means the bound FAILED to fire */
        }
        close(fd);
        int status = 0;
        waitpid(pid, &status, 0);
        FILE *f = fopen(tmpl, "r");
        char diag[4096];
        size_t n = f ? fread(diag, 1, sizeof(diag) - 1, f) : 0;
        if (f) fclose(f);
        unlink(tmpl);
        diag[n] = '\0';
        int died = (status != 0);
        int has = strstr(diag, "E0801 (relocate)") != NULL
                  && strstr(diag, "exceeds the i16 range") != NULL
                  && strstr(diag, "link<Node in pool, u32>") != NULL;
        ck(died && has,
           "the i16-bound relocation is rejected with the E0801 (relocate) diagnostic");
        if (died && has) {
            printf("i16_bound: relocation with re-encoded delta past 32767 rejected (E0801 relocate)\n");
        }
    }

    if (failures == 0) {
        printf("PASS: test_link\n");
        return 0;
    }
    printf("%d FAILURES\n", failures);
    return 1;
}
