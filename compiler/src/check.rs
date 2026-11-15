use crate::ast::*;
use crate::diag::Diagnostics;
use crate::span::Span;
use crate::types::*;
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct EnumVariantInfo {
    pub name: String,
    pub fields: Vec<(String, Type)>,
}

/// SWARM RABBIT HOLE — Wave 6 (Impl-E), W2A §1.1 / DIFF-2: RegionArena id
/// (D0 gap #4, first slice). 0 is the function-outer region; ids grow
/// ⊑ₙₑₛₜ-downward with nesting (an inner region always has a strictly
/// larger id than its parent). Numbering restarts per function activation
/// — regions are function-lexical (ρ_s); cross-function facts travel in
/// FnBorrowSummary, not in RgnIds.
pub type RgnId = u32;

/// SWARM RABBIT HOLE — Wave 6 (Impl-E), DIFF-1 (STRATA lane classes):
/// allocation-site lane class of a local. Wave 8 (Impl-I): the pinned
/// class is now reachable two ways — an explicit `lane_alloc` naming a
/// lane in L2..L6 (the user-declared pin, the "declare it 'pinned'
/// (allocate on L2..L6)" discharge of E0802), or the coloring pass's
/// pre-coloring of a lane-free growable local that flows `inout` into
/// an scc_accum param (recorded in CheckDb.lane_assignment, consumed by
/// the ir.rs lowering).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneClass {
    /// L0/L1 frame-local ping-pong scratch.
    ScratchPingPong,
    /// L2..L6 pinned accumulators (scc_accum_params pre-coloring).
    PinnedAccumulator,
    /// L7 caller-sret DPS-out.
    CallerSretOut,
}

/// SWARM RABBIT HOLE — Wave 6 (Impl-E), DIFF-1: allocation-site lane
/// assignment, recorded at declaration of a lane-backed local.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneAssignment {
    pub lane: u8,
    pub class: LaneClass,
}

/// SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the mutability class
/// of a global pool (kills G1 §1 A6c — the global-pool freelist race).
/// Before DIFF-4 nothing distinguished a global pool from a local one:
/// `Type::Handle` passed freely through `check_spawn_expr` (which
/// matched only Ref/Mut/Ptr/Blueprint), so a global-pool handle
/// crossed into an unscoped spawn → two threads mutating one freelist
/// → freelist corruption → memory unsafety. Three declared modes,
/// frozen by default:
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlobalPoolMode {
    /// DEFAULT (no annotation). Read-only from every thread:
    /// insert/set/remove rejected everywhere outside an
    /// `unfrozen (P) { ... }` block (E0803.3). Handles into a frozen
    /// pool are Sendable — zero-copy shared reads: a frozen pool has
    /// no legal write path, so no thread can corrupt or race the
    /// freelist (stable generations).
    Frozen,
    /// `pool<T> thread_local`. Mutations allowed, on the declaring
    /// thread only. Handles carry a home-thread tag; any spawn/chan
    /// capture or cross-thread use = E0803.1.
    ThreadConfined,
    /// `pool<T> synchronized`. Mutex-protected vault: every
    /// insert/set/remove funnels through a runtime call taking the
    /// pool's lock; handles Sendable everywhere. No two threads can
    /// ever interleave insert/remove on one pool — the
    /// data-race-freedom condition for the vault.
    Synchronized,
}

/// DIFF-4 (Impl-O): the lookup result of `pool_mode_of` — a global
/// pool's declared mode, a local pool (single-thread by construction),
/// or not a pool at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PoolModeCheck {
    Global(GlobalPoolMode),
    Local,
    NotAPool,
}

/// SWARM RABBIT HOLE — Wave 6 (Impl-E), W2A §1.3 (v3) / DIFF-1: the
/// interprocedural inout summary. The three v2 fields (STRATA §2.2.3)
/// are carried for spec conformance but are not computed by the checker
/// yet; `scc_accum_params` is computed (by `scc_accum_fixpoint`) — it is
/// the growth axis the shrink-only summary missed (kills G1 A5a).
#[derive(Clone, Debug, Default)]
pub struct FnBorrowSummary {
    /// Params flowing into the return value (spec-carried; uncomputed).
    pub return_roots_from_params: Vec<usize>,
    /// (dst_param, src_param) store edges (spec-carried; uncomputed).
    pub param_store_edges: Vec<(usize, usize)>,
    /// Inout params shrunk by this function (spec-carried; uncomputed —
    /// the local case is already covered by list_epochs/
    /// check_borrow_epoch). W2A §1.3 requires shrinks_params ⊆
    /// scc_accum_params once this field is computed.
    pub shrinks_params: Vec<usize>,
    /// DIFF-1 Rule 1a / W2A §1.3 S6: inout params GROWN
    /// (push/push_ref/insert) by this function or by any deeper re-entry
    /// of this function's recursion SCC while an activation of this
    /// function is suspended at an SCC call edge. Orthogonal to
    /// shrinks_params — the push-only accumulator appears here and
    /// nowhere in shrinks_params; that is exactly the hole G1 A5a drove
    /// through.
    pub scc_accum_params: Vec<usize>,
    /// W2A §1.3 S6 (the v3 summary contract) / DIFF-1 Rule 1b: this
    /// function's pre-coloring assignment onto a pinned lane L2..L6 —
    /// the lowest lane the coloring pass (Impl-I, Wave 8) assigned to
    /// this function's scc_accum_params; None when the function hosts
    /// no colored accumulator or the L2..L6 budget was exhausted
    /// (Rule 1d → E0802c at the recorded call sites). The authoritative
    /// per-(function, param) coloring lives in the fixpoint's
    /// union-find/lane maps; this field is the summary-level projection.
    pub pinned_lane_req: Option<u8>,
}

/// SWARM RABBIT HOLE — Wave 6 (Impl-E), DIFF-1 Rule 1a steps 2/3: one
/// inout-argument edge — the caller passes its own inout param
/// `caller_param` into `callee`'s param `callee_param` (indices in the
/// callee sig's inout_params space). Consumed by `scc_accum_fixpoint`.
#[derive(Clone, Debug)]
pub struct InoutEdge {
    pub caller: String,
    pub caller_param: Option<usize>,
    pub callee: String,
    pub callee_param: usize,
}

/// SWARM RABBIT HOLE — Wave 6 (Impl-E), DIFF-1 Rule 1b: a deferred
/// allocation-site check — a local was passed `inout` into `callee`'s
/// param `callee_param`. If the fixpoint marks that param ∈
/// scc_accum_params(callee), the coloring pass (Impl-I, Wave 8) assigns
/// the accumulator a pinned lane L2..L6 (W2A §1.7 K1) and the site is
/// resolved: a lane-free local (heap-backed growable — the default) is
/// PRE-COLORED onto that lane (no error — the discharge that makes
/// E0802 conditional); an explicit `lane_alloc` scratch pin still fires
/// E0802; a lane-less accumulator (the L2..L6 budget exhausted) fires
/// E0802c (Rule 1d). `lane` is None for locals with no explicit lane
/// assignment — the pre-coloring target.
#[derive(Clone, Debug)]
pub struct SccAccumSite {
    pub callee: String,
    pub callee_param: usize,
    /// Wave 8 (Impl-I): the caller's summary key — the function whose
    /// frame holds the pre-colored local (CheckDb.lane_assignment is
    /// keyed by it).
    pub caller: String,
    pub local: String,
    pub lane: Option<LaneAssignment>,
    /// Wave 8 (Impl-I), DIFF-1 Rule 1b: the pinned lane the coloring
    /// pass assigned to this site's accumulator (filled after the
    /// fixpoint); None when the callee param is not an SCC accumulator
    /// or the L2..L6 budget was exhausted (Rule 1d → E0802c).
    pub pinned_lane: Option<u8>,
    pub span: Span,
}

pub struct CheckDb {
    pub funcs: HashMap<String, FuncSig>,
    pub blueprints: HashMap<String, BlueprintInfo>,
    pub blueprint_templates: HashMap<String, BlueprintItem>,
    pub records: HashMap<String, RecordInfo>,
    pub enums: HashMap<String, Vec<String>>,
    pub enum_variants: HashMap<String, Vec<EnumVariantInfo>>,
    pub extensions: HashMap<String, Vec<String>>,
    pub contracts: HashMap<String, ContractInfo>,
    pub expr_ty: HashMap<(u32, u32, u32), Type>,
    pub globals: HashMap<String, Type>,
    /// SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the declared mode
    /// of every GLOBAL pool (kills G1 §1 A6c — the global-pool freelist
    /// race). Recorded at collect time from the declaration's parsed
    /// annotation: `pool<T>` bare = Frozen, `pool<T> thread_local` =
    /// ThreadConfined, `pool<T> synchronized` = Synchronized. Local
    /// pools are absent (single-thread by construction — they cannot
    /// even be named from another thread once capture is checked).
    pub global_pools: HashMap<String, GlobalPoolMode>,
    pub method_slots: HashMap<String, usize>,
    pub vtable_methods: Vec<String>,
    /// Libraries referenced by `extern` blocks, passed to the linker.
    pub extern_libs: Vec<String>,
    /// Per-type-parameter variance for each monomorphic blueprint instance,
    /// keyed by monomorphic name (e.g. "Box_Dog").
    pub mono_variance: HashMap<String, Vec<(String, crate::ast::Variance)>>,
    pub instantiated: HashMap<String, BlueprintItem>,
    pub fn_templates: HashMap<String, FnItem>,
    pub instantiated_funcs: HashMap<String, FnItem>,
    pub resolved_calls: HashMap<(u32, u32, u32), String>,
    /// SWARM RABBIT HOLE — Wave 6 (Impl-E, D0 gap #4 first slice /
    /// DIFF-1): module-wide interprocedural state for the region poset's
    /// scc_accum_params fixpoint (W2A §1.3). Recorded during checking;
    /// `scc_accum_fixpoint` runs once, after all functions are checked.
    /// Per-function borrow summaries keyed by the db.funcs key
    /// ("{blueprint}::{method}" for methods, bare name otherwise).
    pub fn_summaries: HashMap<String, FnBorrowSummary>,
    /// pushes(f, i) seeds (DIFF-1 Rule 1a step 1): inout params this
    /// function grows directly.
    pub fn_growth_seeds: HashMap<String, BTreeSet<usize>>,
    /// The module call graph (Tarjan input): caller key -> callee keys.
    pub call_edges: HashMap<String, BTreeSet<String>>,
    /// Rule 1a steps 2/3 propagation edges.
    pub inout_call_edges: Vec<InoutEdge>,
    /// Rule 1b deferred allocation-site checks (E0802 candidates).
    pub scc_accum_sites: Vec<SccAccumSite>,
    /// SWARM RABBIT HOLE — Wave 8 (Impl-I), DIFF-1 Rule 1b / W2A §1.3:
    /// the pre-coloring feed consumed by the IR (ir.rs) — function
    /// summary key → local/param name → pinned lane L2..L6. Populated
    /// by the coloring pass in `scc_accum_fixpoint` after all functions
    /// are checked: (a) each colored SCC accumulator inout param's
    /// provenance gets its pinned lane (the default scratch 0
    /// overridden), and (b) each caller local passed `inout` into a
    /// colored accumulator is pre-colored onto that lane — the ir.rs
    /// declaration lowering then constructs the growable ON the lane
    /// (runtime sn_list_new_lane → sn_lane_alloc(π_j)).
    pub lane_assignment: HashMap<String, HashMap<String, u8>>,
    /// SWARM RABBIT HOLE — Wave 13 (Impl-R, Cohort Delta): the wave
    /// region's module-wide tables (W2B §3/§4 — Thm WS made
    /// executable). Recorded at collect; consulted by the checker's
    /// E0812/E-R1 arms and the ir.rs wave lowering.
    /// (1) The seam cells: module-scope `ptr NAME = lcell_new(k)`
    /// declarations → carrier k (W2B §4.1: "seam cells = module-scope
    /// cells visible under the mode rules"). Constructed at main entry
    /// like DIFF-4's global pools (ir.rs), so the module initializer
    /// never runs as code.
    pub lcell_globals: HashMap<String, i64>,
    /// (2) Every wave block's claim on a seam cell, program-wide
    /// (cell → (fn_summary_key, wave span)), from the collect-time
    /// syntactic pre-walk — the order-independent table the R1
    /// quiescence rules consult (a merge or capture can be checked
    /// before the other side of its ordering claim is checked).
    pub wave_claims: HashMap<String, Vec<(String, Span)>>,
    /// (3) The checked capture set per wave block (wave span → the
    /// sorted seam-cell names), recorded during checking — the ir.rs
    /// lowering emits one sn_wave_capture per name at wave entry.
    pub wave_captures: HashMap<(u32, u32, u32), Vec<String>>,
    /// (4) The wave-purity fixpoint (fn_summary_key → pure-for-waves):
    /// a function is pure-for-waves iff its body (transitively — the
    /// call-graph fixpoint) touches none of the E0812 ban-list
    /// operations. The gate is interprocedural or it is not a gate:
    /// a wall-clock read one call away would inject the schedule
    /// into 𝒰_W undetected (Lemma WD's premise, E0812's purpose).
    pub wave_pure_fns: HashSet<String>,
}

impl CheckDb {
    pub fn is_algebraic_enum(&self, name: &str) -> bool {
        self.enum_variants.get(name).map_or(false, |vars| {
            vars.iter().any(|v| !v.fields.is_empty())
        })
    }

    /* SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): the HasSelfLink
       predicate (kills G1 §1 A2 → S4) — does a BY-VALUE copy of a value
       of this type move bytes that encode a self-relative link delta?

       - link<T, u16>: TRUE — the 2-byte value IS the delta; any copy of
         it decodes against the copy's address.
       - record/blueprint/tuple: TRUE iff any field is by-value
         link-bearing — a nested record's bytes move with the outer
         copy, so the predicate recurses into nested RECORD fields;
         container-header fields (list/map/chan/pool/str) do NOT
         propagate: copying the header pointer does not move the
         elements, so their deltas stay put.
       - list<E>: propagates has_self_link(E) — the ELEMENT is what a
         push/pop/copy moves (the spec's "list<HasSelfLink> element
         types propagate it" clause); a whole-list alias copies only
         the header, so container copies of the list itself are the
         conservative-but-sound case the assignment site rejects.
       - everything else: FALSE. */
    pub fn has_self_link(&self, ty: &Type) -> bool {
        match ty {
            Type::Link(_) => true,
            Type::List(e) => self.has_self_link(e),
            Type::Record(n) => self
                .records
                .get(n)
                .map(|r| r.fields.iter().any(|f| self.field_moves_link_bytes(&f.ty)))
                .unwrap_or(false),
            Type::Blueprint(n) => self
                .blueprints
                .get(n)
                .map(|b| b.fields.iter().any(|f| self.field_moves_link_bytes(&f.ty)))
                .unwrap_or(false),
            Type::Tuple(ts) => ts.iter().any(|t| self.has_self_link(t)),
            _ => false,
        }
    }

    /// The record-field form of the predicate: a field's bytes move with
    /// a copy of the containing record iff the field is by-value
    /// link-bearing (a link field, or a nested record/tuple that is).
    /// Container-header fields (list/map/chan/pool/str) are
    /// pointer-semantic — their ELEMENTS do not move with the record.
    fn field_moves_link_bytes(&self, ty: &Type) -> bool {
        match ty {
            Type::Link(_) => true,
            Type::Record(_) | Type::Tuple(_) => self.has_self_link(ty),
            _ => false,
        }
    }

    /// DIFF-5 (Impl-P): the link-field layout of a record — (offset,
    /// name, target-type label) for every link<T, u16> field, in field
    /// order. Consumed by the checker's intrinsic arms and by the ir.rs
    /// lowering (the runtime re-encode/scan calls carry it).
    pub fn link_fields_of(&self, rec: &str) -> Vec<(i64, String, String)> {
        let Some(r) = self.records.get(rec) else {
            return Vec::new();
        };
        r.fields
            .iter()
            .filter(|f| matches!(f.ty, Type::Link(_)))
            .map(|f| {
                let target = match &f.ty {
                    Type::Link(t) => t.to_string(),
                    _ => String::new(),
                };
                (f.offset, f.name.clone(), target)
            })
            .collect()
    }
}

struct Checker<'a> {
    db: &'a mut CheckDb,
    diag: &'a mut Diagnostics,
    scopes: Vec<HashMap<String, (Type, bool)>>, // type, is_const
    moved: HashMap<String, Span>,
    channel_moved: HashSet<String>,
    freed: HashSet<String>,
    /// owner name -> (borrower name, exclusive)
    borrowed: HashMap<String, (String, bool)>,
    /// borrower name -> owner name
    borrow_owner: HashMap<String, String>,
    /// container name -> current shrink epoch
    list_epochs: HashMap<String, u32>,
    /// borrower name -> (container name, epoch at borrow creation)
    borrow_epoch: HashMap<String, (String, u32)>,
    /// locals of the function currently being checked, innermost last
    local_scopes: Vec<Vec<String>>,
    current_fn_ret: Type,
    in_loop: usize,
    current_bp: Option<String>,
    current_bp_hierarchy: Vec<String>,
    in_async: bool,
    in_static: bool,
    // --- SWARM RABBIT HOLE — Wave 6 (Impl-E): the region poset skeleton
    // (W2A §1.1; DIFF-1/DIFF-2; D0 gap #4 first slice). Bookkeeping only
    // — no checking semantics change in this slice. ---
    /// W2A §1.1: the region stack IS the ⊑ₙ order (the lexical nesting,
    /// innermost on top). Index 0 is the function-outer region
    /// (DIFF-2: RgnId 0); `rgn_exit` pops innermost-first — SR1's
    /// innermost-first is exactly this LIFO (verified: it runs in
    /// lockstep with the existing scope pop in push()/pop()).
    region_stack: Vec<RgnId>,
    /// Next fresh region id; ids grow ⊑ₙ-downward with nesting.
    rgn_next: RgnId,
    /// The poset's parent edges (D0 gap #4's node → parent map): recorded
    /// at region entry; read at exit to verify the ⊑ₙ chain (SR1).
    region_parent: HashMap<RgnId, RgnId>,
    /// DIFF-2 RootOrigin.region_instance, checker side: every borrow's
    /// provenance carries its region — parallel to `borrow_epoch`; the
    /// ir.rs side is a later slice.
    region_instance: HashMap<String, RgnId>,
    /// DIFF-1: allocation-site lane of a local — explicit `lane_alloc`
    /// declarations (class by the STRATA lane ranges, see LaneClass);
    /// lane-free growable locals get their lanes from the coloring pass's
    /// pre-coloring (CheckDb.lane_assignment), recorded after the
    /// fixpoint.
    lane_of: HashMap<String, LaneAssignment>,
    /// SWARM RABBIT HOLE — Wave 8 (Impl-L): the GC-heap half of the W2A
    /// §1.4 memory-category classification (D0 gap #6: lane bytes / pool
    /// slots / GC heap). A binding here is GC-managed: its declaration
    /// routed through the malloc'd GC-heap path — `new` → sn_obj_new,
    /// list/map literals → sn_list_new/sn_map_new, all gc_track'd
    /// (runtime.c). `lane_of` is the lane-backed half (explicit
    /// `lane_alloc`: untracked bytes, never gc_track'd). The collectors
    /// ignore each other's space — GC roots are globals+stacks+compiler-
    /// pushed values only, and lane rewinds never consult the GC heap —
    /// so a cross-category reference is a checked error (E-GC-IN-LANE).
    gc_managed: HashSet<String>,
    /// The checked function's summary key; None outside a function body
    /// (module-level statements, extension-method walks) so nothing is
    /// misattributed to a stale caller.
    current_fn_key: Option<String>,
    /// This function's own inout params: (name, sig-space index).
    current_inout_params: Vec<(String, usize)>,
    // --- SWARM RABBIT HOLE — Wave 10 (Impl-O): DIFF-4, the global-pool
    // discipline's checker state (kills G1 §1 A6c). The mode table is
    // CheckDb.global_pools (module-wide, recorded at collect); the
    // three fields below are per-walk lexical state, parallel to
    // list_epochs/borrow_epoch. ---
    /// DIFF-4: handle binding -> the pool that minted it (the
    /// provenance side table the tree lacked: the pool method calls
    /// produce handles, so the home pool is recorded at the binding
    /// site — `int h = gp.insert(v)` and its transitive copies
    /// `h2 = h1`). Untracked handles (parameters, fabricated ints)
    /// carry no mode check — the runtime generation check remains
    /// the use-after-free guard.
    handle_home: HashMap<String, String>,
    /// DIFF-4: the lexically open `unfrozen (P)` windows, innermost
    /// last — the mutable-window depth per pool (the capture-site
    /// half of E0803.2: a handle into P is Sendable exactly when P
    /// is not on this stack).
    unfrozen_stack: Vec<String>,
    /// DIFF-4: the lexically open `frozen (...)` blocks' name lists
    /// (the mode-conflict check: an `unfrozen (P)` window refuses to
    /// nest inside a `frozen (P)` block).
    frozen_names_stack: Vec<Vec<String>>,
    // --- SWARM RABBIT HOLE — Wave 13 (Impl-R): the wave region's
    // per-walk lexical state (W2B §3/§4; Thm WS's checked side
    // conditions, E0812/E-R1). The module-wide tables are CheckDb's
    // (lcell_globals/wave_claims/wave_captures/wave_pure_fns). ---
    /// The lexically open wave bodies (0 = fallback code). read_W /
    /// contrib are legal only at depth ≥ 1; the wave body's E0812
    /// walk and the seam-mode rules fire at the block, not per-use.
    wave_depth: u32,
    /// The lexically open spawn-family bodies (spawn { } / go { } /
    /// nursery { }). A wave inside a spawned task would fold J(W)
    /// into module-scope seam cells from a non-joining thread —
    /// concurrent with fallback reads — the torn-read class S2
    /// exists to kill; waves are fallback-tier statements.
    spawn_depth: u32,
    /// The J(W) completion credits: one frame per scope (push/pop
    /// aligned), each holding the seam cells whose capturing wave
    /// block CLOSED inside that scope. A read_J is fold-valued
    /// exactly where a credit is live (an enclosing frame holds it):
    /// credits die with the conditional scope that earned them (an
    /// if-branch wave does not prove the join on the fall-through
    /// path; a loop body's credit dies at the loop exit — zero
    /// iterations possible). This is R1's "after J(W)" arm made
    /// lexical: provable, or refused.
    joined_stack: Vec<Vec<String>>,
    /// SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the
    /// lexically open counted-for loop variables (the T5b shape
    /// detector's enclosing context). A counted for pushes its
    /// variable; a nested counted for whose body indexes a morton
    /// list as get(m, outer, inner) is the PROVABLE lexicographic
    /// pair — W-SFC-T5B fires there (a warning: the program is legal,
    /// the layout is wasted — A2 §3/T5(b)).
    lex_loop_vars: Vec<String>,
}

pub fn template_param_names(tp: &[crate::ast::TypeParam]) -> Vec<String> {
    tp.iter().map(|p| p.name.clone()).collect()
}

/// Does `ty` satisfy a generic bound? The bound may name a contract or a
/// blueprint; `T` with no bound is satisfied by anything.
fn satisfies_bound(
    db: &CheckDb,
    ty: &Type,
    bound: &str,
) -> bool {
    match ty {
        Type::Contract(c) => c == bound || db.contracts.contains_key(bound) == false && c == bound,
        Type::Blueprint(b) => {
            if b == bound {
                return true;
            }
            if let Some(bp) = db.blueprints.get(b) {
                if bp.contracts.iter().any(|c| c == bound) {
                    return true;
                }
            }
            // inherits the contract through its parent chain
            let mut cur = db.blueprints.get(b).and_then(|i| i.parent.clone());
            while let Some(name) = cur {
                if let Some(bp) = db.blueprints.get(&name) {
                    if bp.contracts.iter().any(|c| c == bound) {
                        return true;
                    }
                    cur = bp.parent.clone();
                } else {
                    break;
                }
            }
            blueprint_extends(&db.blueprints, b, bound)
        }
        _ => false,
    }
}

/// Validate `blueprint Foo<T: Bound>` type arguments before monomorphization.
fn check_type_arg_bounds(
    db: &CheckDb,
    template: &BlueprintItem,
    args: &[TypeAst],
    span: Span,
    diag: &mut Diagnostics,
) {
    for (i, tp) in template.type_params.iter().enumerate() {
        let Some(bound) = &tp.bound else { continue };
        let Some(arg) = args.get(i) else {
            diag.error(
                span,
                format!("type argument for '{}' is required (bound: {bound})", tp.name),
            );
            continue;
        };
        let aty = resolve_type_ast(arg, db);
        if matches!(aty, Type::Any) {
            continue; // inference placeholder from `[]`
        }
        if !satisfies_bound(db, &aty, bound) {
            diag.error(
                span,
                format!("type argument '{}' does not satisfy bound '{bound}' on '{}'", type_ast_label(arg), tp.name),
            );
        }
    }
}

pub fn check_programs(programs: &[Program], diag: &mut Diagnostics) -> CheckDb {
    let mut db = CheckDb {
        funcs: HashMap::new(),
        blueprints: HashMap::new(),
        blueprint_templates: HashMap::new(),
        records: HashMap::new(),
        enums: HashMap::new(),
        enum_variants: HashMap::new(),
        extensions: HashMap::new(),
        contracts: HashMap::new(),
        expr_ty: HashMap::new(),
        globals: HashMap::new(),
        global_pools: HashMap::new(),
        method_slots: HashMap::new(),
        vtable_methods: Vec::new(),
        extern_libs: Vec::new(),
        mono_variance: HashMap::new(),
        instantiated: HashMap::new(),
        fn_templates: HashMap::new(),
        instantiated_funcs: HashMap::new(),
        resolved_calls: HashMap::new(),
        fn_summaries: HashMap::new(),
        fn_growth_seeds: HashMap::new(),
        call_edges: HashMap::new(),
        inout_call_edges: Vec::new(),
        scc_accum_sites: Vec::new(),
        lane_assignment: HashMap::new(),
        lcell_globals: HashMap::new(),
        wave_claims: HashMap::new(),
        wave_captures: HashMap::new(),
        wave_pure_fns: HashSet::new(),
    };
    collect(&mut db, programs, diag);
    check_contracts(&mut db, diag);
    for p in programs {
        let mut c = Checker {
            db: &mut db,
            diag,
            scopes: vec![HashMap::new()],
            moved: HashMap::new(),
            channel_moved: HashSet::new(),
            freed: HashSet::new(),
            borrowed: HashMap::new(),
            borrow_owner: HashMap::new(),
            list_epochs: HashMap::new(),
            borrow_epoch: HashMap::new(),
            local_scopes: Vec::new(),
            current_fn_ret: Type::Void,
            in_loop: 0,
            current_bp: None,
            current_bp_hierarchy: vec![],
            in_async: false,
            in_static: false,
            region_stack: vec![0],
            rgn_next: 1,
            region_parent: HashMap::new(),
            region_instance: HashMap::new(),
            lane_of: HashMap::new(),
            gc_managed: HashSet::new(),
            current_fn_key: None,
            current_inout_params: Vec::new(),
            handle_home: HashMap::new(),
            unfrozen_stack: Vec::new(),
            frozen_names_stack: Vec::new(),
            wave_depth: 0,
            spawn_depth: 0,
            joined_stack: vec![Vec::new()],
            lex_loop_vars: Vec::new(),
        };
        for item in &p.items {
            match item {
                Item::Extern(x) => {
                    for f in &x.funcs {
                        let mut sig = fn_sig(f, None, &c.db);
                        // The C symbol is emitted under its own name.
                        sig.llvm = f.name.clone();
                        sig.is_extern = true;
                        if sig.params.iter().any(|p| p.2) {
                            c.diag.error(
                                f.span,
                                format!("extern '{}' cannot have default parameters", f.name),
                            );
                        }
                        if c.db.funcs.contains_key(&f.name) {
                            c.diag.error(f.span, format!("duplicate extern '{}'", f.name));
                        }
                        c.db.extern_libs.push(x.lib.clone());
                        c.db.funcs.insert(f.name.clone(), sig);
                    }
                }
                Item::Fn(f) => {
                    if f.type_params.is_empty() {
                        c.check_fn(f);
                    }
                }
                Item::Blueprint(b) => {
                    if !b.type_params.is_empty() {
                        continue;
                    }
                    c.current_bp = Some(b.name.clone());
                    c.current_bp_hierarchy = hierarchy(&c.db.blueprints, &b.name);
                    for m in &b.methods {
                        c.in_async = m.is_async;
                        c.check_fn(m);
                        c.in_async = false;
                    }
                    c.current_bp = None;
                    c.current_bp_hierarchy.clear();
                }
                Item::Record(r) => {
                    for _ in &r.fields {}
                }
                Item::Extension(e) => {
                    let ext_key = extension_key(&e.ty);
                    let self_ty = ast_to_type(&e.ty);
                    for m in &e.methods {
                        c.push();
                        c.declare("self", self_ty.clone(), true, m.span);
                        c.in_async = m.is_async;
                        for p in &m.params {
                            c.declare(&p.name, resolve_type_ast(&p.ty, c.db), false, m.span);
                        }
                        c.current_fn_ret = m.ret.as_ref().map(|t| resolve_type_ast(t, c.db)).unwrap_or(Type::Void);
                        for s in &m.body {
                            c.check_stmt(s);
                        }
                        c.in_async = false;
                        c.pop();
                    }
                    let _ = ext_key;
                }
                Item::Stmt(s) => {
                    c.check_stmt(s);
                }
                _ => {}
            }
        }
    }
    let insts: Vec<BlueprintItem> = db.instantiated.values().cloned().collect();
    for inst in insts {
        let mut c = Checker {
            db: &mut db,
            diag,
            scopes: vec![HashMap::new()],
            moved: HashMap::new(),
            channel_moved: HashSet::new(),
            freed: HashSet::new(),
            borrowed: HashMap::new(),
            borrow_owner: HashMap::new(),
            list_epochs: HashMap::new(),
            borrow_epoch: HashMap::new(),
            local_scopes: Vec::new(),
            current_fn_ret: Type::Void,
            in_loop: 0,
            current_bp: None,
            current_bp_hierarchy: vec![],
            in_async: false,
            in_static: false,
            region_stack: vec![0],
            rgn_next: 1,
            region_parent: HashMap::new(),
            region_instance: HashMap::new(),
            lane_of: HashMap::new(),
            gc_managed: HashSet::new(),
            current_fn_key: None,
            current_inout_params: Vec::new(),
            handle_home: HashMap::new(),
            unfrozen_stack: Vec::new(),
            frozen_names_stack: Vec::new(),
            wave_depth: 0,
            spawn_depth: 0,
            joined_stack: vec![Vec::new()],
            lex_loop_vars: Vec::new(),
        };
        c.current_bp = Some(inst.name.clone());
        c.current_bp_hierarchy = hierarchy(&c.db.blueprints, &inst.name);
        for m in &inst.methods {
            c.in_async = m.is_async;
            c.check_fn(m);
            c.in_async = false;
        }
    }
    // SWARM RABBIT HOLE — Wave 6 (Impl-E, DIFF-1): run the scc_accum_params
    // fixpoint once, after every module function (including monomorphized
    // instances) has been checked, then evaluate the deferred Rule-1b
    // allocation-site checks (the E0802 wiring).
    scc_accum_fixpoint(&mut db, diag);
    db
}

/// SWARM RABBIT HOLE — Wave 6 (Impl-E), DIFF-1: summary-table key — the
/// db.funcs key convention ("{blueprint}::{method}" for methods, bare
/// name otherwise), so producer (check_fn) and consumer (check_arity)
/// sides agree. Wave 8 (Impl-I): also consumed by the ir.rs lowering to
/// index CheckDb.lane_assignment (the pinned-lane pre-coloring feed).
pub fn fn_summary_key(bp: Option<&str>, name: &str) -> String {
    match bp {
        Some(b) => format!("{b}::{name}"),
        None => name.to_string(),
    }
}

/// The same key, derived from a callee's FuncSig.
fn summary_key_of(sig: &FuncSig) -> String {
    fn_summary_key(sig.blueprint.as_deref(), &sig.name)
}

/// SWARM RABBIT HOLE — Wave 6 (Impl-E), DIFF-1 Rule 1a: Tarjan's
/// strongly-connected components over the module call graph. Iterative,
/// so deep call chains cannot overflow the checker stack.
fn tarjan_sccs(nodes: &[String], edges: &HashMap<String, BTreeSet<String>>) -> Vec<Vec<String>> {
    let mut idx: HashMap<&String, usize> = HashMap::new();
    let mut low: HashMap<&String, usize> = HashMap::new();
    let mut on_stack: HashSet<&String> = HashSet::new();
    let mut stack: Vec<&String> = Vec::new();
    let mut sccs: Vec<Vec<String>> = Vec::new();
    let mut counter = 0usize;
    for root in nodes {
        if idx.contains_key(root) {
            continue;
        }
        let mut work: Vec<(&String, usize)> = vec![(root, 0)];
        while let Some((v, pi)) = work.pop() {
            if pi == 0 {
                idx.insert(v, counter);
                low.insert(v, counter);
                counter += 1;
                stack.push(v);
                on_stack.insert(v);
            }
            let succs: Vec<&String> = edges.get(v).map(|s| s.iter().collect()).unwrap_or_default();
            let mut new_low = *low.get(v).unwrap();
            if pi > 0 {
                // Returned from succs[pi-1]: fold the child's low link.
                let child = succs[pi - 1];
                new_low = new_low.min(*low.get(child).unwrap());
            }
            let mut recursed = false;
            for i in pi..succs.len() {
                let w = succs[i];
                if !idx.contains_key(w) {
                    low.insert(v, new_low);
                    work.push((v, i + 1));
                    work.push((w, 0));
                    recursed = true;
                    break;
                } else if on_stack.contains(w) {
                    new_low = new_low.min(*idx.get(w).unwrap());
                }
            }
            if recursed {
                continue;
            }
            low.insert(v, new_low);
            if low.get(v).unwrap() == idx.get(v).unwrap() {
                let mut comp: Vec<String> = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack.remove(w);
                    comp.push((*w).clone());
                    if w == v {
                        break;
                    }
                }
                sccs.push(comp);
            }
        }
    }
    sccs
}

/// SWARM RABBIT HOLE — Wave 8 (Impl-I), DIFF-1 Rule 1b: union-find over
/// (function, inout-param) accumulator keys. Two params joined by an
/// inout edge are the same underlying growable (the accumulator flowing
/// caller-ward along the Rule-1a propagation path), so they must share
/// ONE pinned lane — W2A §1.7 K1: each pinned lane hosts exactly one
/// growable for its SCC lifetime. Iterative path-compressed find (the
/// call graph can be deep; nothing here may recurse).
fn uf_find(
    parent: &mut HashMap<(String, usize), (String, usize)>,
    k: (String, usize),
) -> (String, usize) {
    let mut root = k.clone();
    while let Some(p) = parent.get(&root) {
        if *p == root {
            break;
        }
        root = p.clone();
    }
    let mut cur = k;
    while cur != root {
        let next = parent[&cur].clone();
        parent.insert(cur.clone(), root.clone());
        cur = next;
    }
    root
}

fn uf_union(
    parent: &mut HashMap<(String, usize), (String, usize)>,
    a: (String, usize),
    b: (String, usize),
) {
    let ra = uf_find(parent, a);
    let rb = uf_find(parent, b);
    if ra != rb {
        parent.insert(ra, rb);
    }
}

