// Go escape-analysis twin — SWARM RABBIT HOLE, Impl-C (Cohort Delta, Wave 5).
// D0 §7 #5: (a) escaping struct loop (heap per iter), verified with
// `go build -gcflags=-m` (capture in the build log); (b) non-escaping variant
// (stack, compiler-determined); label the mechanism explicitly — Go has no
// stable user arena, and safe Go cannot get unzeroed heap memory, so every Go
// heap row is a ZEROED row (mallocgc always zeroes; documented in RESULTS.md).
//
// Rows (identical protocol to the C/Rust/SNlang twins):
//   timer     — one time.Now() pair, ns (K=1,000,000)
//   emptyloop — N iterations of acc++
//   heap16    — &S{} (16 B) escaping to the heap per op, checksum via pointer
//               sums, scratch store per op, read pass (word 0 per record)
//   fresh16   — the alloc-only loop (escape per op, no free — Go has no free;
//               GC disabled for parity with the no-collector rows)
//   stack16   — the same allocation NOT escaping: escape analysis keeps it
//               on the stack (verify: build log says "moved to heap: NONE")
//   fresh4k   — new([4096]byte) escaping per op
//   freshlcg  — make([]byte, 16+st%48) from the fixed LCG, per op
//   gc_reset  — runtime.GC(): Go's wholesale reset, TOTAL ns (not per-op) —
//               the honest contrast with the lane's O(1) restore
//
// GC control: debug.SetGCPercent(-1) disables threshold collections during
// the loops (parity: C malloc has no collector; the SNlang heap row pins its
// GC threshold to 2e9). GOMAXPROCS(1): single-threaded (the SNlang GC
// refuses multi-thread; a threaded row would measure GC policy, not alloc).

package main

import (
	"fmt"
	"os"
	"runtime"
	"runtime/debug"
	"strconv"
	"time"
	"unsafe"
)

type S struct{ a, b int } // 16 bytes on arm64

// Helpers: keep the unsafe conversions in one place; the sinks above are
// what force the escapes (see the -gcflags=-m build log).
func unsafePointer(s *S) unsafe.Pointer { return unsafe.Pointer(s) }

func bytePointer(b *byte) unsafe.Pointer { return unsafe.Pointer(b) }

func loadWord(p uintptr) uintptr {
	return *(*uintptr)(unsafe.Pointer(p))
}

// Package-level sinks force the escape (go build -gcflags=-m prints
// "escapes to heap" at each store).
var (
	sink  *S
	sink4 *[4096]byte
	sinkb []byte
)

func report(row string, allocNs, readNs, resetNs int64, ops int64, ck int64) {
	fmt.Printf("BENCH row=%s alloc_ns=%d read_ns=%d reset_ns=%d ops=%d checksum=%d\n",
		row, allocNs, readNs, resetNs, ops, ck)
}

func main() {
	n := 10000000
	if len(os.Args) > 1 {
		if v, err := strconv.Atoi(os.Args[1]); err == nil && v > 0 {
			n = v
		}
	}
	N := int64(n)

	runtime.GOMAXPROCS(1)
	debug.SetGCPercent(-1)

	scratch := make([]uintptr, n)

	// warmup: n/100 escaping allocations (icache + branch state)
	for i := 0; i < n/100; i++ {
		s := &S{}
		sink = s
	}

	// ---- timer overhead: 1M time.Now() pairs
	t0 := time.Now()
	var acct int64
	for i := 0; i < 1000000; i++ {
		now := time.Now()
		acct += now.UnixNano()
	}
	d := time.Since(t0).Nanoseconds()
	report("timer", d/1000000, 0, 0, 1000000, acct)

	// ---- empty loop tax
	t0 = time.Now()
	var acc0 int64
	for i := int64(0); i < N; i++ {
		acc0++
	}
	d = time.Since(t0).Nanoseconds()
	report("emptyloop", d/N, 0, 0, N, acc0)

	// ---- heap16: escaping 16-byte records, all live, read pass
	t0 = time.Now()
	var acc1 int64
	for i := 0; i < n; i++ {
		s := &S{a: i, b: i}
		sink = s
		p := uintptr(unsafePointer(s))
		acc1 += int64(p)
		scratch[i] = p
	}
	t1 := time.Now()
	var sum1 int64
	for i := 0; i < n; i++ {
		sum1 += int64(loadWord(scratch[i]))
	}
	t2 := time.Now()
	report("heap16",
		time.Since(t0).Nanoseconds()/N,
		t2.Sub(t1).Nanoseconds()/N,
		0, N, acc1+sum1)

	// ---- fresh16: alloc-only loop (escape per op, no free mechanism)
	t0 = time.Now()
	var acc2 int64
	for i := 0; i < n; i++ {
		s := &S{a: i, b: i}
		sink = s
		acc2 += int64(uintptr(unsafePointer(s)))
	}
	t1 = time.Now()
	report("fresh16", t1.Sub(t0).Nanoseconds()/N, 0, 0, N, acc2)

	// ---- stack16: NOT escaping (the escape-analysis datapoint; the build
	// log must show the address of s does NOT move to the heap)
	t0 = time.Now()
	var acc3 int64
	for i := 0; i < n; i++ {
		s := &S{a: i, b: i}
		acc3 += int64(s.a + s.b)
	}
	t1 = time.Now()
	report("stack16", t1.Sub(t0).Nanoseconds()/N, 0, 0, N, acc3)

	// ---- fresh4k
	t0 = time.Now()
	var acc4 int64
	for i := 0; i < n; i++ {
		p := new([4096]byte)
		sink4 = p
		acc4 += int64(uintptr(bytePointer(&p[0])))
	}
	t1 = time.Now()
	report("fresh4k", t1.Sub(t0).Nanoseconds()/N, 0, 0, N, acc4)

	// ---- freshlcg (identical constants to the twins; draw BEFORE advance)
	st := uint32(12345)
	t0 = time.Now()
	var acc5 int64
	for i := 0; i < n; i++ {
		sz := 16 + int(st%48)
		b := make([]byte, sz)
		sinkb = b
		acc5 += int64(uintptr(bytePointer(&b[0])))
		st = (st*1103515245 + 12345) & 0x7fffffff
	}
	t1 = time.Now()
	report("freshlcg", t1.Sub(t0).Nanoseconds()/N, 0, 0, N, acc5)
	fmt.Printf("lcg_state_final=%d\n", st)

	// ---- gc_reset: the wholesale reset Go actually has — one forced GC over
	// everything the loops left behind. TOTAL ns, not per-op.
	var m0, m1 runtime.MemStats
	runtime.GC() // settle before measuring
	runtime.ReadMemStats(&m0)
	t0 = time.Now()
	runtime.GC()
	d = time.Since(t0).Nanoseconds()
	runtime.ReadMemStats(&m1)
	report("gc_reset", 0, 0, d, N, int64(m1.NumGC-m0.NumGC))
	fmt.Printf("heap_live_after_gc_mb=%d heap_objects=%d\n",
		m1.HeapAlloc>>20, m1.HeapObjects)

	var m runtime.MemStats
	runtime.ReadMemStats(&m)
	fmt.Printf("total_alloc_mb=%d sys_mb=%d\n", m.TotalAlloc>>20, m.Sys>>20)
	fmt.Println("GO_BENCH_DONE")
}
