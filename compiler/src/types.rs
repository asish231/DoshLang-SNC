use crate::ast::Variance;
use crate::ast::TypeAst;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    Int,
    I8,
    I16,
    I32,
    U8,
    U16,
    U32,
    U64,
    Float,
    Bool,
    Byte,
    Str,
    Error,
    Dec(u32),
    Void,
    None,
    List(Box<Type>),
    Map(Box<Type>, Box<Type>),
    Chan(Box<Type>),
    Ref(Box<Type>),
    /// Exclusive borrow (`mut<T>`).
    Mut(Box<Type>),
    /// Generational slot vault (`pool<T>`).
    Pool(Box<Type>),
    /// 64-bit generational handle (`handle<T>`).
    Handle(Box<Type>),
    /// SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): the self-relative
    /// link (`link<T, u16>`; kills G1 §1 A2 → S4). A value of this type
    /// at address A stores δ = target − A (decoded: target = A + δ);
    /// the u16 range bounds |δ| ≤ 32767. The inner type is the LINK
    /// TARGET. The 2-byte value is position-dependent BY CONSTRUCTION:
    /// a by-value copy of it (or of any type containing it — the
    /// HasSelfLink predicate, check.rs) would decode against the copy's
    /// address, so such copies are checked (E0801) and only
    /// construct-in-place (sret), re-encoding relocation
    /// (relocate_node/relocate_range) or by-reference moves are legal.
    Link(Box<Type>),
    /// SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the morton
    /// layout — `morton list<T, dims=[n1, n2]>` (A2 §2.4: tensor
    /// iteration with per-element indexed access). Lane-backed (one
    /// contiguous reservation, header + values); values only —
    /// position-independent by construction (E-GC-IN-LANE: never
    /// ARC'd, never gc_track'd, never stored into GC holders). Slice
    /// 1: elem = int, both dims powers of two.
    Morton(Box<Type>, Vec<i64>),
    /// SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the
    /// hilbert layout — `hilbert map<K, V>` (A2 §2.4: spatial range
    /// queries). Entries sorted by d=2 Hilbert key on a lane; the
    /// sorted order IS the index. Lane-backed like Morton. Slice 1:
    /// K = int, V = int.
    Hilbert(Box<Type>, Box<Type>),
    /// SWARM RABBIT HOLE — Wave 16 (Impl-U, Cohort Delta): the tape —
    /// `tape<int>`, the append-only δ₀-chronicle (W2C §36–§38, §48
    /// item 4: the P5↔P2 kinship's SHAPE — discrete impulses against
    /// an integrated state, ordered by a clock, replay =
    /// re-integration from a checkpoint — made executable with
    /// tier-parametric record kinds: opaque / delta / checkpoint).
    /// Lane-backed like the SFC layouts (E-GC-IN-LANE: never ARC'd,
    /// never gc_track'd, never stored into GC holders). Slice 1:
    /// elem = int (the payloads are int64 pairs throughout).
    Tape(Box<Type>),
    /// Opaque raw pointer (C FFI). No borrow tracking, no auto-deref.
    Ptr,
    Optional(Box<Type>),
    Tuple(Vec<Type>),
    Blueprint(String),
    Record(String),
    Enum(String),
    Contract(String),
    Any,
    Lock,
    Json,
    Fn {
        params: Vec<Type>,
        ret: Box<Type>,
    },
    Named(String),
}

impl Type {
    pub fn int_min_max(&self) -> Option<(i128, i128)> {
        match self {
            Type::I8 => Some((-128, 127)),
            Type::I16 => Some((-32768, 32767)),
            Type::I32 => Some((-2147483648, 2147483647)),
            Type::Int => Some((-(2i128.pow(63)), 2i128.pow(63) - 1)),
            Type::U8 | Type::Byte => Some((0, 255)),
            Type::U16 => Some((0, 65535)),
            Type::U32 => Some((0, 4294967295)),
            Type::U64 => Some((0, 2i128.pow(64) - 1)),
            _ => None,
        }
    }

    // true if every value of `self` (as an integer type) is representable in `to`
    pub fn int_fits_in(&self, to: &Type) -> bool {
        if *to == Type::Float {
            return self.int_min_max().is_some() || matches!(self, Type::Int);
        }
        let Some((fmin, fmax)) = self.int_min_max() else { return false };
        let Some((tmin, tmax)) = to.int_min_max() else { return false };
        fmin >= tmin && fmax <= tmax
    }

