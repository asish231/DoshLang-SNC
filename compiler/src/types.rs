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
                | Type::Ptr
                | Type::Optional(_)
                | Type::Blueprint(_)
                | Type::Record(_)
                | Type::Contract(_)
                | Type::Any
                | Type::Lock
                | Type::Json
                | Type::Fn { .. }
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
            return matches!(other, Type::Ref(t) if **t == **inner || t.assignable_from(inner, bps));
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
            let k_ok = k1 == k2
                || (k1.is_fixed_int() && (**k2 == Type::Int || k2.is_fixed_int()))
                || k1.assignable_from(k2, bps);
            let v_ok = v1 == v2
                || (v1.is_fixed_int() && (**v2 == Type::Int || v2.is_fixed_int()))
                || v1.assignable_from(v2, bps);
            return k_ok && v_ok;
        }
        // implicit widening: u8 -> int, u8 -> u16, i8 -> i32, int -> float, etc.
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
            Type::I32|Type::U32|Type::Float => 4,
            Type::Int|Type::U64|Type::Dec(_) => 8,
            Type::Enum(_) => 8,
            _ => 8,
        }
    }
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
        TypeAst::Optional(i) => Type::Optional(Box::new(ast_to_type(i))),
        TypeAst::Tuple(xs) => Type::Tuple(xs.iter().map(ast_to_type).collect()),
    }
}

#[derive(Clone, Debug)]
pub struct FuncSig {
    pub name: String,
    pub llvm: String,
    pub params: Vec<(String, Type, bool)>, // name, type, has_default
    pub ret: Type,
    pub is_method: bool,
    pub blueprint: Option<String>,
    pub is_async: bool,
    pub is_static: bool,
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
}

#[derive(Clone, Debug)]
pub struct ContractInfo {
    pub name: String,
    pub methods: Vec<FuncSig>,
}
