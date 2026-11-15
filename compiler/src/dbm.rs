//! SWARM RABBIT HOLE — Wave 15 (Impl-T, Cohort Delta): Salt ε's DBM
//! zone layer — the contract engine's arithmetic backbone, rung
//! (i)/(ii) of A4's Presburger decidable ladder.
//!
//! A DBM (difference-bound matrix) over vertices x_0..x_{n-1} holds
//! the tightest known bound d[i][j] on x_i − x_j ≤ d[i][j]; vertex 0
//! is the zero constant, so interval facts (x ≤ c) are zero-var
//! difference edges — the whole table is one homogeneous system:
//!   x ≤ c        ⟺  d[x][0] ≤ c
//!   x ≥ c        ⟺  d[0][x] ≤ −c
//!   x − y ≤ c    ⟺  d[x][y] ≤ c
//!
//! The canonical closed form is all-pairs shortest paths —
//! Floyd–Warshall, O(V³) (Floyd, CACM 5(6):345, 1962; Dill, CAV'89
//! LNCS 407; Bengtsson–Yi, LNCS 3098). Emptiness ⟺ a negative
//! diagonal cycle (Bellman 1958): the zone {x : x_0 = 0} is empty
//! exactly when the closed matrix says x_i − x_i < 0 for some i.
//!
//! THE HONEST SEMILATTICE (A4 C4.1's correction, verified): ∩ is
//! EXACT but requires re-closure for emptiness detection (Miné, HOSC
//! 19(1) Thm 4.1); ∪ is NOT closed — zones are convex, a union of two
//! zones is generally not a zone — so the lattice join is
//! OVER-APPROXIMATING ONLY (Miné Thm 4.2). The join here computes the
//! pointwise max of two CLOSED matrices (closed ⊔ closed is closed:
//! if c₁ ≤ a₁+b₁ and c₂ ≤ a₂+b₂ then max(c₁,c₂) ≤ max(a₁,a₂)+max(b₁,b₂)
//! — the max of the sums covers both), and the result contains both
//! operands and then some. Exact unions need DBM lists/federations —
//! named, not claimed (the slice-1 boundary). Callers that need a
//! branch join (the ensures exit zone over multiple returns) get the
//! over-approximation and pay for it in discharged atoms, which is the
//! honest direction: sound, not complete.
//!
//! len()/capacity()-style opaque symbols enter as ordinary vertices
//! with DBM-seeded relations (A4 §2's expressiveness map: "any rung,
//! as graph vertices with DBM-seeded relations") — the classic
//! index-vs-len bounds discharge is the closure deriving
//! d[i][len] ≤ d[i][0] + d[0][len] from the two interval edges.
//!
//! The whole engine lives here, compile-time only — the runtime never
//! sees a DBM; it only sees the degraded assertions the discharge walk
//! could not prove (the final report §7.3: the runtime carries the
//! boundary, not the solver).

/// No-bound sentinel: large enough that no honest edge reaches it,
/// small enough that sentinel + sentinel + sentinel cannot overflow
/// i64 (the closure adds two cells: INF + INF must stay "infinite").
pub const INF: i64 = i64::MAX / 8;

/// A difference-bound matrix over n vertices (vertex 0 = zero).
/// d[i*n + j] is the tightest bound on x_i − x_j.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dbm {
    pub n: usize,
    pub d: Vec<i64>,
}

impl Dbm {
    /// The trivially-full zone over n vertices: x_i − x_i ≤ 0, no
    /// other bounds (every assignment satisfies it).
    pub fn top(n: usize) -> Self {
        let mut d = vec![INF; n * n];
        for i in 0..n {
            d[i * n + i] = 0;
        }
        Dbm { n, d }
    }

    pub fn idx(&self, i: usize, j: usize) -> usize {
        i * self.n + j
    }

    /// Tighten x_i − x_j ≤ c (no re-closure: batched seeding; the
    /// caller closes once — the O(V³) floor is paid per closure, not
    /// per edge).
    pub fn seed(&mut self, i: usize, j: usize, c: i64) {
        let k = self.idx(i, j);
        if c < self.d[k] {
            self.d[k] = c;
        }
    }