    pub fn is_ptr(&self) -> bool {
        matches!(
            self,
            Type::Str
                | Type::Error
                | Type::List(_)
                | Type::Map(_, _)
                | Type::Chan(_)
                | Type::Ref(_)
                | Type::Mut(_)
                | Type::Pool(_)
                | Type::Ptr
                | Type::Optional(_)
                | Type::Blueprint(_)
                | Type::Record(_)
                | Type::Contract(_)
                | Type::Any
                | Type::Lock
                | Type::Json
                | Type::Fn { .. }
                // Wave 14 (Impl-S): the SFC layouts are pointer-like
                // values (a lane-backed header pointer).
                | Type::Morton(_, _)
                | Type::Hilbert(_, _)
                // Wave 16 (Impl-U): the tape is a pointer-like value
                // (the lane-backed chronicle header), like the SFC
                // layouts it follows.
                | Type::Tape(_)
        )
    }

    pub fn unwrap_optional(&self) -> Option<&Type> {
        match self {
            Type::Optional(t) => Some(t),
            _ => None,
        }
    }

    pub fn to_type_ast(&self) -> TypeAst {
        match self {
            Type::Int => TypeAst::Int,
            Type::I8 => TypeAst::I8,
            Type::I16 => TypeAst::I16,
            Type::I32 => TypeAst::I32,
            Type::U8 => TypeAst::U8,
            Type::U16 => TypeAst::U16,
            Type::U32 => TypeAst::U32,
            Type::U64 => TypeAst::U64,
            Type::Float => TypeAst::Float,
            Type::Bool => TypeAst::Bool,
            Type::Byte => TypeAst::Byte,
            Type::Str => TypeAst::Str,
            Type::Error => TypeAst::Error,
            Type::Dec(scale) => TypeAst::Dec(*scale),
            Type::Void => TypeAst::Named("void".into()),
            Type::None => TypeAst::Named("none".into()),
            Type::List(inner) => TypeAst::List(Box::new(inner.to_type_ast())),
            Type::Map(k, v) => TypeAst::Map(Box::new(k.to_type_ast()), Box::new(v.to_type_ast())),
            Type::Chan(inner) => TypeAst::Chan(Box::new(inner.to_type_ast())),
            Type::Ref(inner) => TypeAst::Ref(Box::new(inner.to_type_ast())),
            Type::Mut(inner) => TypeAst::Mut(Box::new(inner.to_type_ast())),
            Type::Pool(inner) => TypeAst::Pool(Box::new(inner.to_type_ast())),
            Type::Handle(inner) => TypeAst::Handle(Box::new(inner.to_type_ast())),
            Type::Link(inner) => TypeAst::Link(Box::new(inner.to_type_ast())),
            // Wave 14 (Impl-S): the SFC layouts' round trip through the
            // AST form (the parser's named dims argument included).
            Type::Morton(elem, dims) => TypeAst::Morton {
                elem: Box::new(elem.to_type_ast()),
                dims: dims.clone(),
            },
            Type::Hilbert(k, v) => {
                TypeAst::Hilbert(Box::new(k.to_type_ast()), Box::new(v.to_type_ast()))
            }
            // Wave 16 (Impl-U): the tape's round trip through the AST
            // form. (Type::TapeDelta does not exist — the annotation
            // resolves to Type::Ptr at parse-resolve time; the tier
            // lives in the checker's extern table, not the type.)
            Type::Tape(e) => TypeAst::Tape(Box::new(e.to_type_ast())),
            Type::Ptr => TypeAst::Ptr,
            Type::Optional(inner) => TypeAst::Optional(Box::new(inner.to_type_ast())),
            Type::Tuple(ts) => TypeAst::Tuple(ts.iter().map(|t| t.to_type_ast()).collect()),
            Type::Blueprint(name) => TypeAst::Named(name.clone()),
            Type::Record(name) => TypeAst::Named(name.clone()),
            Type::Enum(name) => TypeAst::Named(name.clone()),
            Type::Contract(name) => TypeAst::Named(name.clone()),
            Type::Named(name) => TypeAst::Named(name.clone()),
            _ => TypeAst::Any,
        }
    }

