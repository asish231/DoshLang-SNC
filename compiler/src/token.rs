use crate::span::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Eof,
    Ident,
    Int,
    FloatLit,
    Dec,
    String,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Dot,
    Colon,
    Arrow,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    StarStar,
    Eq,
    EqEq,
    BangEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Shl,
    Shr,
    AmpEq,
    PipeEq,
    CaretEq,
    ShlEq,
    ShrEq,
    Question,
    BangBang,
    Fn,
    IntKw,
    I8Kw,
    I16Kw,
    I32Kw,
    U8Kw,
    U16Kw,
    U32Kw,
    U64Kw,
    FloatKw,
    StrKw,
    BoolKw,
    ByteKw,
    EnumKw,
    PackedKw,
    DecKw,
    ListKw,
    MapKw,
    Const,
    Let,
    If,
    Else,
    While,
    For,
    In,
    Match,
    Default,
    Stop,
    Skip,
    Return,
    Use,
    And,
    Or,
    Not,
    True,
    False,
    None,
    Otherwise,
    Blueprint,
    Contract,
    Follows,
    From,
    New,
    SelfKw,
    Open,
    Closed,
    Guarded,
    Spawn,
    Static,
    Abstract,
    Out,
    Extern,
    Ptr,
    Mut,
    Chan,
    Async,
    Await,
    Lock,
    ErrorKw,
    RefKw,
    Defer,
    TryKw,
    Record,
    Extension,
    AnyKw,
    Super,
    Goroutine,
    Asm,
    Inout,
    Guard,
    Errdefer,
    Verified,
    Frozen,
    /* SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the global-pool
       discipline's mutation-window keyword — the Stmt::Frozen lowering
       shape's syntactic mirror for pools (kills G1 §1 A6c). */
    Unfrozen,
    Resource,
    Region,
    PoolKw,
    HandleKw,
    /* SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): the self-relative
       link type's keyword — `link<T, u16>` (the 2-byte self-relative
       delta field; kills G1 §1 A2 → S4). A field of type link<T, u16>
       at address A stores δ = target − A (decoded: target = A + δ);
       the u16 range bounds |δ| ≤ 32767. */
    LinkKw,
    Nursery,
    /* SWARM RABBIT HOLE — Wave 13 (Impl-R, Cohort Delta): the wave
       region keyword — `wave { ... }` (W2B §4.1's wave regions; Thm
       WS, the wave-seam composition theorem, made executable). The
       body is a pure observation bracket over module-scope Tier-L
       seam cells: read_W (the frozen capture κ), contrib (payloads
       staged into the J(W) join), and the block exit is the J2
       batched canonical fold. */
    Wave,
    /* SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the SFC
       layout type keywords — A2's space-filling-geometry decision
       table made executable (§2.4). `morton list<T, dims=[n1, n2]>`:
       tensor iteration with per-element indexed access (addr =
       lane_base + interleave(i, j)·sizeof(T); no directory for
       fixed-size T). `hilbert map<K, V>`: spatial range queries (the
       d=2 sorted-key lane IS the index). */
    MortonKw,
    HilbertKw,
    /* SWARM RABBIT HOLE — Wave 15 (Impl-T, Cohort Delta): Salt ε's
       clause keywords — `fn f(...) requires(cond) ensures(cond) { }`
       (the numeric contract layer; A4's ladder + the final report
       §7.3's boundary rule). NOTE: the `contract` keyword is the
       trait/interface system (Type::Contract) — the numeric layer's
       surface is these two clause heads, and `result` inside an
       ensures condition is a contextual identifier, NOT a keyword
       (zero backward-compat risk). */
    Requires,
    Ensures,
    /* SWARM RABBIT HOLE — Wave 16 (Impl-U, Cohort Delta): the tape
       keywords — W2C's P5↔P2 elevation plan made executable (§36–§38,
       §48 item 4). `tape<int>`: the append-only δ₀-chronicle type
       (the record kinds: opaque / delta / checkpoint). `tape_delta(G)`
       in extern RESULT position: E1 — the extern declares the delta
       type of its results (the dec(n) keyword+parens type precedent). */
    TapeKw,
    TapeDeltaKw,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub text: String,
    pub int: i64,
    pub scale: u32,
    pub float_val: f64,
}

impl Token {
    pub fn dummy() -> Self {
        Self {
            kind: TokenKind::Eof,
            span: Span::dummy(),
            text: String::new(),
            int: 0,
            scale: 0,
            float_val: 0.0,
        }
    }
}