    /// All-pairs closure in place: the canonical closed form
    /// (Floyd–Warshall — Floyd CACM 1962; Dill CAV'89; Bengtsson–Yi).
    /// After this, d[i][j] is the tightest bound IMPLIED by the seeded
    /// system, and d[i][i] < 0 anywhere means the system is empty.
    /// A detour leg at INF (no known bound) is SKIPPED — relaxing
    /// through it would store finite+INF as a "bound" the seeded
    /// system does not imply (the shortest-path discipline: only
    /// finite legs carry).
    pub fn close(&mut self) {
        let n = self.n;
        for k in 0..n {
            // The detour row is snapshotted first: row k is read while
            // every row is written (the classic in-place FW hazard).
            let dk: Vec<i64> = self.d[k * n..(k + 1) * n].to_vec();
            for i in 0..n {
                let dik = self.d[i * n + k];
                if dik >= INF {
                    continue;
                }
                for j in 0..n {
                    if dk[j] >= INF {
                        continue;
                    }
                    let via = dik.saturating_add(dk[j]);
                    let cell = i * n + j;
                    if via < self.d[cell] {
                        self.d[cell] = via;
                    }
                }
            }
        }
        // Saturate any residual infinities so joins/intersects stay
        // in-range.
        for v in self.d.iter_mut() {
            if *v > INF {
                *v = INF;
            }
        }
    }

    /// Emptiness ⟺ a negative diagonal (Bellman 1958: the shortest
    /// x_i → x_i cycle went negative). Only meaningful on a CLOSED
    /// matrix (Miné Thm 4.1: an unclosed matrix can hide the cycle).
    pub fn is_empty(&self) -> bool {
        (0..self.n).any(|i| self.d[self.idx(i, i)] < 0)
    }

    /// Does the closed zone imply x_i − x_j ≤ c? (The discharge test:
    /// an atom normalizing to this edge is proven when the closed
    /// bound is at least as tight.)
    pub fn implies(&self, i: usize, j: usize, c: i64) -> bool {
        self.d[self.idx(i, j)] <= c
    }

    /// The tightened zone (conjoin one edge), re-closed — Miné Thm
    /// 4.1's discipline: tightening can create a negative cycle, and
    /// only the re-closure exposes it. Used by the refutation test
    /// (atom ∧ facts ⟹ ⊥).
    pub fn conjoined(&self, i: usize, j: usize, c: i64) -> Dbm {
        let mut z = self.clone();
        z.seed(i, j, c);
        z.close();
        z
    }

    /// Intersection: the edgewise min, re-closed (Miné Thm 4.1 — ∩ is
    /// EXACT: a point satisfies both systems iff it satisfies the
    /// min system; the re-closure is required for emptiness
    /// detection, not for soundness of the bounds).
    pub fn intersect(&self, other: &Dbm) -> Dbm {
        let mut z = Dbm {
            n: self.n,
            d: self
                .d
                .iter()
                .zip(other.d.iter())
                .map(|(a, b)| (*a).min(*b))
                .collect(),
        };
        z.close();
        z
    }

    /// THE OVER-APPROXIMATING LATTICE JOIN (Miné HOSC 2006 Thm 4.2).
    /// Zones are convex: zone(A) ∪ zone(B) is generally NOT a zone, so
    /// this computes the smallest ZONE containing both — pointwise max
    /// over the two CLOSED matrices (closed ⊔ closed is closed: for
    /// every detour k, max(c₁,c₂) ≤ max(a₁,a₂) + max(b₁,b₂) covers
    /// both operands' triangle inequalities). Sound for merging
    /// branch/return-site zones; NOT exact — the systems test pins a
    /// witness (x = 0 ⊔ x = 2 contains x = 1). Exact unions need DBM
    /// federations (named, not claimed).
    pub fn join(&self, other: &Dbm) -> Dbm {
        debug_assert_eq!(self.n, other.n);
        let mut d = Vec::with_capacity(self.d.len());
        for (a, b) in self.d.iter().zip(other.d.iter()) {
            d.push((*a).max(*b));
        }
        // Pointwise max of closed matrices is closed; a re-close is a
        // no-op (cheap insurance against a caller mutating mid-join —
        // and the is_empty discipline requires closure anyway).
        let mut z = Dbm { n: self.n, d };
        z.close();
        z
    }

    /// The closed upper bound on x_i (interval readout), or None when
    /// unbounded. Used by the facts table (the mod range axiom needs
    /// the modulus vertex's interval).
    pub fn ub(&self, i: usize) -> Option<i64> {
        let v = self.d[self.idx(i, 0)];
        if v >= INF { None } else { Some(v) }
    }

    /// The closed lower bound on x_i.
    pub fn lb(&self, i: usize) -> Option<i64> {
        let v = self.d[self.idx(0, i)];
        if v >= INF { None } else { Some(-v) }
    }
}