    pub fn assignable_from(&self, other: &Type, bps: &std::collections::HashMap<String, BlueprintInfo>) -> bool {
        if self == other {
            return true;
        }
        if matches!(self, Type::Any) {
            return true;
        }
        if matches!(other, Type::Any) {
            return matches!(self, Type::Any);
        }
        // list is covariant: list<Animal> accepts list<Dog>
        if let (Type::List(ta), Type::List(tb)) = (self, other) {
            // empty list placeholder (Type::Any) assigns to any list
            if matches!(**tb, Type::Any) { return true }
            return ta.assignable_from(tb, bps);
        }
        if let (Type::Contract(c), Type::Blueprint(child)) = (self, other) {
            let mut cur = child.clone();
            loop {
                if let Some(bp) = bps.get(&cur) {
                    if bp.contracts.contains(c) {
                        return true;
                    }
                    match &bp.parent {
                        Some(p) => cur = p.clone(),
                        None => break,
                    }
                } else {
                    break;
                }
            }
            return false;
        }
        if let (Type::Blueprint(parent), Type::Blueprint(child)) = (self, other) {
            if parent == child {
                return true;
            }
            // Monomorphic generic instances (`Box_Dog`, `Box_Cat`) are related
            // only through their declared variance, never by inheritance.
            let (si, oi) = (bps.get(parent), bps.get(child));
            if let (Some(si), Some(oi)) = (si, oi) {
                if let (Some(sb), Some(ob)) = (&si.mono_base, &oi.mono_base) {
                    if sb == ob {
                        for (i, (a, b)) in si
                            .mono_args
                            .iter()
                            .zip(oi.mono_args.iter())
                            .enumerate()
                        {
                            let v = si.variance.get(i).copied().unwrap_or(Variance::Invariant);
                            let ok = match v {
                                Variance::Covariant => a.assignable_from(b, bps),
                                Variance::Contravariant => b.assignable_from(a, bps),
                                Variance::Invariant => a == b,
                            };
                            if !ok {
                                return false;
                            }
                        }
                        return true;
                    }
                }
            }
            return blueprint_extends(bps, child, parent);
        }
        // `address(x)` produces a shared borrow; a `mut<T>` binding takes the
        // exclusive form of the same borrow, which the checker validates.
        if let Type::Mut(inner) = self {
            return matches!(other, Type::Ref(t) | Type::Mut(t) if **t == **inner || t.assignable_from(inner, bps));
        }
        if let Type::Ref(inner) = self {
            return matches!(other, Type::Mut(t) | Type::Ref(t) if **t == **inner || t.assignable_from(inner, bps));
        }
        if let (Type::Pool(a), Type::Pool(b)) = (self, other) {
            return matches!(**b, Type::Any) || a == b || a.assignable_from(b, bps);
        }
        if let (Type::Handle(a), Type::Handle(b)) = (self, other) {
            return a == b || a.assignable_from(b, bps);
        }
        // DIFF-5 (Impl-P): `link<T, u16>` — identical target types assign
        // (the 2-byte delta is target-typed but width-fixed); and an int
        // literal assigns to a link (the RAW-delta escape hatch: 0 is the
        // null link — position-independent, so constructing with 0 is
        // sound; any other literal writes a delta the position discipline
        // will check at every decode boundary). The by-value COPIES of
        // link-bearing values are what E0801 governs (check.rs).
        if let (Type::Link(a), Type::Link(b)) = (self, other) {
            return a == b || a.assignable_from(b, bps);
        }
        if let Type::Link(_) = self {
            return matches!(other, Type::Int);
        }
        // Wave 14 (Impl-S): the SFC layouts assign ONLY to the identical
        // type (the dims are part of the morton identity; the lane-backed
        // value never coerces to int/ptr — E-GC-IN-LANE polices the store
        // sites separately).
        if let (Type::Morton(ea, da), Type::Morton(eb, db)) = (self, other) {
            return ea == eb && da == db;
        }
        if let (Type::Hilbert(ka, va), Type::Hilbert(kb, vb)) = (self, other) {
            return ka == kb && va == vb;
        }
        // Wave 16 (Impl-U): the tape assigns ONLY to the identical type
        // (the lane-backed chronicle never coerces to int/ptr —
        // E-GC-IN-LANE polices the store sites separately; the SFC
        // layouts' rule, verbatim).
        if let (Type::Tape(a), Type::Tape(b)) = (self, other) {
            return a == b;
        }
        if matches!(self, Type::Morton(_, _) | Type::Hilbert(_, _) | Type::Tape(_)) {
            return false;
        }
        if matches!(other, Type::Morton(_, _) | Type::Hilbert(_, _) | Type::Tape(_)) {
            return false;
        }
        if (matches!(self, Type::Int) && matches!(other, Type::Handle(_)))
            || (matches!(self, Type::Handle(_)) && matches!(other, Type::Int))
        {
            return true;
        }
        if (matches!(self, Type::Ptr) && matches!(other, Type::Pool(_)))
            || (matches!(self, Type::Pool(_)) && matches!(other, Type::Ptr))
        {
            return true;
        }
        // A reference is already a pointer; `ptr p = alloc(n)` is the natural
        // way to obtain an FFI handle.
        if matches!(self, Type::Ptr) {
            return matches!(
                other,
                Type::Ptr | Type::Ref(_) | Type::Int | Type::U64
            );
        }
        if let (Type::Tuple(a), Type::Tuple(b)) = (self, other) {
            return a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|(l, r)| l.assignable_from(r, bps));
        }
        if let (Type::List(a), Type::List(b)) = (self, other) {
            if a == b {
                return true;
            }
            if a.is_fixed_int() && (**b == Type::Int || b.is_fixed_int()) {
                return true;
            }
            return a.assignable_from(b, bps);
        }
        if let (Type::Map(k1, v1), Type::Map(k2, v2)) = (self, other) {
            if matches!(**k2, Type::Any) && matches!(**v2, Type::Any) {
                return true;
            }
            let k_ok = k1 == k2
                || (k1.is_fixed_int() && (**k2 == Type::Int || k2.is_fixed_int()))
                || k1.assignable_from(k2, bps);
            let v_ok = v1 == v2
                || (v1.is_fixed_int() && (**v2 == Type::Int || v2.is_fixed_int()))
                || v1.assignable_from(v2, bps);
            return k_ok && v_ok;
        }
        // implicit widening: u8 -> int, u8 -> u16, i8 -> i32, int -> float, dec -> float, etc.
        if matches!(self, Type::Float) && matches!(other, Type::Dec(_)) {
            return true;
        }
        if (self.int_min_max().is_some() || matches!(self, Type::Int | Type::Float))
            && (other.int_min_max().is_some() || matches!(other, Type::Int))
        {
            if other.int_fits_in(self) {
                return true;
            }
        }
        if matches!(other, Type::None) {
            return matches!(self, Type::Optional(_) | Type::Error);
        }
        if let Type::Optional(inner) = self {
            return inner.as_ref() == other || inner.assignable_from(other, bps) || matches!(other, Type::None);
        }
        false
    }
}

