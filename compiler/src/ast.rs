use crate::span::Span;

#[derive(Clone, Debug)]
pub struct Program {
    pub file: u32,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub enum Item {
    Use(UseItem),
    Fn(FnItem),
    Blueprint(BlueprintItem),
    Record(RecordItem),
    Enum(EnumItem),
    Extension(ExtensionItem),
    Contract(ContractItem),
    Extern(ExternItem),
    Stmt(Stmt),
}

#[derive(Clone, Debug)]
pub struct ExternItem {
    /// Library passed to the linker as `-l<name>`, or a bare object file.
    pub lib: String,
    pub funcs: Vec<FnItem>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct UseItem {
    pub path: Vec<String>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FnItem {
    pub is_async: bool,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_verified: bool,
    pub access: Access,
    pub name: String,
    pub type_params: Vec<TypeParam>,
    pub params: Vec<Param>,
    pub ret: Option<TypeAst>,
    /// SWARM RABBIT HOLE — Wave 15 (Impl-T, Cohort Delta): Salt ε —
    /// the numeric contract layer's requires clauses (A4 §4's
    /// ladder; the final report §7.3's boundary rule). Each clause is
    /// an int-valued boolean expression over the parameters; multiple
    /// clauses are allowed and conjoin. Comparison chains
    /// (`0 <= i < xs.length()`) are desugared at parse time into
    /// `and` conjunctions (the left-associative parse of a chain is
    /// the standard mathematical reading inside a clause —
    /// `(0 <= i) < len` would compare a bool against an int). The
    /// requires condition is checked at every call site: the caller
    /// proves it or the call degrades.
    pub requires: Vec<Expr>,
    /// The ensures clauses — the function's own proof obligation over
    /// its parameters and `result` (the contextual return-value
    /// name); discharged at the fn exit or degraded to a typed
    /// runtime assertion at every return.
    pub ensures: Vec<Expr>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub is_inout: bool,
    pub ty: TypeAst,
    pub name: String,
    pub default: Option<Expr>,
}

#[derive(Clone, Debug)]
pub struct RecordItem {
    pub name: String,
    pub fields: Vec<Field>,
    pub packed: bool,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct EnumItem {
    pub name: String,
    pub variants: Vec<EnumVariant>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct EnumVariant {
    pub name: String,
    pub fields: Vec<Field>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ExtensionItem {
    pub ty: TypeAst,
    pub methods: Vec<FnItem>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct TypeParam {
    pub name: String,
    /// Contract (or blueprint) the argument must satisfy, e.g. `T: Printable`.
    pub bound: Option<String>,
    pub variance: Variance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variance {
    Invariant,
    Covariant,
    Contravariant,
}

/// SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the declared mode of
/// a `pool<T>` binding (kills G1 §1 A6c — the global-pool freelist
/// race). The mutability class the tree lacked: nothing distinguished
/// a global pool from a local one, so a global-pool handle passed
/// freely into spawn/chan → two threads on one freelist → freelist
/// corruption → memory unsafety. Three modes, FROZEN by default:
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolMode {
    /// DEFAULT (no annotation). Read-only from every thread:
    /// insert/set/remove rejected everywhere outside an
    /// `unfrozen (P) { ... }` block (E0803.3). Handles into a frozen
    /// pool are Sendable — zero-copy shared reads (no legal write
    /// path exists, so no thread can race or corrupt the freelist).
    Frozen,
    /// `pool<T> thread_local`. Mutations allowed, on the declaring
    /// thread only. Handles carry a home-thread tag; any spawn/chan
    /// capture or cross-thread use = E0803.1.
    ThreadConfined,
    /// `pool<T> synchronized`. Mutex-protected vault: every
    /// insert/set/remove funnels through a runtime call taking the
    /// pool's lock (sn_pool_lock/sn_pool_unlock around the op); handles
    /// Sendable everywhere. No two threads can ever interleave
    /// insert/remove on one pool — the data-race-freedom condition
    /// for the vault.
    Synchronized,
}

impl std::fmt::Display for PoolMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PoolMode::Frozen => "frozen",
            PoolMode::ThreadConfined => "thread_local",
            PoolMode::Synchronized => "synchronized",
        })
    }
}

#[derive(Clone, Debug)]
pub struct BlueprintItem {
    pub name: String,
    pub is_resource: bool,
    pub type_params: Vec<TypeParam>,
    pub parent: Option<String>,
    pub contracts: Vec<String>,
    pub fields: Vec<Field>,
    pub methods: Vec<FnItem>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub access: Access,
    pub ty: TypeAst,
    pub name: String,
    pub init: Option<Expr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Open,
    Closed,
    Guarded,
}

#[derive(Clone, Debug)]
pub struct ContractItem {
    pub name: String,
    pub methods: Vec<ContractMethod>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ContractMethod {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<TypeAst>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TypeAst {
    Int,
    I8,
    I16,
    I32,
    U8,
    U16,
    U32,
    U64,
    Float,
    Str,
    Bool,
    Byte,
    Error,
    Dec(u32),
    /// Exclusive borrow, written `mut<T>`.
    Mut(Box<TypeAst>),
    /// Opaque raw pointer, used for C FFI (`ptr`).
    Ptr,
    Named(String),
    /// Generic instantiation in type position, e.g. `Producer<Animal>`.
    Generic(String, Vec<TypeAst>),
    Any,
    Record(String),
    Enum(String),
    List(Box<TypeAst>),
    Map(Box<TypeAst>, Box<TypeAst>),
    Chan(Box<TypeAst>),
    Ref(Box<TypeAst>),
    Pool(Box<TypeAst>),
    Handle(Box<TypeAst>),
    /// SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): the self-relative
    /// link — `link<T, u16>` (shorthand `link<T>`). A field of this type
    /// at address A stores δ = target − A (decoded: target = A + δ); the
    /// u16 width bounds |δ| ≤ 32767. Makes the containing struct
    /// HasSelfLink: its values may only be constructed in place (sret),
    /// relocated via re-encode, or copied by reference — a by-value copy
    /// of the stored bytes would decode every delta against the wrong
    /// address (E0801, kills G1 §1 A2 → S4).
    Link(Box<TypeAst>),
    /// SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the morton
    /// layout — `morton list<T, dims=[n1, n2]>` (A2 §2.4's decision
    /// table: tensor iteration with per-element indexed access).
    /// One contiguous lane reservation, fixed-size T (int, slice 1 —
    /// no directory), addr = lane_base + interleave(i, j)·sizeof(T);
    /// the dims are part of the type and must be powers of two.
    Morton {
        elem: Box<TypeAst>,
        dims: Vec<i64>,
    },
    /// SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the hilbert
    /// layout — `hilbert map<K, V>` (A2 §2.4: spatial range queries).
    /// Entries sorted by d=2 Hilbert key on a lane; the sorted order
    /// IS the index. Slice 1 implements hilbert map<int, int>.
    Hilbert(Box<TypeAst>, Box<TypeAst>),
    /// SWARM RABBIT HOLE — Wave 16 (Impl-U, Cohort Delta): the tape —
    /// `tape<int>`, the append-only δ₀-chronicle (W2C §36–§38: the
    /// P5↔P2 kinship's shape, made executable with tier-parametric
    /// record kinds). Lane-backed header (the hilbert precedent:
    /// never gc_track'd — E-GC-IN-LANE); slice 1 implements
    /// tape<int> (the payloads are int64 pairs throughout).
    Tape(Box<TypeAst>),
    /// SWARM RABBIT HOLE — Wave 16 (Impl-U, Cohort Delta): E1's
    /// extern-result annotation — `-> tape_delta(G)` / `-> tape_delta(L)`
    /// (W2C §38: the extern declares the delta type of its results;
    /// unannotated externs keep P2's default-unpure OPAQUE form). The
    /// dec(n) keyword+parens type precedent; resolves to Type::Ptr
    /// (the carrier object's ABI) with the tier recorded at the extern
    /// registration — the annotation is the compile-time signature
    /// discipline, not a runtime type. Tier code: 1 = G, 2 = L
    /// (runtime.c's TAPE_TIER_G/TAPE_TIER_L).
    TapeDelta(i64),
    Optional(Box<TypeAst>),
    Tuple(Vec<TypeAst>),
    Fn {
        params: Vec<TypeAst>,
        ret: Option<Box<TypeAst>>,
    },
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Expr(Expr),
    Decl {
        is_const: bool,
        ty: Option<TypeAst>,
        names: Vec<String>,
        value: Option<Expr>,
        /// DIFF-4 (Impl-O): the pool mutability class — `pool<T>` bare
        /// = Frozen, `pool<T> thread_local` = ThreadConfined,
        /// `pool<T> synchronized` = Synchronized. Default for every
        /// non-pool binding.
        mode: PoolMode,
        span: Span,
    },
    Assign {
        target: Expr,
        op: AssignOp,
        value: Expr,
        span: Span,
    },
    If {
        cond: Expr,
        then_body: Vec<Stmt>,
        else_ifs: Vec<(Expr, Vec<Stmt>)>,
        else_body: Option<Vec<Stmt>>,
        span: Span,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
    ForCount {
        init: Box<Stmt>,
        cond: Expr,
        step: Box<Stmt>,
        body: Vec<Stmt>,
        span: Span,
    },
    ForIn {
        name: String,
        iter: Expr,
        body: Vec<Stmt>,
        span: Span,
    },
    Match {
        expr: Expr,
        arms: Vec<(Expr, Vec<Stmt>)>,
        default: Option<Vec<Stmt>>,
        span: Span,
    },
    Return {
        value: Option<Expr>,
        from_fn: Option<String>,
        span: Span,
    },
    Guard {
        cond: Expr,
        else_body: Vec<Stmt>,
        span: Span,
    },
    ErrDefer {
        body: Vec<Stmt>,
        span: Span,
    },
    Region {
        body: Vec<Stmt>,
        span: Span,
    },
    NurseryBlock {
        body: Vec<Stmt>,
        span: Span,
    },
    Frozen {
        names: Vec<String>,
        body: Vec<Stmt>,
        span: Span,
    },
    /* SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the global-pool
       discipline's mutation window (kills G1 §1 A6c). `unfrozen (P)
       { ... }` — the structured single-thread mutation window on a
       FROZEN global pool: enter = unfreeze, exit/unwind = re-freeze
       (the errdefer-guaranteed re-freeze, Stmt::Frozen's lowering
       shape). Inside: insert/set/remove legal on the declaring
       thread; capture of P's handles by spawn/chan ILLEGAL (E0803.2).
       Outside: mutations rejected (E0803.3). */
    Unfrozen {
        pool: String,
        body: Vec<Stmt>,
        span: Span,
    },
    Stop(Span),
    Skip(Span),
    SpawnBlock {
        body: Vec<Stmt>,
        span: Span,
    },
    SpawnExpr {
        expr: Expr,
        span: Span,
    },
    LockBlock {
        name: String,
        body: Vec<Stmt>,
        span: Span,
    },
    Asm {
        template: String,
        clobbers: Vec<String>,
        span: Span,
    },
    New {
        heap: bool,
        ty: String,
        type_args: Vec<TypeAst>,
        name: String,
        args: Vec<Arg>,
        span: Span,
    },
    GoroutineBlock {
        body: Vec<Stmt>,
        span: Span,
    },
    Defer {
        body: Vec<Stmt>,
        span: Span,
    },
    /* SWARM RABBIT HOLE — Wave 13 (Impl-R, Cohort Delta): the wave
       region — W2B §4.1's composed syntax (phases = fallback segments
       + wave regions) and the executable form of Thm WS (§4.6). The
       body is checked pure (E0812 — the kill switch; Lemma WD's
       premise), captures the module-scope Tier-L seam cells it names
       (R1 quiescence checked at the capture), stages payloads via
       contrib, and the block exit is J(W) — the J2 batched canonical
       fold of the staged multiset into the seam cells (one atomic ⊔
       per cell; Lemma NI(ii)). Slice-1 execution model: the body
       runs ONCE (single-tile), and the frozen κ is realized by the
       static discipline, not a runtime copy. */
    Wave {
        body: Vec<Stmt>,
        span: Span,
    },
    NestedFn(FnItem),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOp {
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,
    ShlEq,
    ShrEq,
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Dec { scaled: i64, scale: u32 },
    Bool(bool),
    Str(String),
    None,
    Ident(String),
    Self_,
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Unary { op: UnOp, expr: Box<Expr> },
    Call {
        callee: Box<Expr>,
        type_args: Vec<TypeAst>,
        args: Vec<Arg>,
    },
    Index { base: Box<Expr>, index: Box<Expr> },
    Member { base: Box<Expr>, name: String },
    Interpolate { parts: Vec<InterpPart> },
    List(Vec<Expr>),
    Map(Vec<(Expr, Expr)>),
    Tuple(Vec<Expr>),
    Cast { expr: Box<Expr>, ty: TypeAst },
    SuperCall { method: String, args: Vec<Arg> },
    Await(Box<Expr>),
    Try(Box<Expr>),
    OptionalChain(Box<Expr>),
    ForceUnwrap(Box<Expr>),
    Lambda {
        params: Vec<Param>,
        ret: Option<TypeAst>,
        body: Vec<Stmt>,
    },
}

#[derive(Clone, Debug)]
pub enum InterpPart {
    Lit(String),
    Expr(Expr),
}

#[derive(Clone, Debug)]
pub enum Arg {
    Pos(Expr),
    Inout(Expr),
    Named { name: String, value: Expr },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    Otherwise,
    In,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
}

impl Expr {
    pub fn ident(name: impl Into<String>, span: Span) -> Self {
        Self {
            kind: ExprKind::Ident(name.into()),
            span,
        }
    }
}