/// SWARM RABBIT HOLE — Wave 6 (Impl-E), DIFF-1 Rule 1a — the SCC
/// fixed-point propagation rule (W2A §1.3 S6, the refutation-killing
/// field), plus Rule 1b's deferred allocation-site checks:
///
/// 1. **Seed** (Rule 1a step 1+2): pushes(f, i) — an inout param grown
///    directly by f — lands in scc_accum(f) only when f sits in a
///    NONTRIVIAL SCC (a self-loop or |C| ≥ 2, i.e. some activation of f
///    can be suspended at an SCC call edge while a deeper activation
///    grows the same accumulator). Growth outside any recursion SCC is
///    single-episode and carries no cross-activation hazard.
/// 2. **Propagation** (Rule 1a steps 2/3 — one uniform edge rule):
///    i ∈ scc_accum(g) ∧ the caller passes its own inout param j into
///    g's param i ⇒ j ∈ scc_accum(caller). Monotone union iterated to
///    the Knaster–Tarski least fixpoint (W2A §1.3 S5: the "≤ 3 passes"
///    figure is an empirical remark, not a lattice-height bound).
/// 3. **Coloring** (Rule 1b pre-coloring / Rule 1d budget, Impl-I Wave 8):
///    pinned lanes L2..L6 are assigned to the SCC's accumulators — one
///    lane per underlying growable (W2A §1.7 K1), first-come-first-served
///    at the SCC level from the module-wide pool (distinct
///    simultaneously-live accumulators always interfere — the budget
///    lemma: 2 ping-pong + 5 pinned + DPS = 8 lanes; never silent reuse).
/// 4. **Rule 1b** (deferred, now conditional): every recorded call site
///    that passed a LOCAL inout into a colored param is resolved against
///    its allocation site: a lane-free local (the default) is PRE-COLORED
///    onto the pinned lane (no error — the discharge that makes E0802
///    conditional; the ir.rs lowering reads the coloring and constructs
///    the growable on the lane); an explicit `lane_alloc` pin matching
///    the coloring is the user-declared pin; an explicit scratch
///    `lane_alloc` still fires E0802 (DIFF-1's buried-top hazard); a
///    lane-less accumulator (the L2..L6 budget exhausted) fires E0802c.
fn scc_accum_fixpoint(db: &mut CheckDb, diag: &mut Diagnostics) {
    // 1. SCCs over the call graph.
    let mut node_set: BTreeSet<String> = db.call_edges.keys().cloned().collect();
    for tos in db.call_edges.values() {
        node_set.extend(tos.iter().cloned());
    }
    let nodes: Vec<String> = node_set.into_iter().collect();
    let sccs = tarjan_sccs(&nodes, &db.call_edges);

    // 2. Seed from growth, gated on nontrivial SCC membership.
    let mut acc: HashMap<String, BTreeSet<usize>> = HashMap::new();
    for c in &sccs {
        let nontrivial = c.len() > 1
            || db
                .call_edges
                .get(&c[0])
                .map(|s| s.contains(&c[0]))
                .unwrap_or(false);
        if !nontrivial {
            continue;
        }
        for f in c {
            if let Some(seeds) = db.fn_growth_seeds.get(f) {
                acc.entry(f.clone()).or_default().extend(seeds.iter().copied());
            }
        }
    }

    // 3. Knaster–Tarski fixpoint over the inout edges.
    loop {
        let mut changed = false;
        for e in &db.inout_call_edges {
            let Some(callee_acc) = acc.get(&e.callee) else { continue };
            if !callee_acc.contains(&e.callee_param) {
                continue;
            }
            if let Some(j) = e.caller_param {
                if acc.entry(e.caller.clone()).or_default().insert(j) {
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    // 4. The coloring pass (Impl-I, Wave 8): DIFF-1 Rule 1b pre-coloring +
    //    Rule 1d lane budget (W2A §1.3 S6 + §1.7 K1 + the budget lemma).
    //    K1 — each pinned lane hosts exactly one growable for its SCC
    //    lifetime — so the same underlying growable gets ONE lane: a
    //    union-find over (function, param) pairs joins every pair an
    //    inout edge connects (the accumulator flowing caller-ward, Rule 1a
    //    steps 2/3). Lanes are assigned first-come-first-served at the
    //    SCC level (deterministic SCC/function/param order) from the
    //    module-wide pool {2..6} — distinct simultaneously-live
    //    accumulators always interfere, so lanes are never silently
    //    reused across SCCs either. An explicit `lane_alloc` pin (L2..L6)
    //    at a call site touching the accumulator is the coloring's
    //    preference when that lane is still free. The sixth distinct
    //    accumulator leaves its root lane-less → E0802c at its sites
    //    (Rule 1d: never silent reuse).
    let mut parent: HashMap<(String, usize), (String, usize)> = HashMap::new();
    for (f, params) in &acc {
        for i in params {
            parent.insert((f.clone(), *i), (f.clone(), *i));
        }
    }
    for e in &db.inout_call_edges {
        let Some(j) = e.caller_param else { continue };
        let (Some(callee_acc), Some(caller_acc)) = (acc.get(&e.callee), acc.get(&e.caller))
        else {
            continue;
        };
        if !callee_acc.contains(&e.callee_param) || !caller_acc.contains(&j) {
            continue;
        }
        uf_union(&mut parent, (e.caller.clone(), j), (e.callee.clone(), e.callee_param));
    }
    let mut sites = std::mem::take(&mut db.scc_accum_sites);
    let mut lane_of_root: HashMap<(String, usize), u8> = HashMap::new();
    // (lane - 2) -> owning accumulator label, for the E0802c diagnostic.
    let mut owner_of_lane: [Option<String>; 5] = [None, None, None, None, None];
    for c in &sccs {
        let nontrivial = c.len() > 1
            || db
                .call_edges
                .get(&c[0])
                .map(|s| s.contains(&c[0]))
                .unwrap_or(false);
        if !nontrivial {
            continue;
        }
        for f in c {
            let Some(params) = acc.get(f) else { continue };
            for i in params {
                let root = uf_find(&mut parent, (f.clone(), *i));
                if lane_of_root.contains_key(&root) {
                    continue;
                }
                // preference: a call site that already pinned its local
                // onto a still-free L2..L6 keeps that lane (the
                // user-declared pin)
                let mut picked: Option<u8> = None;
                for s in sites.iter() {
                    let in_acc = acc
                        .get(&s.callee)
                        .map(|x| x.contains(&s.callee_param))
                        .unwrap_or(false);
                    if !in_acc {
                        continue;
                    }
                    if uf_find(&mut parent, (s.callee.clone(), s.callee_param)) != root {
                        continue;
                    }
                    if let Some(LaneAssignment {
                        lane: k,
                        class: LaneClass::PinnedAccumulator,
                    }) = &s.lane
                    {
                        if (2..=6).contains(k) && owner_of_lane[(*k - 2) as usize].is_none() {
                            picked = Some(*k);
                            break;
                        }
                    }
                }
                let lane =
                    picked.or_else(|| (2u8..=6).find(|&j| owner_of_lane[(j - 2) as usize].is_none()));
                if let Some(l) = lane {
                    owner_of_lane[(l - 2) as usize] = Some(format!("{}:param{}", root.0, root.1));
                    lane_of_root.insert(root, l);
                }
                // None → the L2..L6 budget is exhausted: the root stays
                // lane-less and every site into it fires E0802c below.
            }
        }
    }

    // 5. Commit the summaries (scc_accum_params is the one computed field;
    //    the other three stay spec-carried — see FnBorrowSummary).
    //    pinned_lane_req (W2A §1.3 S6): the lowest pinned lane the
    //    coloring assigned to this function's accumulators.
    let mut keys: BTreeSet<String> = db.fn_growth_seeds.keys().cloned().collect();
    keys.extend(acc.keys().cloned());
    for k in keys {
        let mut s = db.fn_summaries.remove(&k).unwrap_or_default();
        s.scc_accum_params = acc
            .remove(&k)
            .map(|x| x.into_iter().collect())
            .unwrap_or_default();
        s.pinned_lane_req = s
            .scc_accum_params
            .iter()
            .filter_map(|i| lane_of_root.get(&uf_find(&mut parent, (k.clone(), *i))))
            .min()
            .copied();
        db.fn_summaries.insert(k, s);
    }

    // 6. Part A step 2 (Impl-I, Wave 8): the SCC accumulator params' own
    //    provenance — each colored inout param's lane is recorded into
    //    db.lane_assignment (function → param name → lane), overriding
    //    the default scratch 0. (Index-space note: seeds live in the
    //    FnItem param space — self excluded — so a method's sig.params,
    //    which holds self at 0, is indexed with the offset.)
    let mut param_lanes: Vec<(String, String, u8)> = Vec::new();
    for (f, s) in &db.fn_summaries {
        for i in &s.scc_accum_params {
            let Some(lane) = lane_of_root.get(&uf_find(&mut parent, (f.clone(), *i))) else {
                continue;
            };
            let Some(sig) = db.funcs.get(f) else { continue };
            let adj = usize::from(
                sig.is_method
                    && !sig.is_static
                    && sig.params.first().map(|p| p.0 == "self").unwrap_or(false),
            );
            let Some((pname, ..)) = sig.params.get(i + adj) else {
                continue;
            };
            param_lanes.push((f.clone(), pname.clone(), *lane));
        }
    }
    for (f, pname, lane) in param_lanes {
        db.lane_assignment.entry(f).or_default().insert(pname, lane);
    }

    // 7. Rule 1b (deferred, Wave 8 — now conditional): resolve every
    //    recorded allocation site against the coloring. Before the
    //    coloring, a scratch lane was always wrong; now it is wrong only
    //    when the coloring failed to assign.
    for site in sites.iter_mut() {
        let grows = db
            .fn_summaries
            .get(&site.callee)
            .map(|s| s.scc_accum_params.contains(&site.callee_param))
            .unwrap_or(false);
        if !grows {
            continue;
        }
        let root = uf_find(&mut parent, (site.callee.clone(), site.callee_param));
        let Some(lane) = lane_of_root.get(&root).copied() else {
            // Rule 1d (DIFF-1): the L2..L6 budget is exhausted — the
            // budget lemma's sixth distinct simultaneously-live
            // accumulator. Never silent reuse.
            let owners: Vec<String> = owner_of_lane
                .iter()
                .map(|o| o.clone().unwrap_or_default())
                .collect();
            diag.error(
                site.span,
                format!(
                    "SCC accumulator '{}' needs a pinned lane but L2..L6 are all live ('{}'); at most five distinct simultaneously-live pinned accumulators per thread (the budget lemma: 2 ping-pong + 5 pinned + DPS = 8 lanes); make '{}' pool-backed. [E0802]",
                    site.local,
                    owners.join("', '"),
                    site.local
                ),
            );
            continue;
        };
        site.pinned_lane = Some(lane);
        match &site.lane {
            // PRE-COLORING (DIFF-1 Rule 1b): a lane-free allocation site
            // is upgraded onto the pinned lane — the discharge path that
            // makes E0802 conditional. db.lane_assignment carries the
            // coloring to the ir.rs lowering, which constructs the
            // growable ON the lane (W2A §1.3: allocation sites flowing
            // into scc_accum_params positions are colored onto pinned π_j
            // at compile time).
            None => {
                let entry = db
                    .lane_assignment
                    .entry(site.caller.clone())
                    .or_default();
                if let Some(&prev) = entry.get(&site.local) {
                    if prev != lane {
                        diag.error(
                            site.span,
                            format!(
                                "SCC accumulator '{}' is pre-colored onto pinned lane L{lane} here but onto L{prev} by another 'inout' call: one growable lives on exactly one pinned lane (W2A §1.7 K1). [E0802]",
                                site.local
                            ),
                        );
                    }
                } else {
                    entry.insert(site.local.clone(), lane);
                }
            }
            // the user-declared pin matches the coloring — discharged
            Some(LaneAssignment { lane: k, class: LaneClass::PinnedAccumulator }) if *k == lane => {}
            Some(LaneAssignment { lane: k, class: LaneClass::PinnedAccumulator }) => {
                diag.error(
                    site.span,
                    format!(
                        "SCC accumulator '{}' is declared on pinned lane L{k} but its recursion SCC is colored onto pinned lane L{lane} (each pinned lane hosts exactly one growable for its SCC lifetime, W2A §1.7 K1); declare it on L{lane} or leave it lane-free so the pre-coloring assigns it. [E0802]",
                        site.local
                    ),
                );
            }
            // explicit ping-pong scratch (or DPS-out): the buried-top
            // hazard, unchanged from the Wave-6 wiring — the coloring
            // does not rescue a user-declared scratch pin
            Some(a) => {
                diag.error(
                    site.span,
                    format!(
                        "SCC accumulator '{}' is allocated on ping-pong scratch (lane L{}) but is passed 'inout' to '{}', whose summary grows it across recursive SCC re-entries while caller frames are suspended; intervening scope exits would truncate its tail while its length still counts the truncated bytes (type-confused reads). Declare it 'pinned' (allocate on L2..L6) or make it pool-backed. [E0802]",
                        site.local, a.lane, site.callee
                    ),
                );
            }
        }
    }
    db.scc_accum_sites = sites;
}

fn hierarchy(bps: &HashMap<String, BlueprintInfo>, name: &str) -> Vec<String> {
    let mut out = vec![name.to_string()];
    let mut cur = name.to_string();
    while let Some(p) = bps.get(&cur).and_then(|b| b.parent.clone()) {
        out.push(p.clone());
        cur = p;
    }
    out
}

fn extension_key(ty: &TypeAst) -> String {
    match ty {
        TypeAst::Str => "str".into(),
        TypeAst::Int => "int".into(),
        TypeAst::Bool => "bool".into(),
        TypeAst::List(_) => "list".into(),
        TypeAst::Named(n) => n.clone(),
        _ => "unknown".into(),
    }
}

pub fn mono_name(base: &str, args: &[TypeAst]) -> String {
    if args.is_empty() {
        return base.to_string();
    }
    let parts: Vec<String> = args.iter().map(type_ast_label).collect();
    format!("{}_{}", base, parts.join("_"))
}

pub fn type_ast_label(t: &TypeAst) -> String {
    match t {
        TypeAst::Int => "int".into(),
        TypeAst::I8 => "i8".into(),
        TypeAst::I16 => "i16".into(),
        TypeAst::I32 => "i32".into(),
        TypeAst::U8 => "u8".into(),
        TypeAst::U16 => "u16".into(),
        TypeAst::U32 => "u32".into(),
        TypeAst::U64 => "u64".into(),
        TypeAst::Float => "float".into(),
        TypeAst::Str => "str".into(),
        TypeAst::Bool => "bool".into(),
        TypeAst::Byte => "byte".into(),
        TypeAst::Named(n) => n.clone(),
        TypeAst::List(i) => format!("list_{}", type_ast_label(i)),
        TypeAst::Generic(n, args) => {
            let parts: Vec<String> = args.iter().map(type_ast_label).collect();
            format!("{}_{}", n, parts.join("_"))
        }
        _ => "T".into(),
    }
}

pub fn substitute_type(t: &TypeAst, params: &[String], args: &[TypeAst]) -> TypeAst {
    if let TypeAst::Named(n) = t {
        if let Some(i) = params.iter().position(|p| p == n) {
            return args.get(i).cloned().unwrap_or(TypeAst::Int);
        }
    }
    match t {
        TypeAst::List(i) => TypeAst::List(Box::new(substitute_type(i, params, args))),
        TypeAst::Optional(i) => TypeAst::Optional(Box::new(substitute_type(i, params, args))),
        TypeAst::Ref(i) => TypeAst::Ref(Box::new(substitute_type(i, params, args))),
        TypeAst::Mut(i) => TypeAst::Mut(Box::new(substitute_type(i, params, args))),
        TypeAst::Pool(i) => TypeAst::Pool(Box::new(substitute_type(i, params, args))),
        TypeAst::Handle(i) => TypeAst::Handle(Box::new(substitute_type(i, params, args))),
        TypeAst::Chan(i) => TypeAst::Chan(Box::new(substitute_type(i, params, args))),
        TypeAst::Map(k, v) => TypeAst::Map(
            Box::new(substitute_type(k, params, args)),
            Box::new(substitute_type(v, params, args)),
        ),
        TypeAst::Tuple(parts) => TypeAst::Tuple(
            parts.iter().map(|p| substitute_type(p, params, args)).collect(),
        ),
        TypeAst::Generic(name, g_args) => TypeAst::Generic(
            name.clone(),
            g_args.iter().map(|a| substitute_type(a, params, args)).collect(),
        ),
        _ => t.clone(),
    }
}

pub fn infer_type_param(
    param_ast: &TypeAst,
    arg_type: &Type,
    type_param_names: &[String],
    inferred: &mut HashMap<String, TypeAst>,
) {
    match (param_ast, arg_type) {
        (TypeAst::Named(name), ty) if type_param_names.contains(name) => {
            inferred.entry(name.clone()).or_insert_with(|| ty.to_type_ast());
        }
        (TypeAst::List(p_inner), Type::List(a_inner)) => {
            infer_type_param(p_inner, a_inner, type_param_names, inferred);
        }
        (TypeAst::Map(pk, pv), Type::Map(ak, av)) => {
            infer_type_param(pk, ak, type_param_names, inferred);
            infer_type_param(pv, av, type_param_names, inferred);
        }
        (TypeAst::Optional(p_inner), Type::Optional(a_inner)) => {
            infer_type_param(p_inner, a_inner, type_param_names, inferred);
        }
        (TypeAst::Ref(p_inner), Type::Ref(a_inner)) => {
            infer_type_param(p_inner, a_inner, type_param_names, inferred);
        }
        (TypeAst::Mut(p_inner), Type::Mut(a_inner)) => {
            infer_type_param(p_inner, a_inner, type_param_names, inferred);
        }
        (TypeAst::Pool(p_inner), Type::Pool(a_inner)) => {
            infer_type_param(p_inner, a_inner, type_param_names, inferred);
        }
        (TypeAst::Handle(p_inner), Type::Handle(a_inner)) => {
            infer_type_param(p_inner, a_inner, type_param_names, inferred);
        }
        (TypeAst::Chan(p_inner), Type::Chan(a_inner)) => {
            infer_type_param(p_inner, a_inner, type_param_names, inferred);
        }
        (TypeAst::Tuple(p_parts), Type::Tuple(a_parts)) => {
            for (p, a) in p_parts.iter().zip(a_parts.iter()) {
                infer_type_param(p, a, type_param_names, inferred);
            }
        }
        _ => {}
    }
}

pub fn substitute_stmts(stmts: &mut [Stmt], params: &[String], args: &[TypeAst]) {
    for s in stmts {
        substitute_stmt(s, params, args);
    }
}

pub fn substitute_stmt(s: &mut Stmt, params: &[String], args: &[TypeAst]) {
    match s {
        Stmt::Expr(e) => substitute_expr(e, params, args),
        Stmt::Decl { ty, value, .. } => {
            if let Some(t) = ty {
                *t = substitute_type(t, params, args);
            }
            if let Some(v) = value {
                substitute_expr(v, params, args);
            }
        }
        Stmt::Assign { target, value, .. } => {
            substitute_expr(target, params, args);
            substitute_expr(value, params, args);
        }
        Stmt::If { cond, then_body, else_ifs, else_body, .. } => {
            substitute_expr(cond, params, args);
            substitute_stmts(then_body, params, args);
            for (c, b) in else_ifs {
                substitute_expr(c, params, args);
                substitute_stmts(b, params, args);
            }
            if let Some(eb) = else_body {
                substitute_stmts(eb, params, args);
            }
        }
        Stmt::While { cond, body, .. } => {
            substitute_expr(cond, params, args);
            substitute_stmts(body, params, args);
        }
        Stmt::ForCount { init, cond, step, body, .. } => {
            substitute_stmt(init, params, args);
            substitute_expr(cond, params, args);
            substitute_stmt(step, params, args);
            substitute_stmts(body, params, args);
        }
        Stmt::ForIn { iter, body, .. } => {
            substitute_expr(iter, params, args);
            substitute_stmts(body, params, args);
        }
        Stmt::Match { expr, arms, default, .. } => {
            substitute_expr(expr, params, args);
            for (p, b) in arms {
                substitute_expr(p, params, args);
                substitute_stmts(b, params, args);
            }
            if let Some(d) = default {
                substitute_stmts(d, params, args);
            }
        }
        Stmt::Return { value, .. } => {
            if let Some(v) = value {
                substitute_expr(v, params, args);
            }
        }
        Stmt::SpawnBlock { body, .. }
        | Stmt::GoroutineBlock { body, .. }
        | Stmt::LockBlock { body, .. }
        | Stmt::Defer { body, .. }
        | Stmt::ErrDefer { body, .. }
        | Stmt::Region { body, .. }
        | Stmt::NurseryBlock { body, .. }
        | Stmt::Frozen { body, .. } => {
            substitute_stmts(body, params, args);
        }
        // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the
        // substitute walk recurses into the Unfrozen window body.
        Stmt::Unfrozen { body, .. } => {
            substitute_stmts(body, params, args);
        }
        Stmt::Guard { cond, else_body, .. } => {
            substitute_expr(cond, params, args);
            substitute_stmts(else_body, params, args);
        }
        Stmt::SpawnExpr { expr, .. } => {
            substitute_expr(expr, params, args);
        }
        Stmt::New { ty, type_args, args: new_args, .. } => {
            if let Some(i) = params.iter().position(|p| p == ty) {
                if let Some(replacement) = args.get(i) {
                    *ty = type_ast_label(replacement);
                }
            }
            for a in type_args {
                *a = substitute_type(a, params, args);
            }
            for a in new_args {
                match a {
                    Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => substitute_expr(e, params, args),
                }
            }
        }
        Stmt::NestedFn(f) => {
            for p in &mut f.params {
                p.ty = substitute_type(&p.ty, params, args);
            }
            if let Some(r) = &mut f.ret {
                *r = substitute_type(r, params, args);
            }
            substitute_stmts(&mut f.body, params, args);
        }
        _ => {}
    }
}

pub fn substitute_expr(e: &mut Expr, params: &[String], args: &[TypeAst]) {
    match &mut e.kind {
        ExprKind::Binary { lhs, rhs, .. } => {
            substitute_expr(lhs, params, args);
            substitute_expr(rhs, params, args);
        }
        ExprKind::Unary { expr, .. }
        | ExprKind::Await(expr)
        | ExprKind::Try(expr)
        | ExprKind::OptionalChain(expr)
        | ExprKind::ForceUnwrap(expr) => {
            substitute_expr(expr, params, args);
        }
        ExprKind::Call { callee, type_args, args: call_args } => {
            substitute_expr(callee, params, args);
            for a in type_args {
                *a = substitute_type(a, params, args);
            }
            for a in call_args {
                match a {
                    Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => substitute_expr(e, params, args),
                }
            }
        }
        ExprKind::Index { base, index } => {
            substitute_expr(base, params, args);
            substitute_expr(index, params, args);
        }
        ExprKind::Member { base, .. } => {
            substitute_expr(base, params, args);
        }
        ExprKind::List(items) | ExprKind::Tuple(items) => {
            for item in items {
                substitute_expr(item, params, args);
            }
        }
        ExprKind::Map(entries) => {
            for (k, v) in entries {
                substitute_expr(k, params, args);
                substitute_expr(v, params, args);
            }
        }
        ExprKind::Cast { expr, ty } => {
            substitute_expr(expr, params, args);
            *ty = substitute_type(ty, params, args);
        }
        ExprKind::SuperCall { args: sc_args, .. } => {
            for a in sc_args {
                match a {
                    Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => substitute_expr(e, params, args),
                }
            }
        }
        ExprKind::Interpolate { parts } => {
            for p in parts {
                if let InterpPart::Expr(ex) = p {
                    substitute_expr(ex, params, args);
                }
            }
        }
        ExprKind::Lambda { params: l_params, ret, body } => {
            for p in l_params {
                p.ty = substitute_type(&p.ty, params, args);
            }
            if let Some(r) = ret {
                *r = substitute_type(r, params, args);
            }
            substitute_stmts(body, params, args);
        }
        _ => {}
    }
}

fn check_type_arg_bounds_fn(
    db: &CheckDb,
    template: &FnItem,
    args: &[TypeAst],
    span: Span,
    diag: &mut Diagnostics,
) {
    for (i, tp) in template.type_params.iter().enumerate() {
        let Some(bound) = &tp.bound else { continue };
        let Some(arg) = args.get(i) else { continue };
        let aty = resolve_type_ast(arg, db);
        if matches!(aty, Type::Any) {
            continue;
        }
        if !satisfies_bound(db, &aty, bound) {
            diag.error(
                span,
                format!(
                    "type argument '{}' does not satisfy bound '{bound}' on '{}'",
                    type_ast_label(arg),
                    tp.name
                ),
            );
        }
    }
}

fn instantiate_blueprint(db: &mut CheckDb, template: &BlueprintItem, args: &[TypeAst], diag: &mut Diagnostics) {
    let mono = mono_name(&template.name, args);
    if db.blueprints.contains_key(&mono) && db.blueprints[&mono].size > 0 {
        return;
    }
    let mut inst = template.clone();
    inst.name = mono.clone();
    inst.type_params.clear();
    for f in &mut inst.fields {
        f.ty = substitute_type(&f.ty, &template_param_names(&template.type_params), args);
    }
    for m in &mut inst.methods {
        for p in &mut m.params {
            p.ty = substitute_type(&p.ty, &template_param_names(&template.type_params), args);
        }
        if let Some(r) = &mut m.ret {
            *r = substitute_type(r, &template_param_names(&template.type_params), args);
        }
    }
    db.blueprints.insert(
        mono.clone(),
        BlueprintInfo {
            name: mono.clone(),
            type_params: vec![],
            parent: inst.parent.clone(),
            contracts: inst.contracts.clone(),
            fields: vec![],
            methods: inst.methods.iter().filter(|m| !m.is_static).map(|m| m.name.clone()).collect(),
            abstract_methods: vec![],
            concrete_methods: inst
                .methods
                .iter()
                .filter(|m| !m.is_static && !m.is_abstract)
                .map(|m| m.name.clone())
                .collect(),
            closed_methods: inst
                .methods
                .iter()
                .filter(|m| !m.is_static && m.access == Access::Closed)
                .map(|m| m.name.clone())
                .collect(),
            mono_base: Some(template.name.clone()),
            mono_args: args.iter().map(|a| resolve_type_ast(a, db)).collect(),
            variance: template
                .type_params
                .iter()
                .map(|tp| tp.variance)
                .collect(),
            size: 0,
            type_id: db.blueprints.values().filter(|b| !b.is_generic && b.size > 0).count() as i64 + 1,
            is_generic: false,
            is_resource: template.is_resource,
        },
    );
    db.instantiated.insert(mono.clone(), inst.clone());
    layout_blueprint_from_item(db, &inst, diag);
    for m in &inst.methods {
        let mut sig = fn_sig(m, Some(mono.clone()), db);
        sig.is_method = true;
        sig.blueprint = Some(mono.clone());
        db.funcs.insert(format!("{}::{}", mono, m.name), sig);
    }
}

fn layout_blueprint_from_item(db: &mut CheckDb, b: &BlueprintItem, diag: &mut Diagnostics) {
    let name = &b.name;
    let parent = b.parent.clone();
    let mut fields = Vec::new();
    let mut offset: i64 = 24;
    if let Some(p) = parent.clone() {
        if let Some(pb) = db.blueprints.get(&p) {
            fields.extend(pb.fields.clone());
            offset = pb.size;
        }
    }
    for f in &b.fields {
        if fields.iter().any(|x: &FieldInfo| x.name == f.name) {
            diag.error(b.span, format!("duplicate field '{}'", f.name));
        }
        let ty = resolve_type_ast(&f.ty, db);
        fields.push(FieldInfo {
            name: f.name.clone(),
            ty,
            access: f.access,
            offset,
            init: f.init.clone(),
        });
        offset += 8;
    }
    if let Some(info) = db.blueprints.get_mut(name) {
        info.fields = fields;
        info.size = offset;
    }
}

fn walk_item_for_mono(item: &Item, db: &mut CheckDb, _programs: &[Program], diag: &mut Diagnostics) {
    match item {
        Item::Fn(f) => walk_stmts_for_mono(&f.body, db, diag),
        Item::Blueprint(b) => {
            for m in &b.methods {
                walk_stmts_for_mono(&m.body, db, diag);
            }
        }
        Item::Stmt(s) => walk_stmt_for_mono(s, db, diag),
        _ => {}
    }
}

fn walk_stmts_for_mono(stmts: &[Stmt], db: &mut CheckDb, diag: &mut Diagnostics) {
    for s in stmts {
        walk_stmt_for_mono(s, db, diag);
    }
}

fn walk_stmt_for_mono(s: &Stmt, db: &mut CheckDb, diag: &mut Diagnostics) {
    match s {
        Stmt::New { ty, type_args, span, .. } if !type_args.is_empty() => {
            if let Some(template) = db.blueprint_templates.get(ty).cloned() {
                if type_args.len() != template.type_params.len() {
                    diag.error(
                        *span,
                        format!(
                            "blueprint '{ty}' expects {} type arguments, got {}",
                            template.type_params.len(),
                            type_args.len()
                        ),
                    );
                } else {
                    check_type_arg_bounds(db, &template, type_args, *span, diag);
                    instantiate_blueprint(db, &template, type_args, diag);
                }
            }
        }
        Stmt::Decl { ty: Some(t), span, .. } => {
            // `Producer<Animal> x = ...` must monomorphize `Producer_Animal`.
            if let TypeAst::Generic(name, args) = t {
                if let Some(template) = db.blueprint_templates.get(name).cloned() {
                    if args.len() != template.type_params.len() {
                        diag.error(
                            *span,
                            format!(
                                "blueprint '{name}' expects {} type arguments, got {}",
                                template.type_params.len(),
                                args.len()
                            ),
                        );
                    } else {
                        check_type_arg_bounds(db, &template, args, *span, diag);
                        instantiate_blueprint(db, &template, args, diag);
                    }
                }
            }
        }
        Stmt::If { then_body, else_ifs, else_body, .. } => {
            walk_stmts_for_mono(then_body, db, diag);
            for (_, b) in else_ifs {
                walk_stmts_for_mono(b, db, diag);
            }
            if let Some(b) = else_body {
                walk_stmts_for_mono(b, db, diag);
            }
        }
        Stmt::While { body, .. }
        | Stmt::ForIn { body, .. }
        | Stmt::Defer { body, .. }
        | Stmt::ErrDefer { body, .. }
        | Stmt::Region { body, .. }
        | Stmt::NurseryBlock { body, .. }
        | Stmt::Frozen { body, .. } => {
            walk_stmts_for_mono(body, db, diag);
        }
        // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the
        // mono walk recurses into the Unfrozen window body.
        Stmt::Unfrozen { body, .. } => {
            walk_stmts_for_mono(body, db, diag);
        }
        Stmt::Guard { else_body, .. } => {
            walk_stmts_for_mono(else_body, db, diag);
        }
        Stmt::ForCount { init, step, body, .. } => {
            walk_stmt_for_mono(init, db, diag);
            walk_stmt_for_mono(step, db, diag);
            walk_stmts_for_mono(body, db, diag);
        }
        Stmt::Match { arms, default, .. } => {
            for (_, b) in arms {
                walk_stmts_for_mono(b, db, diag);
            }
            if let Some(b) = default {
                walk_stmts_for_mono(b, db, diag);
            }
        }
        Stmt::SpawnBlock { body, .. } | Stmt::GoroutineBlock { body, .. } | Stmt::LockBlock { body, .. } => {
            walk_stmts_for_mono(body, db, diag);
        }
        _ => {}
    }
}

fn collect(db: &mut CheckDb, programs: &[Program], diag: &mut Diagnostics) {
    for p in programs {
        for item in &p.items {
            if let Item::Contract(c) = item {
                let methods = c
                    .methods
                    .iter()
                    .map(|m| FuncSig {
                        name: m.name.clone(),
                        llvm: format!("sn_ct_{}_{}", c.name, m.name),
                        params: m
                            .params
                            .iter()
                            .map(|p| (p.name.clone(), ast_to_type(&p.ty), p.default.is_some()))
                            .collect(),
                        inout_params: m.params.iter().map(|p| p.is_inout).collect(),
                        ret: m.ret.as_ref().map(ast_to_type).unwrap_or(Type::Void),
                        is_method: true,
                        blueprint: None,
                        is_async: false,
                        is_static: false,
                        is_verified: false,
                        is_extern: false,
                        defaults: vec![],
                    })
                    .collect();
                for m in &c.methods {
                    if !db.vtable_methods.contains(&m.name) {
                        db.method_slots.insert(m.name.clone(), db.vtable_methods.len());
                        db.vtable_methods.push(m.name.clone());
                    }
                }
                db.contracts.insert(
                    c.name.clone(),
                    ContractInfo {
                        name: c.name.clone(),
                        methods,
                    },
                );
            }
        }
    }
    for p in programs {
        for item in &p.items {
            if let Item::Record(r) = item {
                db.records.insert(
                    r.name.clone(),
                    RecordInfo {
                        name: r.name.clone(),
                        fields: vec![],
                        size: 0,
                        packed: r.packed,
                    },
                );
            }
            if let Item::Enum(e) = item {
                if db.enums.contains_key(&e.name) {
                    diag.error(e.span, format!("duplicate enum '{}'", e.name));
                }
                let mut seen = std::collections::HashSet::new();
                let mut v_names = Vec::new();
                for v in &e.variants {
                    if !seen.insert(&v.name) {
                        diag.error(v.span, format!("duplicate variant '{}' in enum '{}'", v.name, e.name));
                    }
                    v_names.push(v.name.clone());
                }
                db.enums.insert(e.name.clone(), v_names);
            }
        }
    }
    for p in programs {
        for item in &p.items {
            if let Item::Blueprint(b) = item {
                if !b.type_params.is_empty() {
                    db.blueprint_templates.insert(b.name.clone(), b.clone());
                }
                if let Some(parent) = &b.parent {
                    if !programs.iter().any(|pr| {
                        pr.items.iter().any(|it| matches!(it, Item::Blueprint(x) if x.name == *parent))
                    }) && !db.blueprints.contains_key(parent)
                    {
                        let _ = parent;
                    }
                }
                if b.type_params.is_empty() {
                    db.blueprints.insert(
                        b.name.clone(),
                        BlueprintInfo {
                            name: b.name.clone(),
                            type_params: b.type_params.iter().map(|p| p.name.clone()).collect(),
                            parent: b.parent.clone(),
                            contracts: b.contracts.clone(),
                            fields: vec![],
                            methods: b.methods.iter().filter(|m| !m.is_static).map(|m| m.name.clone()).collect(),
                            abstract_methods: b.methods.iter().filter(|m| m.is_abstract).map(|m| m.name.clone()).collect(),
                            concrete_methods: b.methods.iter().filter(|m| !m.is_static && !m.is_abstract).map(|m| m.name.clone()).collect(),
                            closed_methods: b.methods.iter().filter(|m| !m.is_static && m.access == Access::Closed).map(|m| m.name.clone()).collect(),
                            mono_base: None,
                            mono_args: vec![],
                            variance: vec![],
                            size: 0,
                            type_id: db.blueprints.len() as i64 + 1,
                            is_generic: false,
                            is_resource: b.is_resource,
                        },
                    );
                } else {
                    db.blueprints.insert(
                        b.name.clone(),
                        BlueprintInfo {
                            name: b.name.clone(),
                            type_params: b.type_params.iter().map(|p| p.name.clone()).collect(),
                            parent: b.parent.clone(),
                            contracts: b.contracts.clone(),
                            fields: vec![],
                            methods: b.methods.iter().filter(|m| !m.is_static).map(|m| m.name.clone()).collect(),
                            abstract_methods: b.methods.iter().filter(|m| m.is_abstract).map(|m| m.name.clone()).collect(),
                            concrete_methods: b.methods.iter().filter(|m| !m.is_static && !m.is_abstract).map(|m| m.name.clone()).collect(),
                            closed_methods: b.methods.iter().filter(|m| !m.is_static && m.access == Access::Closed).map(|m| m.name.clone()).collect(),
                            mono_base: None,
                            mono_args: vec![],
                            variance: vec![],
                            size: 0,
                            type_id: 0,
                            is_generic: true,
                            is_resource: b.is_resource,
                        },
                    );
                }
                for m in b.methods.iter().filter(|m| !m.is_static) {
                    if !db.vtable_methods.contains(&m.name) {
                        db.method_slots.insert(m.name.clone(), db.vtable_methods.len());
                        db.vtable_methods.push(m.name.clone());
                    }
                }
            }
        }
    }
    for p in programs {
        for item in &p.items {
            walk_item_for_mono(item, db, programs, diag);
        }
    }
    let names: Vec<String> = db
        .blueprints
        .keys()
        .filter(|n| !db.blueprints[*n].is_generic)
        .cloned()
        .collect();
    for name in names {
        layout_blueprint(db, programs, &name, diag);
    }
    for p in programs {
        for item in &p.items {
            if let Item::Record(r) = item {
                layout_record(db, r, diag);
            }
        }
    }
    for p in programs {
        for item in &p.items {
            if let Item::Enum(e) = item {
                let mut v_infos = Vec::new();
                for v in &e.variants {
                    let mut f_infos = Vec::new();
                    for f in &v.fields {
                        let ty = resolve_type_ast(&f.ty, db);
                        f_infos.push((f.name.clone(), ty));
                    }
                    v_infos.push(EnumVariantInfo {
                        name: v.name.clone(),
                        fields: f_infos,
                    });
                }
                db.enum_variants.insert(e.name.clone(), v_infos);
            }
        }
    }
    for p in programs {
        for item in &p.items {
            if let Item::Extension(e) = item {
                let key = extension_key(&e.ty);
                for m in &e.methods {
                    let ext_name = format!("__ext_{}_{}", key, m.name);
                    let mut sig = fn_sig(m, None, db);
                    sig.name = ext_name.clone();
                    sig.llvm = format!("sn_ext_{}_{}", key, m.name);
                    sig.is_method = true;
                    sig.params.insert(0, ("self".into(), ast_to_type(&e.ty), false));
                    sig.inout_params.insert(0, false);
                    db.funcs.insert(format!("{key}::{mname}", key = key, mname = m.name), sig);
                    db.extensions
                        .entry(key.clone())
                        .or_default()
                        .push(m.name.clone());
                }
            }
        }
    }
    for p in programs {
        for item in &p.items {
            match item {
                Item::Fn(f) => {
                    if !f.type_params.is_empty() {
                        db.fn_templates.insert(f.name.clone(), f.clone());
                    } else {
                        let sig = fn_sig(f, None, db);
                        db.funcs.insert(f.name.clone(), sig);
                    }
                }
                Item::Blueprint(b) if b.type_params.is_empty() => {
                    for m in &b.methods {
                        let mut sig = fn_sig(m, Some(b.name.clone()), db);
                        sig.is_method = true;
                        sig.blueprint = Some(b.name.clone());
                        db.funcs.insert(format!("{}::{}", b.name, m.name), sig);
                    }
                }
                Item::Stmt(Stmt::Decl { names, ty, mode, value, span, .. }) => {
                    if let (Some(n), Some(t)) = (names.first(), ty) {
                        let rt = resolve_type_ast(t, db);
                        // SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort
                        // Delta): the SFC layout types' discipline at
                        // the module-scope site (the same check the
                        // function-body declaration arm runs — the
                        // dims/elem rules are type facts, not
                        // expression facts).
                        check_sfc_type(&rt, *span, diag);
                        db.globals.insert(n.clone(), rt);
                        // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O):
                        // the global-declaration arm records the pool's
                        // mode (the annotation parsed WITH the
                        // declaration: `pool<T>` bare = Frozen,
                        // `pool<T> thread_local` = ThreadConfined,
                        // `pool<T> synchronized` = Synchronized). This
                        // is the table E0803.1/.2/.3 consult at the
                        // spawn/chan capture seam and the mutation seam
                        // (kills G1 §1 A6c).
                        if matches!(t, crate::ast::TypeAst::Pool(_)) {
                            let gm = match *mode {
                                crate::ast::PoolMode::Frozen => GlobalPoolMode::Frozen,
                                crate::ast::PoolMode::ThreadConfined => {
                                    GlobalPoolMode::ThreadConfined
                                }
                                crate::ast::PoolMode::Synchronized => {
                                    GlobalPoolMode::Synchronized
                                }
                            };
                            db.global_pools.insert(n.clone(), gm);
                        }
                        // SWARM RABBIT HOLE — Wave 13 (Impl-R): the
                        // SEAM-CELL declaration — a module-scope
                        // `ptr NAME = lcell_new(k)` (W2B §4.1: the
                        // wave region's seam cells ARE the Tier-L
                        // module-scope cells). The carrier literal k
                        // is recorded here (the module initializer
                        // never runs as code — ir.rs constructs the
                        // cell at main entry, the DIFF-4 global-pool
                        // pattern); a non-literal carrier is
                        // rejected: the seam's carrier must be a
                        // compile-time fact for the fold (J2) and
                        // the R1 analysis to be well-defined.
                        if matches!(t, crate::ast::TypeAst::Ptr) {
                            if let Some(value) = &value {
                                if let ExprKind::Call { callee, args, .. } = &value.kind {
                                    if let ExprKind::Ident(cn) = &callee.kind {
                                        if cn == "lcell_new" {
                                            let carrier = match args.first() {
                                                Some(Arg::Pos(e)) => match &e.kind {
                                                    ExprKind::Int(k) => Some(*k),
                                                    _ => None,
                                                },
                                                _ => None,
                                            };
                                            match carrier {
                                                Some(k) => {
                                                    db.lcell_globals.insert(n.clone(), k);
                                                }
                                                None => {
                                                    diag.error(
                                                        value.span,
                                                        format!(
                                                            "E-R1: seam cell '{n}' must declare a \
                                                             literal carrier: lcell_new(0|max, \
                                                             1|gset, 2|lww) — the J(W) fold and \
                                                             the R1 capture analysis are \
                                                             defined over a compile-time \
                                                             carrier [E-R1]"
                                                        ),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // SWARM RABBIT HOLE — Wave 13 (Impl-R): the wave tables' collect
    // pre-pass, run AFTER every program's declarations are recorded
    // (order-independent by construction — a merge checked before
    // the wave claiming its cell must still see the claim):
    // (a) the wave-claims pre-walk (every wave block's captured seam
    //     cells, per function), then (b) the wave-purity fixpoint
    //     (which functions are pure-for-waves).
    wave_collect_claims(db, programs);
    wave_purity_fixpoint(db, programs);
}

// ---------------------------------------------------------------------
// SWARM RABBIT HOLE — Wave 13 (Impl-R, Cohort Delta): the wave
// region's shared walkers and collect pre-passes (W2B §3/§4 — Thm WS
// made executable). One ban list, one expression visitor, two
// consumers (the wave-body E0812 gate and the fn-purity fixpoint) —
// no drift between what the body gate checks and what the
// interprocedural gate certifies.
// ---------------------------------------------------------------------

/// The E0812 ban list, CALL-NAME class: the schedule injectors (wall
/// clock, env, argv, file/OS IO, the select/chan recv surface, the GC
/// knobs — a collection mid-wave is schedule-adjacent and the tracked
/// engine's mid-bracket story is not this slice's to promise) plus the
/// FALLBACK SYNC OPS (lcell_inflate — the S1 payload application;
/// lcell_merge — the J0 immediate merge; neither is the wave's join
/// discipline W3, and in-wave payloads stage via contrib ONLY).
fn wave_banned_call(n: &str) -> bool {
    matches!(
        n,
        "time_now_ms"
            | "os_getenv"
            | "input"
            | "os_args"
            | "os_exec"
            | "os_system"
            | "select_read"
            | "file_read"
            | "file_read_ex"
            | "file_write"
            | "file_write_ex"
            | "file_append"
            | "file_delete"
            | "file_copy"
            | "file_move"
            | "file_mkdir"
            | "file_rmdir"
            | "file_exists"
            | "file_list"
            | "gc_collect"
            | "gc_set_threshold"
            | "lcell_inflate"
            | "lcell_merge"
    )
}

/// The banned METHOD names (the chan class: receive/send/close; the
/// Tier-L syncop wrappers: merge/inflate/retract — the std.lattice
/// surface's mutators). Method calls on other receivers are handled
/// by the callers' conservatism policy (see the walk).
fn wave_banned_method(n: &str) -> bool {
    matches!(n, "receive" | "send" | "close" | "merge" | "inflate" | "retract")
}

/// Visit every sub-expression of `e` (including `e` itself).
fn wave_visit_expr(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    match &e.kind {
        ExprKind::Binary { lhs, rhs, .. } | ExprKind::Index { base: lhs, index: rhs } => {
            wave_visit_expr(lhs, f);
            wave_visit_expr(rhs, f);
        }
        ExprKind::Unary { expr, .. }
        | ExprKind::Cast { expr, .. }
        | ExprKind::Await(expr)
        | ExprKind::Try(expr)
        | ExprKind::OptionalChain(expr)
        | ExprKind::ForceUnwrap(expr) => wave_visit_expr(expr, f),
        ExprKind::Call { callee, args, .. } => {
            wave_visit_expr(callee, f);
            for a in args {
                match a {
                    Arg::Pos(x) | Arg::Inout(x) | Arg::Named { value: x, .. } => {
                        wave_visit_expr(x, f)
                    }
                }
            }
        }
        ExprKind::SuperCall { args, .. } => {
            for a in args {
                match a {
                    Arg::Pos(x) | Arg::Inout(x) | Arg::Named { value: x, .. } => {
                        wave_visit_expr(x, f)
                    }
                }
            }
        }
        ExprKind::Member { base, .. } => wave_visit_expr(base, f),
        ExprKind::Interpolate { parts } => {
            for p in parts {
                if let InterpPart::Expr(x) = p {
                    wave_visit_expr(x, f);
                }
            }
        }
        ExprKind::List(xs) | ExprKind::Tuple(xs) => {
            for x in xs {
                wave_visit_expr(x, f);
            }
        }
        ExprKind::Map(pairs) => {
            for (k, v) in pairs {
                wave_visit_expr(k, f);
                wave_visit_expr(v, f);
            }
        }
        ExprKind::Lambda { body, .. } => {
            for s in body {
                wave_visit_stmt(s, f);
            }
        }
        _ => {}
    }
}

/// Visit every statement of `s` (including `s`), descending into all
/// nested statements and their expressions.
fn wave_visit_stmt(s: &Stmt, f: &mut dyn FnMut(&Expr)) {
    fn walk(bs: &[Stmt], f: &mut dyn FnMut(&Expr)) {
        for b in bs {
            wave_visit_stmt(b, f);
        }
    }
    match s {
        Stmt::Expr(e) => wave_visit_expr(e, f),
        Stmt::Decl { value, .. } => {
            if let Some(v) = value {
                wave_visit_expr(v, f);
            }
        }
        Stmt::Assign { target, value, .. } => {
            wave_visit_expr(target, f);
            wave_visit_expr(value, f);
        }
        Stmt::If { cond, then_body, else_ifs, else_body, .. } => {
            wave_visit_expr(cond, f);
            walk(then_body, f);
            for (c, b) in else_ifs {
                wave_visit_expr(c, f);
                walk(b, f);
            }
            if let Some(b) = else_body {
                walk(b, f);
            }
        }
        Stmt::While { cond, body, .. } => {
            wave_visit_expr(cond, f);
            walk(body, f);
        }
        Stmt::ForCount { init, cond, step, body, .. } => {
            wave_visit_stmt(init, f);
            wave_visit_expr(cond, f);
            wave_visit_stmt(step, f);
            walk(body, f);
        }
        Stmt::ForIn { iter, body, .. } => {
            wave_visit_expr(iter, f);
            walk(body, f);
        }
        Stmt::Match { expr, arms, default, .. } => {
            wave_visit_expr(expr, f);
            for (a, b) in arms {
                wave_visit_expr(a, f);
                walk(b, f);
            }
            if let Some(d) = default {
                walk(d, f);
            }
        }
        Stmt::Return { value, .. } => {
            if let Some(v) = value {
                wave_visit_expr(v, f);
            }
        }
        Stmt::Guard { cond, else_body, .. } => {
            wave_visit_expr(cond, f);
            walk(else_body, f);
        }
        Stmt::ErrDefer { body, .. }
        | Stmt::Region { body, .. }
        | Stmt::NurseryBlock { body, .. }
        | Stmt::Frozen { body, .. }
        | Stmt::Unfrozen { body, .. }
        | Stmt::SpawnBlock { body, .. }
        | Stmt::LockBlock { body: body, .. }
        | Stmt::GoroutineBlock { body, .. }
        | Stmt::Defer { body, .. }
        | Stmt::Wave { body, .. } => walk(body, f),
        Stmt::SpawnExpr { expr, .. } => wave_visit_expr(expr, f),
        Stmt::Asm { .. } | Stmt::Stop(_) | Stmt::Skip(_) | Stmt::New { .. } => {
            // Asm carries no AST exprs; New's args are walked by the
            // checker's own machinery — the wave gate bans Asm
            // wholesale below (a template can read cycle counters).
            if let Stmt::New { args, .. } = s {
                for a in args {
                    if let Arg::Pos(x) | Arg::Inout(x) | Arg::Named { value: x, .. } = a {
                        wave_visit_expr(x, f);
                    }
                }
            }
        }
        Stmt::NestedFn(nf) => {
            for st in &nf.body {
                wave_visit_stmt(st, f);
            }
        }
    }
}

/// The capture set of a wave body: every module-scope Tier-L seam cell
/// NAMED in the body (W2B §4.1) — the conservative over-approximation:
/// an alias `ptr p = seam` names the cell too, so the registered set
/// is a superset of the read_W/contrib uses (freezing more is safe —
/// R1 constrains captures, and the constraint only tightens). Sorted
/// for deterministic slot assignment in the ir.rs lowering.
fn wave_seam_cells_of(db: &CheckDb, body: &[Stmt]) -> Vec<String> {
    let mut cells = BTreeSet::new();
    for s in body {
        wave_visit_stmt(s, &mut |e| {
            if let ExprKind::Ident(n) = &e.kind {
                if db.lcell_globals.contains_key(n) {
                    cells.insert(n.clone());
                }
            }
        });
    }
    cells.into_iter().collect()
}

/// Does this function body contain an E0812-banned operation DIRECTLY
/// (schedule call, banned method call, await, spawn-family statement,
/// unfrozen window, asm)? The transitive half is the call-graph
/// fixpoint in wave_purity_fixpoint. Also returns the called
/// user-function names (the edges).
fn wave_body_purity_scan(body: &[Stmt]) -> (bool, Vec<String>) {
    let mut impure = false;
    let mut edges = Vec::new();
    for s in body {
        wave_purity_scan_stmt(s, &mut impure, &mut edges);
    }
    (impure, edges)
}

fn wave_purity_scan_stmt(s: &Stmt, impure: &mut bool, edges: &mut Vec<String>) {
    // Statement-class bans.
    match s {
        Stmt::SpawnBlock { .. }
        | Stmt::GoroutineBlock { .. }
        | Stmt::NurseryBlock { .. }
        | Stmt::Unfrozen { .. }
        | Stmt::Asm { .. } => *impure = true,
        Stmt::Return { .. } | Stmt::Stop(_) => {} // legal in helpers
        _ => {}
    }
    let body_of = |s: &Stmt| -> Option<Vec<Stmt>> {
        match s {
            Stmt::If { then_body, else_ifs, else_body, .. } => {
                let mut all = then_body.clone();
                for (_, b) in else_ifs {
                    all.extend(b.clone());
                }
                if let Some(b) = else_body {
                    all.extend(b.clone());
                }
                Some(all)
            }
            Stmt::While { body, .. }
            | Stmt::ErrDefer { body, .. }
            | Stmt::Region { body, .. }
            | Stmt::Frozen { body, .. }
            | Stmt::SpawnBlock { body, .. }
            | Stmt::GoroutineBlock { body, .. }
            | Stmt::LockBlock { body, .. }
            | Stmt::Defer { body, .. }
            | Stmt::Wave { body, .. } => Some(body.clone()),
            _ => None,
        }
    };
    if let Some(bs) = body_of(s) {
        for b in &bs {
            wave_purity_scan_stmt(b, impure, edges);
        }
    }
    // Match arms / guard bodies / for bodies.
    match s {
        Stmt::Match { arms, default, .. } => {
            for (_, b) in arms {
                for x in b {
                    wave_purity_scan_stmt(x, impure, edges);
                }
            }
            if let Some(d) = default {
                for x in d {
                    wave_purity_scan_stmt(x, impure, edges);
                }
            }
        }
        Stmt::Guard { else_body, .. } => {
            for x in else_body {
                wave_purity_scan_stmt(x, impure, edges);
            }
        }
        Stmt::ForCount { init, step, body, .. } => {
            wave_purity_scan_stmt(init, impure, edges);
            wave_purity_scan_stmt(step, impure, edges);
            for x in body {
                wave_purity_scan_stmt(x, impure, edges);
            }
        }
        Stmt::ForIn { body, .. } => {
            for x in body {
                wave_purity_scan_stmt(x, impure, edges);
            }
        }
        Stmt::NestedFn(nf) => {
            for x in &nf.body {
                wave_purity_scan_stmt(x, impure, edges);
            }
        }
        _ => {}
    }
    // Expression-class bans: banned builtin calls, banned method
    // names, awaits, extern calls, plain-fn call edges, and —
    // conservatively — ANY other method call (the receiver's mutators
    // are untracked through the call; the wave-purity fragment is
    // plain functions).
    wave_visit_stmt(s, &mut |e| match &e.kind {
        ExprKind::Await(_) => *impure = true,
        ExprKind::Call { callee, args, .. } => match &callee.kind {
            ExprKind::Ident(n) => {
                if wave_banned_call(n) {
                    *impure = true;
                } else {
                    edges.push(n.clone());
                }
            }
            ExprKind::Member { name, .. } => {
                if wave_banned_method(name) || !args.is_empty() || true {
                    // ANY method call is outside the wave-purity
                    // fragment: the receiver's method set is untracked
                    // through the call (receive/merge hide behind
                    // innocent names on other receivers).
                    *impure = true;
                }
            }
            _ => {}
        },
        _ => {}
    });
}

/// The collect-time pre-walk: every wave block's claim on its captured
/// seam cells, keyed by the enclosing function's summary key
/// (program-wide, so the checking-time R1 rules are order-independent
/// — a merge checked before the wave claiming its cell must still see
/// the claim).
fn wave_collect_claims(db: &mut CheckDb, programs: &[Program]) {
    let mut claims: HashMap<String, Vec<(String, Span)>> = HashMap::new();
    for p in programs {
        for item in &p.items {
            let fns: Vec<(String, &FnItem)> = match item {
                Item::Fn(f) => vec![(fn_summary_key(None, &f.name), f)],
                Item::Blueprint(b) => b
                    .methods
                    .iter()
                    .map(|m| (fn_summary_key(Some(&b.name), &m.name), m))
                    .collect(),
                _ => continue,
            };
            for (key, f) in fns {
                for s in &f.body {
                    wave_collect_claims_stmt(db, &key, s, &mut claims);
                }
            }
        }
    }
    db.wave_claims = claims;
}

fn wave_collect_claims_stmt(
    db: &CheckDb,
    key: &str,
    s: &Stmt,
    claims: &mut HashMap<String, Vec<(String, Span)>>,
) {
    if let Stmt::Wave { body, span } = s {
        for c in wave_seam_cells_of(db, body) {
            claims.entry(c).or_default().push((key.to_string(), *span));
        }
    }
    // Descend into every nested statement (the bodies of ifs/loops/
    // matches/guards/spawn blocks may host wave blocks too).
    let kids: Vec<Stmt> = match s {
        Stmt::If { then_body, else_ifs, else_body, .. } => {
            let mut all = then_body.clone();
            for (_, b) in else_ifs {
                all.extend(b.clone());
            }
            if let Some(b) = else_body {
                all.extend(b.clone());
            }
            all
        }
        Stmt::While { body, .. }
        | Stmt::ForIn { body, .. }
        | Stmt::ErrDefer { body, .. }
        | Stmt::Region { body, .. }
        | Stmt::NurseryBlock { body, .. }
        | Stmt::Frozen { body, .. }
        | Stmt::Unfrozen { body, .. }
        | Stmt::SpawnBlock { body, .. }
        | Stmt::LockBlock { body, .. }
        | Stmt::GoroutineBlock { body, .. }
        | Stmt::Defer { body, .. }
        | Stmt::Wave { body, .. } => body.clone(),
        Stmt::ForCount { init, step, body, .. } => {
            let mut all = vec![*init.clone(), *step.clone()];
            all.extend(body.clone());
            all
        }
        Stmt::Match { arms, default, .. } => {
            let mut all = Vec::new();
            for (_, b) in arms {
                all.extend(b.clone());
            }
            if let Some(d) = default {
                all.extend(d.clone());
            }
            all
        }
        Stmt::Guard { else_body, .. } => else_body.clone(),
        Stmt::NestedFn(nf) => nf.body.clone(),
        _ => Vec::new(),
    };
    for k in &kids {
        wave_collect_claims_stmt(db, key, k, claims);
    }
}

/// The wave-purity fixpoint: fn key → pure-for-waves. A function is
/// pure-for-waves iff its body contains no banned operation DIRECTLY
/// and every plain function it calls is pure-for-waves (least
/// fixpoint — cycles of banned-op-free functions are pure; one banned
/// op anywhere in a cycle poisons it, which is the conservative and
/// correct direction for a kill switch). The wave body's E0812 gate
/// consults this set at every plain-fn call: a wall-clock read one
/// call away would otherwise inject the schedule into 𝒰_W undetected
/// (Lemma WD's premise — E0812's whole purpose).
fn wave_purity_fixpoint(db: &mut CheckDb, programs: &[Program]) {
    // Direct scans + edges, keyed by fn summary key (plain fns,
    // blueprint methods, nested fns walked within their hosts).
    let mut direct: HashMap<String, bool> = HashMap::new(); // key → impure-directly
    let mut edges: HashMap<String, Vec<String>> = HashMap::new();
    let mut record = |key: String, body: &[Stmt]| {
        let (imp, es) = wave_body_purity_scan(body);
        direct.insert(key.clone(), imp);
        edges.entry(key).or_default().extend(es);
    };
    for p in programs {
        for item in &p.items {
            match item {
                Item::Fn(f) => {
                    if f.type_params.is_empty() {
                        record(fn_summary_key(None, &f.name), &f.body);
                    } else {
                        record(format!("{}<T>", f.name), &f.body);
                    }
                }
                Item::Blueprint(b) => {
                    for m in &b.methods {
                        record(fn_summary_key(Some(&b.name), &m.name), &m.body);
                    }
                }
                _ => {}
            }
        }
    }
    // Least fixpoint: pure(k) = !direct[k] ∧ ∀callee pure(callee).
    // Unknown callees (builtins, externs, unresolved) don't create
    // edges — the banned-call check already covered them by name, and
    // extern fns are banned at the wave-body gate by their sig.
    let mut pure: HashMap<String, bool> = direct
        .keys()
        .map(|k| (k.clone(), !direct[k]))
        .collect();
    loop {
        let mut changed = false;
        for (k, es) in edges.iter() {
            if !*pure.get(k).unwrap_or(&false) {
                continue;
            }
            for callee in es {
                let callee_pure = match pure.get(callee) {
                    Some(v) => *v,
                    // A callee whose body the fixpoint never scanned
                    // (a template instantiated later): conservative —
                    // impure until its instance is scanned. Templates
                    // are re-scanned at instantiation via check_fn's
                    // walk of the instantiated body... the wave-body
                    // gate re-checks the call site against the
                    // instantiated body's own scan there.
                    None => false,
                };
                if !callee_pure {
                    pure.insert(k.clone(), false);
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }
    db.wave_pure_fns = pure.into_iter().filter(|(_, v)| *v).map(|(k, _)| k).collect();
}

pub fn resolve_type_ast(t: &TypeAst, db: &CheckDb) -> Type {
    match t {
        TypeAst::Named(n) => {
            if db.records.contains_key(n) {
                return Type::Record(n.clone());
            }
            if db.enums.contains_key(n) {
                return Type::Enum(n.clone());
            }
            if db.contracts.contains_key(n) {
                return Type::Contract(n.clone());
            }
            ast_to_type(t)
        }
        TypeAst::Generic(name, args) => {
            // `Producer<Dog>` names the monomorphic instance `Producer_Dog`.
            let labels: Vec<String> = args.iter().map(type_ast_label).collect();
            let mono = format!("{}_{}", name, labels.join("_"));
            if db.blueprints.contains_key(&mono) {
                return Type::Blueprint(mono);
            }
            // Not yet monomorphized: the reference will be created by the
            // monomorphization pass once the program mentions `new Producer<Dog>`.
            Type::Blueprint(mono)
        }
        TypeAst::List(inner) => Type::List(Box::new(resolve_type_ast(inner, db))),
        TypeAst::Map(k, v) => Type::Map(Box::new(resolve_type_ast(k, db)), Box::new(resolve_type_ast(v, db))),
        TypeAst::Chan(inner) => Type::Chan(Box::new(resolve_type_ast(inner, db))),
        TypeAst::Ref(inner) => Type::Ref(Box::new(resolve_type_ast(inner, db))),
        TypeAst::Pool(inner) => Type::Pool(Box::new(resolve_type_ast(inner, db))),
        TypeAst::Handle(inner) => Type::Handle(Box::new(resolve_type_ast(inner, db))),
        // SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): the link's inner
        // type is the TARGET type — resolve it through the record table so
        // `link<Node>` names the record (the runtime diagnostics and the
        // HasSelfLink predicate both read the resolved target).
        TypeAst::Link(inner) => Type::Link(Box::new(resolve_type_ast(inner, db))),
        // SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the SFC
        // layouts' inner types resolve through the record table like
        // the link target; the elem/dims discipline (fixed-size int,
        // power-of-two dims) is validated at the declaration and
        // constructor sites, where a diagnostic can fire.
        TypeAst::Morton { elem, dims } => {
            Type::Morton(Box::new(resolve_type_ast(elem, db)), dims.clone())
        }
        TypeAst::Hilbert(k, v) => Type::Hilbert(
            Box::new(resolve_type_ast(k, db)),
            Box::new(resolve_type_ast(v, db)),
        ),
        TypeAst::Optional(inner) => Type::Optional(Box::new(resolve_type_ast(inner, db))),
        TypeAst::Tuple(ts) => Type::Tuple(ts.iter().map(|x| resolve_type_ast(x, db)).collect()),
        TypeAst::Fn { params, ret } => Type::Fn {
            params: params.iter().map(|x| resolve_type_ast(x, db)).collect(),
            ret: Box::new(ret.as_ref().map(|x| resolve_type_ast(x, db)).unwrap_or(Type::Void)),
        },
        _ => ast_to_type(t),
    }
}

/* SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the SFC layout
   types' discipline checks (A2 §2.4's decision table, executable —
   the slices' honest boundaries as named, verbatim refusals). Shared
   by the module-scope collect site and the function-body declaration
   site (both hold a Diagnostics); the constructor arms re-run the
   dims rule on their literal arguments. */
fn check_sfc_type(t: &Type, span: Span, diag: &mut Diagnostics) {
    match t {
        // The morton layout: fixed-size int elements (A2 §3: "no
        // directory for fixed-size T" — slice 1 is int only), both
        // dims powers of two (the grid maps onto the code space
        // exactly — no padding).
        Type::Morton(elem, dims) => {
            if !matches!(**elem, Type::Int) {
                diag.error(
                    span,
                    format!(
                        "E-SFC: slice 1 implements morton list<int, dims=[n1, n2]> — \
                         fixed-size int elements only (no directory for fixed-size \
                         T; the Morton-keyed directory lane is the slice-2 form), \
                         got element type {elem} [E-SFC]"
                    ),
                );
            }
            if dims.len() != 2 {
                diag.error(
                    span,
                    format!(
                        "E-SFC: morton lists take exactly two dims — \
                         'morton list<T, dims=[n1, n2]>' (d=2 first; d=3 as the \
                         0x9249…/0x2492…/0x4924… masks allow is the slice-2 \
                         boundary), got {} [E-SFC]",
                        morton_dims_label(dims)
                    ),
                );
            } else if !morton_dims_valid(dims) {
                diag.error(
                    span,
                    format!(
                        "E-SFC: morton dims must be equal powers of two (slice 1 \
                         maps the SQUARE grid onto the code space exactly — no \
                         padding; unequal dims want the padded reservation / the \
                         hybrid 2^t tile floor, the slice-2 form), got {} [E-SFC]",
                        morton_dims_label(dims)
                    ),
                );
            }
        }
        // The hilbert layout: the d=2 point key is the (x, y) int
        // pair, the value is int (the sorted lane carries
        // key/x/y/val); other element types are the slice-2 boundary.
        Type::Hilbert(k, v) => {
            if !matches!(**k, Type::Int) || !matches!(**v, Type::Int) {
                diag.error(
                    span,
                    format!(
                        "E-SFC: slice 1 implements hilbert map<int, int> — the \
                         d=2 point key is the (x, y) int pair, the value is int; \
                         other key/value types are the slice-2 boundary, got \
                         hilbert map<{k}, {v}> [E-SFC]"
                    ),
                );
            }
        }
        _ => {}
    }
}

/// SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the morton
/// constructors' literal dims — morton_new(d1, d2), morton_from_lex(src,
/// d1, d2), morton_from_bytes(bytes, d1, d2) — extracted for the
/// declaration-site dims-agreement rule (the dims are in the type; the
/// constructor must agree). None when the value is not a morton
/// constructor or a dim is not a literal (the constructor arm reports
/// the non-literal case separately).
fn sfc_ctor_dims_of(value: Option<&Expr>) -> Option<(String, Vec<i64>)> {
    let e = value?;
    let ExprKind::Call { callee, args, .. } = &e.kind else { return None };
    let ExprKind::Ident(f) = &callee.kind else { return None };
    let offs: &[usize] = match f.as_str() {
        "morton_new" => &[0, 1],
        "morton_from_lex" | "morton_from_bytes" => &[1, 2],
        _ => return None,
    };
    let mut dims = Vec::new();
    for i in offs {
        let d = args.get(*i).and_then(|a| match a {
            Arg::Pos(x) => match &x.kind {
                ExprKind::Int(v) => Some(*v),
                _ => None,
            },
            _ => None,
        })?;
        dims.push(d);
    }
    Some((f.clone(), dims))
}

/// The T5b detector's loop-variable reader: a counted for's variable
/// comes from its init — `for (int i = 0, ...)` declares it, `for (i =
/// 0, ...)` assigns it. None when the init is neither shape (the
/// detector stays silent — provable shapes only).
fn sfc_loop_var_of(s: &Stmt) -> Option<String> {
    match s {
        Stmt::Decl { names, .. } => names.first().cloned(),
        Stmt::Assign { target, .. } => match &target.kind {
            ExprKind::Ident(n) => Some(n.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// The T5b detector's positional-ident reader: arg i as an Ident name
/// (the loop variables are names in scope).
fn sfc_pos_ident(args: &[Arg], i: usize) -> Option<String> {
    match args.get(i)? {
        Arg::Pos(e) => match &e.kind {
            ExprKind::Ident(n) => Some(n.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn layout_record(db: &mut CheckDb, r: &RecordItem, diag: &mut Diagnostics) {
    let mut fields = Vec::new();
    let mut offset: i64 = 0;
    for f in &r.fields {
        if fields.iter().any(|x: &FieldInfo| x.name == f.name) {
            diag.error(r.span, format!("duplicate field '{}'", f.name));
        }
        let ty = resolve_type_ast(&f.ty, db);
        // SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): a link<T, u16>
        // field is a 2-byte delta — 2-align it in packed records so the
        // u16 load/store in the runtime (sn_link_*) is never unaligned.
        // (Non-packed records slot every field 8-aligned already.)
        if matches!(ty, Type::Link(_)) && r.packed && offset % 2 != 0 {
            offset += 1;
        }
        fields.push(FieldInfo {
            name: f.name.clone(),
            ty: ty.clone(),
            access: f.access,
            offset,
            init: f.init.clone(),
        });
        let sz = match &ty {
            Type::Record(rn) => db.records.get(rn).map(|x| x.size).unwrap_or(8),
            _ if r.packed => ty.abi_size(),
            _ => 8,
        };
        offset += sz;
    }
    if let Some(info) = db.records.get_mut(&r.name) {
        info.fields = fields;
        info.size = offset.max(8);
        info.packed = r.packed;
    }
}

fn layout_blueprint(
    db: &mut CheckDb,
    programs: &[Program],
    name: &str,
    diag: &mut Diagnostics,
) {
    if db.blueprints.get(name).map(|b| b.size > 0).unwrap_or(false) {
        return;
    }
    let parent = db.blueprints.get(name).and_then(|b| b.parent.clone());
    if let Some(p) = parent.clone() {
        layout_blueprint(db, programs, &p, diag);
    }
    let mut fields = Vec::new();
    let mut offset: i64 = 24; // rc + type_id + vptr
    if let Some(p) = parent {
        if let Some(pb) = db.blueprints.get(&p) {
            fields.extend(pb.fields.clone());
            offset = pb.size;
        }
    }
    let bp_item = programs.iter().find_map(|pr| {
        pr.items.iter().find_map(|it| match it {
            Item::Blueprint(b) if b.name == name => Some(b),
            _ => None,
        })
    });
    if let Some(b) = bp_item {
        for f in &b.fields {
            if fields.iter().any(|x: &FieldInfo| x.name == f.name) {
                diag.error(b.span, format!("duplicate field '{}'", f.name));
            }
            let ty = resolve_type_ast(&f.ty, db);
            fields.push(FieldInfo {
                name: f.name.clone(),
                ty,
                access: f.access,
                offset,
                init: f.init.clone(),
            });
            offset += 8;
        }
    }
    if let Some(info) = db.blueprints.get_mut(name) {
        info.fields = fields;
        info.size = offset;
    }
}

fn fn_sig(f: &FnItem, bp: Option<String>, db: &CheckDb) -> FuncSig {
    let llvm = if let Some(b) = &bp {
        format!("sn_m_{}_{}", b, f.name)
    } else {
        format!("sn_fn_{}", f.name)
    };
    FuncSig {
        name: f.name.clone(),
        llvm: llvm.replace('.', "_"),
        params: f
            .params
            .iter()
            .map(|p| (p.name.clone(), resolve_type_ast(&p.ty, db), p.default.is_some()))
            .collect(),
        inout_params: f.params.iter().map(|p| p.is_inout).collect(),
        ret: f.ret.as_ref().map(|t| resolve_type_ast(t, db)).unwrap_or(Type::Void),
        is_method: bp.is_some(),
        blueprint: bp,
        is_async: f.is_async,
        is_static: f.is_static,
        is_verified: f.is_verified,
        is_extern: false,
        defaults: f.params.iter().map(|p| p.default.clone()).collect(),
    }
}

fn check_contracts(db: &mut CheckDb, diag: &mut Diagnostics) {
    let bps: Vec<BlueprintInfo> = db.blueprints.values().cloned().collect();
    for bp in bps {
        for cname in &bp.contracts {
            let Some(ct) = db.contracts.get(cname).cloned() else {
                diag.error(Span::dummy(), format!("unknown contract '{cname}'"));
                continue;
            };
            for m in &ct.methods {
                let key = format!("{}::{}", bp.name, m.name);
                let Some(got) = db.funcs.get(&key) else {
                    diag.error(
                        Span::dummy(),
                        format!("blueprint '{}' does not implement contract method '{}'", bp.name, m.name),
                    );
                    continue;
                };
                if got.ret != m.ret || got.params.len() != m.params.len() {
                    diag.error(
                        Span::dummy(),
                        format!("method '{}' does not match contract '{cname}'", m.name),
                    );
                    continue;
                }
                for (a, b) in got.params.iter().zip(m.params.iter()) {
                    if a.1 != b.1 {
                        diag.error(
                            Span::dummy(),
                            format!("method '{}' parameter types do not match contract '{cname}'", m.name),
                        );
                    }
                }
            }
        }
    }
}

impl Checker<'_> {
    fn push(&mut self) {
        self.scopes.push(HashMap::new());
        self.local_scopes.push(Vec::new());
        // Wave 13 (Impl-R): the J(W) credit frame — aligned with the
        // scope machinery so conditional bodies' credits die with the
        // scope (read_J's temporal discipline is lexical, and the
        // lexical structure IS the scope tree).
        self.joined_stack.push(Vec::new());
    }
    fn pop(&mut self) {
        if let Some(frame) = self.local_scopes.pop() {
            self.end_local_scope(&frame);
        }
        self.scopes.pop();
        self.joined_stack.pop();
    }

    /// Wave 13 (Impl-R): is seam cell `c` J(W)-complete at this point?
    /// True iff some live frame carries its credit (the capturing
    /// wave block closed inside a scope that still encloses this
    /// point) — R1's "after J(W)" arm, proved lexically.
    fn joined_has(&self, c: &str) -> bool {
        self.joined_stack.iter().any(|f| f.iter().any(|n| n == c))
    }

    // --- SWARM RABBIT HOLE — Wave 6 (Impl-E): region poset primitives
    // (W2A §1.1; DIFF-2; D0 gap #4 first slice). ---

    /// W2A §1.1: the innermost open region — the ⊑ₙ-maximum of the stack.
    /// The stack is never empty in practice (index 0 is the function-outer
    /// region); 0 is the safe fallback (the outermost region).
    fn current_rgn(&self) -> RgnId {
        self.region_stack.last().copied().unwrap_or(0)
    }

    /// Enter a lexical region (DIFF-2): allocate a fresh RgnId (ids grow
    /// ⊑ₙ-downward with nesting), record the parent edge to the previous
    /// innermost region (the poset's order relation), push it on top of
    /// the stack. Provenance stamped afterwards with `current_rgn()` is
    /// attributed to this region.
    fn rgn_enter(&mut self) -> RgnId {
        let id = self.rgn_next;
        self.rgn_next += 1;
        let parent = self.current_rgn();
        self.region_parent.insert(id, parent);
        self.region_stack.push(id);
        id
    }

    /// Exit the innermost region: pop innermost-first. SR1 (W2A §1.6)
    /// requires multi-region exits to reset ⊑ₙ-descending — this pop IS
    /// that order (the LIFO of the block-structured walk); the parent-edge
    /// check below verifies the ⊑ₙ chain at every exit.
    fn rgn_exit(&mut self) {
        if let Some(out) = self.region_stack.pop() {
            if let (Some(&parent), Some(&top)) =
                (self.region_parent.get(&out), self.region_stack.last())
            {
                debug_assert_eq!(parent, top, "region stack discipline broken");
            }
        }
    }

    /// SWARM RABBIT HOLE — Wave 8 (Impl-J): W2A §1.5 T-PUSH/T-STORE —
    /// the ⊑ₗ test of the safety law. W2A §1.1 (Def 1.2 formulation):
    /// ∀ stored ref component with target ρ_t: ρ_t ⊑ₗ reg(holder), the
    /// RI invariant of §4.1 P-I (every live cell holding a ref to `a`
    /// has reg(a) ⊑ₗ reg(cell)). `target ⊑ₗ holder` — the target's region
    /// OUTLIVES the holder's region (⊑ₗ = phase superset, Rust's
    /// `'long:'short`). On the RgnId stack (ids grow ⊑ₙ-downward with
    /// nesting; Prop 0.2 duality: ⊑ₗ = (⊑ₙ)ᵒᵖ) this is exactly "target
    /// is holder or an ancestor of holder" on the region_parent chain —
    /// the target was entered before the holder, so the holder's
    /// region exits first and the holder can never dangle.
    fn rgn_outlives(&self, target: RgnId, holder: RgnId) -> bool {
        let mut cur = holder;
        loop {
            if cur == target {
                return true;
            }
            match self.region_parent.get(&cur) {
                Some(&parent) => cur = parent,
                None => return false,
            }
        }
    }
    fn declare(&mut self, name: &str, ty: Type, is_const: bool, span: Span) {
        if let Some(frame) = self.local_scopes.last_mut() {
            frame.push(name.to_string());
        }
        // SWARM RABBIT HOLE — Wave 6 (Impl-E, DIFF-2 / W2A §1.2): every
        // binding's provenance carries its region — the lexical home
        // region of the declaration (RootOrigin.region_instance). Borrow
        // creation copies the ROOT's region into the borrow's provenance,
        // parallel to how borrow_epoch copies the root's shrink epoch.
        self.region_instance.insert(name.to_string(), self.current_rgn());
        if let Some(scope) = self.scopes.last_mut() {
            if scope.contains_key(name) {
                self.diag
                    .error(span, format!("duplicate variable '{name}'"));
            }
            scope.insert(name.to_string(), (ty, is_const));
        }
    }
    fn lookup(&self, name: &str) -> Option<(Type, bool)> {
        for s in self.scopes.iter().rev() {
            if let Some(v) = s.get(name) {
                return Some(v.clone());
            }
        }
        if let Some(t) = self.db.globals.get(name) {
            return Some((t.clone(), false));
        }
        None
    }

    fn scope_depth_of(&self, name: &str) -> Option<usize> {
        for (idx, scope) in self.scopes.iter().enumerate().rev() {
            if scope.contains_key(name) {
                return Some(idx);
            }
        }
        None
    }

    fn set_ty(&mut self, span: Span, ty: Type) -> Type {
        self.db
            .expr_ty
            .insert((span.file, span.start, span.end), ty.clone());
        ty
    }

    fn instantiate_fn(&mut self, template: &FnItem, args: &[TypeAst], _span: Span) -> String {
        let mono = mono_name(&template.name, args);
        if self.db.funcs.contains_key(&mono) {
            return mono;
        }
        let mut inst = template.clone();
        inst.name = mono.clone();
        inst.type_params.clear();
        let param_names = template_param_names(&template.type_params);
        for p in &mut inst.params {
            p.ty = substitute_type(&p.ty, &param_names, args);
        }
        if let Some(r) = &mut inst.ret {
            *r = substitute_type(r, &param_names, args);
        }
        substitute_stmts(&mut inst.body, &param_names, args);

        let sig = fn_sig(&inst, None, self.db);
        self.db.funcs.insert(mono.clone(), sig);
        self.db.instantiated_funcs.insert(mono.clone(), inst.clone());

        let prev_bp = self.current_bp.take();
        let prev_bp_hierarchy = std::mem::take(&mut self.current_bp_hierarchy);
        let prev_ret = self.current_fn_ret.clone();
        let prev_in_async = self.in_async;
        let prev_in_static = self.in_static;

        self.check_fn(&inst);

        self.current_bp = prev_bp;
        self.current_bp_hierarchy = prev_bp_hierarchy;
        self.current_fn_ret = prev_ret;
        self.in_async = prev_in_async;
        self.in_static = prev_in_static;

        mono
    }

    fn check_fn(&mut self, f: &FnItem) {
        let prev_static = self.in_static;
        self.in_static = f.is_static;
        self.moved.clear();
        self.channel_moved.clear();
        self.freed.clear();
        self.borrowed.clear();
        self.borrow_owner.clear();
        self.list_epochs.clear();
        self.borrow_epoch.clear();
        // SWARM RABBIT HOLE — Wave 6 (Impl-E, D0 gap #4 first slice): a
        // fresh region poset per activation — RgnId 0 is the
        // function-outer region (DIFF-2); ids restart at 1 and grow
        // ⊑ₙ-downward with nesting (W2A §1.1).
        self.region_stack.clear();
        self.region_stack.push(0);
        self.rgn_next = 1;
        self.region_parent.clear();
        self.region_instance.clear();
        self.lane_of.clear();
        self.gc_managed.clear();
        // DIFF-1: summaries are keyed module-wide; the fixpoint runs
        // after all functions are checked. Save/restore so a nested or
        // monomorphized check never leaks its key into the caller.
        let prev_fn_key = self.current_fn_key.take();
        let prev_inout_params = std::mem::take(&mut self.current_inout_params);
        self.current_fn_key = Some(fn_summary_key(self.current_bp.as_deref(), &f.name));
        self.push();
        if let Some(bp) = &self.current_bp {
            if !f.is_static {
                self.declare("self", Type::Blueprint(bp.clone()), true, f.span);
            }
        }
        for (i, p) in f.params.iter().enumerate() {
            self.declare(&p.name, resolve_type_ast(&p.ty, self.db), false, f.span);
            // DIFF-1 Rule 1a seed space: this function's own inout params,
            // with their sig.inout_params indices — the same space
            // check_arity's param_offset logic maps call-site args into.
            if p.is_inout {
                self.current_inout_params.push((p.name.clone(), i));
            }
            if let Some(d) = &p.default {
                let dt = self.check_expr(d);
                let pt = resolve_type_ast(&p.ty, self.db);
                if !pt.assignable_from(&dt, &self.db.blueprints) {
                    self.diag.error(d.span, "default parameter type mismatch");
                }
            }
        }
        let was_async = self.in_async;
        self.in_async = f.is_async;
        self.current_fn_ret = f.ret.as_ref().map(|t| resolve_type_ast(t, self.db)).unwrap_or(Type::Void);
        if f.is_async {
            if self.current_fn_ret == Type::Void {
                self.current_fn_ret = Type::Named("future".into());
            }
        }
        for s in &f.body {
            self.check_stmt(s);
        }
        self.in_async = was_async;
        self.pop();
        self.in_static = prev_static;
        self.current_fn_key = prev_fn_key;
        self.current_inout_params = prev_inout_params;
    }

    fn check_stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Expr(e) => {
                self.check_expr(e);
            }
            Stmt::Decl {
                is_const,
                ty,
                names,
                value,
                mode: _,
                span,
            } => {
                if names.len() == 1 {
                    let name = &names[0];
                    let val_ty = value.as_ref().map(|v| self.check_expr(v));
                    let decl_ty = if let Some(t) = ty {
                        let t = resolve_type_ast(t, self.db);
                        // SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort
                        // Delta): the SFC layout types' discipline at
                        // the declaration site — fixed-size int
                        // elements, power-of-two dims (the grid maps
                        // onto the code space exactly), hilbert
                        // map<int, int> (the verbatim E-SFC arms the
                        // golden suite pins).
                        check_sfc_type(&t, *span, &mut self.diag);
                        // The dims-are-in-the-type rule: a morton
                        // constructor's literal dims must agree with
                        // the declared type — the annotation must not
                        // lie about the bounds the runtime checks.
                        if let Type::Morton(_, ddims) = &t {
                            if let Some((ctor, cdims)) = sfc_ctor_dims_of(value.as_ref()) {
                                if &cdims != ddims {
                                    self.diag.error(
                                        *span,
                                        format!(
                                            "E-SFC: {ctor}'s dims [{}] do not match the \
                                             declared morton list type's {} — the dims \
                                             are in the type; the constructor must agree \
                                             [E-SFC]",
                                            cdims
                                                .iter()
                                                .map(|d| d.to_string())
                                                .collect::<Vec<_>>()
                                                .join(", "),
                                            morton_dims_label(ddims)
                                        ),
                                    );
                                }
                            }
                        }
                        if let Some(vt) = &val_ty {
                            let ok = t.assignable_from(vt, &self.db.blueprints)
                                || *vt == Type::None
                                || (value.as_ref().map(|v| Self::int_lit_value(v).is_some()).unwrap_or(false)
                                    && matches!(t, Type::I8|Type::I16|Type::I32|Type::U8|Type::U16|Type::U32|Type::U64|Type::Float|Type::Enum(_)))
                                || (matches!(value.as_ref().map(|v| &v.kind), Some(ExprKind::Float(_))) && t == Type::Float);
                            if !ok {
                                self.diag.error(*span, format!("cannot assign {vt} to {t}"));
                            }
                        }
                        t
                    } else {
                        val_ty.clone().unwrap_or(Type::Int)
                    };
                    let decl_ty = if matches!(decl_ty, Type::Named(_)) {
                        val_ty.clone().unwrap_or(decl_ty)
                    } else {
                        decl_ty
                    };
                    // SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): the
                    // value's type, kept for the E0801 declaration-site
                    // check below (val_ty is consumed by the Named
                    // resolution above).
                    let val_ty_kept = val_ty.clone();
                    Self::check_int_literal_range(&decl_ty, value.as_ref(), *span, self.diag);
                    if let Type::List(elem) = &decl_ty {
                        if let Some(Expr { kind: ExprKind::List(els), .. }) = value.as_ref() {
                            for el in els {
                                Self::check_int_literal_range(elem, Some(el), el.span, self.diag);
                            }
                        }
                    }
                    if let Type::Map(k_ty, v_ty) = &decl_ty {
                        if let Some(Expr { kind: ExprKind::Map(entries), span }) = value.as_ref() {
                            if entries.is_empty() {
                                self.set_ty(*span, decl_ty.clone());
                            }
                            for (k_el, v_el) in entries {
                                Self::check_int_literal_range(k_ty, Some(k_el), k_el.span, self.diag);
                                Self::check_int_literal_range(v_ty, Some(v_el), v_el.span, self.diag);
                            }
                        }
                    }
                    let decl_ty_clone = decl_ty.clone();
                    self.declare(name, decl_ty, *is_const, *span);
                    // SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): the
                    // E0801 by-value-copy rejection, DECLARATION-
                    // INITIALIZATION form (kills G1 §1 A2 → S4).
                    // `Node d = s` copies the HasSelfLink value's bytes
                    // (sn_record_copy in the lowering) to d's address:
                    // every 2-byte delta in the copy would decode
                    // against d's address, not the link's target. The
                    // spec's rule-2 copy — the closure extends it from
                    // plain assignment to the declaration initializer.
                    // Fires iff has_self_link(type_of(rhs)) ∧ the RHS is
                    // an lvalue-load (a call/literal RHS materializes
                    // fresh — sret builds it in place — and stays
                    // legal; the fresh-construction discharge is
                    // link_push_fresh_ok.sn).
                    if let Some(v) = value {
                        if let Some(vt) = &val_ty_kept {
                            if self.db.has_self_link(vt) && Self::is_lvalue_load(v) {
                                let src = Self::expr_label(v);
                                let t = vt.to_string();
                                self.diag.error(
                                    v.span,
                                    format!(
                                        "E0801 (assign): cannot assign '{src}' to '{name}' by \
                                         value because '{t}' contains a self-relative \
                                         'link<{t}, u16>' (the copied delta would decode \
                                         against '{name}''s address, not the link's target); \
                                         assign a freshly constructed value (built in place \
                                         via sret), pass 'ref<{t}>', or use \
                                         'relocate_node(src, dst)'. [E0801]"
                                    ),
                                );
                            }
                        }
                    }
                    // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the
                    // handle-home provenance side table (kills G1 §1 A6c).
                    // The pool method calls produce handles, so the
                    // binding site records which pool minted the handle
                    // — `int h = gp.insert(v)`, the intrinsic twin
                    // `pool_insert(gp, v)`, and transitive copies
                    // `h2 = h1` (INCLUDING int-typed copies: handles are
                    // held in int vars on this surface, so the declared
                    // type does not identify them — the copy of a
                    // handle into pool P is a handle into pool P, and
                    // E0803.2 must fire on the copy exactly as on the
                    // original). E0803.1/.2 consult this table at the
                    // spawn/chan capture seam; untracked handles
                    // (parameters, fabricated ints, arithmetic results
                    // — handle_pool_of returns None for anything but
                    // the minting call or an already-tracked ident)
                    // carry no mode check — the runtime generation
                    // check remains the use-after-free guard.
                    if let Some(pool) = value.as_ref().and_then(|v| self.handle_pool_of(v)) {
                        self.handle_home.insert(name.clone(), pool);
                    }
                    // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the
                    // alias hole's closure — a `pool<T>` binding whose
                    // value is another pool binding (`pool<int> q = p`)
                    // would rebind the pool object under a LOCAL name:
                    // locals accept unguarded mutation (single-thread by
                    // construction is the LIE once the local aliases a
                    // global), and the mode cannot follow the alias.
                    // Pool objects are therefore not copyable values —
                    // only the Synchronized mode's safety follows the
                    // runtime object, and even that would desync the
                    // checker's mode table. Reject the copy outright.
                    if matches!(decl_ty_clone, Type::Pool(_)) {
                        if let Some(v) = value {
                            if let Some(src) = Self::root_var_of_expr(v) {
                                let src_is_pool = self
                                    .lookup(&src)
                                    .map(|(t, _)| matches!(t, Type::Pool(_)))
                                    .unwrap_or(false);
                                if src_is_pool {
                                    self.diag.error(
                                        *span,
                                        format!(
                                            "cannot alias pool '{src}' into '{name}': pool \
                                             objects are not copyable values — the declared \
                                             mode (frozen/thread_local/synchronized) cannot \
                                             follow an alias, and a local binding accepts \
                                             unguarded mutation (the freelist race E0803 \
                                             exists to prevent). Use '{src}' by name, or send \
                                             its handles. [E0803]"
                                        ),
                                    );
                                }
                            }
                        }
                    }
                    // SWARM RABBIT HOLE — Wave 8 (Impl-L): D0 gap #6 /
                    // W2A §1.4 — list/map literals are the GC-managed
                    // holders (sn_list_new/sn_map_new → gc_track): the
                    // literal half of the GC-heap classification, next
                    // to the lane_alloc stamping below.
                    if matches!(
                        value.as_ref().map(|v| &v.kind),
                        Some(ExprKind::List(_)) | Some(ExprKind::Map(_))
                    ) {
                        self.gc_managed.insert(name.clone());
                    }
                    // SWARM RABBIT HOLE — Wave 6 (Impl-E, DIFF-1
                    // insertion point 6): allocation-site lane recording.
                    // Wave 8 (Impl-I): the class follows the STRATA lane
                    // classes — `lane_alloc` names its lane; L0/L1 are
                    // ScratchPingPong, L2..L6 are PinnedAccumulator (the
                    // user-declared pin — the "declare it 'pinned'
                    // (allocate on L2..L6)" discharge of E0802, which the
                    // coloring pass prefers when it colors the SCC's
                    // accumulator), L7 is CallerSretOut (DPS). Growable
                    // locals without an explicit `lane_alloc` stay
                    // lane-free here — the coloring pass pre-colors them.
                    if let Some(Expr { kind: ExprKind::Call { callee, args, .. }, .. }) =
                        value.as_ref()
                    {
                        if let ExprKind::Ident(f) = &callee.kind {
                            if f == "lane_alloc" {
                                let lane = match args.first() {
                                    Some(Arg::Pos(e)) => match e.kind {
                                        ExprKind::Int(n) => n.clamp(0, 255) as u8,
                                        _ => 0,
                                    },
                                    _ => 0,
                                };
                                let class = match lane {
                                    2..=6 => LaneClass::PinnedAccumulator,
                                    7 => LaneClass::CallerSretOut,
                                    _ => LaneClass::ScratchPingPong,
                                };
                                self.lane_of.insert(
                                    name.to_string(),
                                    LaneAssignment { lane, class },
                                );
                            }
                        }
                    }
                    // Borrow creation and move-out.
                    let exclusive = matches!(decl_ty_clone, Type::Mut(_));
                    if Self::is_borrow_type(&decl_ty_clone) {
                        if let Some(val) = value {
                            match &val.kind {
                                // ref<T> r = address(x)  /  mut<T> m = address(x)  /  xs.push_ref(v)
                                ExprKind::Call { callee, args, .. } => {
                                    let is_address = matches!(&callee.kind,
                                        ExprKind::Ident(f) if f == "address" || f == "as_ptr");
                                    if is_address {
                                        if let Some(Arg::Pos(a)) = args.first() {
                                            match &a.kind {
                                                ExprKind::Ident(owner) => {
                                                    self.register_borrow(
                                                        name, owner, exclusive, *span,
                                                    );
                                                }
                                                ExprKind::Index { base, .. } | ExprKind::Member { base, .. } => {
                                                    if let Some(root) = Self::root_var_of_expr(base) {
                                                        self.borrow_owner.insert(name.to_string(), root.clone());
                                                        let ep = self.list_epochs.get(&root).copied().unwrap_or(0);
                                                        self.borrow_epoch.insert(name.to_string(), (root.clone(), ep));
                                                        // SWARM RABBIT HOLE — Wave 6 (Impl-E, DIFF-2):
                                                        // the borrow's provenance carries its target's
                                                        // region (RootOrigin.region_instance).
                                                        let rgn = self
                                                            .region_instance
                                                            .get(&root)
                                                            .copied()
                                                            .unwrap_or_else(|| self.current_rgn());
                                                        self.region_instance.insert(name.to_string(), rgn);
                                                        self.borrowed.insert(root, (name.to_string(), exclusive));
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                    } else if let ExprKind::Member { base, name: mname } = &callee.kind {
                                        if mname == "push_ref" {
                                            if let Some(root) = Self::root_var_of_expr(base) {
                                                self.borrow_owner.insert(name.to_string(), root.clone());
                                                let ep = self.list_epochs.get(&root).copied().unwrap_or(0);
                                                self.borrow_epoch.insert(name.to_string(), (root.clone(), ep));
                                                // SWARM RABBIT HOLE — Wave 6 (Impl-E, DIFF-2):
                                                // the borrow's provenance carries its target's
                                                // region (RootOrigin.region_instance).
                                                let rgn = self
                                                    .region_instance
                                                    .get(&root)
                                                    .copied()
                                                    .unwrap_or_else(|| self.current_rgn());
                                                self.region_instance.insert(name.to_string(), rgn);
                                                self.borrowed.insert(root, (name.to_string(), exclusive));
                                            }
                                        }
                                    }
                                }
                                // ref<T> r = other  -> reborrow, owner moves
                                ExprKind::Ident(src) => {
                                    let src_is_borrow = self
                                        .lookup(src)
                                        .map(|(t, _)| Self::is_borrow_type(&t))
                                        .unwrap_or(false);
                                    if src_is_borrow {
                                        if let Some(owner) =
                                            self.borrow_owner.get(src).cloned()
                                        {
                                            if self.borrowed.contains_key(&owner) {
                                                self.borrowed.remove(&owner);
                                                self.borrowed.insert(
                                                    owner.clone(),
                                                    (name.to_string(), exclusive),
                                                );
                                            }
                                            self.borrow_owner
                                                .insert(name.to_string(), owner);
                                        }
                                        if let Some(ep) = self.borrow_epoch.get(src).cloned() {
                                            self.borrow_epoch.insert(name.to_string(), ep);
                                        }
                                        // SWARM RABBIT HOLE — Wave 6 (Impl-E,
                                        // DIFF-2): a reborrow aliases the same
                                        // storage — carry the original borrow's
                                        // region forward in the provenance.
                                        if let Some(rgn) = self.region_instance.get(src).cloned() {
                                            self.region_instance.insert(name.to_string(), rgn);
                                        }
                                        self.moved.insert(src.clone(), *span);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                } else {
                    let mut vt = None;
                    if names.len() == 2 {
                        if let Some(v) = value {
                            if let Some(elem) = self.chan_recv_elem_ty(v) {
                                let tuple_t = Type::Tuple(vec![elem, Type::Bool]);
                                self.set_ty(v.span, tuple_t.clone());
                                vt = Some(tuple_t);
                            }
                        }
                    }
                    let vt = vt.or_else(|| value.as_ref().map(|v| self.check_expr(v)));
                    match vt {
                        Some(Type::Tuple(ts)) if ts.len() == names.len() => {
                            for (n, t) in names.iter().zip(ts.into_iter()) {
                                self.declare(n, t, *is_const, *span);
                            }
                        }
                        Some(t) => self.diag.error(*span, format!("expected tuple, got {t}")),
                        None => self.diag.error(*span, "tuple declaration needs a value"),
                    }
                }
            }
            Stmt::Assign {
                target,
                op,
                value,
                span,
            } => {
                if let ExprKind::Tuple(els) = &target.kind {
                    if els.len() == 2 {
                        if let Some(elem) = self.chan_recv_elem_ty(value) {
                            let tuple_t = Type::Tuple(vec![elem.clone(), Type::Bool]);
                            self.set_ty(value.span, tuple_t);
                            for (i, el) in els.iter().enumerate() {
                                if let ExprKind::Ident(n) = &el.kind {
                                    if self.lookup(n).is_none() {
                                        let ty = if i == 0 { elem.clone() } else { Type::Bool };
                                        self.declare(n, ty, false, el.span);
                                    }
                                }
                            }
                        }
                    }
                }
                let tt = self.check_expr(target);
                let vt = self.check_expr(value);
                // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the
                // handle-home table's assignment half — `h2 = h1` and
                // `h2 = gp.insert(v)` keep the provenance (a copy of a
                // handle into pool P is a handle into pool P; E0803.2
                // fires on the copy exactly as on the original). The
                // key is handle_pool_of (provenance), not the declared
                // type: handles are held in int vars on this surface.
                // A re-assignment of a non-handle value CLEARS a stale
                // entry (x = 1 after x = h: x is no longer that
                // handle — keeping the entry would misattribute a
                // later capture).
                if let ExprKind::Ident(n) = &target.kind {
                    match self.handle_pool_of(value) {
                        Some(pool) => {
                            self.handle_home.insert(n.clone(), pool);
                        }
                        None => {
                            self.handle_home.remove(n);
                        }
                    }
                }
                if let ExprKind::Ident(n) = &target.kind {
                    if self.lookup(n).map(|(_, c)| c).unwrap_or(false) {
                        self.diag.error(*span, format!("cannot assign to const '{n}'"));
                    }
                    // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O):
                    // the Assign twin of the Decl alias gate —
                    // re-aliasing a pool object through any pool-typed
                    // target rebinds the runtime object under a new
                    // name whose mode bookkeeping cannot follow.
                    if matches!(tt, Type::Pool(_)) {
                        if let Some(src) = Self::root_var_of_expr(value) {
                            let src_is_pool = self
                                .lookup(&src)
                                .map(|(t, _)| matches!(t, Type::Pool(_)))
                                .unwrap_or(false);
                            if src_is_pool && src != *n {
                                self.diag.error(
                                    *span,
                                    format!(
                                        "cannot alias pool '{src}' into '{n}': pool objects \
                                         are not copyable values — the declared mode cannot \
                                         follow an alias, and the receiving binding accepts \
                                         unguarded mutation. Use '{src}' by name. [E0803]"
                                    ),
                                );
                            }
                        }
                    }
                }
                // SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): the
                // E0801 by-value-copy rejection, ASSIGNMENT form (the
                // spec's rule 2, kills G1 §1 A2 → S4). `d = s` where s
                // is an lvalue-load of a HasSelfLink type copies the
                // stored bytes to d's address — the copied deltas
                // would decode against d's address, not the link's
                // target. Fires iff has_self_link(type_of(rhs)) ∧
                // rhs_is_lvalue_load(rhs); a freshly materializing RHS
                // (call/literal — sret) stays legal.
                if *op == AssignOp::Eq
                    && self.db.has_self_link(&vt)
                    && Self::is_lvalue_load(value)
                {
                    let src = Self::expr_label(value);
                    let dst = Self::expr_label(target);
                    let t = vt.to_string();
                    self.diag.error(
                        value.span,
                        format!(
                            "E0801 (assign): cannot assign '{src}' to '{dst}' by value \
                             because '{t}' contains a self-relative 'link<{t}, u16>' (the \
                             copied delta would decode against '{dst}''s address, not the \
                             link's target); assign a freshly constructed value (built in \
                             place via sret), pass 'ref<{t}>', or use \
                             'relocate_node(src, dst)'. [E0801]"
                        ),
                    );
                }
                if *op == AssignOp::Eq {
                    if !tt.assignable_from(&vt, &self.db.blueprints) {
                        // allow Int literal -> fixed int / float, checked for range below
                        let int_lit_ok = Self::int_lit_value(value).is_some()
                            && matches!(tt, Type::I8|Type::I16|Type::I32|Type::U8|Type::U16|Type::U32|Type::U64|Type::Float);
                        let float_lit_ok = matches!(&value.kind, ExprKind::Float(_)) && tt == Type::Float;
                        let enum_ok = matches!(&tt, Type::Enum(_)) && matches!(&vt, Type::Enum(_)|Type::Int);
                        if !(int_lit_ok || float_lit_ok || enum_ok) {
                            self.diag.error(*span, format!("cannot assign {vt} to {tt}"));
                        }
                    }
                    Self::check_int_literal_range(&tt, Some(value), *span, self.diag);
                    if let Type::List(elem) = &tt {
                        if let ExprKind::List(els) = &value.kind {
                            for el in els {
                                Self::check_int_literal_range(elem, Some(el), el.span, self.diag);
                            }
                        }
                    }
                    if let Type::Map(k_ty, v_ty) = &tt {
                        if let ExprKind::Map(entries) = &value.kind {
                            if entries.is_empty() {
                                self.set_ty(value.span, tt.clone());
                            }
                            for (k_el, v_el) in entries {
                                Self::check_int_literal_range(k_ty, Some(k_el), k_el.span, self.diag);
                                Self::check_int_literal_range(v_ty, Some(v_el), v_el.span, self.diag);
                            }
                        }
                    }
                    // SWARM RABBIT HOLE — Wave 8 (Impl-J): W2A §1.5
                    // T-STORE — the escape rule, after the type check
                    // and before the borrow check. A ref-valued value
                    // stored into a binding (`x = r`), a container
                    // element (`xs[i] = r`), or a record field
                    // (`rec.f = r`) must have its target region ⊑ₗ the
                    // destination's region (§4.1 P-I's RI invariant).
                    // Wave 8 (Impl-L): the GC-scoping companion check
                    // runs alongside — the memory-CATEGORY rule
                    // (E-GC-IN-LANE), which the single-region ⊑ₗ poset
                    // cannot express (D0 gap #6).
                    if let Some(dst_root) = match &target.kind {
                        ExprKind::Ident(n) => Some(n.clone()),
                        ExprKind::Member { base, .. } | ExprKind::Index { base, .. } => {
                            Self::root_var_of_expr(base)
                        }
                        _ => None,
                    } {
                        self.escape_check_store(value, &dst_root, *span);
                        self.gc_scoping_check_store(value, &dst_root, *span);
                    }
                    if let Some(root) = Self::root_var_of_expr(target) {
                        let borrower = self.borrowed.get(&root).map(|(b, _)| b.clone()).or_else(|| {
                            self.borrow_owner.iter().find(|(_, o)| *o == &root).map(|(b, _)| b.clone())
                        });
                        if let Some(borrower) = borrower {
                            self.diag.error(*span, format!("cannot mutate '{root}' while borrowed by '{borrower}'"));
                        }
                    }
                    if let Some(local_name) = self.local_address_source(value) {
                        if let Some(local_depth) = self.scope_depth_of(&local_name) {
                            let mut outlives = false;
                            match &target.kind {
                                ExprKind::Member { base, .. } => {
                                    let bt = self.check_expr(base);
                                    if matches!(bt, Type::Blueprint(_)) {
                                        outlives = true;
                                    } else if let Some(base_root) = Self::root_var_of_expr(base) {
                                        let base_depth = self.scope_depth_of(&base_root);
                                        if base_depth.map(|d| d < local_depth).unwrap_or(true) {
                                            outlives = true;
                                        }
                                    }
                                }
                                ExprKind::Ident(target_name) => {
                                    let target_depth = self.scope_depth_of(target_name);
                                    if target_depth.map(|d| d < local_depth).unwrap_or(true) {
                                        outlives = true;
                                    }
                                }
                                ExprKind::Index { base, .. } => {
                                    if let Some(base_root) = Self::root_var_of_expr(base) {
                                        let base_depth = self.scope_depth_of(&base_root);
                                        if base_depth.map(|d| d < local_depth).unwrap_or(true) {
                                            outlives = true;
                                        }
                                    }
                                }
                                _ => {}
                            }
                            if outlives {
                                self.diag.error(
                                    *span,
                                    format!("cannot assign address of local '{local_name}': target outlives declaring scope"),
                                );
                            }
                        }
                    }
                    if let ExprKind::Ident(n) = &target.kind {
                        self.freed.remove(n);
                        self.moved.remove(n);
                        self.channel_moved.remove(n);
                        if let Some((t, _)) = self.lookup(n) {
                            if matches!(t, Type::List(_)) {
                                let ep = self.list_epochs.entry(n.clone()).or_insert(0);
                                *ep += 1;
                            }
                        }
                    } else if let ExprKind::Tuple(els) = &target.kind {
                        for el in els {
                            if let ExprKind::Ident(n) = &el.kind {
                                self.freed.remove(n);
                                self.moved.remove(n);
                                self.channel_moved.remove(n);
                            }
                        }
                    }
                    if let ExprKind::Ident(n) = &value.kind {
                        if let Some((t, _)) = self.lookup(n) {
                            if Self::is_borrow_type(&t) {
                                self.moved.insert(n.clone(), *span);
                            }
                        }
                    }
                } else {
                    let mut ok = tt == vt
                        || matches!((&tt, &vt), (Type::Int, Type::Byte) | (Type::Byte, Type::Int))
                        || vt.int_fits_in(&tt)
                        || (tt == Type::Float && (vt.int_min_max().is_some() || vt == Type::Int));
                    if !ok && tt.is_fixed_int() && vt.is_fixed_int() && matches!(op, AssignOp::ShlEq | AssignOp::ShrEq) {
                        ok = true;
                    }
                    if !ok && tt.is_fixed_int() && Self::int_lit_value(value).is_some() {
                        ok = true;
                        // Skip range check for bitwise compound ops — masks work at bit level
                        if !matches!(op, AssignOp::AmpEq | AssignOp::PipeEq | AssignOp::CaretEq | AssignOp::ShlEq | AssignOp::ShrEq) {
                            Self::check_int_literal_range(&tt, Some(value), *span, self.diag);
                        }
                    }
                    if !ok && tt == Type::Float && Self::int_lit_value(value).is_some() {
                        ok = true;
                    }
                    if !ok {
                        self.diag.error(*span, "compound assignment type mismatch");
                    }
                }
            }
            Stmt::If {
                cond,
                then_body,
                else_ifs,
                else_body,
                ..
            } => {
                let ct = self.check_expr(cond);
                if ct != Type::Bool {
                    self.diag.error(cond.span, "if condition must be bool");
                }
                let narrow = none_narrow(cond);
                self.push();
                if let Some((n, true)) = &narrow {
                    self.rebind_unwrap(n);
                }
                for s in then_body {
                    self.check_stmt(s);
                }
                self.pop();
                for (c, b) in else_ifs {
                    if self.check_expr(c) != Type::Bool {
                        self.diag.error(c.span, "else-if condition must be bool");
                    }
                    let n2 = none_narrow(c);
                    self.push();
                    if let Some((n, true)) = &n2 {
                        self.rebind_unwrap(n);
                    }
                    for s in b {
                        self.check_stmt(s);
                    }
                    self.pop();
                }
                if let Some(b) = else_body {
                    self.push();
                    if let Some((n, false)) = &narrow {
                        self.rebind_unwrap(n);
                    }
                    for s in b {
                        self.check_stmt(s);
                    }
                    self.pop();
                }
            }
            Stmt::While { cond, body, .. } => {
                if self.check_expr(cond) != Type::Bool {
                    self.diag.error(cond.span, "while condition must be bool");
                }
                self.in_loop += 1;
                self.push();
                for s in body {
                    self.check_stmt(s);
                }
                self.pop();
                self.in_loop -= 1;
            }
            Stmt::ForCount {
                init,
                cond,
                step,
                body,
                span: for_span,
            } => {
                self.push();
                self.check_stmt(init);
                if self.check_expr(cond) != Type::Bool {
                    self.diag.error(cond.span, "for condition must be bool");
                }
                self.check_stmt(step);
                self.in_loop += 1;
                // SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta):
                // the T5b shape detector (A2 §3/T5(b): wrong-order
                // iteration over SFC storage = Θ(N) misses, a factor
                // B/w over the stride-1 curve scan — "the language
                // must bind iteration order to storage order"). The
                // PROVABLE shape: this counted for's variable is the
                // INNER axis of a lexicographic pair — an enclosing
                // counted for's variable is the OUTER — and the body
                // indexes a morton list directly as
                // morton_get/set(m, outer, inner). A WARNING, never
                // an error: the program is legal, the layout is
                // wasted. Curve-ordered iteration (morton_scan_get
                // over the code) is shapeless here and stays silent.
                if let Some(mv) = sfc_loop_var_of(init) {
                    if !self.lex_loop_vars.is_empty() {
                        let outers = self.lex_loop_vars.clone();
                        let mut hit: Option<(String, String)> = None;
                        for s in body {
                            wave_visit_stmt(s, &mut |e| {
                                if hit.is_some() {
                                    return;
                                }
                                if let ExprKind::Call { callee, args, .. } = &e.kind {
                                    if let ExprKind::Ident(f) = &callee.kind {
                                        if f == "morton_get" || f == "morton_set" {
                                            let a1 = sfc_pos_ident(args, 1);
                                            let a2 = sfc_pos_ident(args, 2);
                                            if let (Some(o), Some(w)) = (a1, a2) {
                                                if w == mv && outers.contains(&o) {
                                                    let coll = args
                                                        .first()
                                                        .and_then(|a| match a {
                                                            Arg::Pos(x) => {
                                                                Some(Self::expr_label(x))
                                                            }
                                                            _ => None,
                                                        })
                                                        .unwrap_or_else(|| "m".into());
                                                    hit = Some((o, coll));
                                                }
                                            }
                                        }
                                    }
                                }
                            });
                        }
                        if let Some((ov, coll)) = hit {
                            self.diag.warn(
                                *for_span,
                                format!(
                                    "W-SFC-T5B: lexicographic iteration ('{ov}' then \
                                     '{mv}') over morton list '{coll}' wastes the \
                                     space-filling layout — iterate the curve order \
                                     (morton_scan_get over the code) or use soa for \
                                     axis-committed access [W-SFC-T5B]"
                                ),
                            );
                        }
                    }
                    self.lex_loop_vars.push(mv);
                }
                for s in body {
                    self.check_stmt(s);
                }
                if sfc_loop_var_of(init).is_some() {
                    self.lex_loop_vars.pop();
                }
                self.in_loop -= 1;
                self.pop();
            }
            Stmt::ForIn { name, iter, body, span } => {
                let it = self.check_expr(iter);
                let elem = match &it {
                    Type::List(e) => *e.clone(),
                    Type::Optional(inner) => match inner.as_ref() {
                        Type::List(e) => *e.clone(),
                        _ => {
                            self.diag.error(iter.span, "for-in expects a list");
                            Type::Int
                        }
                    },
                    Type::Map(_, v) => *v.clone(),
                    Type::Str => Type::Str,
                    _ => {
                        self.diag.error(iter.span, format!("cannot iterate {it}"));
                        Type::Int
                    }
                };
                self.push();
                self.declare(name, elem, false, *span);
                self.in_loop += 1;
                for s in body {
                    self.check_stmt(s);
                }
                self.in_loop -= 1;
                self.pop();
            }
            Stmt::Match {
                expr,
                arms,
                default,
                span,
            } => {
                let et = self.check_expr(expr);
                let mut has_none = false;
                let mut has_other = false;
                for (pat, body) in arms {
                    if let Type::Enum(en) = &et {
                        if self.db.is_algebraic_enum(en) {
                            has_other = true;
                            self.push();
                            match &pat.kind {
                                ExprKind::Call { callee, args, .. } => {
                                    if let ExprKind::Member { base, name } = &callee.kind {
                                        if let ExprKind::Ident(ref pen) = &base.kind {
                                            if pen == en {
                                                if let Some(v_info) = self.db.enum_variants.get(en).and_then(|vs| vs.iter().find(|v| &v.name == name)).cloned() {
                                                    if v_info.fields.len() != args.len() {
                                                        self.diag.error(
                                                            pat.span,
                                                            format!(
                                                                "pattern '{en}.{name}' expects {} arguments, got {}",
                                                                v_info.fields.len(),
                                                                args.len()
                                                            ),
                                                        );
                                                    } else {
                                                        for (i, a) in args.iter().enumerate() {
                                                            let arg_e = match a {
                                                                Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => e,
                                                            };
                                                            if let ExprKind::Ident(var_name) = &arg_e.kind {
                                                                // SWARM RABBIT HOLE — Wave 11
                                                                // / DIFF-5 (Impl-P): the E0801
                                                                // by-value-copy rejection,
                                                                // MATCH-BINDING form (the spec's
                                                                // verbatim diagnostic, kills G1
                                                                // §1 A2 → S4). The spec-as-written
                                                                // rule 2 was silent on match arms:
                                                                // binding a HasSelfLink payload to
                                                                // a by-value pattern variable
                                                                // copies the payload bytes into
                                                                // the binding's address — the
                                                                // copied delta would decode against
                                                                // the binding's address, not the
                                                                // link's target. Fires iff
                                                                // has_self_link(payload) ∧ the
                                                                // pattern arg is a positional
                                                                // (by-value) binding; an `inout`
                                                                // (ref) pattern binding and `_`
                                                                // (no binding) stay legal.
                                                                if var_name != "_"
                                                                    && matches!(a, Arg::Pos(_))
                                                                    && self.db.has_self_link(
                                                                        &v_info.fields[i].1,
                                                                    )
                                                                {
                                                                    self.diag.error(
                                                                        arg_e.span,
                                                                        format!(
                                                                            "E0801 (match): cannot \
                                                                             bind '{var_name}' by \
                                                                             value from the match \
                                                                             arm on '{en}': '{en}' \
                                                                             contains a self-relative \
                                                                             'link<{en}, u16>'; bind \
                                                                             'ref<{en}>' in the pattern, \
                                                                             or 'relocate_node' into a \
                                                                             fresh destination. [E0801]"
                                                                        ),
                                                                    );
                                                                }
                                                                if var_name != "_" {
                                                                    self.scopes.last_mut().unwrap().insert(
                                                                        var_name.clone(),
                                                                        (v_info.fields[i].1.clone(), false),
                                                                    );
                                                                }
                                                            }
                                                        }
                                                    }
                                                } else {
                                                    self.diag.error(pat.span, format!("unknown variant '{name}' for enum '{en}'"));
                                                }
                                            } else {
                                                self.diag.error(pat.span, format!("expected variant of '{en}', got '{pen}'"));
                                            }
                                        }
                                    }
                                }
                                ExprKind::Member { base, name } => {
                                    if let ExprKind::Ident(ref pen) = &base.kind {
                                        if pen == en {
                                            if let Some(v_info) = self.db.enum_variants.get(en).and_then(|vs| vs.iter().find(|v| &v.name == name)) {
                                                if !v_info.fields.is_empty() {
                                                    self.diag.error(
                                                        pat.span,
                                                        format!("variant '{en}.{name}' has fields, pattern must bind them"),
                                                    );
                                                }
                                            } else {
                                                self.diag.error(pat.span, format!("unknown variant '{name}' for enum '{en}'"));
                                            }
                                        } else {
                                            self.diag.error(pat.span, format!("expected variant of '{en}', got '{pen}'"));
                                        }
                                    }
                                }
                                _ => {
                                    self.diag.error(pat.span, format!("invalid pattern for enum '{en}'"));
                                }
                            }
                            for s in body {
                                self.check_stmt(s);
                            }
                            self.pop();
                            continue;
                        }
                    }
                    let pt = self.check_expr(pat);
                    if matches!(pat.kind, ExprKind::None) {
                        has_none = true;
                    } else {
                        has_other = true;
                    }
                    let int_compat = et.is_fixed_int() && pt.is_fixed_int();
                    if !int_compat && pt != et && !et.assignable_from(&pt, &self.db.blueprints) && !matches!(et, Type::Optional(_)) {
                        self.diag.error(pat.span, "match arm type mismatch");
                    }
                    if int_compat {
                        Self::check_int_literal_range(&et, Some(pat), pat.span, self.diag);
                    }
                    self.push();
                    for s in body {
                        self.check_stmt(s);
                    }
                    self.pop();
                }
                if let Some(b) = default {
                    self.push();
                    for s in b {
                        self.check_stmt(s);
                    }
                    self.pop();
                }
                if matches!(et, Type::Optional(_) | Type::Error)
                    && default.is_none()
                    && !(has_none && has_other)
                {
                    self.diag.error(
                        *span,
                        "match on T? / error is not exhaustive; cover none and a value, or add default",
                    );
                }
            }
            Stmt::Return { value, span, .. } => {
                let vt = value.as_ref().map(|v| self.check_expr(v)).unwrap_or(Type::Void);
                if !self.current_fn_ret.assignable_from(&vt, &self.db.blueprints) && self.current_fn_ret != vt {
                    self.diag.error(
                        *span,
                        format!("return type {vt} does not match {}", self.current_fn_ret),
                    );
                }
                // A borrow of a local cannot outlive the function.
                if Self::is_borrow_type(&self.current_fn_ret) || Self::is_borrow_type(&vt) {
                    if let Some(val) = value {
                        match &val.kind {
                            ExprKind::Ident(n) => {
                                if let Some(owner) = self.borrow_owner.get(n).cloned() {
                                    if self.scopes.iter().any(|s| s.contains_key(&owner)) {
                                        self.diag.error(
                                            *span,
                                            format!(
                                                "cannot return '{n}': it borrows '{owner}', \
                                                 which is local to this function"
                                            ),
                                        );
                                    }
                                }
                            }
                            ExprKind::Call { callee, args, .. } => {
                                let is_address = matches!(&callee.kind,
                                    ExprKind::Ident(f) if f == "address" || f == "as_ptr");
                                if is_address {
                                    if let Some(Arg::Pos(a)) = args.first() {
                                        if let Some(owner) = Self::root_var_of_expr(a) {
                                            if self.scopes.iter().any(|s| s.contains_key(&owner)) {
                                                self.diag.error(
                                                    *span,
                                                    format!(
                                                        "cannot assign address of local '{owner}': target outlives declaring scope"
                                                    ),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Stmt::Stop(span) | Stmt::Skip(span) => {
                if self.in_loop == 0 {
                    self.diag.error(*span, "stop/skip outside loop");
                }
            }
            Stmt::SpawnBlock { body, .. } => {
                self.check_spawn_block(body);
                // Wave 13 (Impl-R): the spawn-depth counter — a wave
                // inside a spawned task would fold J(W) into module-
                // scope seam cells from a non-joining thread (the
                // torn-read class S2 exists to kill). See the Wave arm.
                self.spawn_depth += 1;
                for s in body {
                    self.check_stmt(s);
                }
                self.spawn_depth -= 1;
            }
            Stmt::GoroutineBlock { body, .. } => {
                self.spawn_depth += 1;
                for s in body {
                    self.check_stmt(s);
                }
                self.spawn_depth -= 1;
            }
            Stmt::SpawnExpr { expr, .. } => {
                if let ExprKind::Call { callee: _, args, .. } = &expr.kind {
                    for a in args {
                        let arg_expr = match a {
                            Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => e,
                        };
                        let t = self.check_expr(arg_expr);
                        if matches!(t, Type::Ref(_) | Type::Mut(_) | Type::Ptr) {
                            self.diag.error(
                                arg_expr.span,
                                format!("cannot pass reference of type '{t}' to spawn: data race hazard"),
                            );
                        } else if let Type::Blueprint(bp) = &t {
                            self.diag.error(
                                arg_expr.span,
                                format!("cannot pass blueprint '{bp}' to spawn without locking"),
                            );
                        } else if matches!(t, Type::Pool(_)) {
                            // SWARM RABBIT HOLE — Wave 10 / DIFF-4
                            // (Impl-O): the pool object is not a
                            // Sendable argument (locals never; globals
                            // per their mode).
                            if let Some(n) = Self::root_var_of_expr(arg_expr) {
                                self.check_pool_capture(&n, arg_expr.span);
                            }
                        } else if let Some(h) = Self::root_var_of_expr(arg_expr) {
                            // DIFF-4 (Impl-O): a handle argument is
                            // checked against its minting pool's mode
                            // + the capture site's window (E0803.1/.2).
                            // Provenance keys on handle_home — handles
                            // are held in int vars on this surface, so
                            // the declared type does not identify them.
                            if let Some(p) = self.handle_home.get(&h).cloned() {
                                self.check_handle_capture(&h, &p, arg_expr.span);
                            }
                        }
                    }
                } else {
                    self.check_expr(expr);
                }
            }
            Stmt::LockBlock { name, body, span } => {
                match self.lookup(name) {
                    Some((Type::Lock, _)) => {}
                    Some((t, _)) => self.diag.error(*span, format!("'{name}' is {t}, not lock")),
                    None => self.diag.error(*span, format!("unknown lock '{name}'")),
                }
                for s in body {
                    self.check_stmt(s);
                }
            }
            Stmt::New {
                ty,
                type_args,
                name,
                args,
                span,
                ..
            } => {
                let resolved = if !type_args.is_empty() {
                    if let Some(template) = self.db.blueprint_templates.get(ty).cloned() {
                        instantiate_blueprint(self.db, &template, type_args, self.diag);
                        mono_name(ty, type_args)
                    } else {
                        self.diag.error(*span, format!("unknown generic blueprint '{ty}'"));
                        ty.clone()
                    }
                } else {
                    ty.clone()
                };
                let Some(bp) = self.db.blueprints.get(&resolved).cloned() else {
                    self.diag.error(*span, format!("unknown blueprint '{ty}'"));
                    return;
                };
                if bp.is_generic {
                    self.diag.error(*span, format!("blueprint '{ty}' needs type arguments"));
                    return;
                }
                let unimplemented = self.unimplemented_abstracts(&resolved);
                if !unimplemented.is_empty() {
                    self.diag.error(
                        *span,
                        format!(
                            "cannot instantiate abstract blueprint '{ty}': not implemented {}",
                            unimplemented
                                .iter()
                                .map(|m| format!("'{m}'"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    );
                }
                self.check_ctor_args(&bp, args, *span);
                self.declare(name, Type::Blueprint(resolved.clone()), false, *span);
                // SWARM RABBIT HOLE — Wave 8 (Impl-L): D0 gap #6 / W2A
                // §1.4 — `new` is the GC-heap constructor path
                // (sn_obj_new → gc_track, runtime.c): the binding is
                // GC-managed, the category the lane store rule guards.
                self.gc_managed.insert(name.clone());
            }
            Stmt::Defer { body, .. } | Stmt::ErrDefer { body, .. } => {
                self.push();
                for s in body {
                    self.check_stmt(s);
                }
                self.pop();
            }
            // Wave 13 (Impl-R): the nursery is the spawn family's
            // structured form — same wave prohibition as spawn/go
            // (a wave's J(W) fold must never run on a non-joining
            // thread; see the Wave arm).
            Stmt::NurseryBlock { body, .. } => {
                self.spawn_depth += 1;
                self.push();
                for s in body {
                    self.check_stmt(s);
                }
                self.pop();
                self.spawn_depth -= 1;
            }
            // SWARM RABBIT HOLE — Wave 6 (Impl-E, D0 gap #4 first slice):
            // `region {}` is a node of the region poset (W2A §1.1) —
            // enter/exit the RgnId stack in lockstep with the existing
            // scope machinery. The lexical nesting IS the ⊑ₙ order; the
            // exit pop is SR1's innermost-first (the existing LIFO,
            // verified). No semantics change: the ids are bookkeeping
            // until the escape-checker slice consumes them.
            Stmt::Region { body, .. } => {
                self.rgn_enter();
                self.push();
                for s in body {
                    self.check_stmt(s);
                }
                self.pop();
                self.rgn_exit();
            }
            Stmt::Frozen { names, body, span } => {
                // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the
                // mode-conflict rule — `frozen (P) {...}` refuses to
                // list a Synchronized pool (the vault's lock IS its
                // discipline; lane-freezing language around it would be
                // a lie about what is protected).
                for n in names {
                    if self.db.global_pools.get(n) == Some(&GlobalPoolMode::Synchronized) {
                        self.diag.error(
                            *span,
                            format!(
                                "E0803.3: cannot freeze synchronized pool '{n}': a \
                                 'synchronized' pool is already protected by its own lock — \
                                 'frozen ({n})' would claim a protection it does not provide. [E0803]"
                            ),
                        );
                    }
                }
                self.frozen_names_stack.push(names.clone());
                self.push();
                for s in body {
                    self.check_stmt(s);
                }
                self.pop();
                self.frozen_names_stack.pop();
            }            // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the
            // `unfrozen (P) { ... }` window — the structured
            // single-thread mutation window on a Frozen global pool
            // (kills G1 §1 A6c). Enter = unfreeze (P's frozen_depth++
            // at runtime, thread-exclusively), exit = re-freeze; inside
            // the window insert/set/remove are legal on the declaring
            // thread and capture of P's handles by spawn/chan is
            // E0803.2 (the receiver could race the freelist with the
            // mutator). Mode conflicts: a window on a ThreadConfined
            // or Synchronized pool refuses to open (those modes have
            // their own mutation discipline), and a window refuses to
            // nest inside a `frozen (P)` block.
            Stmt::Unfrozen { pool, body, span } => {
                let mode = self.pool_mode_of(pool);
                match mode {
                    PoolModeCheck::Local => {
                        // A local pool is single-thread by construction
                        // (capture is rejected below); the window is a
                        // harmless no-op that keeps the surface total.
                    }
                    PoolModeCheck::Global(GlobalPoolMode::Frozen) => {}
                    PoolModeCheck::Global(GlobalPoolMode::ThreadConfined) => {
                        self.diag.error(
                            *span,
                            format!(
                                "E0803.3: cannot open 'unfrozen ({pool})': '{pool}' is \
                                 'thread_local' — mutations are already legal on its declaring \
                                 thread, and the window would only widen them. [E0803]"
                            ),
                        );
                    }
                    PoolModeCheck::Global(GlobalPoolMode::Synchronized) => {
                        self.diag.error(
                            *span,
                            format!(
                                "E0803.3: cannot open 'unfrozen ({pool})': '{pool}' is \
                                 'synchronized' — every mutation is already legal under the \
                                 pool's lock, and the window would bypass it. [E0803]"
                            ),
                        );
                    }
                    PoolModeCheck::NotAPool => {
                        self.diag.error(
                            *span,
                            format!("unknown pool '{pool}' in 'unfrozen' window"),
                        );
                    }
                }
                if self
                    .frozen_names_stack
                    .iter()
                    .any(|names| names.contains(pool))
                {
                    self.diag.error(
                        *span,
                        format!(
                            "E0803.3: cannot open 'unfrozen ({pool})' inside a \
                             'frozen ({pool})' block: the pool cannot be simultaneously \
                             mutable and frozen in the same scope. [E0803]"
                        ),
                    );
                }
                self.unfrozen_stack.push(pool.clone());
                self.push();
                for s in body {
                    self.check_stmt(s);
                }
                self.pop();
                self.unfrozen_stack.pop();
            }
            // SWARM RABBIT HOLE — Wave 13 (Impl-R, Cohort Delta): the
            // WAVE REGION — Thm WS made executable (W2B §4; the typed
            // read-mode surface §3). The block is the composed
            // syntax's wave phase; its checking realizes the
            // theorem's hypotheses as compile-time side conditions:
            //   C1  Rule R1 (quiescent capture) — W2B §4.3 — here;
            //   C2  compiler-enforced wave determinism — the E0812
            //       purity gate (the kill switch), here;
            //   W2  the seam typing — read_W/contrib only in the
            //       body (read_S and the fallback sync ops refused);
            //   W3  the join discipline — J(W) is the block exit
            //       (ir.rs lowers the J2 batched fold).
            // C3/C4 (the fallback side) live at the merge sites (the
            // E-R1 merge rule in check_call) and the read modes.
            Stmt::Wave { body, span } => {
                // (0) Structural prohibitions. A wave inside a spawn
                // family would fold J(W) into module-scope seam cells
                // from a non-joining thread — concurrent with
                // fallback reads, the torn-read class S2 exists to
                // kill; waves are fallback-tier statements. A nested
                // wave is case (b) by construction: neither capture
                // can be quiescent w.r.t. the other.
                if self.spawn_depth > 0 {
                    self.diag.error(
                        *span,
                        "E0812: a wave may not run inside a spawn/nursery body — \
                         J(W) would fold into seam cells from a non-joining thread \
                         (the torn-read class S2 exists to kill); waves are \
                         fallback-tier statements [E0812]",
                    );
                }
                if self.wave_depth > 0 {
                    self.diag.error(
                        *span,
                        "E0812: a wave body may not nest a wave — the wave IS the \
                         parallelism; nest phases by sequencing wave blocks \
                         (J(Wᵢ) → capture(W_{i+1}), WS case (c)) [E0812]",
                    );
                }
                // (1) The capture set: every module-scope Tier-L seam
                // cell named in the body (the conservative
                // over-approximation — an alias `ptr p = seam` names
                // the cell too, so the capture is a superset of the
                // read_W/contrib uses; freezing more is safe: R1
                // constrains captures, and a capture that provably
                // never races is the only thing the rule demands).
                let captured = wave_seam_cells_of(self.db, body);
                // (2) Rule R1, the two-waves arm (W2B §4.6 case (b) —
                // a theorem, not a defect): for every PRIOR claim on a
                // captured cell (claims are collected program-wide at
                // collect, so this is order-independent), the claim
                // must be the SAME function's earlier wave — provably
                // joined, the sequential-composition case (c) — or it
                // imposes nothing on this capture (a later wave in
                // the same function runs after J(W) of this one).
                // Anything else (cross-function, unordered) is the
                // checked refusal: Part 3 is unavailable by
                // construction, and the checker says so.
                let cur_fn = self.current_fn_key.clone();
                for c in &captured {
                    if let Some(claims) = self.db.wave_claims.get(c) {
                        for (fnk, wspan) in claims {
                            if wspan == span {
                                continue; // this block's own claim
                            }
                            let same_fn = Some(fnk.as_str()) == cur_fn.as_deref();
                            if same_fn && wspan.start < span.start {
                                // Same function, textually earlier:
                                // J(W_prev) provably happened (the
                                // sequential walk closed it) — case
                                // (c)'s induction step, LEGAL.
                                continue;
                            }
                            if same_fn && wspan.start > span.start {
                                // Same function, textually later:
                                // imposes nothing on THIS capture.
                                continue;
                            }
                            self.diag.error(
                                *span,
                                format!(
                                    "E-R1: two waves share seam cell '{c}' — \
                                     neither capture can be R1-quiescent w.r.t. the \
                                     other; compose them sequentially [E-R1]"
                                ),
                            );
                            break;
                        }
                    }
                }
                // (3) Record the capture set for the ir.rs lowering
                // (one sn_wave_capture per cell at wave entry).
                self.db
                    .wave_captures
                    .insert((span.file, span.start, span.end), captured.clone());
                // (4) The E0812 purity gate — the kill switch (W2B
                // §4.6 case (d)): the body must be a pure function of
                // the frozen capture κ (Lemma WD's premise; without it
                // the determinism grade degrades UNDETECTED — so it
                // is a compile error, never a warning).
                self.wave_e0812_walk(body, &captured);
                // (5) Check the body as an ordinary scope, with the
                // wave depth open (read_W/contrib legal; the seam
                // typing enforced at their check sites).
                self.wave_depth += 1;
                self.push();
                for s in body {
                    self.check_stmt(s);
                }
                self.pop();
                self.wave_depth -= 1;
                // (6) J(W) is structural: the block's completion is
                // the join. Credit the captured cells in the
                // ENCLOSING frame — the credit dies with the scope
                // (read_J's temporal discipline is lexical; an
                // if-branch wave does not prove the join on the
                // fall-through path), and a later same-function wave
                // may capture them again (case (c)).
                if let Some(frame) = self.joined_stack.last_mut() {
                    frame.extend(captured.iter().cloned());
                }
            }
            Stmt::Guard { cond, else_body, .. } => {
                let ct = self.check_expr(cond);
                if ct != Type::Bool {
                    self.diag.error(cond.span, "guard condition must be bool");
                }
                let narrow = none_narrow(cond);
                self.push();
                if let Some((n, false)) = &narrow {
                    self.rebind_unwrap(n);
                }
                for s in else_body {
                    self.check_stmt(s);
                }
                self.pop();
                if let Some((n, true)) = &narrow {
                    self.rebind_unwrap(n);
                }
            }
            Stmt::NestedFn(f) => {
                let saved = self.current_fn_ret.clone();
                let ty = Type::Fn {
                    params: f.params.iter().map(|p| resolve_type_ast(&p.ty, self.db)).collect(),
                    ret: Box::new(f.ret.as_ref().map(|t| resolve_type_ast(t, self.db)).unwrap_or(Type::Void)),
                };
                self.declare(&f.name, ty, true, f.span);
                self.check_fn(f);
                self.current_fn_ret = saved;
            }
            Stmt::Asm { .. } => {}
        }
    }

    fn check_ctor_args(&mut self, bp: &BlueprintInfo, args: &[Arg], span: Span) {
        for a in args {
            let val_expr = match a {
                Arg::Pos(e) | Arg::Inout(e) => e,
                Arg::Named { value, .. } => value,
            };
            if let Some(local_name) = self.local_address_source(val_expr) {
                if self.scope_depth_of(&local_name).is_some() {
                    self.diag.error(
                        val_expr.span,
                        format!("cannot assign address of local '{local_name}': target outlives declaring scope"),
                    );
                }
            }
            match a {
                Arg::Pos(e) | Arg::Inout(e) => {
                    self.check_expr(e);
                }
                Arg::Named { name, value } => {
                    let vt = self.check_expr(value);
                    if let Some(f) = bp.fields.iter().find(|f| f.name == *name) {
                        if !f.ty.assignable_from(&vt, &self.db.blueprints) {
                            self.diag.error(value.span, "constructor field type mismatch");
                        }
                    } else {
                        self.diag.error(span, format!("unknown field '{name}'"));
                    }
                }
            }
        }
    }

    fn check_expr(&mut self, e: &Expr) -> Type {
        let ty = match &e.kind {
            ExprKind::Int(_) => Type::Int,
            ExprKind::Float(_) => Type::Float,
            ExprKind::Dec { scale, .. } => Type::Dec(*scale),
            ExprKind::Bool(_) => Type::Bool,
            ExprKind::Str(_) => Type::Str,
            ExprKind::None => Type::None,
            ExprKind::Self_ => {
                if self.in_static {
                    self.diag.error(e.span, "'self' not allowed in a static method");
                    return Type::Int;
                }
                if let Some(bp) = &self.current_bp {
                    Type::Blueprint(bp.clone())
                } else if let Some((t, _)) = self.lookup("self") {
                    t
                } else {
                    self.diag.error(e.span, "self outside method");
                    Type::Int
                }
            }
            ExprKind::Ident(n) => {
                if self.freed.contains(n) {
                    self.diag.error(e.span, format!("use of freed pointer '{n}'"));
                } else if self.moved.contains_key(n) {
                    if self.channel_moved.contains(n) {
                        self.diag.error(
                            e.span,
                            format!("use of moved value '{n}': value was transferred over channel"),
                        );
                    } else if let Some((t, _)) = self.lookup(n) {
                        if Self::is_borrow_type(&t) {
                            self.diag.error(e.span, format!("use of moved ref '{n}'"));
                        } else {
                            self.diag.error(
                                e.span,
                                format!("use of moved value '{n}': value was transferred over channel"),
                            );
                        }
                    } else {
                        self.diag.error(
                            e.span,
                            format!("use of moved value '{n}': value was transferred over channel"),
                        );
                    }
                }
                self.check_borrow_epoch(n, e.span);
                if let Some((t, _)) = self.lookup(n) {
                    t
                } else if self.db.blueprints.contains_key(n) {
                    Type::Blueprint(n.to_string())
                } else if let Some(sig) = self.db.funcs.get(n) {
                    Type::Fn {
                        params: sig.params.iter().map(|p| p.1.clone()).collect(),
                        ret: Box::new(sig.ret.clone()),
                    }
                } else if n == "print"
                    || n == "printn"
                    || n == "input"
                    || n == "file_read"
                    || n == "file_write"
                    || n == "file_read_ex"
                    || n == "file_write_ex"
                    || n == "file_append"
                    || n == "file_exists"
                    || n == "file_delete"
                    || n == "file_copy"
                    || n == "file_move"
                    || n == "file_mkdir"
                    || n == "file_rmdir"
                    || n == "file_list"
                    || n == "path_join"
                    || n == "path_base"
                    || n == "path_dir"
                    || n == "path_ext"
                    || n == "os_getenv"
                    || n == "os_exit"
                    || n == "os_args"
                    || n == "os_exec"
                    || n == "os_system"
                    || n == "panic"
                    || n == "alloc"
                    || n == "free"
                    || n == "address"
                    || n == "value"
                    || n == "set"
                    || n == "error"
                    || n == "json_parse"
                    || n == "json_encode"
                    || n == "http_get"
                    || n == "http_post"
                    || n == "http_serve_once"
                    || n == "time_now_ms"
                    || n == "time_sleep_ms"
                    || n == "time_format"
                    || n == "sleep_async"
                    || n == "get_async"
                    || n == "future_data"
                    || n == "tcp_listen"
                    || n == "tcp_listen_any"
                    || n == "tcp_local_port"
                    || n == "tcp_connect"
                    || n == "tcp_accept"
                    || n == "tcp_read"
                    || n == "tcp_write"
                    || n == "tcp_close"
                    || n == "select_read"
                    || n == "ptr_int"
                    || n == "bytearray_new"
                    || n == "sb_new"
                    || n == "sb_append"
                    || n == "sb_append_str"
                    || n == "sb_append_cstr"
                    || n == "sb_append_int"
                    || n == "sb_append_byte"
                    || n == "sb_to_str"
                    || n == "sb_len"
                    || n == "sb_clear"
                    || n == "bytearray_len"
                    || n == "bytearray_reserve"
                    || n == "bytearray_push"
                    || n == "bytearray_get"
                    || n == "bytearray_set"
                    || n == "bytearray_append_bytes"
                    || n == "bytearray_append_str"
                    || n == "bytearray_slice"
                    || n == "bytearray_write_at"
                    || n == "bytearray_find"
                    || n == "bytearray_rfind"
                    || n == "bytearray_truncate"
                    || n == "bytearray_clear"
                    || n == "bytearray_data"
                    || n == "bytearray_fill"
                    || n == "sha256"
                    || n == "digest_sha256"
                    || n == "digest_sha512"
                    || n == "digest_md5"
                    || n == "digest_hmac_sha256"
                    || n == "sha512"
                    || n == "md5"
                    || n == "hmac_sha256"
                    || n == "crc32"
                    || n == "crc32_update"
                    || n == "digest_crc32"
                    || n == "digest_crc32_update"
                    || n == "consttime_eq"
                    || n == "secure_zero"
                    || n == "gc_count"
                    || n == "free_checked"
                    || n == "check_live"
                    || n == "alloc_size"
                    || n == "alloc_live_count"
                    || n == "alloc_freed_count"
                    || n == "alloc_guard"
                    || n == "gc_collect"
                    || n == "gc_set_threshold"

                    || n == "ptr_load"
                    || n == "ptr_store"
                    || n == "ptr_load_byte"
                    || n == "ptr_store_byte"
                    || n == "ptr_load_f64"
                    || n == "ptr_store_f64"
                    || n == "ptr_load_str"
                    || n == "ptr_store_str"
                    || n == "ptr_alloc"
                    || n == "int_ptr"
                    || n == "ptr_free"
                    || n == "any_box"
                    || n == "hton16" || n == "ntoh16"
                    || n == "hton32" || n == "ntoh32"
                    || n == "hton64" || n == "ntoh64"
                    || n == "swap16" || n == "swap32" || n == "swap64"
                    || n == "lane_alloc"
                    || n == "lane_save"
                    || n == "lane_restore"
                    || n == "lane_used"
                    || n == "lane_freeze"
                    || n == "lane_unfreeze"
                    || n == "pool_new"
                    || n == "pool_insert"
                    || n == "pool_get"
                    || n == "pool_set"
                    || n == "pool_remove"
                    || n == "pool_is_valid"
                    || n == "pool_len"
                    || n == "zset_new"
                    || n == "zset_add"
                    || n == "zset_remove"
                    || n == "zset_merge"
                    || n == "zset_delta"
                    || n == "zset_apply"
                    || n == "zset_copy"
                    || n == "zset_count"
                    || n == "zset_total"
                    || n == "zset_len"
                    || n == "zset_keys"
                    || n == "zset_set_prune"
                    /* Tier-L join-semilattice builtins (SWARM RABBIT
                       HOLE, Wave 9, Impl-M; W2C §1.1 Tier L). lcell_retract
                       is RECOGNIZED but ALWAYS rejected in check_call —
                       the (L, exact) capability declaration, E-LRET. */
                    || n == "lcell_new"
                    || n == "lcell_inflate"
                    || n == "lcell_merge"
                    || n == "lcell_delta"
                    || n == "lcell_apply"
                    || n == "lcell_len"
                    || n == "lcell_value"
                    || n == "lcell_has"
                    || n == "lcell_carrier"
                    || n == "lcell_retract"
                    /* Salt-ζ fixpoint builtins (SWARM RABBIT HOLE, Wave 9,
                       Impl-N; W2C §3.3–§3.5). zfix_count / zfix_negate —
                       the two named divergence classes — are RECOGNIZED
                       but ALWAYS rejected in check_call with E-ZADM (the
                       admission checker: refusals are compile-time
                       errors, never silent divergence). */
                    || n == "zfix_new"
                    || n == "zfix_new_l"
                    || n == "zfix_round"
                    || n == "zfix_run"
                    || n == "zfix_rounds"
                    || n == "zfix_len"
                    || n == "zfix_has"
                    || n == "zfix_probes"
                    || n == "zfix_state"
                    || n == "zfix_quiescent_g"
                    || n == "zfix_quiescent_l"
                    || n == "zfix_count"
                    || n == "zfix_negate"
                    /* SFC layout builtins (SWARM RABBIT HOLE, Wave 14,
                       Impl-S, Cohort Delta; A2 §2.4's decision table —
                       morton list = tensor iteration, hilbert map =
                       range queries). The pure index arithmetic
                       (morton_index/x/y, hilbert_code/dx/dy) and the
                       collection ops; the arg-type and dims rules fire
                       in check_call's arms below. */
                    || n == "morton_new"
                    || n == "morton_from_lex"
                    || n == "morton_from_bytes"
                    || n == "morton_get"
                    || n == "morton_set"
                    || n == "morton_index"
                    || n == "morton_x"
                    || n == "morton_y"
                    || n == "morton_len"
                    || n == "morton_scan_get"
                    || n == "morton_to_bytes"
                    || n == "hilbert_new"
                    || n == "hilbert_code"
                    || n == "hilbert_dx"
                    || n == "hilbert_dy"
                    || n == "hilbert_insert"
                    || n == "hilbert_get"
                    || n == "hilbert_has"
                    || n == "hilbert_len"
                    || n == "hilbert_entry_key"
                    || n == "hilbert_entry_x"
                    || n == "hilbert_entry_y"
                    || n == "hilbert_entry_val"
                    || n == "hilbert_box_first"
                    || n == "hilbert_box_next"
                    || n == "sfc_path"
                {
                    Type::Named(format!("builtin:{n}"))
                } else {
                    self.diag.error(e.span, format!("unknown name '{n}'"));
                    Type::Int
                }
            }
            ExprKind::Binary { op, lhs, rhs } => self.check_bin(*op, lhs, rhs),
            ExprKind::Unary { op, expr } => {
                let t = self.check_expr(expr);
                match op {
                    UnOp::Neg => {
                        if !matches!(
                            t,
                            Type::Int
                                | Type::I8
                                | Type::I16
                                | Type::I32
                                | Type::U8
                                | Type::U16
                                | Type::U32
                                | Type::U64
                                | Type::Byte
                                | Type::Float
                                | Type::Dec(_)
                        ) {
                            self.diag.error(e.span, "cannot negate this type");
                        }
                        t
                    }
                    UnOp::Not => {
                        if t != Type::Bool {
                            self.diag.error(e.span, "not requires bool");
                        }
                        Type::Bool
                    }
                    UnOp::BitNot => {
                        if !t.is_fixed_int() {
                            self.diag.error(e.span, "bitwise not requires integer type");
                        }
                        t
                    }
                }
            }
            ExprKind::Call { callee, type_args, args } => self.check_call(callee, type_args, args, e.span),
            ExprKind::Index { base, index } => {
                let bt = self.check_expr(base);
                let it = self.check_expr(index);
                match bt {
                    Type::List(el) => {
                        if it != Type::Int && !it.is_fixed_int() {
                            self.diag.error(index.span, "list index must be int");
                        }
                        *el
                    }
                    Type::Map(k, v) => {
                        if it != *k && !(k.is_fixed_int() && (it == Type::Int || it.is_fixed_int())) {
                            self.diag.error(index.span, format!("map key must be {k}"));
                        }
                        *v
                    }
                    Type::Str => {
                        if it != Type::Int && !it.is_fixed_int() {
                            self.diag.error(index.span, "string index must be int");
                        }
                        Type::Str
                    }
                    other => {
                        self.diag.error(base.span, format!("cannot index {other}"));
                        Type::Int
                    }
                }
            }
            ExprKind::Member { base, name } => {
                let t = self.check_member(base, name, e.span);
                if matches!(base.kind, ExprKind::OptionalChain(_))
                    && !matches!(t, Type::Optional(_) | Type::Void)
                {
                    Type::Optional(Box::new(t))
                } else {
                    t
                }
            }
            ExprKind::Interpolate { parts } => {
                for p in parts {
                    if let InterpPart::Expr(ex) = p {
                        self.check_expr(ex);
                    }
                }
                Type::Str
            }
            ExprKind::List(els) => {
                let mut et = None;
                for el in els {
                    let t = self.check_expr(el);
                    // SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort
                    // Delta): the E-GC-IN-LANE store rule, list-
                    // literal form — a lane-backed SFC collection
                    // cannot enter a list (the GC-managed holder
                    // would release lane bytes as heap).
                    if matches!(t, Type::Morton(_, _) | Type::Hilbert(_, _)) {
                        let src = Self::expr_label(el);
                        self.diag.error(
                            el.span,
                            format!(
                                "E-GC-IN-LANE: cannot store '{src}' into a list \
                                 literal — a lane-backed SFC collection cannot enter \
                                 a GC-managed holder (the collector cannot trace \
                                 lane memory and the holder's release would free it); \
                                 SFC collections live in their region's lane and die \
                                 at its rewind [E-GC-IN-LANE]"
                            ),
                        );
                    }
                    match &et {
                        None => et = Some(t),
                        Some(prev) if prev != &t => {
                            self.diag.error(el.span, "list elements must share a type");
                        }
                        _ => {}
                    }
                }
                Type::List(Box::new(et.unwrap_or(Type::Any)))
            }
            ExprKind::Map(entries) => {
                let mut kt = None;
                let mut vt = None;
                for (k, v) in entries {
                    let kty = self.check_expr(k);
                    let vty = self.check_expr(v);
                    if kt.as_ref().is_some_and(|t| t != &kty) {
                        self.diag.error(k.span, "map keys must share a type");
                    }
                    if vt.as_ref().is_some_and(|t| t != &vty) {
                        self.diag.error(v.span, "map values must share a type");
                    }
                    kt = Some(kty);
                    vt = Some(vty);
                }
                if entries.is_empty() {
                    Type::Map(
                        Box::new(Type::Any),
                        Box::new(Type::Any),
                    )
                } else {
                    Type::Map(
                        Box::new(kt.unwrap_or(Type::Str)),
                        Box::new(vt.unwrap_or(Type::Int)),
                    )
                }
            }
            ExprKind::Tuple(xs) => Type::Tuple(xs.iter().map(|x| self.check_expr(x)).collect()),
            ExprKind::Cast { expr, ty } => {
                self.check_expr(expr);
                resolve_type_ast(ty, self.db)
            }
            ExprKind::SuperCall { method, args } => {
                let bp = self.current_bp.clone().unwrap_or_default();
                let parent = self
                    .db
                    .blueprints
                    .get(&bp)
                    .and_then(|b| b.parent.clone())
                    .unwrap_or_default();
                if parent.is_empty() {
                    self.diag.error(e.span, "super outside inherited method");
                    return Type::Void;
                }
                let key = format!("{parent}::{method}");
                let Some(sig) = self.db.funcs.get(&key).cloned() else {
                    self.diag.error(e.span, format!("super has no method '{method}'"));
                    return Type::Void;
                };
                self.check_arity(&sig, args, e.span);
                for a in args {
                    if let Arg::Pos(ex) | Arg::Inout(ex) = a {
                        self.check_expr(ex);
                    } else if let Arg::Named { value, .. } = a {
                        self.check_expr(value);
                    }
                }
                sig.ret
            }
            ExprKind::Await(inner) => {
                let t = self.check_expr(inner);
                if !self.in_async && self.current_fn_ret != Type::Void {
                    // allow await in main for MVP async I/O demos
                }
                match t {
                    Type::Named(n) if n == "future" => Type::Int,
                    _ => {
                        self.diag.error(inner.span, format!("cannot await non-future type '{t}'"));
                        Type::Int
                    }
                }
            }
            ExprKind::Try(inner) => {
                let t = self.check_expr(inner);
                match t {
                    Type::Tuple(ts) if ts.len() == 2 && ts[1] == Type::Error => ts[0].clone(),
                    Type::Error => {
                        self.diag.error(e.span, "try expects (T, error), not error alone");
                        Type::Error
                    }
                    other => {
                        self.diag.error(
                            e.span,
                            format!("try expects (T, error), got {other}"),
                        );
                        other
                    }
                }
            }
            ExprKind::OptionalChain(inner) => {
                let t = self.check_expr(inner);
                if matches!(t, Type::Optional(_)) {
                    t
                } else {
                    Type::Optional(Box::new(t))
                }
            }
            ExprKind::ForceUnwrap(inner) => {
                let t = self.check_expr(inner);
                match t {
                    Type::Optional(inner_ty) => *inner_ty,
                    Type::Error => {
                        self.diag.error(e.span, "use try or match for error, not !!");
                        Type::Error
                    }
                    other => {
                        self.diag
                            .error(e.span, format!("!! requires T?, got {other}"));
                        other
                    }
                }
            }
            ExprKind::Lambda { params, ret, body } => {
                let saved = self.current_fn_ret.clone();
                self.push();
                for p in params {
                    self.declare(&p.name, resolve_type_ast(&p.ty, self.db), false, e.span);
                }
                self.current_fn_ret = ret.as_ref().map(|t| resolve_type_ast(t, self.db)).unwrap_or(Type::Void);
                for s in body {
                    self.check_stmt(s);
                }
                self.pop();
                self.current_fn_ret = saved;
                Type::Fn {
                    params: params.iter().map(|p| resolve_type_ast(&p.ty, self.db)).collect(),
                    ret: Box::new(ret.as_ref().map(|t| resolve_type_ast(t, self.db)).unwrap_or(Type::Void)),
                }
            }
        };
        self.set_ty(e.span, ty)
    }

    fn int_lit_value(e: &Expr) -> Option<i64> {
        match &e.kind {
            ExprKind::Int(n) => Some(*n),
            ExprKind::Unary { op, expr } if *op == UnOp::Neg => {
                match &expr.kind {
                    ExprKind::Int(n) => Some(-*n),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /* SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the SFC
       intrinsic arms' argument readers. ident_call pre-checks every
       positional argument (check_expr at the call head), so the
       recorded expr_ty is the checked type; these helpers read it
       back and report the E-SFC arg-type mismatches at the call's
       span (the verbatim arms the golden suite pins). */
    fn sfc_arg_ty(&self, args: &[Arg], i: usize) -> Type {
        args.get(i)
            .and_then(|a| match a {
                Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => self
                    .db
                    .expr_ty
                    .get(&(e.span.file, e.span.start, e.span.end))
                    .cloned(),
                _ => None,
            })
            .unwrap_or(Type::Int)
    }

    fn sfc_lit_arg(&self, args: &[Arg], i: usize) -> Option<i64> {
        args.get(i).and_then(|a| match a {
            Arg::Pos(e) => Self::int_lit_value(e),
            _ => None,
        })
    }

    fn sfc_int_arg(&mut self, args: &[Arg], i: usize, op: &str, span: Span) {
        let t = self.sfc_arg_ty(args, i);
        if !t.is_fixed_int() && !matches!(t, Type::Int | Type::Any) {
            self.diag
                .error(span, format!("{op}'s argument {} must be an int, got {t}", i + 1));
        }
    }

    fn sfc_morton_arg(&mut self, args: &[Arg], i: usize, op: &str, span: Span) {
        let t = self.sfc_arg_ty(args, i);
        if !matches!(t, Type::Morton(_, _)) {
            self.diag.error(
                span,
                format!(
                    "{op} operates on a morton list (morton list<T, dims=[n1, n2]>), got {t}"
                ),
            );
        }
    }

    fn sfc_hilbert_arg(&mut self, args: &[Arg], i: usize, op: &str, span: Span) {
        let t = self.sfc_arg_ty(args, i);
        if !matches!(t, Type::Hilbert(_, _)) {
            self.diag
                .error(span, format!("{op} operates on a hilbert map<K, V>, got {t}"));
        }
    }

    fn check_bin(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) -> Type {
        let lt = self.check_expr(lhs);
        let rt = self.check_expr(rhs);
        match op {
            BinOp::Otherwise => {
                if let Some(inner) = lt.unwrap_optional() {
                    if inner != &rt && rt != Type::None {
                        self.diag.error(rhs.span, "otherwise type mismatch");
                    }
                    inner.clone()
                } else if lt == Type::None {
                    rt
                } else {
                    self.diag
                        .error(lhs.span, "otherwise requires a nullable left operand");
                    rt
                }
            }
            BinOp::And | BinOp::Or => {
                if lt != Type::Bool || rt != Type::Bool {
                    self.diag.error(lhs.span, "logical ops require bool");
                }
                Type::Bool
            }
            BinOp::Eq | BinOp::Ne => {
                if let (Type::Enum(a), Type::Enum(b)) = (&lt, &rt) {
                    if a != b {
                        self.diag.error(
                            lhs.span,
                            format!("cannot compare enums '{a}' and '{b}'"),
                        );
                    }
                }
                Type::Bool
            }
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                let float_mix = (lt == Type::Float && (matches!(rt, Type::Dec(_) | Type::Int) || rt.int_min_max().is_some()))
                    || (rt == Type::Float && (matches!(lt, Type::Dec(_) | Type::Int) || lt.int_min_max().is_some()));
                if lt != rt && !(lt.int_min_max().is_some() && rt.int_min_max().is_some()) && !float_mix {
                    self.diag.error(lhs.span, "comparison type mismatch");
                }
                Type::Bool
            }
            BinOp::Add if lt == Type::Str && rt == Type::Str => Type::Str,
            BinOp::In => {
                match (&lt, &rt) {
                    (elem, Type::List(el)) if **el == *elem || el.assignable_from(elem, &self.db.blueprints) => Type::Bool,
                    (Type::Str, Type::Str) => Type::Bool,
                    (elem, Type::Map(k, _)) if **k == *elem || k.assignable_from(elem, &self.db.blueprints) => Type::Bool,
                    _ => {
                        self.diag.error(
                            lhs.span,
                            format!("'in' requires elem in list<T>, substring in str, or key in map<K, V>, got {lt} and {rt}"),
                        );
                        Type::Bool
                    }
                }
            }
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div if lt == Type::Float && rt == Type::Float => {
                Type::Float
            }
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow => {
                if lt == Type::Float || rt == Type::Float {
                    if lt != Type::Float || rt != Type::Float {
                        // allow Int literal / Dec promotion to float at type level
                        let lit_ok = (lt == Type::Float && (Self::int_lit_value(rhs).is_some() || matches!(rt, Type::Dec(_))))
                            || (rt == Type::Float && (Self::int_lit_value(lhs).is_some() || matches!(lt, Type::Dec(_))));
                        if !lit_ok {
                            self.diag.error(lhs.span, format!("float arithmetic on {lt} and {rt} (both must be float)"));
                        }
                    }
                    return Type::Float;
                }
                if lt != rt {
                    // mixed fixed-width ints: C-style promotion to int (i64)
                    if lt.int_min_max().is_some() && rt.int_min_max().is_some() {
                        return Type::Int;
                    }
                    // legacy int/byte mixing allowed
                    let legacy = matches!((&lt, &rt), (Type::Int, Type::Byte) | (Type::Byte, Type::Int));
                    // Int literal may mix with any fixed int if in range
                    let mut lit_mix = false;
                    if lt.is_fixed_int() && rt == Type::Int {
                        if let Some(n) = Self::int_lit_value(rhs) {
                            lit_mix = lt.int_range().map(|(lo, hi)| n >= lo && n <= hi).unwrap_or(true) && !(matches!(lt, Type::U64) && n < 0);
                            if !lit_mix {
                                self.diag.error(rhs.span, format!("integer {n} out of range for {lt}"));
                            }
                        }
                    } else if rt.is_fixed_int() && lt == Type::Int {
                        if let Some(n) = Self::int_lit_value(lhs) {
                            lit_mix = rt.int_range().map(|(lo, hi)| n >= lo && n <= hi).unwrap_or(true) && !(matches!(rt, Type::U64) && n < 0);
                            if !lit_mix {
                                self.diag.error(lhs.span, format!("integer {n} out of range for {rt}"));
                            }
                        }
                    }
                    if !legacy && !lit_mix {
                        // if one side is a plain Int literal and other is fixed, accept as fixed
                        let accept = (lt.is_fixed_int() && Self::int_lit_value(rhs).is_some())
                            || (rt.is_fixed_int() && Self::int_lit_value(lhs).is_some());
                        if !accept {
                            self.diag.error(lhs.span, format!("arithmetic on {lt} and {rt}"));
                        }
                    }
                    if lt.is_fixed_int() {
                        return lt;
                    }
                    if rt.is_fixed_int() {
                        return rt;
                    }
                }
                lt
            }
            BinOp::Shl | BinOp::Shr => {
                if !lt.is_fixed_int() {
                    self.diag.error(
                        lhs.span,
                        format!("shift target must be an integer, got {lt}"),
                    );
                }
                if !rt.is_fixed_int() {
                    self.diag.error(
                        rhs.span,
                        format!("shift amount must be an integer, got {rt}"),
                    );
                }
                lt
            }
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                let mut lt2 = lt.clone();
                let rt2 = rt.clone();
                // allow Int literal to take the fixed type of the other side
                if lt2 == Type::Int && rt2.is_fixed_int() && Self::int_lit_value(lhs).is_some() {
                    lt2 = rt2.clone();
                } else if rt2 == Type::Int && lt2.is_fixed_int() && Self::int_lit_value(rhs).is_some() {
                    // keep lt2
                }
                let ok = lt2.is_fixed_int() && (rt2.is_fixed_int() || rt2 == Type::Int);
                if !ok {
                    self.diag.error(
                        lhs.span,
                        format!("bitwise ops require integer types, got {lt} and {rt}"),
                    );
                }
                if lt != rt {
                    let lit_ok = (lt.is_fixed_int() && Self::int_lit_value(rhs).is_some())
                        || (rt.is_fixed_int() && Self::int_lit_value(lhs).is_some());
                    if !lit_ok {
                        self.diag.error(lhs.span, format!("bitwise on {lt} and {rt}"));
                    }
                }
                if lt.is_fixed_int() {
                    return lt;
                }
                if rt.is_fixed_int() {
                    return rt;
                }
                lt
            }
        }
    }

    /* SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): the link
       intrinsics' checker surface (kills G1 §1 A2 → S4). The
       self-relative link system's runtime half: the block carriers
       (contiguous records with `link<T, u16>` fields — the
       self-contained slice of the spec's model), the position-checked
       encode/decode, relocate_node/relocate_range (the re-encode
       semantics + the constant-shift exemption), and the to_bytes
       sender-side scan. The checker validates the record type
       argument's layout (link fields, the ≤4 bound the runtime calls
       carry) and the static link-field ordinals; the ir.rs lowering
       reads the same layout and emits the runtime calls with the
       offsets, field names, target labels and source labels the
       verbatim E0801 diagnostics name. */
    fn check_link_intrinsic(
        &mut self,
        n: &str,
        type_args: &[TypeAst],
        args: &[Arg],
        span: Span,
    ) -> Option<Type> {
        const LINK_INTRINSICS: [&str; 8] = [
            "link_block_new",
            "link_set",
            "link_get",
            "link_elem",
            "relocate_node",
            "relocate_range",
            "link_to_bytes",
            "link_to_bytes_raw",
        ];
        if !LINK_INTRINSICS.contains(&n) {
            return None;
        }
        let arity = match n {
            "link_block_new" => 1, // (count) — the size comes from the type
            "link_elem" => 2,     // (block, index)
            "link_get" => 3,      // (block, index, link-field ordinal)
            "relocate_node" => 4, // (src, si, dst, di)
            "relocate_range" | "link_to_bytes" | "link_to_bytes_raw" => 3,
            _ => 5, // link_set: (block, i, ordinal, tblock, ti)
        };
        if args.len() != arity {
            self.diag
                .error(span, format!("'{n}' expects {arity} args, got {}", args.len()));
            return Some(Type::Ptr);
        }
        let Some(ta) = type_args.first() else {
            self.diag.error(
                span,
                format!("'{n}' requires an explicit record type argument: {n}<Record>(...)"),
            );
            return Some(Type::Ptr);
        };
        let t = resolve_type_ast(ta, self.db);
        let Type::Record(rec) = &t else {
            self.diag.error(
                span,
                format!(
                    "'{n}' requires a record type argument (the link carriers are \
                     records with 'link<T, u16>' fields); got {t}"
                ),
            );
            return Some(Type::Ptr);
        };
        let links = self.db.link_fields_of(rec);
        if links.len() > 4 {
            self.diag.error(
                span,
                format!(
                    "record '{rec}' has {} link fields; the link intrinsics \
                     support at most 4 per record (the runtime calls carry a \
                     fixed offset/name window)",
                    links.len()
                ),
            );
            return Some(Type::Ptr);
        }
        // link_set/link_get name the link field by its 0-based ordinal
        // among the record's link fields — a static index, so the
        // lowering can emit the byte offset as an immediate.
        if n == "link_set" || n == "link_get" {
            let field_arg = match args.get(2) {
                Some(Arg::Pos(e)) => e,
                _ => {
                    self.diag.error(
                        span,
                        format!("'{n}''s link-field ordinal must be a positional integer literal"),
                    );
                    return Some(Type::Int);
                }
            };
            let Some(idx) = Self::int_lit_value(field_arg) else {
                self.diag.error(
                    field_arg.span,
                    format!(
                        "'{n}''s link-field ordinal must be an integer literal \
                         (0-based, among '{rec}''s link fields)"
                    ),
                );
                return Some(Type::Int);
            };
            if links.is_empty() {
                self.diag.error(
                    field_arg.span,
                    format!(
                        "record '{rec}' has no 'link<T, u16>' fields; there is no \
                         link field for '{n}' to name"
                    ),
                );
            } else if idx < 0 || idx as usize >= links.len() {
                self.diag.error(
                    field_arg.span,
                    format!(
                        "link-field ordinal {idx} out of range: record '{rec}' has \
                         {} link field(s)",
                        links.len()
                    ),
                );
            }
        }
        Some(match n {
            "link_block_new" | "link_elem" | "link_to_bytes" | "link_to_bytes_raw" => Type::Ptr,
            "link_get" => Type::Int,
            _ => Type::Void,
        })
    }

    fn check_call(&mut self, callee: &Expr, type_args: &[TypeAst], args: &[Arg], span: Span) -> Type {
        if let ExprKind::Member { base, name } = &callee.kind {
            // Static call: BlueprintName.static_method(...)
            if let ExprKind::Ident(bpname) = &base.kind {
                if self.lookup(bpname).is_none() && self.db.blueprints.contains_key(bpname) {
                    for a in args {
                        match a {
                            Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => {
                                self.check_expr(e);
                            }
                        }
                    }
                    let Some(sig) = self.blueprint_sig(bpname, name).cloned() else {
                        self.diag.error(span, format!("unknown method '{name}' on {bpname}"));
                        return Type::Void;
                    };
                    if !sig.is_static {
                        self.diag.error(span, format!("'{name}' is not static; call it on an instance"));
                        return sig.ret;
                    }
                    self.check_arity(&sig, args, span);
                    return sig.ret;
                }
                if self.lookup(bpname).is_none() && self.db.enums.contains_key(bpname) {
                    if let Some(variant_infos) = self.db.enum_variants.get(bpname).cloned() {
                        if let Some(var) = variant_infos.iter().find(|v| &v.name == name) {
                            if var.fields.len() != args.len() {
                                self.diag.error(
                                    span,
                                    format!(
                                        "enum variant '{bpname}.{name}' expects {} arguments, got {}",
                                        var.fields.len(),
                                        args.len()
                                    ),
                                );
                            } else {
                                for (i, a) in args.iter().enumerate() {
                                    let arg_expr = match a {
                                        Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => e,
                                    };
                                    let at = self.check_expr(arg_expr);
                                    let expected_ty = &var.fields[i].1;
                                    if !expected_ty.assignable_from(&at, &self.db.blueprints) && expected_ty != &at {
                                        self.diag.error(
                                            arg_expr.span,
                                            format!(
                                                "argument {} for '{bpname}.{name}' expected {}, got {}",
                                                i + 1,
                                                expected_ty,
                                                at
                                            ),
                                        );
                                    }
                                }
                            }
                            return Type::Enum(bpname.clone());
                        }
                    }
                }
            }
            let bt = self.check_expr(base);
            for a in args {
                match a {
                    Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => {
                        self.check_expr(e);
                    }
                }
            }
            if matches!(bt, Type::List(_) | Type::Map(_, _)) {
                if matches!(name.as_str(), "push" | "push_ref" | "insert" | "clear" | "pop" | "truncate" | "remove") {
                    if let Some(root) = Self::root_var_of_expr(base) {
                        let borrower = self.borrowed.get(&root).map(|(b, _)| b.clone()).or_else(|| {
                            self.borrow_owner.iter().find(|(_, o)| *o == &root).map(|(b, _)| b.clone())
                        });
                        if let Some(borrower) = borrower {
                            self.diag.error(
                                span,
                                format!("cannot mutate '{root}' while borrowed by '{borrower}'"),
                            );
                        }
                        // SWARM RABBIT HOLE — Wave 6 (Impl-E, DIFF-1 Rule 1a
                        // step 1 / W2A §1.3 S6): growth on a path rooted at
                        // THIS function's own inout param records pushes(f, i)
                        // — the scc_accum_params seed. Growth is NOT an epoch
                        // tick (growth never invalidates refs, STRATA:75);
                        // this is the growth axis the shrinks-only summary
                        // missed (G1 A5a's hole).
                        if matches!(name.as_str(), "push" | "push_ref" | "insert") {
                            if let (Some(key), Some((_, idx))) = (
                                self.current_fn_key.clone(),
                                self.current_inout_params.iter().find(|(pn, _)| pn == &root),
                            ) {
                                self.db.fn_growth_seeds.entry(key).or_default().insert(*idx);
                            }
                            // SWARM RABBIT HOLE — Wave 8 (Impl-J): W2A
                            // §1.5 T-PUSH — the escape rule at the
                            // container store sites. push/insert store
                            // the argument into a cell of `root`'s
                            // container, so a ref-valued argument's
                            // target region must ⊑ₗ root's region
                            // (§4.1 P-I's RI invariant); a violation is
                            // E-ESCAPE. (push_ref's argument is the
                            // elem type — escape_check_store's type
                            // gate skips it; pool insert is
                            // handle/slot_gen tier, not ⊑ₗ.)
                            // Wave 8 (Impl-L): the GC-scoping companion
                            // (E-GC-IN-LANE, D0 gap #6) runs alongside —
                            // same container sites, category rule.
                            for a in args {
                                if let Arg::Pos(e) = a {
                                    self.escape_check_store(e, &root, e.span);
                                    self.gc_scoping_check_store(e, &root, e.span);
                                }
                            }
                        }
                    }
                }
            }
            if matches!(bt, Type::List(_)) && (name == "pop" || name == "clear" || name == "truncate") {
                if let Some(root) = Self::root_var_of_expr(base) {
                    let ep = self.list_epochs.entry(root).or_insert(0);
                    *ep += 1;
                }
            }
            let t = self.method_ret(base, &bt, name, args, span);
            if matches!(base.kind, ExprKind::OptionalChain(_))
                && !matches!(t, Type::Optional(_) | Type::Void)
            {
                return Type::Optional(Box::new(t));
            }
            return t;
        }
        if let ExprKind::Ident(n) = &callee.kind {
            if let Some(template) = self.db.fn_templates.get(n).cloned() {
                let mut effective_args = type_args.to_vec();
                if effective_args.is_empty() {
                    let type_param_names = template_param_names(&template.type_params);
                    let mut inferred: HashMap<String, TypeAst> = HashMap::new();
                    for (i, p) in template.params.iter().enumerate() {
                        if let Some(arg) = args.get(i) {
                            let arg_expr = match arg {
                                Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => e,
                            };
                            let aty = self.check_expr(arg_expr);
                            infer_type_param(&p.ty, &aty, &type_param_names, &mut inferred);
                        }
                    }
                    for tp in &template.type_params {
                        if let Some(t) = inferred.get(&tp.name) {
                            effective_args.push(t.clone());
                        } else {
                            self.diag.error(
                                span,
                                format!("could not infer type argument for '{}' on '{}'", tp.name, n),
                            );
                            effective_args.push(TypeAst::Int);
                        }
                    }
                } else if effective_args.len() != template.type_params.len() {
                    self.diag.error(
                        span,
                        format!(
                            "generic function '{n}' expects {} type arguments, got {}",
                            template.type_params.len(),
                            effective_args.len()
                        ),
                    );
                }
                check_type_arg_bounds_fn(self.db, &template, &effective_args, span, self.diag);
                let mono = self.instantiate_fn(&template, &effective_args, span);
                self.db.resolved_calls.insert((span.file, span.start, span.end), mono.clone());
                self.db.resolved_calls.insert((callee.span.file, callee.span.start, callee.span.end), mono.clone());
                for a in args {
                    match a {
                        Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => {
                            self.check_expr(e);
                        }
                    }
                }
                if let Some(sig) = self.db.funcs.get(&mono).cloned() {
                    self.check_arity(&sig, args, span);
                    return sig.ret;
                }
            }
            if n == "free" {
                if args.len() != 1 {
                    self.diag.error(span, "free expects 1 arg");
                    return Type::Void;
                }
                let arg = &args[0];
                let arg_expr = match arg {
                    Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => e,
                };
                if let Some(p_name) = Self::root_var_of_expr(arg_expr) {
                    if self.freed.contains(&p_name) {
                        self.diag.error(span, format!("double free of '{p_name}'"));
                        return Type::Void;
                    }
                    self.check_expr(arg_expr);
                    self.freed.insert(p_name.clone());
                    self.moved.insert(p_name, span);
                    return Type::Void;
                }
            }
            for a in args {
                match a {
                    Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => {
                        self.check_expr(e);
                    }
                }
            }
            if n == "pool_new" {
                if let Some(arg) = type_args.first() {
                    return Type::Pool(Box::new(resolve_type_ast(arg, self.db)));
                }
                return Type::Ptr;
            }
            // SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): the link
            // intrinsics — the runtime half of the E0801 seam closure
            // (relocate_node / relocate_range / the to_bytes scan, plus
            // the block constructors the discipline's tests and
            // benchmarks drive). Recognized here (before ident_call)
            // because they all carry an explicit record type argument
            // whose layout the checker validates.
            if let Some(t) = self.check_link_intrinsic(n, type_args, args, span) {
                return t;
            }
            return self.ident_call(n, args, span);
        }
        let ct = self.check_expr(callee);
        for a in args {
            match a {
                Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => {
                    self.check_expr(e);
                }
            }
        }
        match ct {
            Type::Fn { ret, params } => {
                if args.len() != params.len() {
                    self.diag.error(
                        span,
                        format!("function value expects {} args, got {}", params.len(), args.len()),
                    );
                }
                *ret
            }
            _ => Type::Void,
        }
    }

    fn ident_call(&mut self, n: &str, args: &[Arg], span: Span) -> Type {
        // A function brought in by `use` (or declared in the file) shadows a
        // builtin of the same name, which is what lets the stdlib wrap
        // builtins like `sha256`.
        if let Some(sig) = self.db.funcs.get(n).cloned() {
            self.check_arity(&sig, args, span);
            if sig.is_extern {
                self.check_extern_args(&sig, args, span);
            }
            return sig.ret;
        }
        match n {
            "print" | "printn" | "free" | "panic" => Type::Void,
            "set" => {
                if args.len() != 2 {
                    self.diag.error(span, "set expects 2 args: set(ptr, value)");
                    return Type::Void;
                }
                if let (Some(Arg::Pos(p_expr)), Some(Arg::Pos(v_expr))) = (args.first(), args.get(1)) {
                    let pt = self.db.expr_ty.get(&(p_expr.span.file, p_expr.span.start, p_expr.span.end)).cloned().unwrap_or(Type::Ptr);
                    let vt = self.db.expr_ty.get(&(v_expr.span.file, v_expr.span.start, v_expr.span.end)).cloned().unwrap_or(Type::Int);
                    match &pt {
                        Type::Ref(inner) | Type::Mut(inner) => {
                            let int_compat = matches!(**inner, Type::Byte | Type::Int | Type::I8 | Type::I16 | Type::I32 | Type::U8 | Type::U16 | Type::U32 | Type::U64)
                                && matches!(vt, Type::Byte | Type::Int | Type::I8 | Type::I16 | Type::I32 | Type::U8 | Type::U16 | Type::U32 | Type::U64);
                            if !inner.assignable_from(&vt, &self.db.blueprints) && !int_compat {
                                self.diag.error(v_expr.span, format!("cannot store {vt} into pointer to {inner}"));
                            }
                        }
                        Type::Ptr => {}
                        other => {
                            self.diag.error(p_expr.span, format!("set requires a ref<T>, mut<T>, or ptr, got {other}"));
                        }
                    }
                }
                Type::Void
            }
            "input" | "file_read" => Type::Str,
            "file_write" => Type::Bool,
            "file_read_ex" => Type::Tuple(vec![Type::Str, Type::Error]),
            "file_write_ex" | "file_append" | "file_delete" | "file_copy" | "file_move"
            | "file_mkdir" | "file_rmdir" => Type::Error,
            "file_exists" => Type::Bool,
            "file_list" => Type::Tuple(vec![Type::List(Box::new(Type::Str)), Type::Error]),
            "path_join" | "path_base" | "path_dir" | "path_ext" => Type::Str,
            "os_getenv" => Type::Optional(Box::new(Type::Str)),
            "os_exit" => Type::Void,
            "os_args" => Type::List(Box::new(Type::Str)),
            "os_exec" | "os_system" => Type::Tuple(vec![Type::Int, Type::Str, Type::Error]),
            "alloc" => Type::Ref(Box::new(Type::Byte)),
            "address" => {
                if let Some(Arg::Pos(e)) = args.first() {
                    Type::Ref(Box::new(self.db.expr_ty.get(&(e.span.file, e.span.start, e.span.end)).cloned().unwrap_or(Type::Int)))
                } else {
                    Type::Ref(Box::new(Type::Int))
                }
            }
            "value" => {
                if let Some(Arg::Pos(e)) = args.first() {
                    match self.db.expr_ty.get(&(e.span.file, e.span.start, e.span.end)) {
                        Some(Type::Ref(t)) => *t.clone(),
                        Some(Type::Mut(t)) => *t.clone(),
                        _ => Type::Int,
                    }
                } else {
                    Type::Int
                }
            }
            "error" => Type::Error,
            "json_parse" => Type::Tuple(vec![Type::Json, Type::Error]),
            "json_encode" => Type::Str,
            "http_get" | "http_post" => Type::Tuple(vec![Type::Str, Type::Error]),
            "http_serve_once" => Type::Error,
            "time_now_ms" => Type::Int,
            "time_sleep_ms" => Type::Void,
            "time_format" => Type::Str,
            "sleep_async" => Type::Named("future".into()),
            "get_async" => Type::Named("future".into()),
            "future_data" => Type::Str,
            "tcp_listen" | "tcp_listen_any" => Type::Int,
            "tcp_local_port" => Type::Int,
            "tcp_connect" => Type::Int,
            "tcp_accept" => Type::Int,
            "tcp_read" => Type::Str,
            "tcp_write" => Type::Int,
            "tcp_close" => Type::Void,
            "select_read" => Type::Int,
            "bytearray_new" | "bytearray_slice" | "sb_new" => Type::Ptr,
            "sb_to_str" => Type::Str,
            "sb_len" => Type::Int,
            "sb_append" | "sb_append_str" | "sb_append_cstr" | "sb_append_int" | "sb_append_byte"
            | "sb_clear" => Type::Void,
            "bytearray_data" => Type::Ptr,
            "sha256" | "sha512" | "md5" | "hmac_sha256" => Type::Ptr,
            "digest_sha256" | "digest_sha512" | "digest_md5" | "digest_hmac_sha256" => Type::Ptr,
            "digest_crc32" | "digest_crc32_update" => Type::Int,
            "bytearray_len" | "bytearray_push" | "bytearray_get" | "bytearray_write_at"
            | "bytearray_find" | "bytearray_rfind" | "crc32" | "crc32_update" => Type::Int,
            "consttime_eq" => Type::Bool,
            "bytearray_reserve" | "bytearray_set" | "bytearray_clear" | "bytearray_fill"
            | "bytearray_truncate" | "bytearray_append_bytes" | "secure_zero" => Type::Void,
            "bytearray_append_str" => Type::Ptr,
            "gc_count" | "free_checked" | "check_live" | "alloc_size"
            | "alloc_live_count" | "alloc_freed_count" => Type::Int,
            "alloc_guard" => Type::Void,
            "gc_collect" | "gc_set_threshold" => Type::Void,
            "ptr_int" => Type::Int,
            "ptr_load" | "ptr_load_byte" => Type::Int,
            "ptr_alloc" | "int_ptr" => Type::Ptr,
            "ptr_load_f64" => Type::Float,
            "ptr_load_str" => Type::Str,
            "ptr_store" | "ptr_store_byte" | "ptr_store_str" | "ptr_free" => Type::Void,
            "ptr_store_f64" => Type::Void,
            "any_box" => Type::Any,
            "select" => {
                if args.len() < 2 || args.len() > 3 {
                    self.diag.error(span, "select expects (chan, chan[, timeout_ms])");
                }
                // chan-typed args required
                for a in args {
                    if let Arg::Pos(e) = a {
                        if let Some(t) = self.db.expr_ty.get(&(e.span.file, e.span.start, e.span.end)) {
                            if !matches!(t, Type::Chan(_) | Type::Int) {
                                self.diag.error(e.span, format!("select arg must be chan<T> or int timeout, got {t}"));
                            }
                        }
                    }
                }
                Type::Int
            }
            "hton16" | "ntoh16" | "swap16" => Type::U16,
            "hton32" | "ntoh32" | "swap32" => Type::U32,
            "hton64" | "ntoh64" | "swap64" => Type::U64,
            "lane_alloc" | "lane_save" | "pool_new" => Type::Ptr,
            "lane_restore" | "lane_freeze" | "lane_unfreeze" => Type::Void,
            // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the
            // E0803.3 gate on the intrinsic forms (`pool_set(gp, h, v)`
            // / `pool_remove(gp, h)`) — the pool is the first
            // argument's root, exactly like the pool_insert arm above.
            "pool_set" | "pool_remove" => {
                if let Some(Arg::Pos(p_expr)) = args.first() {
                    self.check_global_pool_mutation(
                        p_expr,
                        if n == "pool_set" { "set" } else { "remove" },
                        span,
                    );
                }
                Type::Void
            }
            "lane_used" | "pool_len" => Type::Int,
            "pool_is_valid" => Type::Bool,
            /* Tier-G Z-set builtins (SWARM RABBIT HOLE, P5 slice 1; W2C §1.1).
               zset_merge mutates dst; zset_add/zset_remove mutate the Z-set's
               coefficients (count += delta / count -= delta, O(1)/key). */
            "zset_new" | "zset_copy" | "zset_delta" | "zset_apply" => Type::Ptr,
            "zset_keys" => Type::List(Box::new(Type::Int)),
            "zset_add" | "zset_remove" | "zset_merge" | "zset_set_prune" => Type::Void,
            "zset_count" | "zset_total" | "zset_len" => Type::Int,
            /* Tier-L join-semilattice builtins (SWARM RABBIT HOLE, Wave 9,
               Impl-M; W2C §1.1 Tier L; final report §8.1). lcell_merge
               mutates dst; lcell_inflate is the ONLY mutator on the L
               surface (values only inflate). Carrier confusion between
               two cells is invisible here (the surface type is an untyped
               ptr) and is fail-fast policed at runtime [E-LKIND]. */
            "lcell_new" | "lcell_delta" | "lcell_apply" => Type::Ptr,
            /* SWARM RABBIT HOLE — Wave 13 (Impl-R): the E-R1
               MERGE-POSITION RULE — Rule R1's static arm (W2B §4.3:
               "capture legal only if every fallback merge into c is
               HB-before the capture or after J(W)"). The provable arm
               is program-order-before: a fallback merge
               (lcell_inflate — the S1 payload application; lcell_merge
               — the join) into a SEAM cell is legal only when every
               wave claiming that cell sits textually LATER in the
               SAME function. The "after J(W)" arm is deliberately
               NOT implemented: lexical order cannot prove it (a
               textually-later merge can sit BETWEEN two J(W)s when
               the capture is loop-shaped — the exact
               conditional-determinism window R1 exists to close),
               and a cross-function merge's order is unknowable.
               Mixed use without proof ⇒ conditional determinism ⇒
               the checked refusal, fired at the merge. In-wave
               merges are the E0812 walk's jurisdiction (the J0
               immediate-merge arm), so this rule is fallback-only. */
            "lcell_inflate" | "lcell_merge" => {
                if self.wave_depth == 0 {
                    if let Some(Arg::Pos(e)) = args.first() {
                        if let ExprKind::Ident(n) = &e.kind {
                            if self.db.lcell_globals.contains_key(n) {
                                let cur_fn = self.current_fn_key.clone();
                                let mut violating = false;
                                if let Some(claims) = self.db.wave_claims.get(n) {
                                    for (fnk, wspan) in claims {
                                        let same_fn_later =
                                            Some(fnk.as_str()) == cur_fn.as_deref()
                                                && wspan.start > span.start;
                                        if !same_fn_later {
                                            violating = true;
                                            break;
                                        }
                                    }
                                }
                                if violating {
                                    self.diag.error(
                                        span,
                                        format!(
                                            "E-R1: capture of '{n}' is not R1-quiescent: a \
                                             fallback merge into '{n}' is not HB-before this \
                                             wave — determinism would be conditional; make \
                                             the merge program-order-before the wave, or use \
                                             contrib-only [E-R1]"
                                        ),
                                    );
                                }
                            }
                        }
                    }
                }
                Type::Void
            }
            /* SWARM RABBIT HOLE — Wave 13 (Impl-R): the wave seam's
               TYPED READ-MODE SURFACE (W2B §3 — "the mode is in the
               type"). read_W: the frozen capture's readout (wave
               bodies only — the deterministic read mode). contrib:
               stage the payload pair (a,b) — exactly an lcell_inflate
               offer — into the enclosing wave's J(W) (wave bodies
               only). read_J: the fold-valued readout, legal only
               where J(W) provably happened (the joined-credit
               discipline); lowered to the same sn_lcell_value the
               fallback read uses — the MODE is the checked temporal
               claim, the value is the fold. */
            "read_W" => {
                if self.wave_depth == 0 {
                    self.diag.error(
                        span,
                        "E-R1: read_W reads the frozen capture of an enclosing wave; \
                         there is no enclosing wave here — use lcell_value (read_S: \
                         defined + session-monotone, schedule-dependent) [E-R1]",
                    );
                }
                self.wave_seam_arg(args, "read_W");
                Type::Int
            }
            "contrib" => {
                if self.wave_depth == 0 {
                    self.diag.error(
                        span,
                        "E-R1: contrib stages a payload into the enclosing wave's \
                         join (J(W)); there is no enclosing wave here — use the \
                         fallback ops (lcell_inflate / lcell_merge, the S1 path) \
                         [E-R1]",
                    );
                }
                self.wave_seam_arg(args, "contrib");
                Type::Void
            }
            "read_J" => {
                if let Some(name) = self.wave_seam_arg(args, "read_J") {
                    if !self.joined_has(&name) {
                        self.diag.error(
                            span,
                            format!(
                                "E-R1: read_J requires J(W) to have happened — \
                                 '{name}' has no wave joined at this point; the value \
                                 is not yet fold-valued; use lcell_value (read_S) [E-R1]"
                            ),
                        );
                    }
                }
                Type::Int
            }
            "lcell_len" | "lcell_value" | "lcell_carrier" => Type::Int,
            "lcell_has" => Type::Bool,
            /* THE (L, exact) REJECTION — the Grothendieck argument as a
               checked compile error. Tier L's merge operator is
               idempotent (x⊔x = x), and the Grothendieck completion of
               an idempotent operator is trivial: in the group of
               fractions [x] = [x⊔x] − [x] = 0, so the only
               group-embeddable join-semilattice is the trivial carrier.
               ⊖ therefore does not exist on the Tier-L surface, and the
               exact-retraction capability declared via lcell_retract is
               ALWAYS rejected — retraction on Tier L is tombstoned or
               none, never exact (W2C §1.1; final report §8.1's compile-
               time rejection list). The name is RECOGNIZED (a named,
               checked diagnostic — not "unknown function"), and no IR
               lowering exists for it, so the runtime never sees it. */
            "lcell_retract" => {
                self.diag.error(
                    span,
                    "exact retraction on a Tier-L join-semilattice is impossible \
                     (the merge operator is non-embeddable: x⊔x = x ⇒ [x] = 0 — \
                     Grothendieck triviality; retraction on Tier L is tombstoned \
                     or none, never exact); use a Tier-G Z-set (zset_*) for exact \
                     retraction [E-LRET]",
                );
                Type::Void
            }
            /* THE SALT-ζ ADMISSION CHECKER (E-ZADM) — the convergence
               theorem's sufficient condition made executable (SWARM
               RABBIT HOLE, Wave 9, Impl-N; W2C §3.3–§3.5).
               THEOREM (convergence): a MONOTONE body (in the recursive
               variable) over a FINITE-HEIGHT per-key lattice ⇒ the
               δ₀/∫ bracket terminates in ≤ height rounds at the LEAST
               fixpoint (Knaster–Tarski; Cousot–Cousot Pacific J. Math.
               82(1) 1979). Both premises are REQUIRED — monotone alone
               is not enough (counting-through-recursion is monotone
               and diverges on the infinite-height carrier), and
               finite-height alone is not enough (negation-through-
               recursion oscillates on any carrier). ADMISSION IS
               UNDECIDABLE IN GENERAL — the check below is the
               SUFFICIENT CONDITION priced at expressiveness ("arbitrary
               SQL/Datalog" shrinks to the stratified-monotone fragment),
               mandatory per the mandate's no-handwaving rule (W2C §3.5:
               refusals are compile-time errors, never silent
               divergence). HONEST SCOPE: the surface has no user-
               defined fixpoint bodies yet, so the check is per-mode on
               the built-in body; a user-body certifier (the Propel-
               style checked-not-proven class, W2C §6.4) is a later
               slice.
               ADMITTED — zfix_new (transitive closure reach(a) :-
               edge(a,b), reach(b)): the body is monotone in the
               recursive variable, and the carrier is presence {0,1}
               over the keys of the FINITE edge universe (sets over a
               finite universe qualify; ℤ-valued recursion through the
               recursive variable does not). ADMITTED — zfix_new_l (the
               Tier-L GSET-closure variant): Tier-L carriers qualify BY
               CONSTRUCTION (W2C §3.3's admission rule — the premise
               Impl-M's SnLCell landed). REJECTED, always, with the
               divergence class named: zfix_count and zfix_negate below
               — recognized intrinsics in the E-LRET shape (a named,
               checked diagnostic; no IR lowering, no runtime fn). */
            "zfix_new" | "zfix_new_l" | "zfix_state" => Type::Ptr,
            "zfix_round" | "zfix_run" | "zfix_rounds" | "zfix_len" | "zfix_probes" => Type::Int,
            "zfix_has" | "zfix_quiescent_g" | "zfix_quiescent_l" => Type::Bool,
            /* DIVERGENCE CLASS 1 — counting-through-recursion
               (r(a) :- r(a) with seed 1, generalized to r(a) += incr
               every round): monotone, but the per-key carrier is ℤ —
               INFINITE height — so the convergence theorem's premise
               fails and the bracket would never quiesce. ALWAYS
               REJECTED (W2C §3.3's first named divergence class). */
            "zfix_count" => {
                self.diag.error(
                    span,
                    "fixpoint body diverges: counting-through-recursion — \
                     the body increments a ℤ-valued count through the recursive \
                     variable each round, so the per-key carrier has infinite \
                     height and the convergence theorem's premise fails \
                     (monotone ≠ finite-height, both required); the bracket \
                     would never quiesce [E-ZADM]",
                );
                Type::Void
            }
            /* DIVERGENCE CLASS 2 — negation-through-recursion
               (μy. 1⊖y): ⊖/negation through the recursive variable
               makes the body NON-MONOTONE, so Knaster–Tarski does not
               apply and the iterates oscillate. The failure is the
               non-monotone BODY, tier-orthogonal (1⊖y is group
               arithmetic — no carrier choice repairs it). ALWAYS
               REJECTED (W2C §3.3's second named divergence class). */
            "zfix_negate" => {
                self.diag.error(
                    span,
                    "fixpoint body diverges: negation-through-recursion — \
                     the body applies ⊖/negation through the recursive variable, \
                     so it is NOT monotone in the recursive variable and \
                     Knaster–Tarski does not apply (μy. 1⊖y oscillates; the \
                     failure is the non-monotone body, tier-orthogonal) [E-ZADM]",
                );
                Type::Void
            }
            /* SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the
               SFC layouts' intrinsic arms (A2 §2.4's decision table,
               executable). morton list<T, dims=[n1, n2]> — tensor
               iteration with per-element indexed access: the
               constructors carry the dims as literals (the dims are
               in the type; the declaration arm cross-checks
               agreement), the coordinate ops are bounds-checked at
               runtime ([E-SFC], fail-fast), and the pure index
               arithmetic (morton_index/x/y) is the feature-dispatched
               interleave (C2.8). hilbert map<int, int> — range
               queries: the Butz/Skilling key (hilbert_code/dx/dy,
               Θ(k), no LUT), the append-in-key-order insert (the
               sorted lane IS the index), the binary-search point
               lookup, and the segment-advance box iterator
               (hilbert_box_first/next — the explicit slice-1 form of
               the FUR-Hilbert lowering shape). */
            "morton_new" => {
                if args.len() != 2 {
                    self.diag.error(span, format!("morton_new expects 2 args (d1, d2), got {}", args.len()));
                    return Type::Morton(Box::new(Type::Int), vec![8, 8]);
                }
                let dims = match (self.sfc_lit_arg(args, 0), self.sfc_lit_arg(args, 1)) {
                    (Some(d1), Some(d2)) => vec![d1, d2],
                    _ => {
                        self.diag.error(
                            span,
                            "morton_new's dims must be integer literals — the dims \
                             are part of the type (morton list<T, dims=[n1, n2]>) \
                             and the power-of-two rule is a compile-time fact",
                        );
                        return Type::Morton(Box::new(Type::Int), vec![8, 8]);
                    }
                };
                check_sfc_type(&Type::Morton(Box::new(Type::Int), dims.clone()), span, &mut self.diag);
                Type::Morton(Box::new(Type::Int), dims)
            }
            "morton_from_lex" => {
                if args.len() != 3 {
                    self.diag.error(span, format!("morton_from_lex expects 3 args (src, d1, d2), got {}", args.len()));
                    return Type::Morton(Box::new(Type::Int), vec![8, 8]);
                }
                let st = self.sfc_arg_ty(args, 0);
                if !matches!(&st, Type::List(e) if e.is_fixed_int() || matches!(**e, Type::Any)) {
                    self.diag.error(
                        span,
                        format!("morton_from_lex's source must be a list<int> (the lexicographic row-major twin), got {st}"),
                    );
                }
                let dims = match (self.sfc_lit_arg(args, 1), self.sfc_lit_arg(args, 2)) {
                    (Some(d1), Some(d2)) => vec![d1, d2],
                    _ => {
                        self.diag.error(
                            span,
                            "morton_from_lex's dims must be integer literals — the dims are part of the type",
                        );
                        return Type::Morton(Box::new(Type::Int), vec![8, 8]);
                    }
                };
                check_sfc_type(&Type::Morton(Box::new(Type::Int), dims.clone()), span, &mut self.diag);
                Type::Morton(Box::new(Type::Int), dims)
            }
            "morton_from_bytes" => {
                if args.len() != 3 {
                    self.diag.error(span, format!("morton_from_bytes expects 3 args (bytes, d1, d2), got {}", args.len()));
                    return Type::Morton(Box::new(Type::Int), vec![8, 8]);
                }
                let st = self.sfc_arg_ty(args, 0);
                if !matches!(st, Type::Ptr | Type::Any) {
                    self.diag.error(
                        span,
                        format!("morton_from_bytes expects the to_bytes blob (a ptr), got {st}"),
                    );
                }
                let dims = match (self.sfc_lit_arg(args, 1), self.sfc_lit_arg(args, 2)) {
                    (Some(d1), Some(d2)) => vec![d1, d2],
                    _ => {
                        self.diag.error(
                            span,
                            "morton_from_bytes's dims must be integer literals — the dims are part of the type (the runtime checks the blob's header agrees)",
                        );
                        return Type::Morton(Box::new(Type::Int), vec![8, 8]);
                    }
                };
                Type::Morton(Box::new(Type::Int), dims)
            }
            "morton_get" | "morton_set" => {
                self.sfc_morton_arg(args, 0, n, span);
                self.sfc_int_arg(args, 1, n, span);
                self.sfc_int_arg(args, 2, n, span);
                if n == "morton_set" {
                    if args.len() != 4 {
                        self.diag.error(span, format!("morton_set expects 4 args (m, i, j, v), got {}", args.len()));
                    }
                    Type::Void
                } else {
                    if args.len() != 3 {
                        self.diag.error(span, format!("morton_get expects 3 args (m, i, j), got {}", args.len()));
                    }
                    Type::Int
                }
            }
            "morton_index" => {
                if args.len() != 2 {
                    self.diag.error(span, format!("morton_index expects 2 args (i, j), got {}", args.len()));
                }
                self.sfc_int_arg(args, 0, n, span);
                self.sfc_int_arg(args, 1, n, span);
                Type::Int
            }
            "morton_x" | "morton_y" => {
                if args.len() != 1 {
                    self.diag.error(span, format!("{n} expects 1 arg (the code), got {}", args.len()));
                }
                self.sfc_int_arg(args, 0, n, span);
                Type::Int
            }
            "morton_len" => {
                if args.len() != 1 {
                    self.diag.error(span, format!("morton_len expects 1 arg (m), got {}", args.len()));
                }
                self.sfc_morton_arg(args, 0, n, span);
                Type::Int
            }
            "morton_scan_get" => {
                if args.len() != 2 {
                    self.diag.error(span, format!("morton_scan_get expects 2 args (m, k), got {}", args.len()));
                }
                self.sfc_morton_arg(args, 0, n, span);
                self.sfc_int_arg(args, 1, n, span);
                Type::Int
            }
            "morton_to_bytes" => {
                if args.len() != 1 {
                    self.diag.error(span, format!("morton_to_bytes expects 1 arg (m), got {}", args.len()));
                }
                self.sfc_morton_arg(args, 0, n, span);
                Type::Ptr
            }
            "hilbert_new" => {
                if args.len() != 1 {
                    self.diag.error(span, format!("hilbert_new expects 1 arg (the order k), got {}", args.len()));
                    return Type::Hilbert(Box::new(Type::Int), Box::new(Type::Int));
                }
                match self.sfc_lit_arg(args, 0) {
                    Some(k) if (1..=31).contains(&k) => {}
                    Some(_) => {
                        self.diag.error(
                            span,
                            "hilbert_new's order k must be in 1..=31 (the d=2 grid \
                             side is 2^k and the code must fit the 62-bit space)",
                        );
                    }
                    None => {
                        self.diag.error(
                            span,
                            "hilbert_new's order k must be an integer literal (the grid side 2^k is a compile-time fact)",
                        );
                    }
                }
                Type::Hilbert(Box::new(Type::Int), Box::new(Type::Int))
            }
            "hilbert_code" | "hilbert_dx" | "hilbert_dy" => {
                let want = if n == "hilbert_code" { 3 } else { 2 };
                if args.len() != want {
                    self.diag.error(span, format!("{n} expects {want} int args, got {}", args.len()));
                }
                for i in 0..want {
                    self.sfc_int_arg(args, i, n, span);
                }
                Type::Int
            }
            "hilbert_insert" => {
                if args.len() != 4 {
                    self.diag.error(span, format!("hilbert_insert expects 4 args (h, x, y, v), got {}", args.len()));
                }
                self.sfc_hilbert_arg(args, 0, n, span);
                self.sfc_int_arg(args, 1, n, span);
                self.sfc_int_arg(args, 2, n, span);
                self.sfc_int_arg(args, 3, n, span);
                Type::Void
            }
            "hilbert_get" | "hilbert_has" => {
                if args.len() != 3 {
                    self.diag.error(span, format!("{n} expects 3 args (h, x, y), got {}", args.len()));
                }
                self.sfc_hilbert_arg(args, 0, n, span);
                self.sfc_int_arg(args, 1, n, span);
                self.sfc_int_arg(args, 2, n, span);
                if n == "hilbert_get" { Type::Int } else { Type::Bool }
            }
            "hilbert_len" => {
                if args.len() != 1 {
                    self.diag.error(span, format!("hilbert_len expects 1 arg (h), got {}", args.len()));
                }
                self.sfc_hilbert_arg(args, 0, n, span);
                Type::Int
            }
            "hilbert_entry_key" | "hilbert_entry_x" | "hilbert_entry_y" | "hilbert_entry_val" => {
                if args.len() != 2 {
                    self.diag.error(span, format!("{n} expects 2 args (h, idx), got {}", args.len()));
                }
                self.sfc_hilbert_arg(args, 0, n, span);
                self.sfc_int_arg(args, 1, n, span);
                Type::Int
            }
            "hilbert_box_first" | "hilbert_box_next" => {
                let want = if n == "hilbert_box_first" { 5 } else { 6 };
                if args.len() != want {
                    self.diag.error(span, format!("{n} expects {want} args, got {}", args.len()));
                }
                self.sfc_hilbert_arg(args, 0, n, span);
                for i in 1..want {
                    self.sfc_int_arg(args, i, n, span);
                }
                Type::Int
            }
            /* The dispatch readout (C2.8's honest self-report): 0 =
               the software shift-or path (this AArch64 machine), 1 =
               BMI2 PDEP/PEXT. */
            "sfc_path" => Type::Int,
            "pool_insert" => {
                // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the
                // E0803.3 gate on the intrinsic form — the pool is the
                // first argument's root (`pool_insert(gp, v)`).
                if let Some(Arg::Pos(p_expr)) = args.first() {
                    self.check_global_pool_mutation(p_expr, "insert", span);
                    let pt = self.db.expr_ty.get(&(p_expr.span.file, p_expr.span.start, p_expr.span.end)).cloned().unwrap_or(Type::Ptr);
                    if let Type::Pool(elem) = pt {
                        Type::Handle(elem)
                    } else {
                        Type::Int
                    }
                } else {
                    Type::Int
                }
            }
            "pool_get" => {
                if let Some(Arg::Pos(p_expr)) = args.first() {
                    let pt = self.db.expr_ty.get(&(p_expr.span.file, p_expr.span.start, p_expr.span.end)).cloned().unwrap_or(Type::Ptr);
                    if let Type::Pool(elem) = pt {
                        *elem
                    } else {
                        Type::Int
                    }
                } else {
                    Type::Int
                }
            }
            _ => {
                if let Some((Type::Fn { ret, params }, _)) = self.lookup(n) {
                    if args.len() != params.len() {
                        self.diag.error(
                            span,
                            format!(
                                "'{n}' expects {} args, got {}",
                                params.len(),
                                args.len()
                            ),
                        );
                    }
                    return *ret;
                }
                if let Some(sig) = self.db.funcs.get(n).cloned() {
                    self.check_arity(&sig, args, span);
                    if sig.is_extern {
                        self.check_extern_args(&sig, args, span);
                    }
                    sig.ret
                } else if self.db.blueprints.contains_key(n) {
                    Type::Blueprint(n.to_string())
                } else {
                    self.diag.error(span, format!("unknown function '{n}'"));
                    Type::Int
                }
            }
        }
    }

    /// FFI calls marshal `str` to `char *`. Handing a `str` to a `ptr`
    /// parameter would pass an internal object pointer where C expects a C
    /// string, so reject it instead of corrupting memory silently.
    fn check_extern_args(&mut self, sig: &FuncSig, args: &[Arg], _span: Span) {
        for (i, a) in args.iter().enumerate() {
            let Some((_, pty, _)) = sig.params.get(i) else {
                continue;
            };
            let expr = match a {
                Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => e,
            };
            let aty = self.db
                .expr_ty
                .get(&(expr.span.file, expr.span.start, expr.span.end))
                .cloned()
                .unwrap_or(Type::Int);
            if matches!(pty, Type::Ptr) && matches!(aty, Type::Str) {
                self.diag.error(
                    expr.span,
                    format!(
                        "'{}' takes ptr for parameter {}; declare it as `str` to pass a C string",
                        sig.name,
                        i + 1
                    ),
                );
            }
        }
    }

    /// Record `borrower` as holding a borrow of `owner`, enforcing that a
    /// location has at most one live borrow and that a mutable borrow is
    /// exclusive.
    fn register_borrow(&mut self, borrower: &str, owner: &str, exclusive: bool, span: Span) {
        if let Some((holder, held_excl)) = self.borrowed.get(owner).cloned() {
            if exclusive && held_excl {
                self.diag.error(
                    span,
                    format!("'{owner}' is already mutably borrowed by '{holder}'"),
                );
            } else {
                self.diag.error(
                    span,
                    format!("'{owner}' is already borrowed by '{holder}'"),
                );
            }
            return;
        }
        self.borrowed.insert(owner.to_string(), (borrower.to_string(), exclusive));
        self.borrow_owner.insert(borrower.to_string(), owner.to_string());
        // SWARM RABBIT HOLE — Wave 6 (Impl-E, DIFF-2): the borrow's
        // provenance carries its target's region (RootOrigin.
        // region_instance) — the owner's home region, parallel to
        // borrow_epoch's target-epoch pairing.
        let rgn = self
            .region_instance
            .get(owner)
            .copied()
            .unwrap_or_else(|| self.current_rgn());
        self.region_instance.insert(borrower.to_string(), rgn);
    }

    /// Drop borrow state for everything a scope introduced: leaving a scope
    /// ends the borrows taken inside it, exactly like a real lifetime.
    fn end_local_scope(&mut self, names: &[String]) {
        for n in names {
            if let Some(owner) = self.borrow_owner.remove(n) {
                if let Some((holder, _)) = self.borrowed.get(&owner).cloned() {
                    if holder == *n {
                        self.borrowed.remove(&owner);
                    }
                }
            }
            self.borrow_epoch.remove(n);
            self.list_epochs.remove(n);
            // SWARM RABBIT HOLE — Wave 6 (Impl-E, DIFF-2/DIFF-1): scope
            // exit drops the region stamps and lane assignments of the
            // scope's bindings, exactly like the borrow epochs.
            self.region_instance.remove(n);
            self.lane_of.remove(n);
            // Wave 8 (Impl-L): the GC-heap classification is as
            // scope-local as the lane stamp — a binding leaving scope
            // takes its memory category with it.
            self.gc_managed.remove(n);
            // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the
            // handle-home provenance is as scope-local as the borrow
            // epochs — a binding leaving scope takes its pool tag
            // with it (a stale entry would misattribute a later
            // same-named binding's captures).
            self.handle_home.remove(n);
            // A moved binding becomes valid again in the enclosing scope only if
            // it was declared here; otherwise the move still stands.
            self.moved.remove(n);
            self.channel_moved.remove(n);
            self.freed.remove(n);
        }
    }

    fn root_var_of_expr(e: &Expr) -> Option<String> {
        match &e.kind {
            ExprKind::Ident(n) => Some(n.clone()),
            ExprKind::Index { base, .. } | ExprKind::Member { base, .. } => Self::root_var_of_expr(base),
            _ => None,
        }
    }

    /* SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): the lvalue-load
       half of the E0801 predicate (kills G1 §1 A2 → S4). E0801 fires
       iff `has_self_link(type_of(rhs)) ∧ rhs_is_lvalue_load(rhs)`: the
       RHS READS a HasSelfLink value from storage that survives the
       copy (Ident/Index/Member rooted at a storage location — the
       loaded bytes carry deltas encoded for the SOURCE field
       addresses), vs. CONSTRUCTS one fresh (constructor/literal/call —
       the DPS sret destination-projection semantics: the value is
       built in place at its final address, so the deltas encode
       correctly and the copy discipline does not apply). */
    fn is_lvalue_load(e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Ident(_) => true,
            ExprKind::Index { base, .. } | ExprKind::Member { base, .. } => {
                Self::is_lvalue_load(base)
            }
            _ => false,
        }
    }

    /// DIFF-5 (Impl-P): a compact source-shaped rendering of an
    /// expression for the E0801 diagnostics — the pushed/assigned
    /// expression appears in the message as the user wrote it
    /// (`src[0]`, `n.next`, `xs`).
    fn expr_label(e: &Expr) -> String {
        match &e.kind {
            ExprKind::Int(v) => v.to_string(),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::None => "none".to_string(),
            ExprKind::Ident(n) => n.clone(),
            ExprKind::Self_ => "self".to_string(),
            ExprKind::Index { base, index } => {
                format!("{}[{}]", Self::expr_label(base), Self::expr_label(index))
            }
            ExprKind::Member { base, name } => {
                format!("{}.{}", Self::expr_label(base), name)
            }
            ExprKind::Call { .. } => "ctor(...)".to_string(),
            ExprKind::List(_) => "[...]".to_string(),
            ExprKind::Map(_) => "{...}".to_string(),
            ExprKind::Tuple(_) => "(...)".to_string(),
            _ => "expr".to_string(),
        }
    }

    /// SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the pool that
    /// minted the handle this expression evaluates to — the
    /// provenance the tree lacked (kills G1 §1 A6c). Recognized
    /// shapes: the method form `gp.insert(v)` (root of the base), the
    /// intrinsic form `pool_insert(gp, v)` (first arg's root), and a
    /// transitive copy through an already-tracked handle ident
    /// (`h2 = h1`). None = untracked (no mode check; the runtime
    /// generation check remains the use-after-free guard).
    fn handle_pool_of(&self, e: &Expr) -> Option<String> {
        match &e.kind {
            ExprKind::Call { callee, args, .. } => {
                match &callee.kind {
                    ExprKind::Member { base, name } if name == "insert" => {
                        Self::root_var_of_expr(base)
                    }
                    ExprKind::Ident(f) if f == "pool_insert" => {
                        args.first().and_then(|a| match a {
                            Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => {
                                Self::root_var_of_expr(e)
                            }
                        })
                    }
                    _ => None,
                }
            }
            ExprKind::Ident(h) => self.handle_home.get(h).cloned(),
            _ => None,
        }
    }

    /// SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the mutability
    /// class of the pool a name refers to — the global mode table
    /// first (CheckDb.global_pools), then the local scopes (a local
    /// pool: single-thread by construction, mode irrelevant).
    fn pool_mode_of(&self, name: &str) -> PoolModeCheck {
        if let Some(&mode) = self.db.global_pools.get(name) {
            return PoolModeCheck::Global(mode);
        }
        if self
            .lookup(name)
            .map(|(t, _)| matches!(t, Type::Pool(_)))
            .unwrap_or(false)
        {
            return PoolModeCheck::Local;
        }
        PoolModeCheck::NotAPool
    }

    /// DIFF-4, the E0803.3 gate (frozen-by-default enforcement):
    /// insert/set/remove on a Frozen GLOBAL pool outside an
    /// `unfrozen` window are rejected. ThreadConfined pools mutate on
    /// their declaring thread (E0803.1 handles capture; the runtime
    /// home-thread tag is the backstop); Synchronized pools mutate
    /// under their lock; local pools are single-thread by
    /// construction. The verbatim diagnostic names the op and both
    /// discharges (the window, the two mode declarations).
    fn check_global_pool_mutation(&mut self, pool_expr: &Expr, op: &str, span: Span) {
        let Some(p) = Self::root_var_of_expr(pool_expr) else {
            return;
        };
        if self.db.global_pools.get(&p) == Some(&GlobalPoolMode::Frozen)
            && !self.unfrozen_stack.contains(&p)
        {
            self.diag.error(
                span,
                format!(
                    "E0803.3: cannot mutate global pool '{p}' ({op}): global pools are \
                     frozen by default. Wrap the mutation in 'unfrozen ({p}) {{ ... }}', \
                     or declare the pool 'thread_local' (single-thread) or \
                     'synchronized' (locked). [E0803]"
                ),
            );
        }
    }

    fn local_address_source(&self, expr: &Expr) -> Option<String> {
        match &expr.kind {
            ExprKind::Call { callee, args, .. } => {
                if let ExprKind::Ident(f) = &callee.kind {
                    if f == "address" || f == "as_ptr" {
                        if let Some(arg) = args.first() {
                            let arg_expr = match arg {
                                Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => e,
                            };
                            return Self::root_var_of_expr(arg_expr);
                        }
                    }
                }
                None
            }
            ExprKind::Ident(name) => {
                if let Some(owner) = self.borrow_owner.get(name) {
                    Some(owner.clone())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn check_borrow_epoch(&mut self, borrower: &str, span: Span) {
        if let Some((container, taken_epoch)) = self.borrow_epoch.get(borrower).cloned() {
            let cur_epoch = self.list_epochs.get(&container).copied().unwrap_or(0);
            if taken_epoch < cur_epoch {
                self.diag.error(
                    span,
                    format!(
                        "cannot use '{borrower}': container '{container}' was shrunk or cleared after borrow was taken (shrink-epoch violation)"
                    ),
                );
            }
        }
    }

    fn is_borrow_type(t: &Type) -> bool {
        matches!(t, Type::Ref(_) | Type::Mut(_))
    }

    /// SWARM RABBIT HOLE — Wave 8 (Impl-J): W2A §1.5 T-PUSH/T-STORE —
    /// the escape rule (the E-ESCAPE diagnostic). A value containing
    /// references stored into a destination must have every
    /// reference's target region ⊑ₗ the destination's region:
    /// ∀ρᵢ ∈ refs(v): ρᵢ ⊑ₗ reg(dst) (§1.1 safety law; §4.1 P-I's RI
    /// invariant). Violation = the target's region is nested INSIDE the
    /// holder's region (dies first): when the target region exits, the
    /// holder would keep a dangling reference — the class the region
    /// poset was built to kill. `dst` is the destination's provenance
    /// root: the assigned binding for T-STORE, the container root for
    /// T-PUSH (`xs.push(v)` stores into a cell of `xs`), the record
    /// root for a field store. Bindings with no region_instance are
    /// exempt: function params live in region 0 (the caller's region —
    /// the existing return-escape machinery owns the interprocedural
    /// side), globals live in γ, which outlives everything (W2A §1.1).
    fn escape_check_store(&mut self, value: &Expr, dst: &str, span: Span) {
        let Some(target) = self.value_ref_target(value) else {
            return;
        };
        let (Some(&target_rgn), Some(&holder_rgn)) = (
            self.region_instance.get(&target),
            self.region_instance.get(dst),
        ) else {
            return;
        };
        if !self.rgn_outlives(target_rgn, holder_rgn) {
            self.diag.error(
                span,
                format!(
                    "reference to '{target}' (declared in region {target_rgn}) escapes into \
                     region {holder_rgn} which does not outlive it — the target dies before \
                     the holder; move the declaration of '{target}' to an enclosing scope, \
                     or copy the value instead of taking a reference [E-ESCAPE]"
                ),
            );
        }
    }

    /// SWARM RABBIT HOLE — Wave 8 (Impl-L): the W2A §1.4 memory-category
    /// classification predicates (D0 gap #6). A binding is lane-backed
    /// iff it was created by an explicit `lane_alloc` (the `lane_of`
    /// stamp, Impl-E/Impl-I); GC-managed iff its declaration routed
    /// through the GC-heap constructor path (`new` → sn_obj_new,
    /// list/map literals → sn_list_new/sn_map_new — all gc_track'd).
    /// Everything else (plain locals, params, globals) belongs to no
    /// collector-tracked category and stays under the lexical E-ESCAPE
    /// rule's sole jurisdiction.
    fn is_lane_backed(&self, name: &str) -> bool {
        self.lane_of.contains_key(name)
    }

    fn is_gc_managed(&self, name: &str) -> bool {
        self.gc_managed.contains(name)
    }

    /// SWARM RABBIT HOLE — Wave 8 (Impl-L): GC scoping — D0 gap #6's
    /// checked-error discharge ("a GC-managed pointer stored into lane
    /// memory is invisible and can be swept while live") over W2A
    /// §1.4's memory categories. The single-region ⊑ₗ poset cannot see
    /// this hazard: a lane_alloc'd binding and a `new`'d object sit in
    /// the SAME lexical region (region 0 here), yet they live in memory
    /// spaces with different lifetimes and collectors — lane bytes are
    /// untracked mmap the GC never scans (its roots are globals+stacks+
    /// compiler-pushed values only), and the GC heap survives lane
    /// rewinds. So the category confusion is a checked error in BOTH
    /// directions (E-GC-IN-LANE):
    ///   1. GC-managed ref → lane-backed dst: the GC cannot trace into
    ///      lanes, so the object can be swept while the lane cell still
    ///      references it.
    ///   2. lane-backed ref → GC-managed dst: the lane rewinds at its
    ///      extent, but the GC holder persists with a dangling lane
    ///      reference.
    /// Neither collector knows about the other — the checker is the
    /// only layer that can enforce the boundary, at the same T-STORE /
    /// T-PUSH sites the E-ESCAPE rule guards.
    fn gc_scoping_check_store(&mut self, value: &Expr, dst: &str, span: Span) {
        let Some(target) = self.value_ref_target(value) else {
            return;
        };
        // Borrow indirection: the borrow binding carries its root's
        // category (register_borrow pairs borrower and owner, the same
        // pairing the region stamping uses) — classify the ROOT.
        let root = self
            .borrow_owner
            .get(&target)
            .cloned()
            .unwrap_or(target);
        let (src_cat, dst_cat) = if self.is_gc_managed(&root) && self.is_lane_backed(dst) {
            ("GC-managed", "lane-backed")
        } else if self.is_lane_backed(&root) && self.is_gc_managed(dst) {
            ("lane-backed", "GC-managed")
        } else {
            return;
        };
        self.diag.error(
            span,
            format!(
                "reference to '{root}' ({src_cat}) stored into '{dst}' ({dst_cat}) — \
                 GC-managed and lane-backed memory cannot reference each other: \
                 the GC cannot trace into lanes and lane rewinds don't \
                 respect the GC heap [E-GC-IN-LANE]"
            ),
        );
    }

    /// The provenance root of a ref-valued expression (W2A §1.2): the
    /// binding whose region_instance is the reference's target region.
    /// Borrow bindings carry the ROOT's region (register_borrow's and
    /// the reborrow path's DIFF-2 stamping); address()/as_ptr() name the
    /// root directly. Only ref/mut values are gated — the types whose
    /// target region Impl-E's stamping records. handle<T>/pool refs are
    /// Theorem-B (slot_gen) tier (T-POOLDEREF, W2A §1.5), not the
    /// lexical ⊑ₗ law's jurisdiction; call results are the callee's
    /// return-escape check's jurisdiction (no provenance here ⇒ exempt).
    fn value_ref_target(&self, value: &Expr) -> Option<String> {
        match &value.kind {
            ExprKind::Ident(n) => {
                let is_ref = self
                    .lookup(n)
                    .map(|(t, _)| Self::is_borrow_type(&t))
                    .unwrap_or(false);
                if is_ref {
                    Some(n.clone())
                } else {
                    None
                }
            }
            ExprKind::Call { callee, args, .. } => {
                if let ExprKind::Ident(f) = &callee.kind {
                    if f == "address" || f == "as_ptr" {
                        if let Some(Arg::Pos(a)) = args.first() {
                            return Self::root_var_of_expr(a);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn check_arity(&mut self, sig: &FuncSig, args: &[Arg], span: Span) {
        let is_method = sig.is_method && !sig.is_static
            && sig.params.first().map(|p| p.0 == "self").unwrap_or(false);
        let param_offset = if is_method { 1 } else { 0 };
        let params = if is_method {
            &sig.params[param_offset..]
        } else {
            &sig.params[..]
        };
        let given = args.len();
        let required = params.iter().filter(|p| !p.2).count();
        if given < required || given > params.len() {
            self.diag.error(
                span,
                format!(
                    "'{}' expects {}..{} args, got {given}",
                    sig.name,
                    required,
                    params.len()
                ),
            );
            return;
        }
        // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the param
        // form of the alias gate — passing a pool OBJECT by value into
        // a pool-typed parameter rebinds the runtime object under a
        // local name whose mutation the checker cannot mode-check
        // (the callee's body sees a single-thread local). Pool
        // objects are not copyable values; only handles cross.
        for (i, a) in args.iter().enumerate() {
            if let Some(p) = params.get(i) {
                if !matches!(p.1, Type::Pool(_)) {
                    continue;
                }
                let expr = match a {
                    Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => e,
                };
                if let Some(src) = Self::root_var_of_expr(expr) {
                    let src_is_pool = self
                        .lookup(&src)
                        .map(|(t, _)| matches!(t, Type::Pool(_)))
                        .unwrap_or(false);
                    if src_is_pool {
                        self.diag.error(
                            expr.span,
                            format!(
                                "cannot pass pool '{src}' by value to '{}': pool objects \
                                 are not copyable values — the parameter binding accepts \
                                 unguarded mutation and the declared mode cannot follow \
                                 the call. Pass the pool's handles (ints), or make the \
                                 callee take the pool as 'ptr' and re-prove its \
                                 discipline. [E0803]",
                                sig.name
                            ),
                        );
                    }
                }
            }
        }
        // SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): the E0801
        // by-value-copy rejection, PASS-BY-VALUE form (the spec's rule
        // 2, kills G1 §1 A2 → S4). A HasSelfLink lvalue-load passed
        // into a by-value parameter copies the stored bytes to the
        // parameter's address — every 2-byte delta in the copy would
        // decode against the parameter's address, not the link's
        // target. Fires iff has_self_link(param) ∧ the arg is a
        // positional lvalue-load; `Arg::Inout` (the ref/inout form) and
        // freshly materializing args (call/literal — sret) stay legal.
        for (i, a) in args.iter().enumerate() {
            let Some(p) = params.get(i) else { continue };
            if !self.db.has_self_link(&p.1) {
                continue;
            }
            let expr = match a {
                Arg::Pos(e) => e,
                _ => continue,
            };
            if Self::is_lvalue_load(expr) {
                let src = Self::expr_label(expr);
                let t = p.1.to_string();
                self.diag.error(
                    expr.span,
                    format!(
                        "E0801 (call): cannot pass '{src}' by value to parameter \
                         '{}' of '{}': '{t}' contains a self-relative \
                         'link<{t}, u16>' (the copied delta would decode against \
                         the parameter's address, not the link's target); pass \
                         'ref<{t}>' (or 'inout'), or use 'relocate_node(src, dst)'. \
                         [E0801]",
                        p.0, sig.name
                    ),
                );
            }
        }
        let inout_flags = if is_method && sig.inout_params.len() > param_offset {
            &sig.inout_params[param_offset..]
        } else {
            &sig.inout_params[..]
        };
        // SWARM RABBIT HOLE — Wave 6 (Impl-E, DIFF-1 Rule 1a): record the
        // module call-graph edge (Tarjan input) for the function being
        // checked — every resolved call, inout or not: the SCC of a
        // growth seed is defined by the full call graph, not only its
        // inout edges.
        if let Some(caller) = self.current_fn_key.clone() {
            self.db
                .call_edges
                .entry(caller)
                .or_default()
                .insert(summary_key_of(sig));
        }
        let mut inout_roots: Vec<String> = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let is_param_inout = inout_flags.get(i).copied().unwrap_or(false);
            let (expr, arg_marked_inout) = match a {
                Arg::Inout(e) => (e, true),
                Arg::Pos(e) | Arg::Named { value: e, .. } => (e, false),
            };
            if arg_marked_inout && !is_param_inout {
                self.diag.error(
                    expr.span,
                    format!("parameter {} of '{}' is not declared 'inout'", i + 1, sig.name),
                );
            }
            if is_param_inout {
                // SWARM RABBIT HOLE — Wave 6 (Impl-E, DIFF-1 Rules 1a/1b):
                // record the inout-argument edge and the deferred
                // allocation-site check; `scc_accum_fixpoint` consumes them
                // after all functions are checked.
                self.record_scc_accum_arg(sig, param_offset + i, expr);
                match &expr.kind {
                    ExprKind::Ident(var) => {
                        if self.lookup(var).map(|(_, is_c)| is_c).unwrap_or(false) {
                            self.diag.error(expr.span, format!("cannot pass const '{var}' as 'inout'"));
                        }
                        if self.borrowed.contains_key(var) {
                            self.diag.error(
                                expr.span,
                                format!("cannot pass '{var}' as 'inout' while it is borrowed"),
                            );
                        }
                        if inout_roots.contains(var) {
                            self.diag.error(
                                expr.span,
                                format!("cannot pass '{var}' to multiple 'inout' parameters in the same call (law of exclusivity)"),
                            );
                        }
                        inout_roots.push(var.clone());
                    }
                    ExprKind::Index { .. } | ExprKind::Member { .. } => {
                        if let Some(root) = Self::root_var_of_expr(expr) {
                            if inout_roots.contains(&root) {
                                self.diag.error(
                                    expr.span,
                                    format!("cannot pass '{root}' to multiple 'inout' parameters in the same call (law of exclusivity)"),
                                );
                            }
                            inout_roots.push(root);
                        }
                    }
                    _ => {
                        self.diag.error(expr.span, "'inout' argument must be a mutable variable");
                    }
                }
            }
        }
        for (i, a) in args.iter().enumerate() {
            let is_param_inout = inout_flags.get(i).copied().unwrap_or(false);
            if !is_param_inout {
                let expr = match a {
                    Arg::Inout(e) | Arg::Pos(e) | Arg::Named { value: e, .. } => e,
                };
                if let ExprKind::Ident(var) = &expr.kind {
                    if inout_roots.contains(var) {
                        self.diag.error(
                            expr.span,
                            format!("cannot pass '{var}' as both 'inout' and shared argument in the same call (law of exclusivity)"),
                        );
                    }
                }
            }
        }
    }

    /// SWARM RABBIT HOLE — Wave 6 (Impl-E), DIFF-1 Rules 1a/1b bookkeeping
    /// at an inout call site: the own-param propagation edge (Rule 1a
    /// steps 2/3 — the pin obligation travels caller-ward along inout
    /// chains) or, for a plain local, the deferred Rule-1b
    /// allocation-site candidate whose lane class decides E0802 after
    /// the fixpoint.
    fn record_scc_accum_arg(&mut self, sig: &FuncSig, param_idx: usize, arg: &Expr) {
        if self.current_fn_key.is_none() {
            return;
        }
        let callee = summary_key_of(sig);
        let root = match &arg.kind {
            ExprKind::Ident(v) => v.clone(),
            ExprKind::Index { base, .. } | ExprKind::Member { base, .. } => {
                match Self::root_var_of_expr(base) {
                    Some(r) => r,
                    None => return,
                }
            }
            _ => return,
        };
        if let Some((_, j)) = self.current_inout_params.iter().find(|(n, _)| *n == root) {
            self.db.inout_call_edges.push(InoutEdge {
                caller: self.current_fn_key.clone().unwrap(),
                caller_param: Some(*j),
                callee,
                callee_param: param_idx,
            });
        } else {
            let lane = self.lane_of.get(&root).cloned();
            self.db.scc_accum_sites.push(SccAccumSite {
                callee,
                callee_param: param_idx,
                // Wave 8 (Impl-I): the pre-colored local lives in the
                // caller's frame — db.lane_assignment is keyed by it.
                caller: self.current_fn_key.clone().unwrap(),
                local: root,
                lane,
                pinned_lane: None,
                span: arg.span,
            });
        }
    }

    /// Abstract methods declared anywhere in the chain that this blueprint
    /// never provides a concrete implementation for.
    fn unimplemented_abstracts(&self, bp: &str) -> Vec<String> {
        // Collect every abstract declaration up the inheritance chain.
        let mut declared: Vec<String> = Vec::new();
        let mut cur = Some(bp.to_string());
        while let Some(name) = cur {
            if let Some(info) = self.db.blueprints.get(&name) {
                for m in &info.abstract_methods {
                    if !declared.contains(m) {
                        declared.push(m.clone());
                    }
                }
            }
            cur = self.db.blueprints.get(&name).and_then(|b| b.parent.clone());
        }
        declared
            .into_iter()
            .filter(|m| !self.has_concrete_impl(bp, m))
            .collect()
    }

    fn has_concrete_impl(&self, bp: &str, m: &str) -> bool {
        let mut cur = Some(bp.to_string());
        while let Some(name) = cur {
            if let Some(info) = self.db.blueprints.get(&name) {
                if info.concrete_methods.iter().any(|n| n == m) {
                    return true;
                }
            }
            cur = self.db.blueprints.get(&name).and_then(|b| b.parent.clone());
        }
        false
    }

    fn blueprint_sig(&self, bp: &str, m: &str) -> Option<&FuncSig> {
        let mut cur: Option<String> = Some(bp.to_string());
        while let Some(c) = cur {
            let key = format!("{c}::{m}");
            if let Some(sig) = self.db.funcs.get(&key) {
                return Some(sig);
            }
            cur = self.db.blueprints.get(&c).and_then(|b| b.parent.clone());
        }
        None
    }

    fn chan_recv_elem_ty(&mut self, expr: &Expr) -> Option<Type> {
        if let ExprKind::Call { callee, args, .. } = &expr.kind {
            if args.is_empty() {
                if let ExprKind::Member { base, name } = &callee.kind {
                    if name == "receive" {
                        let bt = self.check_expr(base);
                        if let Type::Chan(inner) = bt {
                            return Some(*inner);
                        }
                    }
                }
            }
        }
        None
    }

    /// SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): `recv` (the
    /// method-call receiver expression) is now carried alongside the
    /// receiver type — the E0803.3 mutation gate roots the pool's
    /// identity from the receiver (`gp.insert(..)` → pool `gp`),
    /// exactly like the borrow bookkeeping at the list/map mutation
    /// sites.
    fn method_ret(&mut self, recv: &Expr, bt: &Type, name: &str, args: &[Arg], span: Span) -> Type {
        let base = match bt {
            Type::Optional(inner) => inner.as_ref(),
            other => other,
        };
        match (base, name) {
            (Type::Str, m) => {
                let key = format!("str::{m}");
                if let Some(sig) = self.db.funcs.get(&key).cloned() {
                    self.check_arity(&sig, args, span);
                    return sig.ret;
                }
                match m {
                    "length" => Type::Int,
                    "slice" => Type::Str,
                    "contains" => Type::Bool,
                    "split" => Type::List(Box::new(Type::Str)),
                    "replace" => Type::Str,
                    "upper" | "lower" => Type::Str,
                    _ => {
                        self.diag.error(span, format!("unknown method '{m}' on str"));
                        Type::Void
                    }
                }
            }
            (Type::Any, m) => match m {
                "as_int" => Type::Int,
                "as_str" => Type::Str,
                "as_bool" => Type::Bool,
                "type_name" => Type::Str,
                _ => {
                    self.diag.error(span, format!("unknown method '{m}' on any"));
                    Type::Void
                }
            },
            (Type::List(elem), "push") => {
                if args.len() != 1 {
                    self.diag.error(span, "push expects 1 arg");
                    return Type::Void;
                }
                if let Some(Arg::Pos(e)) = args.first() {
                    let t = self.check_expr(e);
                    // SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort
                    // Delta): the E-GC-IN-LANE store rule at the SFC
                    // boundary. A lane-backed SFC collection cannot
                    // enter a GC-managed holder: the collector cannot
                    // trace into lane memory (a lane pointer is not a
                    // heap object — tracking it would put lane bytes
                    // on the free list), and the holder's release
                    // would free lane bytes as heap. SFC collections
                    // live in their region's lane and die at its
                    // rewind — the lane-backed list discipline.
                    if matches!(t, Type::Morton(_, _) | Type::Hilbert(_, _)) {
                        let src = Self::expr_label(e);
                        let dst = Self::root_var_of_expr(recv)
                            .unwrap_or_else(|| Self::expr_label(recv));
                        self.diag.error(
                            e.span,
                            format!(
                                "E-GC-IN-LANE: cannot store '{src}' into '{dst}' — a \
                                 lane-backed SFC collection cannot enter a \
                                 GC-managed holder (the collector cannot trace lane \
                                 memory and the holder's release would free it); SFC \
                                 collections live in their region's lane and die at \
                                 its rewind [E-GC-IN-LANE]"
                            ),
                        );
                    }
                    if !elem.assignable_from(&t, &self.db.blueprints) {
                        self.diag.error(e.span, format!("cannot push {t} into list<{elem}>"));
                    }
                    // SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P):
                    // THE A2 KILL — the E0801 by-value-copy rejection,
                    // PUSH form (the spec's verbatim diagnostic, kills
                    // G1 §1 A2 → S4). The spec-as-written rule 2 covered
                    // assignment and pass-by-value but was SILENT on
                    // push/insert: `dst.push(src[0])` copies a
                    // HasSelfLink value by value — the pushed bytes
                    // carry deltas encoded against the SOURCE field
                    // addresses, which would decode against dst's
                    // address. Fires iff has_self_link(elem type) ∧ the
                    // pushed RHS is an lvalue-load; a freshly
                    // materializing RHS (`xs.push(make())` — sret
                    // builds it at the destination) and push_ref (the
                    // ref<T> form — no byte copy) stay legal.
                    if self.db.has_self_link(elem) && Self::is_lvalue_load(e) {
                        let src = Self::expr_label(e);
                        let dst = Self::root_var_of_expr(recv)
                            .unwrap_or_else(|| Self::expr_label(recv));
                        let t = elem.to_string();
                        self.diag.error(
                            e.span,
                            format!(
                                "E0801 (push): cannot push '{src}' into '{dst}' by \
                                 value because '{t}' contains a self-relative \
                                 'link<{t}, u16>' (the copied delta would decode \
                                 against '{dst}''s address, not the link's target); \
                                 push a freshly constructed value (built in place \
                                 via sret), pass 'ref<{t}>', or use \
                                 'relocate_node(src, dst)'. [E0801]"
                            ),
                        );
                    }
                }
                Type::Void
            }
            (Type::List(elem), "push_ref") => {
                if args.len() != 1 {
                    self.diag.error(span, "push_ref expects 1 arg");
                    return Type::Ref(elem.clone());
                }
                if let Some(Arg::Pos(e)) = args.first() {
                    let t = self.check_expr(e);
                    if !elem.assignable_from(&t, &self.db.blueprints) {
                        self.diag.error(e.span, format!("cannot push {t} into list<{elem}>"));
                    }
                }
                Type::Ref(elem.clone())
            }
            (Type::List(elem), "pop") => {
                if !args.is_empty() {
                    self.diag.error(span, "pop expects 0 args");
                }
                *elem.clone()
            }
            (Type::List(_), "clear") => {
                if !args.is_empty() {
                    self.diag.error(span, "clear expects 0 args");
                }
                Type::Void
            }
            (Type::List(_), "length") => Type::Int,
            (Type::Map(_, _), "length") => Type::Int,
            (Type::Map(k, _), "keys") => Type::List(k.clone()),
            (Type::Map(_, v), "values") => Type::List(v.clone()),
            (Type::Map(k, _), "contains") | (Type::Map(k, _), "has") => {
                if args.len() != 1 {
                    self.diag.error(span, "contains expects 1 arg");
                    return Type::Bool;
                }
                if let Some(Arg::Pos(e)) = args.first() {
                    let t = self.check_expr(e);
                    if !k.assignable_from(&t, &self.db.blueprints) {
                        self.diag.error(e.span, format!("cannot check key {t} in map<{k}, ...>"));
                    }
                }
                Type::Bool
            }
            (Type::Map(k, v), "insert") => {
                if args.len() != 2 {
                    self.diag.error(span, "insert expects 2 args");
                    return Type::Void;
                }
                if let (Some(Arg::Pos(ke)), Some(Arg::Pos(ve))) = (args.first(), args.get(1)) {
                    let kt = self.check_expr(ke);
                    let vt = self.check_expr(ve);
                    if !k.assignable_from(&kt, &self.db.blueprints) {
                        self.diag.error(ke.span, format!("cannot use key of type {kt} for map<{k}, {v}>"));
                    }
                    if !v.assignable_from(&vt, &self.db.blueprints) {
                        self.diag.error(ve.span, format!("cannot insert value of type {vt} into map<{k}, {v}>"));
                    }
                    // SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): the
                    // E0801 push rejection's INSERT twin — the spec's
                    // rejection set is "push/insert of an lvalue-read":
                    // inserting a HasSelfLink lvalue-load into a map
                    // cell copies the stored bytes (the same
                    // delta-decode hazard as the push form). Same
                    // predicate, same verbatim diagnostic family,
                    // dst = the map.
                    if self.db.has_self_link(v) && Self::is_lvalue_load(ve) {
                        let src = Self::expr_label(ve);
                        let dst = Self::root_var_of_expr(recv)
                            .unwrap_or_else(|| Self::expr_label(recv));
                        let t = v.to_string();
                        self.diag.error(
                            ve.span,
                            format!(
                                "E0801 (push): cannot push '{src}' into '{dst}' by \
                                 value because '{t}' contains a self-relative \
                                 'link<{t}, u16>' (the copied delta would decode \
                                 against '{dst}''s address, not the link's target); \
                                 push a freshly constructed value (built in place \
                                 via sret), pass 'ref<{t}>', or use \
                                 'relocate_node(src, dst)'. [E0801]"
                            ),
                        );
                    }
                }
                Type::Void
            }
            (Type::Map(_, _), "clear") => {
                if !args.is_empty() {
                    self.diag.error(span, "clear expects 0 args");
                }
                Type::Void
            }
            (Type::Chan(t), "send") => {
                if args.len() != 1 {
                    self.diag.error(span, "send expects 1 argument");
                    return Type::Void;
                }
                let arg_expr = match args.first() {
                    Some(Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. }) => e,
                    None => return Type::Void,
                };
                let at = self.check_expr(arg_expr);
                if matches!(at, Type::Ref(_) | Type::Mut(_) | Type::Ptr) || matches!(**t, Type::Ref(_) | Type::Mut(_) | Type::Ptr) {
                    self.diag.error(arg_expr.span, "cannot send references or pointers across channels");
                } else if !t.assignable_from(&at, &self.db.blueprints) && at != **t {
                    self.diag.error(arg_expr.span, format!("cannot send value of type {at} over chan<{t}>"));
                }
                // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the
                // chan-send Sendable gate for pools and handles (kills
                // G1 §1 A6c). A channel handoff crosses threads
                // exactly like an unscoped spawn: a local pool is not
                // Sendable; a ThreadConfined pool/handle never leaves
                // its declaring thread (E0803.1); a Frozen pool's
                // handle sent while the pool is unfrozen at the send
                // site is E0803.2 (the receiver could race the
                // freelist with the mutator); Frozen-outside-window and
                // Synchronized handles send freely. Handle provenance
                // keys on handle_home — handles are held in int vars on
                // this surface, so the declared type does not identify
                // them.
                if matches!(at, Type::Pool(_)) {
                    if let Some(n) = Self::root_var_of_expr(arg_expr) {
                        self.check_pool_capture(&n, arg_expr.span);
                    }
                } else if let Some(h) = Self::root_var_of_expr(arg_expr) {
                    if let Some(p) = self.handle_home.get(&h).cloned() {
                        self.check_handle_capture(&h, &p, arg_expr.span);
                    }
                }
                if let Some(x) = Self::root_var_of_expr(arg_expr) {
                    self.moved.insert(x.clone(), span);
                    self.channel_moved.insert(x);
                }
                Type::Void
            }
            (Type::Chan(t), "receive") => {
                if let Some(Type::Tuple(ts)) = self.db.expr_ty.get(&(span.file, span.start, span.end)).cloned() {
                    Type::Tuple(ts)
                } else {
                    *t.clone()
                }
            }
            (Type::Chan(_), "close") => Type::Void,
            (Type::Error, "message") => Type::Str,
            (Type::Json, "encode") | (Type::Json, "as_str") => Type::Str,
            (Type::Json, "as_int") => Type::Int,
            (Type::Json, "as_bool") => Type::Bool,
            (Type::Json, "is_null") | (Type::Json, "is_object") | (Type::Json, "is_list") => {
                Type::Bool
            }
            (Type::Json, "length") => Type::Int,
            (Type::Json, "get") => Type::Json,
            (Type::Json, "at") => Type::Json,
            (Type::Json, "keys") => Type::List(Box::new(Type::Str)),
            (Type::Pool(elem), "insert") => {
                // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the
                // E0803.3 gate at the mutation seam — insert/set/
                // remove on a Frozen global pool outside an
                // `unfrozen` window (frozen-by-default enforcement).
                self.check_global_pool_mutation(recv, "insert", span);
                if let Some(Arg::Pos(e)) = args.first() {
                    let vt = self.check_expr(e);
                    if !elem.assignable_from(&vt, &self.db.blueprints) {
                        self.diag.error(e.span, format!("cannot insert {vt} into pool<{elem}>"));
                    }
                }
                Type::Handle(elem.clone())
            }
            (Type::Pool(elem), "get") => {
                if let Some(Arg::Pos(e)) = args.first() {
                    self.check_expr(e);
                }
                (**elem).clone()
            }
            (Type::Pool(elem), "get_ref") => {
                if let Some(Arg::Pos(e)) = args.first() {
                    self.check_expr(e);
                }
                Type::Ref(elem.clone())
            }
            (Type::Pool(elem), "set") => {
                // DIFF-4 (Impl-O): the E0803.3 gate — see the insert arm.
                self.check_global_pool_mutation(recv, "set", span);
                if let Some(Arg::Pos(e)) = args.first() {
                    self.check_expr(e);
                }
                if let Some(Arg::Pos(e)) = args.get(1) {
                    let vt = self.check_expr(e);
                    if !elem.assignable_from(&vt, &self.db.blueprints) {
                        self.diag.error(e.span, format!("cannot set {vt} in pool<{elem}>"));
                    }
                }
                Type::Void
            }
            (Type::Pool(_), "remove") => {
                // DIFF-4 (Impl-O): the E0803.3 gate — see the insert arm.
                self.check_global_pool_mutation(recv, "remove", span);
                if let Some(Arg::Pos(e)) = args.first() {
                    self.check_expr(e);
                }
                Type::Void
            }
            (Type::Pool(_), "is_valid") => {
                if let Some(Arg::Pos(e)) = args.first() {
                    self.check_expr(e);
                }
                Type::Bool
            }
            (Type::Pool(_), "length") | (Type::Pool(_), "len") => Type::Int,
            (Type::Blueprint(bp), m) => {
                // Static methods must be invoked as Blueprint.method(), not on an instance.
                if let Some(sig) = self.blueprint_sig(bp, m).cloned() {
                    if sig.is_static {
                        self.diag.error(span, format!("'{m}' is static; call it as {bp}.{m}()"));
                        return sig.ret;
                    }
                    self.check_method_access(bp, m, span);
                    self.check_arity(&sig, args, span);
                    return sig.ret;
                }
                self.diag.error(span, format!("unknown method '{m}' on {bp}"));
                Type::Void
            }
            (Type::Contract(c), m) => {
                let cms = self
                    .db
                    .contracts
                    .get(c)
                    .map(|ct| ct.methods.clone())
                    .unwrap_or_default();
                for cm in cms {
                    if cm.name == m {
                        if args.len() != cm.params.len() {
                            self.diag.error(span, format!("contract method '{m}' expects {} args, got {}", cm.params.len(), args.len()));
                        }
                        for a in args {
                            match a {
                                Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => { self.check_expr(e); }
                            }
                        }
                        return cm.ret.clone();
                    }
                }
                self.diag.error(span, format!("'{c}' contract has no method '{m}'"));
                Type::Void
            }
            _ => {
                self.diag.error(span, format!("unknown method '{name}'"));
                Type::Void
            }
        }
    }

    fn check_member(&mut self, base: &Expr, name: &str, span: Span) -> Type {
        if let ExprKind::Ident(en) = &base.kind {
            if let Some(vars) = self.db.enums.get(en).cloned() {
                if vars.iter().any(|v| v == name) {
                    if let Some(v_info) = self.db.enum_variants.get(en).and_then(|vs| vs.iter().find(|v| &v.name == name)) {
                        if !v_info.fields.is_empty() {
                            self.diag.error(span, format!("variant '{en}.{name}' requires arguments"));
                        }
                    }
                    return Type::Enum(en.clone());
                }
                self.diag.error(span, format!("unknown variant '{name}' for enum '{en}'"));
                return Type::Enum(en.clone());
            }
        }
        let bt = self.check_expr(base);
        let base_ty = match &bt {
            Type::Optional(inner) => inner.as_ref(),
            other => other,
        };
        match (base_ty, name) {
            (Type::Str, "length") => Type::Int,
            (Type::List(_), "length") => Type::Int,
            (Type::Map(_, _), "length") => Type::Int,
            (Type::Chan(_), "open") => Type::Bool,
            (Type::Error, "message") => Type::Str,
            (Type::Json, "length") => Type::Int,
            (Type::Pool(_), "length") | (Type::Pool(_), "len") => Type::Int,
            (Type::Record(rn), field) => {
                if let Some(info) = self.db.records.get(rn) {
                    if let Some(f) = info.fields.iter().find(|f| f.name == *field) {
                        return f.ty.clone();
                    }
                }
                self.diag.error(span, format!("unknown record field '{field}'"));
                Type::Int
            }
            (Type::Blueprint(bp), field) => {
                if let Some(info) = self.db.blueprints.get(bp).cloned() {
                    if let Some(f) = info.fields.iter().find(|f| f.name == *field) {
                        self.check_access(&info, f, span);
                        return f.ty.clone();
                    }
                    if info.methods.iter().any(|m| m == field) {
                        return Type::Named(format!("method:{bp}::{field}"));
                    }
                }
                self.diag.error(span, format!("unknown member '{field}'"));
                Type::Int
            }
            _ => {
                // method used as property; typecheck as method later if called
                Type::Named(format!("member:{name}"))
            }
        }
    }

    /// `closed` methods are private to the blueprint that declares them, but a
    /// subclass may still call an inherited closed method on itself.
    fn check_method_access(&mut self, bp: &str, m: &str, span: Span) {
        let declares_closed = self
            .db
            .blueprints
            .get(bp)
            .map(|i| i.closed_methods.iter().any(|n| n == m))
            .unwrap_or(false);
        if declares_closed && self.current_bp.as_deref() != Some(bp) {
            self.diag.error(span, format!("method '{m}' is closed on {bp}"));
        }
    }

    fn check_access(&mut self, bp: &BlueprintInfo, field: &FieldInfo, span: Span) {
        match field.access {
            Access::Open => {}
            Access::Closed => {
                if self.current_bp.as_deref() != Some(bp.name.as_str()) {
                    self.diag.error(
                        span,
                        format!("field '{}' is closed on {}", field.name, bp.name),
                    );
                }
            }
            Access::Guarded => {
                let ok = self
                    .current_bp_hierarchy
                    .iter()
                    .any(|n| n == &bp.name)
                    || self.current_bp.as_deref() == Some(bp.name.as_str());
                if !ok {
                    self.diag.error(
                        span,
                        format!("field '{}' is guarded on {}", field.name, bp.name),
                    );
                }
            }
        }
    }

    fn check_int_literal_range(ty: &Type, val: Option<&Expr>, span: Span, diag: &mut Diagnostics) {
        let Some(v) = val else { return };
        if let Some(n) = Self::int_lit_value(v) {
            if let Some((lo, hi)) = ty.int_range() {
                if n < lo || n > hi {
                    diag.error(span, format!("integer {n} out of range for {ty} ({lo}..={hi})"));
                }
            }
            if matches!(ty, Type::U64) && n < 0 {
                diag.error(span, format!("integer {n} out of range for u64"));
            }
        }
    }

    fn rebind_unwrap(&mut self, name: &str) {
        let found = self.lookup(name);
        if let Some((Type::Optional(inner), is_const)) = found {
            if let Some(scope) = self.scopes.last_mut() {
                scope.insert(name.to_string(), (*inner, is_const));
            }
        }
    }

    fn check_spawn_block(&mut self, body: &[Stmt]) {
        let mut local_decls = HashSet::new();
        self.check_spawn_stmts(body, &mut local_decls, false);
    }

    fn check_spawn_stmts(&mut self, stmts: &[Stmt], locals: &mut HashSet<String>, in_lock: bool) {
        for s in stmts {
            match s {
                Stmt::Decl { names, value, .. } => {
                    if let Some(v) = value {
                        self.check_spawn_expr_root(v, locals, in_lock);
                    }
                    for n in names {
                        locals.insert(n.clone());
                    }
                }
                Stmt::Assign { target, value, .. } => {
                    self.check_spawn_expr_root(target, locals, in_lock);
                    self.check_spawn_expr_root(value, locals, in_lock);
                }
                Stmt::Expr(e) => {
                    self.check_spawn_expr_root(e, locals, in_lock);
                }
                Stmt::LockBlock { body, .. } => {
                    self.check_spawn_stmts(body, locals, true);
                }
                Stmt::If { cond, then_body, else_ifs, else_body, .. } => {
                    self.check_spawn_expr_root(cond, locals, in_lock);
                    self.check_spawn_stmts(then_body, locals, in_lock);
                    for (c, b) in else_ifs {
                        self.check_spawn_expr_root(c, locals, in_lock);
                        self.check_spawn_stmts(b, locals, in_lock);
                    }
                    if let Some(eb) = else_body {
                        self.check_spawn_stmts(eb, locals, in_lock);
                    }
                }
                Stmt::While { cond, body, .. } => {
                    self.check_spawn_expr_root(cond, locals, in_lock);
                    self.check_spawn_stmts(body, locals, in_lock);
                }
                Stmt::ForCount { init, cond, step, body, .. } => {
                    self.check_spawn_stmts(std::slice::from_ref(init), locals, in_lock);
                    self.check_spawn_expr_root(cond, locals, in_lock);
                    self.check_spawn_stmts(std::slice::from_ref(step), locals, in_lock);
                    self.check_spawn_stmts(body, locals, in_lock);
                }
                Stmt::ForIn { name, iter, body, .. } => {
                    self.check_spawn_expr_root(iter, locals, in_lock);
                    locals.insert(name.clone());
                    self.check_spawn_stmts(body, locals, in_lock);
                }
                Stmt::Return { value, .. } => {
                    if let Some(v) = value {
                        self.check_spawn_expr_root(v, locals, in_lock);
                    }
                }
                Stmt::Defer { body, .. }
                | Stmt::ErrDefer { body, .. }
                | Stmt::Region { body, .. }
                | Stmt::NurseryBlock { body, .. }
                | Stmt::Frozen { body, .. } => {
                    self.check_spawn_stmts(body, locals, in_lock);
                }
                // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the
                // spawn walk recurses into the new Unfrozen stmt with
                // the window OPEN for the body — a handle captured
                // inside a task's own window is E0803.2 exactly like
                // the outer window (the task could hand it to yet
                // another thread while the pool is mutable), and the
                // window body's mutations hit the spawn-body
                // mutation seam.
                Stmt::Unfrozen { pool, body, .. } => {
                    self.unfrozen_stack.push(pool.clone());
                    self.check_spawn_stmts(body, locals, in_lock);
                    self.unfrozen_stack.pop();
                }
                _ => {}
            }
        }
    }

    /// SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the E0803.1/.2
    /// capture seam + the spawn-body mutation seam, walked over every
    /// expression an UNSCOPED spawn (or chan send) can see (kills
    /// G1 §1 A6c — the global-pool freelist race).
    ///
    /// Called ONCE per statement-root expression (check_spawn_expr_
    /// root below) — the walker recurses through the whole expression
    /// itself, so calling it at every check_spawn_expr recursion level
    /// would fire each diagnostic several times. Runs REGARDLESS of
    /// `in_lock`: a named user lock never covers a pool's freelist;
    /// pools carry their own mode (Frozen/ThreadConfined/Synchronized).
    ///
    ///   • POOL OBJECT capture (`Type::Pool`): a local pool is not
    ///     Sendable at all; a ThreadConfined pool never leaves its
    ///     declaring thread (E0803.1); a Frozen pool captured INSIDE
    ///     its `unfrozen` window is rejected (E0803.2 — the receiver
    ///     could race the freelist with the mutator); Frozen captured
    ///     outside any window and Synchronized pools are Sendable.
    ///   • HANDLE capture (provenance from `handle_home` — the minting
    ///     pool recorded at the binding site; handles are held in int
    ///     vars on this surface, so the declared type does not
    ///     identify them): the same mode+window rule keyed on the
    ///     minting pool — E0803.1 (ThreadConfined), E0803.2 (Frozen
    ///     in window). Frozen-outside-window handles are the
    ///     zero-copy shared read (no legal write path exists, so the
    ///     freelist cannot be raced); Synchronized handles are
    ///     Sendable everywhere.
    ///   • MUTATION inside a spawned task (insert/set/remove on a
    ///     pool): only Synchronized pools accept cross-thread
    ///     mutation — an `unfrozen` window is single-thread by
    ///     contract and the task runs on another thread.
    fn check_spawn_pool_rules(&mut self, e: &Expr, locals: &HashSet<String>) {
        match &e.kind {
            ExprKind::Ident(n) => {
                if locals.contains(n) {
                    return;
                }
                if let Some((t, _)) = self.lookup(n) {
                    if matches!(t, Type::Pool(_)) {
                        self.check_pool_capture(n, e.span);
                    }
                }
                if let Some(p) = self.handle_home.get(n).cloned() {
                    self.check_handle_capture(n, &p, e.span);
                }
            }
            ExprKind::Call { callee, args, .. } => {
                // The mutation seam: a pool mutation executed inside a
                // spawned task runs on ANOTHER thread — the freelist
                // race the modes exist to prevent.
                match &callee.kind {
                    ExprKind::Member { base, name }
                        if matches!(name.as_str(), "insert" | "set" | "remove") =>
                    {
                        if let Some(root) = Self::root_var_of_expr(base) {
                            let is_pool = self
                                .lookup(&root)
                                .map(|(t, _)| matches!(t, Type::Pool(_)))
                                .unwrap_or(false);
                            if is_pool && !locals.contains(&root) {
                                self.check_spawn_body_mutation(&root, name, e.span);
                            }
                        }
                    }
                    ExprKind::Ident(f)
                        if matches!(f.as_str(), "pool_insert" | "pool_set" | "pool_remove") =>
                    {
                        if let Some(Arg::Pos(p_expr) | Arg::Inout(p_expr)) = args.first() {
                            if let Some(root) = Self::root_var_of_expr(p_expr) {
                                let is_pool = self
                                    .lookup(&root)
                                    .map(|(t, _)| matches!(t, Type::Pool(_)))
                                    .unwrap_or(false);
                                if is_pool && !locals.contains(&root) {
                                    let op = match f.as_str() {
                                        "pool_insert" => "insert",
                                        "pool_set" => "set",
                                        _ => "remove",
                                    };
                                    self.check_spawn_body_mutation(&root, op, e.span);
                                }
                            }
                        }
                    }
                    _ => {}
                }
                self.check_spawn_pool_rules(callee, locals);
                for a in args {
                    let ae = match a {
                        Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => e,
                    };
                    self.check_spawn_pool_rules(ae, locals);
                }
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                self.check_spawn_pool_rules(lhs, locals);
                self.check_spawn_pool_rules(rhs, locals);
            }
            ExprKind::Unary { expr, .. }
            | ExprKind::Cast { expr, .. }
            | ExprKind::Await(expr)
            | ExprKind::Try(expr)
            | ExprKind::ForceUnwrap(expr)
            | ExprKind::OptionalChain(expr) => {
                self.check_spawn_pool_rules(expr, locals);
            }
            ExprKind::Member { base, .. } | ExprKind::Index { base, .. } => {
                self.check_spawn_pool_rules(base, locals);
            }
            _ => {}
        }
    }

    /// DIFF-4 (Impl-O): the pool OBJECT crossing an unscoped spawn/chan
    /// boundary. Locals are not Sendable (the owning frame can die
    /// while the task runs — and two threads on one freelist is
    /// exactly A6c); ThreadConfined never leaves its thread; Frozen is
    /// Sendable EXCEPT inside its own mutable window.
    fn check_pool_capture(&mut self, n: &str, span: Span) {
        match self.pool_mode_of(n) {
            PoolModeCheck::Global(GlobalPoolMode::Frozen)
                if self.unfrozen_stack.iter().any(|p| p == n) =>
            {
                self.diag.error(
                    span,
                    format!(
                        "E0803.2: cannot access pool '{n}' in unscoped spawn/chan: '{n}' is \
                         unfrozen in this scope (mutable window) and the receiving thread could \
                         race its freelist. Move the capture outside the 'unfrozen ({n})' block, \
                         or declare '{n}' 'synchronized'. [E0803]"
                    ),
                );
            }
            PoolModeCheck::Global(GlobalPoolMode::Frozen) => {
                // The Sendable frozen read: no legal write path exists,
                // so no thread can corrupt or race the freelist.
            }
            PoolModeCheck::Global(GlobalPoolMode::Synchronized) => {
                // The locked vault: every op serialized behind the
                // pool's lock — Sendable everywhere.
            }
            PoolModeCheck::Global(GlobalPoolMode::ThreadConfined) => {
                self.diag.error(
                    span,
                    format!(
                        "E0803.1: cannot access pool '{n}' in unscoped spawn/chan: '{n}' is \
                         'thread_local' — its freelist belongs to the declaring thread. Declare \
                         '{n}' 'synchronized' for cross-thread access. [E0803]"
                    ),
                );
            }
            PoolModeCheck::Local => {
                self.diag.error(
                    span,
                    format!(
                        "cannot access local pool '{n}' in unscoped spawn/chan: a local pool's \
                         storage and freelist are not Sendable (the owning frame can die while \
                         the task runs, and two threads on one freelist is the freelist-\
                         corruption race). Send handles from a global pool, or hoist '{n}' to a \
                         global 'frozen'/'synchronized' pool. [E0803]"
                    ),
                );
            }
            PoolModeCheck::NotAPool => {}
        }
    }

    /// DIFF-4 (Impl-O): a HANDLE crossing an unscoped spawn/chan
    /// boundary, keyed on the minting pool's mode and the capture
    /// site's mutable-window depth. E0803.2 is the A6c kill: a handle
    /// into a Frozen pool captured while the pool is unfrozen would
    /// let the receiving thread race the freelist with the mutator.
    fn check_handle_capture(&mut self, h: &str, pool: &str, span: Span) {
        match self.pool_mode_of(pool) {
            PoolModeCheck::Global(GlobalPoolMode::Frozen)
                if self.unfrozen_stack.iter().any(|p| p == pool) =>
            {
                self.diag.error(
                    span,
                    format!(
                        "E0803.2: cannot capture/send handle '{h}' from pool '{pool}' in \
                         unscoped spawn/chan: '{pool}' is unfrozen in this scope (mutable \
                         window) and the receiving thread could race its freelist. Move the \
                         capture outside the 'unfrozen ({pool})' block, or declare '{pool}' \
                         'synchronized'. [E0803]"
                    ),
                );
            }
            PoolModeCheck::Global(GlobalPoolMode::Frozen)
            | PoolModeCheck::Global(GlobalPoolMode::Synchronized) => {
                // Sendable: frozen-outside-window (no legal write path)
                // or synchronized (every op locked).
            }
            PoolModeCheck::Global(GlobalPoolMode::ThreadConfined) => {
                self.diag.error(
                    span,
                    format!(
                        "E0803.1: cannot capture/send handle '{h}' from pool '{pool}' in \
                         unscoped spawn/chan: '{pool}' is 'thread_local' — handles carry its \
                         home-thread tag and never leave the declaring thread. Declare '{pool}' \
                         'synchronized' for cross-thread access. [E0803]"
                    ),
                );
            }
            PoolModeCheck::Local => {
                self.diag.error(
                    span,
                    format!(
                        "cannot capture/send handle '{h}' from pool '{pool}' in unscoped \
                         spawn/chan: '{pool}' is local — its frame can die before the receiving \
                         thread runs. Send handles only from global pools. [E0803]"
                    ),
                );
            }
            PoolModeCheck::NotAPool => {}
        }
    }

    /// DIFF-4 (Impl-O): mutation executed inside a spawned task — the
    /// task body runs on ANOTHER thread, so only the Synchronized
    /// mode's per-op lock makes the mutation safe; a ThreadConfined
    /// freelist belongs to the declaring thread (E0803.1), and a
    /// Frozen pool's `unfrozen` window is single-thread by contract
    /// (the window itself cannot legalize cross-thread mutation).
    fn check_spawn_body_mutation(&mut self, pool: &str, op: &str, span: Span) {
        match self.pool_mode_of(pool) {
            PoolModeCheck::Global(GlobalPoolMode::Synchronized) => {}
            PoolModeCheck::Global(GlobalPoolMode::ThreadConfined) => {
                self.diag.error(
                    span,
                    format!(
                        "E0803.1: cannot mutate pool '{pool}' ({op}) inside a spawned task: \
                         '{pool}' is 'thread_local' — its freelist belongs to the declaring \
                         thread. Declare '{pool}' 'synchronized' for cross-thread mutation. [E0803]"
                    ),
                );
            }
            PoolModeCheck::Global(GlobalPoolMode::Frozen) => {
                self.diag.error(
                    span,
                    format!(
                        "cannot mutate pool '{pool}' ({op}) inside a spawned task: only \
                         'synchronized' pools accept cross-thread mutation — an 'unfrozen' \
                         window is single-thread by contract and the task runs on another \
                         thread. [E0803]"
                    ),
                );
            }
            PoolModeCheck::Local | PoolModeCheck::NotAPool => {
                // A captured LOCAL pool's error fires at the pool-ident
                // capture site (check_pool_capture) — the root cause;
                // a task-local pool is single-thread by construction.
            }
        }
    }

    /// DIFF-4 (Impl-O): the statement-root entry of the spawn walk —
    /// the pool-rules walker runs ONCE over the whole root expression
    /// (it recurses itself; running it per recursion node would fire
    /// each diagnostic several times), then the per-node
    /// Ref/Mut/Ptr/Blueprint walk proceeds unchanged. Pool rules run
    /// regardless of `in_lock`: a named user lock never covers a
    /// pool's freelist.
    fn check_spawn_expr_root(&mut self, e: &Expr, locals: &HashSet<String>, in_lock: bool) {
        self.check_spawn_pool_rules(e, locals);
        self.check_spawn_expr(e, locals, in_lock);
    }

    // --- SWARM RABBIT HOLE — Wave 13 (Impl-R): the wave region's
    // checker methods (W2B §3/§4; Thm WS's checked side conditions). ---

    /// The seam-cell argument check for read_W/contrib/read_J: the
    /// first argument must name a module-scope Tier-L seam cell (W2B
    /// §4.1 — the R1 analysis and the J(W) fold are defined over the
    /// module-scope seam; a function-local cell is wave-internal by
    /// construction, NI(ii)). Returns the seam name when it is one.
    fn wave_seam_arg(&mut self, args: &[Arg], op: &str) -> Option<String> {
        let e = match args.first() {
            Some(Arg::Pos(e)) => e,
            _ => {
                self.diag.error(
                    Span::dummy(),
                    format!("E-R1: {op} expects a seam cell as its first argument [E-R1]"),
                );
                return None;
            }
        };
        let name = match &e.kind {
            ExprKind::Ident(n) => n.clone(),
            other => Self::expr_label(&Expr {
                span: e.span,
                kind: other.clone(),
            }),
        };
        if self.db.lcell_globals.contains_key(&name) {
            Some(name)
        } else {
            self.diag.error(
                e.span,
                format!(
                    "E-R1: '{name}' is not a seam cell — a seam cell is a \
                     module-scope 'ptr' declared with an lcell_new initializer; \
                     waves capture seam cells only [E-R1]"
                ),
            );
            None
        }
    }

    /// The E0812 PURITY GATE — the kill switch (W2B §4.6 case (d)):
    /// the wave body must be a pure function of the frozen capture κ
    /// (Lemma WD's premise — "u pure in τ's deterministic local
    /// state; a wall-clock read would inject schedule into 𝒰_W and
    /// destroy WS-1"). Every arm below is a compile-time error, never
    /// a warning: without the gate the determinism grade degrades
    /// UNDETECTED, and the whole point of a kill switch is that it
    /// fires. The banned-name list is wave_banned_call (shared with
    /// the fn-purity fixpoint — no drift); the transitive half is the
    /// wave_pure_fns fixpoint consulted at plain-fn calls.
    fn wave_e0812_walk(&mut self, body: &[Stmt], captured: &[String]) {
        for s in body {
            self.wave_e0812_stmt(s, captured);
        }
    }

    fn wave_e0812_stmt(&mut self, s: &Stmt, captured: &[String]) {
        match s {
            // The parallelism class: the wave IS the parallelism; a
            // spawn/nursery/goroutine/lock inside the body injects a
            // schedule the determinism kernel cannot see.
            Stmt::SpawnBlock { span, .. }
            | Stmt::GoroutineBlock { span, .. }
            | Stmt::NurseryBlock { span, .. }
            | Stmt::LockBlock { span, .. } => {
                self.diag.error(
                    *span,
                    "E0812: wave body spawns — the wave IS the parallelism; a \
                     spawn/nursery/lock inside the body injects a schedule the \
                     determinism kernel cannot see [E0812]",
                );
            }
            // The exit class: J(W) is structural (contrib→exit→J(W));
            // an early return would strand the staged multiset, and a
            // deferred fallback op runs at wave-internal timing the
            // join discipline does not define. Termination itself is
            // the runtime contract E0810·R — Rice, honestly relocated.
            Stmt::Return { span, .. } | Stmt::Stop(span) => {
                self.diag.error(
                    *span,
                    "E0812: a wave body may not return early — J(W) is structural \
                     (the exit edge contrib→exit→J(W)); termination is the runtime \
                     contract E0810·R, and the join must be reached [E0812]",
                );
            }
            Stmt::Defer { span, .. } | Stmt::ErrDefer { span, .. } => {
                self.diag.error(
                    *span,
                    "E0812: a wave body may not defer — deferred ops run at \
                     wave-internal timing the join discipline W3 does not define; \
                     the body's effects are contrib→exit→J(W), nothing else [E0812]",
                );
            }
            // The fallback-window class: the unfrozen pool window is
            // a fallback sync op (E0803's own machinery governs it);
            // asm can read cycle counters — schedule-adjacent by
            // construction.
            Stmt::Unfrozen { span, .. } | Stmt::Asm { span, .. } => {
                self.diag.error(
                    *span,
                    "E0812: wave body performs a fallback sync/extrinsic \
                     operation (an unfrozen window / inline asm) — wave bodies \
                     are pure functions of the frozen capture [E0812]",
                );
            }
            // The state-escape class: a global write from the wave
            // body is fallback state mutated by wave-internal flow —
            // NI(ii) requires wave-internal state unreachable from
            // fallback ("ports don't escape"); the seam's only write
            // path is contrib at J(W).
            Stmt::Assign { target, span, .. } => {
                if let Some(root) = Self::root_var_of_expr(target) {
                    if self.db.globals.contains_key(&root) {
                        self.diag.error(
                            *span,
                            format!(
                                "E0812: wave body assigns to global '{root}' — \
                                 wave-internal state must not escape (Lemma NI(ii): \
                                 'ports don't escape'); the seam's only write path is \
                                 contrib at J(W) [E0812]"
                            ),
                        );
                    }
                }
            }
            _ => {}
        }
        // The expression class: schedule injectors (verbatim), the
        // fallback sync ops (the J0/S1 forms), the read_S mode rule
        // on captured seam cells, method calls (outside the
        // wave-purity fragment — the receiver's mutators are
        // untracked through the call), awaits, and impure helper
        // calls (the interprocedural fixpoint — E0812's whole
        // purpose is that a wall-clock read one call away does not
        // slip through).
        wave_visit_stmt(s, &mut |e| {
            match &e.kind {
                ExprKind::Await(_) => {
                    self.diag.error(
                        e.span,
                        "E0812: wave body awaits — async IO is schedule-dependent \
                         state; wave bodies are pure functions of the frozen \
                         capture (Lemma WD's premise); this is the kill switch: \
                         the determinism grade degrades without it [E0812]",
                    );
                }
                ExprKind::Call { callee, .. } => match &callee.kind {
                    ExprKind::Ident(n) => {
                        if wave_banned_call(n) {
                            if n == "lcell_inflate" || n == "lcell_merge" {
                                self.diag.error(
                                    e.span,
                                    format!(
                                        "E0812: wave body performs the fallback \
                                         sync op '{n}' — in-wave payloads stage via \
                                         contrib and fold at J(W) (the J0 immediate \
                                         merge is not the default); the join discipline \
                                         W3 is what makes the fold canonical [E0812]"
                                    ),
                                );
                            } else {
                                self.diag.error(
                                    e.span,
                                    format!(
                                        "E0812: wave body reads schedule-dependent \
                                         state '{n}' — wave bodies must be pure \
                                         functions of the frozen capture (Lemma WD's \
                                         premise); this is the kill switch: the \
                                         determinism grade degrades without it [E0812]"
                                    ),
                                );
                            }
                        } else if matches!(n.as_str(), "read_W" | "contrib" | "read_J") {
                            // The wave's own surface — checked at the
                            // call arms (ident_call).
                        } else if self.db.funcs.contains_key(n) {
                            // A user/extern function: externs are
                            // unknown purity (refused); plain fns must
                            // carry the purity fixpoint's certificate.
                            let sig = &self.db.funcs[n];
                            let pure = !sig.is_extern && self.db.wave_pure_fns.contains(n);
                            if !pure {
                                self.diag.error(
                                    e.span,
                                    format!(
                                        "E0812: wave body calls '{n}' which reads \
                                         schedule-dependent state or performs \
                                         fallback sync ops — the purity gate is \
                                         interprocedural (the call-graph fixpoint); \
                                         inline the pure fragment or make the \
                                         function pure [E0812]"
                                    ),
                                );
                            }
                        } else if self.db.fn_templates.contains_key(n) {
                            // A generic fn: the fixpoint certified the
                            // template body under its "{name}<T>" key.
                            if !self
                                .db
                                .wave_pure_fns
                                .contains(&format!("{n}<T>"))
                            {
                                self.diag.error(
                                    e.span,
                                    format!(
                                        "E0812: wave body calls '{n}' which reads \
                                         schedule-dependent state or performs \
                                         fallback sync ops — the purity gate is \
                                         interprocedural (the call-graph fixpoint); \
                                         inline the pure fragment or make the \
                                         function pure [E0812]"
                                    ),
                                );
                            }
                        }
                        // The read_S mode rule: lcell_value/len/has on
                        // a CAPTURED seam cell in-wave (the fallback
                        // read mode on the wave's own seam).
                        if matches!(n.as_str(), "lcell_value" | "lcell_len" | "lcell_has") {
                            if let ExprKind::Call { args, .. } = &e.kind {
                                if let Some(Arg::Pos(a0)) = args.first() {
                                    if let ExprKind::Ident(an) = &a0.kind {
                                        if captured.contains(an) {
                                            self.diag.error(
                                                a0.span,
                                                format!(
                                                    "E0812: wave body reads seam cell \
                                                     '{an}' in fallback mode (read_S) — the \
                                                     in-wave read mode is read_W (the \
                                                     frozen capture); the S-mode read is \
                                                     schedule-dependent the moment the \
                                                     discipline relaxes (J0/concurrent \
                                                     waves) [E0812]"
                                                ),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                    ExprKind::Member { name, .. } => {
                        if wave_banned_method(name) {
                            // The chan class (receive/send/close) and
                            // the Tier-L syncop wrapper methods: the
                            // verbatim schedule/sync diagnostic, the
                            // same text the intrinsic arms use.
                            self.diag.error(
                                e.span,
                                format!(
                                    "E0812: wave body reads schedule-dependent \
                                     state '{name}' — wave bodies must be pure \
                                     functions of the frozen capture (Lemma WD's \
                                     premise); this is the kill switch: the \
                                     determinism grade degrades without it [E0812]"
                                ),
                            );
                        } else {
                            self.diag.error(
                                e.span,
                                format!(
                                    "E0812: wave body calls method '{name}' — method \
                                     calls are outside the wave-purity fragment (the \
                                     receiver's mutators are untracked through the call; \
                                     chan receives and the Tier-L syncop wrappers hide \
                                     behind innocent names); inline the pure fragment \
                                     [E0812]"
                                ),
                            );
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        });
        // Recurse into nested statements.
        let kids: Vec<Stmt> = match s {
            Stmt::If { then_body, else_ifs, else_body, .. } => {
                let mut all = then_body.clone();
                for (_, b) in else_ifs {
                    all.extend(b.clone());
                }
                if let Some(b) = else_body {
                    all.extend(b.clone());
                }
                all
            }
            Stmt::While { body, .. }
            | Stmt::ForIn { body, .. }
            | Stmt::ErrDefer { body, .. }
            | Stmt::Region { body, .. }
            | Stmt::NurseryBlock { body, .. }
            | Stmt::Frozen { body, .. }
            | Stmt::Unfrozen { body, .. }
            | Stmt::SpawnBlock { body, .. }
            | Stmt::LockBlock { body, .. }
            | Stmt::GoroutineBlock { body, .. }
            | Stmt::Defer { body, .. }
            | Stmt::Wave { body, .. } => body.clone(),
            _ => Vec::new(),
        };
        // (Match arms handled explicitly — the pattern above binds
        // only the arm vectors' bodies.)
        let mut all_kids = kids;
        match s {
            Stmt::Match { arms, default, .. } => {
                for (_, b) in arms {
                    all_kids.extend(b.clone());
                }
                if let Some(d) = default {
                    all_kids.extend(d.clone());
                }
            }
            Stmt::Guard { else_body, .. } => all_kids.extend(else_body.clone()),
            Stmt::ForCount { init, step, body, .. } => {
                all_kids.push(*init.clone());
                all_kids.push(*step.clone());
                all_kids.extend(body.clone());
            }
            Stmt::NestedFn(nf) => all_kids.extend(nf.body.clone()),
            _ => {}
        }
        for k in &all_kids {
            self.wave_e0812_stmt(k, captured);
        }
    }

    fn check_spawn_expr(&mut self, e: &Expr, locals: &HashSet<String>, in_lock: bool) {
        if in_lock {
            return;
        }
        match &e.kind {
            ExprKind::Ident(n) => {
                if !locals.contains(n) {
                    if let Some((t, _)) = self.lookup(n) {
                        if matches!(t, Type::Ref(_) | Type::Mut(_) | Type::Ptr) {
                            self.diag.error(
                                e.span,
                                format!("cannot access reference '{n}' in spawn without synchronization: data race hazard"),
                            );
                        } else if let Type::Blueprint(bp) = &t {
                            self.diag.error(
                                e.span,
                                format!("cannot access blueprint '{bp}' in spawn without locking"),
                            );
                        }
                    }
                }
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                self.check_spawn_expr(lhs, locals, in_lock);
                self.check_spawn_expr(rhs, locals, in_lock);
            }
            ExprKind::Unary { expr, .. }
            | ExprKind::Cast { expr, .. }
            | ExprKind::Await(expr)
            | ExprKind::Try(expr)
            | ExprKind::ForceUnwrap(expr)
            | ExprKind::OptionalChain(expr) => {
                self.check_spawn_expr(expr, locals, in_lock);
            }
            ExprKind::Call { callee, args, .. } => {
                self.check_spawn_expr(callee, locals, in_lock);
                for a in args {
                    let ae = match a {
                        Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => e,
                    };
                    self.check_spawn_expr(ae, locals, in_lock);
                }
            }
            ExprKind::Member { base, .. } | ExprKind::Index { base, .. } => {
                self.check_spawn_expr(base, locals, in_lock);
            }
            _ => {}
        }
    }
}

fn none_narrow(cond: &Expr) -> Option<(String, bool)> {
    let ExprKind::Binary { op, lhs, rhs } = &cond.kind else {
        return None;
    };
    let is_ne = match op {
        BinOp::Ne => true,
        BinOp::Eq => false,
        _ => return None,
    };
    match (&lhs.kind, &rhs.kind) {
        (ExprKind::Ident(n), ExprKind::None) | (ExprKind::None, ExprKind::Ident(n)) => {
            Some((n.clone(), is_ne))
        }
        _ => None,
    }
}