impl Type {
    pub fn is_fixed_int(&self) -> bool {
        matches!(self, Type::I8|Type::I16|Type::I32|Type::U8|Type::U16|Type::U32|Type::U64|Type::Int|Type::Byte)
    }
    pub fn int_range(&self) -> Option<(i64,i64)> {
        match self {
            Type::I8 => Some((-128,127)),
            Type::I16 => Some((-32768,32767)),
            Type::I32 => Some((-2147483648,2147483647)),
            Type::U8 | Type::Byte => Some((0,255)),
            Type::U16 => Some((0,65535)),
            Type::U32 => Some((0,4294967295)),
            Type::U64 => None,
            Type::Int => None,
            _ => None,
        }
    }
    pub fn abi_size(&self) -> i64 {
        match self {
            Type::I8|Type::U8|Type::Byte|Type::Bool => 1,
            Type::I16|Type::U16 => 2,
            // DIFF-5 (Impl-P): link<T, u16> IS the 2-byte delta field.
            Type::Link(_) => 2,
            Type::I32|Type::U32|Type::Float => 4,
            Type::Int|Type::U64|Type::Dec(_) => 8,
            Type::Enum(_) => 8,
            _ => 8,
        }
    }
}

/* SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the morton
   type's dims rule — A2 §3's power-of-two discipline, shared by the
   type-resolution site and the constructor's literal check. Slice 1
   maps the grid onto the code space EXACTLY (every code in [0, d1*d2)
   is an in-grid cell), which holds iff the grid is SQUARE and a power
   of two — unequal power-of-two dims spread the code space past d1*d2
   (interleave(2^a−1, 2^b−1) needs 2a−1 bits when a > b) and want the
   PADDED reservation / the hybrid 2^t tile floor (Chatterjee TPDS
   13(11) 2002), the documented slice-2 form. */
pub fn morton_dims_valid(dims: &[i64]) -> bool {
    dims.len() == 2
        && dims[0] > 0
        && dims[0] <= (1i64 << 31)
        && dims[0] == dims[1]
        && (dims[0] & (dims[0] - 1)) == 0
}

