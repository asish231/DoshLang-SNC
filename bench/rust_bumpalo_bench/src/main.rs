// Rust bumpalo twin — SWARM RABBIT HOLE, Impl-C (Cohort Delta, Wave 5).
// D0 §7 #4: bumpalo Bump + alloc loop + reset per region, raw (unwritten)
// vs zeroed variants — mirrors the memset question on the Rust side.
//
// Rows (identical protocol to the C/SNlang/Go twins):
//   timer       — one Instant::now() pair, ns (K=1,000,000)
//   emptyloop   — N iterations of acc += 1
//   amort16     — ONE Bump: raw alloc_layout(16,align 8) x N, checksum via
//                 pointer sums, scratch store per op, read pass (word 0 per
//                 record), then bump.reset() timed separately — the O(1)
//                 wholesale reset lane rows amortize to
//   amort16z    — the same on a fresh Bump with a 16-byte zero-value
//                 allocation (bumpalo alloc(val) writes the initializer:
//                 the zeroing twin)
//   fresh16     — one persistent Bump: alloc + reset per iteration (the
//                 region-per-iteration analog)
//   fresh16new  — Bump::new + alloc + drop per iteration (arena-per-region
//                 via the Rust global allocator; chunk malloc/free per iter)
//   fresh4k     — alloc+reset per iteration at 4096 B
//   freshlcg    — sizes 16..63 from the fixed LCG, alloc+reset per iteration
//
// Raw rows write NO payload bytes (alloc_layout hands out MaybeUninit — the
// same guarantee SNlang's post-DIFF-6 release bump gives); the "z" rows write
// every byte. opt-level=2 (parity with clang -O2 / snc -O2), LTO off,
// single-threaded.

use bumpalo::Bump;
use std::alloc::Layout;
use std::time::Instant;

