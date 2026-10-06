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
    pub access: Access,
    pub name: String,
    pub type_params: Vec<TypeParam>,
    pub params: Vec<Param>,
    pub ret: Option<TypeAst>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Param {
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
    pub variants: Vec<String>,
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

#[derive(Clone, Debug)]
pub struct BlueprintItem {
    pub name: String,
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