/// SWARM RABBIT HOLE — Wave 15 (Impl-T): the MEMOIZED fast path
/// (§7.3: "DBM/zone closure as a memoized fast path"). The facts
/// table discharges simple atoms by LOOKUP, and repeated discharge
/// queries with the same fact signature reuse the closed zone
/// instead of re-running the O(V³) closure — the closure is the
/// floor, not the budget (§7.3's pipeline honesty), and the memo is
/// what keeps the floor from being paid per atom per call site.
#[derive(Default)]
pub struct ZoneMemo {
    cache: std::collections::HashMap<Vec<i64>, Dbm>,
    pub hits: u64,
    pub misses: u64,
}

impl ZoneMemo {
    /// Close the seeded zone, reusing a prior closure of an
    /// identical fact signature when one exists. The key is the
    /// seeded matrix itself (identical seed sets ⟹ identical
    /// closures — the closure is a pure function of the matrix).
    pub fn close(&mut self, mut z: Dbm) -> Dbm {
        if let Some(hit) = self.cache.get(&z.d) {
            self.hits += 1;
            return hit.clone();
        }
        self.misses += 1;
        z.close();
        self.cache.insert(z.d.clone(), z.clone());
        z
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic LCG (the wave's seeded-random discipline) so
    /// the randomized-zone tests are reproducible.
    fn lcg(state: &mut u64) -> u64 {
        *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *state >> 33
    }

    #[test]
    fn dbm_closure_matches_bruteforce_relaxation() {
        // Floyd–Warshall closure vs the naive relax-to-fixpoint
        // reference over randomized zones (the closure fixpoint
        // property and the brute-force agreement). A negative cycle
        // makes the naive relaxation diverge — the reference guards
        // with the same emptiness criterion (a negative diagonal
        // within n passes, the Bellman–Ford bound).
        let mut state = 0xC0FFEEu64;
        for _ in 0..40 {
            let n = 4 + (lcg(&mut state) % 3) as usize; // 4..6 vertices
            let mut z = Dbm::top(n);
            for _ in 0..(n * 2) {
                let i = (lcg(&mut state) as usize) % n;
                let j = (lcg(&mut state) as usize) % n;
                if i == j {
                    continue;
                }
                let c = (lcg(&mut state) % 21) as i64 - 10; // [-10, 10]
                z.seed(i, j, c);
            }
            let mut fw = z.clone();
            fw.close();
            // Brute force: repeat full-matrix relaxations until no
            // cell changes (INF legs skipped — the same discipline as
            // the FW closure: only finite legs carry), stopping early
            // on a negative diagonal (the emptiness witness).
            let mut bf = z.clone();
            let mut bf_empty = false;
            loop {
                let mut changed = false;
                for k in 0..n {
                    for i in 0..n {
                        if bf.d[bf.idx(i, k)] >= INF {
                            continue;
                        }
                        for j in 0..n {
                            if bf.d[bf.idx(k, j)] >= INF {
                                continue;
                            }
                            let via = bf.d[bf.idx(i, k)].saturating_add(bf.d[bf.idx(k, j)]);
                            let cell = bf.idx(i, j);
                            if via < bf.d[cell] {
                                bf.d[cell] = via;
                                changed = true;
                            }
                        }
                    }
                }
                if (0..n).any(|i| bf.d[bf.idx(i, i)] < 0) {
                    bf_empty = true;
                    break;
                }
                if !changed {
                    break;
                }
            }
            if fw.is_empty() || bf_empty {
                assert!(
                    fw.is_empty() && bf_empty,
                    "emptiness must agree (FW negative diagonal == brute-force negative cycle)"
                );
                continue;
            }
            assert_eq!(fw.d, bf.d, "closure must equal the fixpoint relaxation");
            // The closure fixpoint property: closed[i][j] ≤
            // closed[i][k] + closed[k][j] ∀k — vacuous when either
            // leg is INF (no path = no triangle inequality through
            // that leg).
            for i in 0..n {
                for j in 0..n {
                    for k in 0..n {
                        if fw.d[fw.idx(i, k)] >= INF || fw.d[fw.idx(k, j)] >= INF {
                            continue;
                        }
                        assert!(
                            fw.d[fw.idx(i, j)]
                                <= fw.d[fw.idx(i, k)].saturating_add(fw.d[fw.idx(k, j)]),
                            "closure must be a fixpoint"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn dbm_emptiness_is_negative_diagonal() {
        // Empty by construction: x − y ≤ −5 ∧ y − x ≤ −5 (the classic
        // negative cycle), and non-empty zones must expose a witness:
        // x_i := d[i][0] satisfies every closed edge (d[i][0] −
        // d[j][0] ≤ d[i][j] is exactly the closure inequality).
        let mut z = Dbm::top(3);
        z.seed(1, 2, -5);
        z.seed(2, 1, -5);
        z.close();
        assert!(z.is_empty(), "x−y≤−5 ∧ y−x≤−5 is empty");
        let mut ok = Dbm::top(3);
        ok.seed(1, 2, 7);
        ok.seed(2, 1, 3);
        ok.seed(0, 1, 4);
        ok.seed(1, 0, 9); // x ≤ 9 — fully bounded: the witness is constructive
        ok.close();
        assert!(!ok.is_empty(), "the consistent zone is non-empty");
        let w: Vec<i64> = (0..3).map(|i| ok.d[ok.idx(i, 0)]).collect();
        for i in 0..3 {
            assert!(w[i] < INF, "the witness zone is fully bounded");
        }
        for i in 0..3 {
            for j in 0..3 {
                if ok.d[ok.idx(i, j)] < INF {
                    assert!(
                        w[i] - w[j] <= ok.d[ok.idx(i, j)],
                        "the witness must satisfy every bound"
                    );
                }
            }
        }
        // A tightening that creates a cycle is exposed ONLY after
        // re-closure (Miné Thm 4.1's discipline).
        let mut t = Dbm::top(2);
        t.seed(1, 0, 10); // x ≤ 10
        t.close();
        assert!(!t.is_empty());
        let tighter = t.conjoined(0, 1, -11); // x ≥ 11
        assert!(tighter.is_empty(), "10 ∧ ≥11 closes to a negative diagonal");
    }

    #[test]
    fn dbm_intersect_exact_join_overapproximates() {
        // ∩ EXACT: the point set of close(min(A,B)) equals zone(A) ∩
        // zone(B), verified by finite enumeration (V=3, box [−4..4]²):
        // a point is in the closed zone iff it satisfies EVERY finite
        // edge of the closed matrix (the whole closed system — not a
        // hand-picked subset).
        let mut a = Dbm::top(3);
        a.seed(1, 0, 2); // x ≤ 2
        a.seed(0, 2, 1); // y ≥ −1
        let mut b = Dbm::top(3);
        b.seed(1, 0, 0); // x ≤ 0
        b.seed(1, 2, -1); // x − y ≤ −1 (x < y)
        let c = a.intersect(&b);
        let in_zone = |z: &Dbm, x: i64, y: i64| {
            for i in 0..3 {
                for j in 0..3 {
                    let bound = z.d[z.idx(i, j)];
                    if bound >= INF {
                        continue;
                    }
                    let vi = [0i64, x, y][i];
                    let vj = [0i64, x, y][j];
                    if vi - vj > bound {
                        return false;
                    }
                }
            }
            true
        };
        let mut exact = true;
        for x in -4..=4 {
            for y in -4..=4 {
                let in_a = in_zone(&a, x, y);
                let in_b = in_zone(&b, x, y);
                let in_c = in_zone(&c, x, y);
                if (in_a && in_b) != in_c {
                    exact = false;
                }
            }
        }
        assert!(exact, "∩ must be exact (enumerated)");

        // JOIN over-approximates both operands and is strictly larger
        // than the union (zones are convex — Miné Thm 4.2): x=0 ⊔ x=2
        // contains x=1, which is in neither operand.
        let mut z0 = Dbm::top(2);
        z0.seed(1, 0, 0);
        z0.seed(0, 1, 0);
        let mut z2 = Dbm::top(2);
        z2.seed(1, 0, 2);
        z2.seed(0, 1, -2);
        let j = z0.join(&z2);
        assert!(!j.is_empty());
        // x = 1 satisfies the join (0 ≤ 1 ≤ 2) but is in neither zone.
        let x = 1i64;
        assert!(x <= j.d[j.idx(1, 0)] && -x <= j.d[j.idx(0, 1)]);
        assert!(!(x <= 0) && !(x >= 2), "1 is in neither operand");
    }

    #[test]
    fn zone_memo_hits_the_closed_zone() {
        let mut memo = ZoneMemo::default();
        let mut z = Dbm::top(3);
        z.seed(1, 0, 5);
        let closed = memo.close(z.clone());
        assert_eq!(memo.misses, 1);
        let again = memo.close(z);
        assert_eq!(memo.hits, 1);
        assert_eq!(closed.d, again.d);
        assert!(again.implies(1, 0, 5));
    }
}