fn report(row: &str, alloc_ns: i64, read_ns: i64, reset_ns: i64, ops: i64, ck: i64) {
    println!(
        "BENCH row={} alloc_ns={} read_ns={} reset_ns={} ops={} checksum={}",
        row, alloc_ns, read_ns, reset_ns, ops, ck
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = if args.len() > 1 {
        args[1].parse().unwrap_or(10_000_000)
    } else {
        10_000_000
    };
    let n = if n > 0 { n } else { 10_000_000 };

    let mut scratch: Vec<usize> = vec![0usize; n];

    // warmup: n/100 allocs + reset on one bump (icache + branch state)
    {
        let mut b = Bump::new();
        for _ in 0..n / 100 {
            let _ = b.alloc_layout(Layout::from_size_align(16, 8).unwrap());
        }
        b.reset();
    }

    // ---- timer overhead
    let t0 = Instant::now();
    let mut acct: u128 = 0;
    for _ in 0..1_000_000u32 {
        let now = Instant::now();
        acct = acct.wrapping_add(now.duration_since(t0).as_nanos());
    }
    let d = t0.elapsed().as_nanos() as i64;
    report("timer", d / 1_000_000, 0, 0, 1_000_000, acct as i64);

    // ---- empty loop tax
    let t0 = Instant::now();
    let mut acc0: usize = 0;
    for _ in 0..n {
        acc0 = acc0.wrapping_add(1);
    }
    let d0 = t0.elapsed().as_nanos() as i64;
    report("emptyloop", d0 / n as i64, 0, 0, n as i64, acc0 as i64);

    // ---- amort16: raw bump, read pass, wholesale reset
    let mut bump = Bump::new();
    let lay = Layout::from_size_align(16, 8).unwrap();
    let t0 = Instant::now();
    let mut acc1: usize = 0;
    for i in 0..n {
        let p = bump.alloc_layout(lay).as_ptr() as usize;
        let v = p;
        acc1 = acc1.wrapping_add(v);
        scratch[i] = v;
    }
    let t1 = Instant::now();
    let mut sum1: usize = 0;
    for i in 0..n {
        sum1 = sum1.wrapping_add(unsafe { *(scratch[i] as *const u64) as usize });
    }
    let t2 = Instant::now();
    bump.reset();
    let t3 = Instant::now();
    report(
        "amort16",
        (t1 - t0).as_nanos() as i64 / n as i64,
        (t2 - t1).as_nanos() as i64 / n as i64,
        (t3 - t2).as_nanos() as i64,
        n as i64,
        acc1.wrapping_add(sum1) as i64,
    );

    // ---- amort16z: zeroing twin on a fresh bump (alloc writes 16 zero bytes)
    let mut bumpz = Bump::new();
    let t0 = Instant::now();
    let mut acc2: usize = 0;
    for i in 0..n {
        let p = bumpz.alloc([0u64; 2]).as_mut_ptr() as *mut u8;
        let v = p as usize;
        acc2 = acc2.wrapping_add(v);
        scratch[i] = v;
    }
    let t1 = Instant::now();
    let mut sum2: usize = 0;
    for i in 0..n {
        sum2 = sum2.wrapping_add(unsafe { *(scratch[i] as *const u64) as usize });
    }
    let t2 = Instant::now();
    bumpz.reset();
    let t3 = Instant::now();
    report(
        "amort16z",
        (t1 - t0).as_nanos() as i64 / n as i64,
        (t2 - t1).as_nanos() as i64 / n as i64,
        (t3 - t2).as_nanos() as i64,
        n as i64,
        acc2.wrapping_add(sum2) as i64,
    );

    // ---- fresh16: alloc + reset per iteration on one persistent bump
    let mut bumpf = Bump::new();
    let t0 = Instant::now();
    let mut acc3: usize = 0;
    for _ in 0..n {
        let p = bumpf.alloc_layout(lay).as_ptr() as usize;
        acc3 = acc3.wrapping_add(p);
        bumpf.reset();
    }
    let t1 = Instant::now();
    report(
        "fresh16",
        (t1 - t0).as_nanos() as i64 / n as i64,
        0,
        0,
        n as i64,
        acc3 as i64,
    );

    // ---- fresh16new: Bump::new + alloc + drop per iteration
    let t0 = Instant::now();
    let mut acc4: usize = 0;
    for _ in 0..n {
        let mut b = Bump::new();
        let p = b.alloc_layout(lay).as_ptr() as usize;
        acc4 = acc4.wrapping_add(p);
        drop(b);
    }
    let t1 = Instant::now();
    report(
        "fresh16new",
        (t1 - t0).as_nanos() as i64 / n as i64,
        0,
        0,
        n as i64,
        acc4 as i64,
    );

    // ---- fresh4k
    let lay4k = Layout::from_size_align(4096, 8).unwrap();
    let mut bump4k = Bump::new();
    let t0 = Instant::now();
    let mut acc5: usize = 0;
    for _ in 0..n {
        let p = bump4k.alloc_layout(lay4k).as_ptr() as usize;
        acc5 = acc5.wrapping_add(p);
        bump4k.reset();
    }
    let t1 = Instant::now();
    report(
        "fresh4k",
        (t1 - t0).as_nanos() as i64 / n as i64,
        0,
        0,
        n as i64,
        acc5 as i64,
    );

    // ---- freshlcg (identical constants to the twins; draw BEFORE advance)
    let mut st: u32 = 12345;
    let mut bumpl = Bump::new();
    let t0 = Instant::now();
    let mut acc6: usize = 0;
    for _ in 0..n {
        let sz = 16 + (st % 48) as usize;
        let p = bumpl
            .alloc_layout(Layout::from_size_align(sz, 8).unwrap())
            .as_ptr() as usize;
        acc6 = acc6.wrapping_add(p);
        bumpl.reset();
        st = st.wrapping_mul(1103515245).wrapping_add(12345) & 0x7fffffff;
    }
    let t1 = Instant::now();
    report(
        "freshlcg",
        (t1 - t0).as_nanos() as i64 / n as i64,
        0,
        0,
        n as i64,
        acc6 as i64,
    );
    println!("lcg_state_final={}", st);

    println!("RUST_BENCH_DONE");
}
