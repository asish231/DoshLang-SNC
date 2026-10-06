/* Test helper for examples/ffi_extern.sn: exercises C out-parameters. */
#include <stdint.h>

int64_t shim_fill(int64_t *out) {
    *out = 4242;
    return 7;
}

int64_t shim_get(int64_t v) { return v + 1; }