pub fn morton_dims_label(dims: &[i64]) -> String {
    let parts: Vec<String> = dims.iter().map(|d| d.to_string()).collect();
    format!("dims=[{}]", parts.join(", "))
}

pub fn blueprint_extends(
    bps: &std::collections::HashMap<String, BlueprintInfo>,
    child: &str,
    parent: &str,
) -> bool {
    if child == parent {
        return true;
    }
    let mut cur = child.to_string();
    while let Some(bp) = bps.get(&cur) {
        if let Some(p) = &bp.parent {
            if p == parent {
                return true;
            }
            cur = p.clone();
        } else {
            break;
        }
    }
    false
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Int => write!(f, "int"),
            Type::I8 => write!(f, "i8"),
            Type::I16 => write!(f, "i16"),
            Type::I32 => write!(f, "i32"),
            Type::U8 => write!(f, "u8"),
            Type::U16 => write!(f, "u16"),
            Type::U32 => write!(f, "u32"),
            Type::U64 => write!(f, "u64"),
            Type::Float => write!(f, "float"),
            Type::Bool => write!(f, "bool"),
            Type::Byte => write!(f, "byte"),
            Type::Str => write!(f, "str"),
            Type::Error => write!(f, "error"),
            Type::Dec(s) => write!(f, "dec({s})"),
            Type::Void => write!(f, "void"),
            Type::None => write!(f, "none"),
            Type::List(t) => write!(f, "list<{t}>"),
            Type::Map(k, v) => write!(f, "map<{k}, {v}>"),
            Type::Chan(t) => write!(f, "chan<{t}>"),
            Type::Ref(t) => write!(f, "ref<{t}>"),
            Type::Optional(t) => write!(f, "{t}?"),
            Type::Tuple(ts) => {
                let s: Vec<_> = ts.iter().map(|t| t.to_string()).collect();
                write!(f, "({})", s.join(", "))
            }
            Type::Ptr => write!(f, "ptr"),
            Type::Mut(inner) => write!(f, "mut<{inner}>"),
            Type::Pool(inner) => write!(f, "pool<{inner}>"),
            Type::Handle(inner) => write!(f, "handle<{inner}>"),
            // DIFF-5 (Impl-P): the display carries the implemented width —
            // the verbatim E0801 diagnostics name the type as link<T, u16>.
            Type::Link(inner) => write!(f, "link<{inner}, u16>"),
            // Wave 14 (Impl-S): the SFC layouts — the display is the
            // syntax (the verbatim E-SFC diagnostics quote it).
            Type::Morton(elem, dims) => write!(f, "morton list<{elem}, {}>", morton_dims_label(dims)),
            Type::Hilbert(k, v) => write!(f, "hilbert map<{k}, {v}>"),
            Type::Tape(e) => write!(f, "tape<{e}>"),
            Type::Blueprint(n) | Type::Named(n) | Type::Record(n) | Type::Enum(n) | Type::Contract(n) => write!(f, "{n}"),
            Type::Any => write!(f, "any"),
            Type::Lock => write!(f, "lock"),
            Type::Json => write!(f, "json"),
            Type::Fn { params, ret } => {
                let s: Vec<_> = params.iter().map(|t| t.to_string()).collect();
                if matches!(ret.as_ref(), Type::Void) {
                    write!(f, "fn({})", s.join(", "))
                } else {
                    write!(f, "fn({}) -> {ret}", s.join(", "))
                }
            }
        }
    }
}

