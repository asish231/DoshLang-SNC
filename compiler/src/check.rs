use crate::ast::*;
use crate::diag::Diagnostics;
use crate::span::Span;
use crate::types::*;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct EnumVariantInfo {
    pub name: String,
    pub fields: Vec<(String, Type)>,
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
}

impl CheckDb {
    pub fn is_algebraic_enum(&self, name: &str) -> bool {
        self.enum_variants.get(name).map_or(false, |vars| {
            vars.iter().any(|v| !v.fields.is_empty())
        })
    }
}

struct Checker<'a> {
    db: &'a mut CheckDb,
    diag: &'a mut Diagnostics,
    scopes: Vec<HashMap<String, (Type, bool)>>, // type, is_const
    moved: HashMap<String, Span>,
    /// owner name -> (borrower name, exclusive)
    borrowed: HashMap<String, (String, bool)>,
    /// borrower name -> owner name
    borrow_owner: HashMap<String, String>,
    /// locals of the function currently being checked, innermost last
    local_scopes: Vec<Vec<String>>,
    current_fn_ret: Type,
    in_loop: usize,
    current_bp: Option<String>,
    current_bp_hierarchy: Vec<String>,
    in_async: bool,
    in_static: bool,
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
        method_slots: HashMap::new(),
        vtable_methods: Vec::new(),
        extern_libs: Vec::new(),
        mono_variance: HashMap::new(),
        instantiated: HashMap::new(),
        fn_templates: HashMap::new(),
        instantiated_funcs: HashMap::new(),
        resolved_calls: HashMap::new(),
    };
    collect(&mut db, programs, diag);
    check_contracts(&mut db, diag);
    for p in programs {
        let mut c = Checker {
            db: &mut db,
            diag,
            scopes: vec![HashMap::new()],
            moved: HashMap::new(),
            borrowed: HashMap::new(),
            borrow_owner: HashMap::new(),
            local_scopes: Vec::new(),
            current_fn_ret: Type::Void,
            in_loop: 0,
            current_bp: None,
            current_bp_hierarchy: vec![],
            in_async: false,
            in_static: false,
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
            borrowed: HashMap::new(),
            borrow_owner: HashMap::new(),
            local_scopes: Vec::new(),
            current_fn_ret: Type::Void,
            in_loop: 0,
            current_bp: None,
            current_bp_hierarchy: vec![],
            in_async: false,
            in_static: false,
        };
        c.current_bp = Some(inst.name.clone());
        c.current_bp_hierarchy = hierarchy(&c.db.blueprints, &inst.name);
        for m in &inst.methods {
            c.in_async = m.is_async;
            c.check_fn(m);
            c.in_async = false;
        }
    }
    db
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
        | Stmt::Defer { body, .. } => {
            substitute_stmts(body, params, args);
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
                    Arg::Pos(e) | Arg::Named { value: e, .. } => substitute_expr(e, params, args),
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
                    Arg::Pos(e) | Arg::Named { value: e, .. } => substitute_expr(e, params, args),
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
                    Arg::Pos(e) | Arg::Named { value: e, .. } => substitute_expr(e, params, args),
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
        Stmt::While { body, .. } | Stmt::ForIn { body, .. } | Stmt::Defer { body, .. } => {
            walk_stmts_for_mono(body, db, diag);
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
                        ret: m.ret.as_ref().map(ast_to_type).unwrap_or(Type::Void),
                        is_method: true,
                        blueprint: None,
                        is_async: false,
                        is_static: false,
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
                Item::Stmt(Stmt::Decl { names, ty, .. }) => {
                    if let (Some(n), Some(t)) = (names.first(), ty) {
                        db.globals.insert(n.clone(), resolve_type_ast(t, db));
                    }
                }
                _ => {}
            }
        }
    }
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
        TypeAst::Optional(inner) => Type::Optional(Box::new(resolve_type_ast(inner, db))),
        TypeAst::Tuple(ts) => Type::Tuple(ts.iter().map(|x| resolve_type_ast(x, db)).collect()),
        TypeAst::Fn { params, ret } => Type::Fn {
            params: params.iter().map(|x| resolve_type_ast(x, db)).collect(),
            ret: Box::new(ret.as_ref().map(|x| resolve_type_ast(x, db)).unwrap_or(Type::Void)),
        },
        _ => ast_to_type(t),
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
        llvm,
        params: f
            .params
            .iter()
            .map(|p| (p.name.clone(), resolve_type_ast(&p.ty, db), p.default.is_some()))
            .collect(),
        ret: f.ret.as_ref().map(|t| resolve_type_ast(t, db)).unwrap_or(Type::Void),
        is_method: bp.is_some(),
        blueprint: bp,
        is_async: f.is_async,
        is_static: f.is_static,
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
    }
    fn pop(&mut self) {
        self.scopes.pop();
    }
    fn declare(&mut self, name: &str, ty: Type, is_const: bool, span: Span) {
        if let Some(frame) = self.local_scopes.last_mut() {
            frame.push(name.to_string());
        }
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
        self.push();
        self.moved.clear();
        self.borrowed.clear();
        self.borrow_owner.clear();
        self.local_scopes.push(Vec::new());
        if let Some(bp) = &self.current_bp {
            if !f.is_static {
                self.declare("self", Type::Blueprint(bp.clone()), true, f.span);
            }
        }
        for p in &f.params {
            self.declare(&p.name, resolve_type_ast(&p.ty, self.db), false, f.span);
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
        if let Some(frame) = self.local_scopes.pop() {
            self.end_local_scope(&frame);
        }
        self.pop();
        self.in_static = prev_static;
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
                span,
            } => {
                if names.len() == 1 {
                    let name = &names[0];
                    let val_ty = value.as_ref().map(|v| self.check_expr(v));
                    let decl_ty = if let Some(t) = ty {
                        let t = resolve_type_ast(t, self.db);
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
                        val_ty.unwrap_or(decl_ty)
                    } else {
                        decl_ty
                    };
                    Self::check_int_literal_range(&decl_ty, value.as_ref(), *span, self.diag);
                    if let Type::List(elem) = &decl_ty {
                        if let Some(Expr { kind: ExprKind::List(els), .. }) = value.as_ref() {
                            for el in els {
                                Self::check_int_literal_range(elem, Some(el), el.span, self.diag);
                            }
                        }
                    }
                    if let Type::Map(k_ty, v_ty) = &decl_ty {
                        if let Some(Expr { kind: ExprKind::Map(entries), .. }) = value.as_ref() {
                            for (k_el, v_el) in entries {
                                Self::check_int_literal_range(k_ty, Some(k_el), k_el.span, self.diag);
                                Self::check_int_literal_range(v_ty, Some(v_el), v_el.span, self.diag);
                            }
                        }
                    }
                    let decl_ty_clone = decl_ty.clone();
                    self.declare(name, decl_ty, *is_const, *span);
                    // Borrow creation and move-out.
                    let exclusive = matches!(decl_ty_clone, Type::Mut(_));
                    if Self::is_borrow_type(&decl_ty_clone) {
                        if let Some(val) = value {
                            match &val.kind {
                                // ref<T> r = address(x)  /  mut<T> m = address(x)
                                ExprKind::Call { callee, args, .. } => {
                                    let is_address = matches!(&callee.kind,
                                        ExprKind::Ident(f) if f == "address" || f == "as_ptr");
                                    if is_address {
                                        if let Some(Arg::Pos(a)) = args.first() {
                                            if let ExprKind::Ident(owner) = &a.kind {
                                                self.register_borrow(
                                                    name, owner, exclusive, *span,
                                                );
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
                                        // Following a borrow: the source borrow
                                        // is consumed, and the underlying owner
                                        // stays borrowed by the new name.
                                        if let Some(owner) =
                                            self.borrow_owner.get(src).cloned()
                                        {
                                            self.borrowed.remove(&owner);
                                            self.borrowed.insert(
                                                owner.clone(),
                                                (name.to_string(), exclusive),
                                            );
                                            self.borrow_owner
                                                .insert(name.to_string(), owner);
                                        }
                                        self.moved.insert(src.clone(), *span);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                } else {
                    let vt = value.as_ref().map(|v| self.check_expr(v));
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
                let tt = self.check_expr(target);
                let vt = self.check_expr(value);
                if let ExprKind::Ident(n) = &target.kind {
                    if self.lookup(n).map(|(_, c)| c).unwrap_or(false) && *op == AssignOp::Eq {
                        self.diag.error(*span, format!("cannot assign to const '{n}'"));
                    }
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
                            for (k_el, v_el) in entries {
                                Self::check_int_literal_range(k_ty, Some(k_el), k_el.span, self.diag);
                                Self::check_int_literal_range(v_ty, Some(v_el), v_el.span, self.diag);
                            }
                        }
                    }
                    if let ExprKind::Ident(n) = &target.kind {
                        if let Some((Type::Ref(_), _)) = self.lookup(n) {
                            if self.moved.contains_key(n) {
                                self.diag.error(*span, format!("use of moved ref '{n}'"));
                            }
                        }
                    }
                    if let ExprKind::Ident(n) = &value.kind {
                        if let Some((Type::Ref(_), _)) = self.lookup(n) {
                            self.moved.insert(n.clone(), *span);
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
                ..
            } => {
                self.push();
                self.check_stmt(init);
                if self.check_expr(cond) != Type::Bool {
                    self.diag.error(cond.span, "for condition must be bool");
                }
                self.check_stmt(step);
                self.in_loop += 1;
                for s in body {
                    self.check_stmt(s);
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
                                                                Arg::Pos(e) | Arg::Named { value: e, .. } => e,
                                                            };
                                                            if let ExprKind::Ident(var_name) = &arg_e.kind {
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
            Stmt::Return { value, span } => {
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
                        if let ExprKind::Ident(n) = &val.kind {
                            if let Some(owner) = self.borrow_owner.get(n).cloned() {
                                if self.lookup(&owner).is_some() {
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
                    }
                }
            }
            Stmt::Stop(span) | Stmt::Skip(span) => {
                if self.in_loop == 0 {
                    self.diag.error(*span, "stop/skip outside loop");
                }
            }
            Stmt::SpawnBlock { body, .. } => {
                for s in body {
                    self.check_stmt(s);
                }
            }
            Stmt::GoroutineBlock { body, .. } => {
                for s in body {
                    self.check_stmt(s);
                }
            }
            Stmt::SpawnExpr { expr, .. } => {
                self.check_expr(expr);
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
            }
            Stmt::Defer { body, .. } => {
                for s in body {
                    self.check_stmt(s);
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
            match a {
                Arg::Pos(e) => {
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
                if self.moved.contains_key(n) {
                    if let Some((Type::Ref(_), _)) = self.lookup(n) {
                        self.diag.error(e.span, format!("use of moved ref '{n}'"));
                    }
                }
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
                Type::Map(
                    Box::new(kt.unwrap_or(Type::Str)),
                    Box::new(vt.unwrap_or(Type::Int)),
                )
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
                    if let Arg::Pos(ex) = a {
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
                    Type::Tuple(ts) if ts.len() == 2 => ts[0].clone(),
                    other => other,
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
                if lt != rt && !(lt.int_min_max().is_some() && rt.int_min_max().is_some()) {
                    self.diag.error(lhs.span, "comparison type mismatch");
                }
                Type::Bool
            }
            BinOp::Add if lt == Type::Str && rt == Type::Str => Type::Str,
            BinOp::In => {
                match (&lt, &rt) {
                    (elem, Type::List(el)) if **el == *elem => Type::Bool,
                    (Type::Str, Type::Str) => Type::Bool,
                    _ => {
                        self.diag.error(
                            lhs.span,
                            format!("'in' requires elem in list<T> or substring in str, got {lt} and {rt}"),
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
                        // allow Int literal promotion to float at type level; IR needs explicit cast for vars
                        let lit_ok = (lt == Type::Float && Self::int_lit_value(rhs).is_some())
                            || (rt == Type::Float && Self::int_lit_value(lhs).is_some());
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

    fn check_call(&mut self, callee: &Expr, type_args: &[TypeAst], args: &[Arg], span: Span) -> Type {
        if let ExprKind::Member { base, name } = &callee.kind {
            // Static call: BlueprintName.static_method(...)
            if let ExprKind::Ident(bpname) = &base.kind {
                if self.lookup(bpname).is_none() && self.db.blueprints.contains_key(bpname) {
                    for a in args {
                        match a {
                            Arg::Pos(e) | Arg::Named { value: e, .. } => {
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
                                        Arg::Pos(e) | Arg::Named { value: e, .. } => e,
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
                    Arg::Pos(e) | Arg::Named { value: e, .. } => {
                        self.check_expr(e);
                    }
                }
            }
            let t = self.method_ret(&bt, name, args, span);
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
                                Arg::Pos(e) | Arg::Named { value: e, .. } => e,
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
                        Arg::Pos(e) | Arg::Named { value: e, .. } => {
                            self.check_expr(e);
                        }
                    }
                }
                if let Some(sig) = self.db.funcs.get(&mono).cloned() {
                    self.check_arity(&sig, args, span);
                    return sig.ret;
                }
            }
            for a in args {
                match a {
                    Arg::Pos(e) | Arg::Named { value: e, .. } => {
                        self.check_expr(e);
                    }
                }
            }
            return self.ident_call(n, args, span);
        }
        let ct = self.check_expr(callee);
        for a in args {
            match a {
                Arg::Pos(e) | Arg::Named { value: e, .. } => {
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
            "print" | "printn" | "free" | "set" | "panic" => Type::Void,
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
            "bytearray_new" | "bytearray_slice" => Type::Ptr,
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
                Arg::Pos(e) | Arg::Named { value: e, .. } => e,
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
            // A moved binding becomes valid again in the enclosing scope only if
            // it was declared here; otherwise the move still stands.
            self.moved.remove(n);
        }
    }

    fn is_borrow_type(t: &Type) -> bool {
        matches!(t, Type::Ref(_) | Type::Mut(_))
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

    fn method_ret(&mut self, bt: &Type, name: &str, args: &[Arg], span: Span) -> Type {
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
                    if !elem.assignable_from(&t, &self.db.blueprints) {
                        self.diag.error(e.span, format!("cannot push {t} into list<{elem}>"));
                    }
                }
                Type::Void
            }
            (Type::List(_), "length") => Type::Int,
            (Type::Map(_, _), "length") => Type::Int,
            (Type::Map(k, _), "keys") => Type::List(k.clone()),
            (Type::Map(_, v), "values") => Type::List(v.clone()),
            (Type::Chan(_), "send") => Type::Void,
            (Type::Chan(t), "receive") => *t.clone(),
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
                                Arg::Pos(e) | Arg::Named { value: e, .. } => { self.check_expr(e); }
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
