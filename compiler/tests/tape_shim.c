/* SWARM RABBIT HOLE — Wave 16 (Impl-U, Cohort Delta): the E1 shim —
 * the delta-typed extern's producer (examples/tape_extern_delta.sn).
 * The CDC-feed shape: each call returns the ledger's next state, a
 * Tier-G Z-set built through the runtime's own symbols (resolved at
 * the final link, the ffi_extern precedent). The tape checks ILC's
 * a ⊕ (b ⊖ a) = b on every recorded (prev, next) pair at the effect
 * boundary — this producer is honest, so the law holds; the check is
 * the tripwire a lying producer would trip. */
#include <stdint.h>

void *sn_zset_new(int64_t);
void sn_zset_add(void *, int64_t, int64_t);

void *tape_feed(int64_t step) {
    void *z = sn_zset_new(0);
    sn_zset_add(z, 100, step);
    sn_zset_add(z, 200, step * 2);
    return z;
}