pub fn ast_to_type(t: &TypeAst) -> Type {
    match t {
        // Generic application in type position has no standalone runtime type;
        // monomorphization is driven by `new Box<int>`.
        TypeAst::Generic(_, _) => Type::Any,
        TypeAst::Int => Type::Int,
        TypeAst::I8 => Type::I8,
        TypeAst::I16 => Type::I16,
        TypeAst::I32 => Type::I32,
        TypeAst::U8 => Type::U8,
        TypeAst::U16 => Type::U16,
        TypeAst::U32 => Type::U32,
        TypeAst::U64 => Type::U64,
        TypeAst::Float => Type::Float,
        TypeAst::Str => Type::Str,
        TypeAst::Bool => Type::Bool,
        TypeAst::Byte => Type::Byte,
        TypeAst::Error => Type::Error,
        TypeAst::Dec(s) => Type::Dec(*s),
        TypeAst::Ptr => Type::Ptr,
        TypeAst::Mut(inner) => Type::Mut(Box::new(ast_to_type(inner))),
        TypeAst::Named(n) if n == "lock" => Type::Lock,
        TypeAst::Named(n) if n == "future" => Type::Named("future".into()),
        TypeAst::Named(n) if n == "json" => Type::Json,
        TypeAst::Any => Type::Any,
        TypeAst::Record(n) => Type::Record(n.clone()),
        TypeAst::Enum(n) => Type::Enum(n.clone()),
        TypeAst::Named(n) => Type::Blueprint(n.clone()),
        TypeAst::Fn { params, ret } => Type::Fn {
            params: params.iter().map(ast_to_type).collect(),
            ret: Box::new(ret.as_ref().map(|t| ast_to_type(t)).unwrap_or(Type::Void)),
        },
        TypeAst::List(i) => Type::List(Box::new(ast_to_type(i))),
        TypeAst::Map(k, v) => Type::Map(Box::new(ast_to_type(k)), Box::new(ast_to_type(v))),
        TypeAst::Chan(i) => Type::Chan(Box::new(ast_to_type(i))),
        TypeAst::Ref(i) => Type::Ref(Box::new(ast_to_type(i))),
        TypeAst::Pool(i) => Type::Pool(Box::new(ast_to_type(i))),
        TypeAst::Handle(i) => Type::Handle(Box::new(ast_to_type(i))),
        TypeAst::Link(i) => Type::Link(Box::new(ast_to_type(i))),
        TypeAst::Morton { elem, dims } => Type::Morton(Box::new(ast_to_type(elem)), dims.clone()),
        TypeAst::Hilbert(k, v) => Type::Hilbert(Box::new(ast_to_type(k)), Box::new(ast_to_type(v))),
        // Wave 16 (Impl-U): the tape type; and E1's annotation —
        // `tape_delta(G|L)` resolves to Type::Ptr (the carrier
        // object's ABI: the extern returns the carrier pointer), the
        // tier is the SIGNATURE discipline recorded at the extern
        // registration (check.rs) — the annotation is not a runtime
        // type.
        TypeAst::Tape(e) => Type::Tape(Box::new(ast_to_type(e))),
        TypeAst::TapeDelta(_) => Type::Ptr,
        TypeAst::Optional(i) => Type::Optional(Box::new(ast_to_type(i))),
        TypeAst::Tuple(xs) => Type::Tuple(xs.iter().map(ast_to_type).collect()),
    }
}

#[derive(Clone, Debug)]
pub struct FuncSig {
    pub name: String,
    pub llvm: String,
    pub params: Vec<(String, Type, bool)>, // name, type, has_default
    pub inout_params: Vec<bool>,
    pub ret: Type,
    pub is_method: bool,
    pub blueprint: Option<String>,
    pub is_async: bool,
    pub is_static: bool,
    pub is_verified: bool,
    /// True for `extern "lib" { fn ... }` declarations: no body is emitted,
    /// the symbol is resolved by the linker.
    pub is_extern: bool,
    pub defaults: Vec<Option<crate::ast::Expr>>,
}

#[derive(Clone, Debug)]
pub struct FieldInfo {
    pub name: String,
    pub ty: Type,
    pub access: crate::ast::Access,
    pub offset: i64,
    pub init: Option<crate::ast::Expr>,
}

#[derive(Clone, Debug)]
pub struct RecordInfo {
    pub name: String,
    pub fields: Vec<FieldInfo>,
    pub size: i64,
    pub packed: bool,
}

#[derive(Clone, Debug)]
pub struct BlueprintInfo {
    pub name: String,
    pub type_params: Vec<String>,
    pub parent: Option<String>,
    pub contracts: Vec<String>,
    pub fields: Vec<FieldInfo>,
    pub methods: Vec<String>,
    pub abstract_methods: Vec<String>,
    pub concrete_methods: Vec<String>,
    /// Non-static methods declared `closed`: callable only from inside the
    /// blueprint that declares them.
    pub closed_methods: Vec<String>,
    /// For monomorphic generic instances: the template name, the resolved
    /// type arguments, and the per-position declared variance.
    pub mono_base: Option<String>,
    pub mono_args: Vec<Type>,
    pub variance: Vec<crate::ast::Variance>,
    pub size: i64,
    pub type_id: i64,
    pub is_generic: bool,
    pub is_resource: bool,
}

#[derive(Clone, Debug)]
pub struct ContractInfo {
    pub name: String,
    pub methods: Vec<FuncSig>,
}
