use crate::ast::*;
use crate::check::CheckDb;
use crate::check::resolve_type_ast;
use crate::span::SourceFile;
use crate::types::*;
use std::collections::HashMap;

pub struct IrModule {
    pub strings: Vec<String>,
    pub functions: Vec<String>,
    pub has_main: bool,
    pub main_ret_void: bool,
    pub globals: Vec<String>,
    /// When coverage is on, the `(file_id, byte_offset)` behind each counter
    /// slot. The driver turns offsets into line numbers using the source table.
    pub cov: Vec<(u32, u32)>,
    /// DWARF sidecar: `DIFile` / `DISubprogram` / `DILocation` nodes for the
    /// `.sn` sources, consumed by `snc debug` (lldb) via `emit_debug_info`.
    pub dbg: DbgInfo,
}

/// Debug-info sidecar for a lowered module.
///
/// Every metadata node carries the id it was assigned from one shared
/// counter, so the emitter only prints nodes sorted by id — the numbering in
/// `!dbg` attachments and the node table can never disagree.
#[derive(Default)]
pub struct DbgInfo {
    /// Next free metadata id. Ids 0 and 1 are the module flags.
    next_id: u32,
    /// `(id, body)` for `!DIFile`.
    pub files: Vec<(u32, String)>,
    /// Maps a `Span::file` index to its `!DIFile` id.
    pub file_map: Vec<u32>,
    /// `(id, scope_name, file_idx, line, llvm_name)` for `!DISubprogram`.
    pub subs: Vec<(u32, String, u32, u32, String)>,
    /// `(id, subprogram_id)` for `!DISubroutineType`.
    pub tys: Vec<(u32, u32)>,
    /// `(id, subprogram_id, file_idx, line, col)` for `!DILocation`.
    pub lines: Vec<(u32, u32, u32, u32, u32)>,
    /// Id of the `!DICompileUnit`.
    pub cu: u32,
}

impl DbgInfo {
    /// Reserve the compile-unit id up front so it never collides.
    fn new() -> Self {
        let mut d = DbgInfo::default();
        d.cu = d.alloc();
        d
    }

    fn alloc(&mut self) -> u32 {
        // Ids 0 and 1 are taken by the Debug Info Version module flags.
        self.next_id += 1;
        if self.next_id < 2 {
            self.next_id = 2;
        }
        self.next_id
    }

    fn file_id(&mut self, path: &str, dir: &str) -> u32 {
        if let Some((id, _)) = self.files.iter().find(|(_, b)| b.contains(path)) {
            return *id;
        }
        let id = self.alloc();
        self.files.push((
            id,
            format!(
                "!DIFile(filename: {}, directory: {})",
                llvm_str(path),
                llvm_str(dir)
            ),
        ));
        id
    }

    /// The `!DIFile` id for a `Span::file` index.
    pub fn file_id_of(&self, idx: u32) -> u32 {
        self.file_map
            .get(idx as usize)
            .copied()
            .or_else(|| self.files.first().map(|(id, _)| *id))
            .unwrap_or(0)
    }

    /// Register a subprogram; returns its metadata id.
    fn sub(&mut self, name: &str, file: u32, line: u32, llvm_name: &str) -> u32 {
        let id = self.alloc();
        self.subs.push((
            id,
            name.to_string(),
            file,
            line,
            llvm_name.to_string(),
        ));
        let tid = self.alloc();
        self.tys.push((tid, id));
        id
    }

    /// Register (or reuse) a `!DILocation`, returning its metadata id.
    fn diloc(&mut self, scope: u32, file: u32, line: u32, col: u32) -> u32 {
        if let Some((id, _, _, _, _)) = self
            .lines
            .iter()
            .find(|(_, s, f, l, c)| *s == scope && *f == file && *l == line && *c == col)
        {
            return *id;
        }
        let id = self.alloc();
        self.lines.push((id, scope, file, line, col));
        id
    }
}

pub fn llvm_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\0A"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

struct Cx<'a> {
    db: &'a CheckDb,
    buf: String,
    tmp: u32,
    lbl: u32,
    locals: HashMap<String, (String, Type)>,
    loops: Vec<(String, String)>,
    /// Locals declared inside a loop body. Their storage does not dominate
    /// code after the loop, so they must not be referenced from there.
    loop_locals: std::collections::HashSet<String>,
    strings: &'a mut Vec<String>,
    extra_fns: &'a mut Vec<String>,
    current_bp: Option<String>,
    /// Source file table, for resolving spans to `!dbg` line locations.
    files: &'a [SourceFile],
    /// `Some((file_idx, line, col))` for the statement being lowered.
    cur_loc: Option<(u32, u32, u32)>,
    /// Active `!DISubprogram` id; `!dbg` attachments use it as the scope.
    cur_sub: u32,
    dbg: &'a mut DbgInfo,
    terminated: bool,
    current_fn_ret: Type,
    defers: Vec<Vec<Stmt>>,
}

/// Coverage instrumentation state for the current lowering pass.
///
/// Lowering is single-threaded, so a thread-local keeps `Cx` free of another
/// field to thread through every helper. Each distinct source line gets one
/// counter slot; `sn_cov_hit` bumps it at run time.
#[derive(Default)]
struct CovState {
    on: bool,
    slots: Vec<(u32, u32)>,
    index: std::collections::HashMap<(u32, u32), u64>,
}

thread_local! {
    static COV: std::cell::RefCell<CovState> = std::cell::RefCell::new(CovState::default());
}

/// Slot id for a source line, allocating one on first use.
fn cov_slot(file: u32, line: u32) -> Option<u64> {
    COV.with(|c| {
        let mut c = c.borrow_mut();
        if !c.on {
            return None;
        }
        let key = (file, line);
        if let Some(i) = c.index.get(&key) {
            return Some(*i);
        }
        let id = c.slots.len() as u64;
        c.slots.push(key);
        c.index.insert(key, id);
        Some(id)
    })
}

/// Begin (or end) collecting coverage slots. Returns what was collected.
pub fn cov_collect(on: bool) -> Vec<(u32, u32)> {
    COV.with(|c| {
        let mut c = c.borrow_mut();
        if on {
            c.on = true;
            c.slots.clear();
            c.index.clear();
        } else {
            c.on = false;
            let s = std::mem::take(&mut c.slots);
            c.index.clear();
            return s;
        }
        Vec::new()
    })
}

pub fn lower(programs: &[Program], db: &CheckDb, files: &[SourceFile]) -> IrModule {
    let mut strings = Vec::new();
    let mut functions = Vec::new();
    let mut extra = Vec::new();
    let mut has_main = false;
    let mut main_ret_void = true;
    let mut dbg = DbgInfo::new();
    let root_dir = files
        .first()
        .and_then(|f| {
            std::path::Path::new(&f.path)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
        })
        .unwrap_or_default();
    for f in files {
        let id = dbg.file_id(&f.path, &root_dir);
        while dbg.file_map.len() <= f.id as usize {
            dbg.file_map.push(id);
        }
        dbg.file_map[f.id as usize] = id;
    }
    for p in programs {
        for item in &p.items {
            match item {
                Item::Extern(x) => {
                    for f in &x.funcs {
                        functions.push(lower_extern(f, db));
                    }
                }
                Item::Fn(f) => {
                    if !f.type_params.is_empty() {
                        continue;
                    }
                    if f.name == "main" {
                        has_main = true;
                        main_ret_void = f.ret.is_none();
                    }
                    functions.push(lower_fn(
                        f,
                        db,
                        None,
                        &mut strings,
                        &mut extra,
                        files,
                        &mut dbg,
                    ));
                }
                Item::Blueprint(b) => {
                    if !b.type_params.is_empty() {
                        continue;
                    }
                    for m in &b.methods {
                        functions.push(lower_fn(
                            m,
                            db,
                            Some(b.name.clone()),
                            &mut strings,
                            &mut extra,
                            files,
                            &mut dbg,
                        ));
                    }
                }
                Item::Extension(e) => {
                    let (key, self_ty) = match &e.ty {
                        TypeAst::Str => ("str", Type::Str),
                        TypeAst::Int => ("int", Type::Int),
                        TypeAst::Named(n) => (n.as_str(), Type::Named(n.clone())),
                        _ => ("ext", Type::Int),
                    };
                    for m in &e.methods {
                        functions.push(lower_ext_fn(
                            m,
                            db,
                            key,
                            &self_ty,
                            &mut strings,
                            &mut extra,
                            files,
                            &mut dbg,
                        ));
                    }
                }
                _ => {}
            }
        }
    }
    for inst in db.instantiated.values() {
        for m in &inst.methods {
            functions.push(lower_fn(
                m,
                db,
                Some(inst.name.clone()),
                &mut strings,
                &mut extra,
                files,
                &mut dbg,
            ));
        }
    }
    for inst in db.instantiated_funcs.values() {
        functions.push(lower_fn(
            inst,
            db,
            None,
            &mut strings,
            &mut extra,
            files,
            &mut dbg,
        ));
    }
    functions.extend(extra);
    let mut globals = Vec::new();
    for (name, info) in &db.blueprints {
        if info.is_generic || info.size == 0 {
            continue;
        }
        globals.push(build_vtable(db, name));
    }
    IrModule {
        strings,
        functions,
        has_main,
        main_ret_void,
        globals,
        cov: cov_collect(false),
        dbg,
    }
}

/// Emit the per-class vtable constant. Slot i in the emitted array MUST match
/// db.method_slots[db.vtable_methods[i]] so runtime load indices from dispatch
/// agree with the emitted layout for every class.
fn build_vtable(db: &CheckDb, cls: &str) -> String {
    let g = db.vtable_methods.len();
    let mut entries: Vec<String> = vec!["ptr null".to_string(); g];
    let mut chain = Vec::new();
    let mut cur = Some(cls.to_string());
    while let Some(name) = cur {
        chain.push(name.clone());
        cur = db.blueprints.get(&name).and_then(|b| b.parent.clone());
    }
    // child-first: nearest-to-derive override wins the slot
    for (i, m) in db.vtable_methods.iter().enumerate() {
        for owner in &chain {
            if let Some(info) = db.blueprints.get(owner) {
                if info.methods.contains(m) {
                    entries[i] = format!("ptr @sn_m_{}_{}", owner, m);
                    break;
                }
            }
        }
    }
    format!("@vtab_{} = internal constant [{} x ptr] [{}]", cls, g, entries.join(", "))
}

/// `extern` signatures become LLVM `declare`s resolved at link time.
/// `str` parameters and results are marshalled to/from NUL-terminated C
/// strings so ordinary C APIs work unchanged.
fn lower_extern(f: &FnItem, db: &CheckDb) -> String {
    let ret_ty = f
        .ret
        .as_ref()
        .map(|t| resolve_type_ast(t, db))
        .unwrap_or(Type::Void);
    let mut params = Vec::new();
    for p in &f.params {
        let t = resolve_type_ast(&p.ty, db);
        params.push(ffi_param_ty(&t));
    }
    let rty = ffi_ret_ty(&ret_ty);
    format!(
        "declare {rty} @{}({})",
        f.name,
        params.join(", ")
    )
}

/// C ABI mapping for a parameter type.
fn ffi_param_ty(t: &Type) -> String {
    match t {
        Type::Void => "void".into(),
        Type::Float => "double".into(),
        Type::Bool => "i8".into(),
        Type::Byte | Type::U8 | Type::I8 => "i8".into(),
        Type::U16 | Type::I16 => "i16".into(),
        Type::U32 | Type::I32 => "i32".into(),
        // Fixed-width unsigned types are exactly C's fixed-width integers.
        Type::Int | Type::U64 => "i64".into(),
        // SN strings are heap objects; hand C a NUL-terminated char*.
        _ => "ptr".into(),
    }
}

/// C ABI mapping for a return type.
fn ffi_ret_ty(t: &Type) -> String {
    match t {
        Type::Void => "void".into(),
        Type::Float => "double".into(),
        Type::Bool => "i8".into(),
        Type::Byte | Type::U8 | Type::I8 => "i8".into(),
        Type::U16 | Type::I16 => "i16".into(),
        Type::U32 | Type::I32 => "i32".into(),
        Type::Int | Type::U64 => "i64".into(),
        _ => "ptr".into(),
    }
}

fn lower_fn(
    f: &FnItem,
    db: &CheckDb,
    bp: Option<String>,
    strings: &mut Vec<String>,
    extra: &mut Vec<String>,
    files: &[SourceFile],
    dbg: &mut DbgInfo,
) -> String {
    let llvm_name = if let Some(b) = &bp {
        format!("sn_m_{}_{}", b, f.name)
    } else {
        format!("sn_fn_{}", f.name)
    };
    let ret_ty = f.ret.as_ref().map(|t| resolve_type_ast(t, db)).unwrap_or(Type::Void);
    // Register a DISubprogram so `!dbg` locations resolve to this function.
    let decl_line = files
        .get(f.span.file as usize)
        .map(|sf| sf.loc(f.span.start as usize).0)
        .unwrap_or(1);
    let scope_name = match &bp {
        Some(b) => format!("{b}.{}", f.name),
        None => f.name.clone(),
    };
    let sub_id = dbg.sub(&scope_name, f.span.file, decl_line, &llvm_name);
    let mut cx = Cx {
        db,
        buf: String::new(),
        tmp: 0,
        lbl: 0,
        locals: HashMap::new(),
        loops: Vec::new(),
        loop_locals: std::collections::HashSet::new(),
        strings,
        extra_fns: extra,
        current_bp: bp.clone(),
        files,
        cur_loc: None,
        cur_sub: sub_id,
        dbg,
        terminated: false,
        current_fn_ret: ret_ty.clone(),
        defers: Vec::new(),
    };
    let mut params_ll = Vec::new();
    if bp.is_some() && !f.is_static {
        params_ll.push("ptr %p_self".to_string());
    }
    for p in &f.params {
        let t = resolve_type_ast(&p.ty, db);
        params_ll.push(format!("{} %p_{}", llty(&t), p.name));
    }
    let rty = llty_ret(&ret_ty);
    // The `!dbg` attachment links the function to its DISubprogram; without
    // it LLVM emits no DWARF for the body even when every instruction has a
    // location.
    cx.raw(&format!(
        "define {rty} @{llvm_name}({}) !dbg !{sub_id} {{",
        params_ll.join(", ")
    ));
    cx.raw("entry:");
    if bp.is_some() && !f.is_static {
        cx.line("%v_self = alloca ptr");
        cx.line("store ptr %p_self, ptr %v_self");
        cx.locals
            .insert("self".into(), ("%v_self".into(), Type::Blueprint(bp.clone().unwrap())));
    }
    for p in &f.params {
        let t = resolve_type_ast(&p.ty, db);
        let an = format!("%v_{}", p.name);
        if let Type::Record(rn) = &t {
            let sz = db.records.get(rn).map(|r| r.size).unwrap_or(8);
            cx.line(&format!("{an} = alloca i8, i64 {sz}"));
            cx.line(&format!("call void @sn_record_copy(ptr {an}, ptr %p_{}, i64 {sz})", p.name));
        } else {
            cx.line(&format!("{an} = alloca {}", llty(&t)));
            cx.line(&format!("store {} %p_{}, ptr {an}", llty(&t), p.name));
        }
        cx.locals.insert(p.name.clone(), (an, t));
    }
    for s in &f.body {
        cx.stmt(s);
    }
    cx.emit_defers();
    if matches!(ret_ty, Type::Void) {
        cx.line("ret void");
    } else if ret_ty.is_ptr() || matches!(ret_ty, Type::Optional(_)) {
        cx.line("ret ptr null");
    } else if let Type::Tuple(ts) = &ret_ty {
        let st = tuple_ll(ts);
        cx.line(&format!("ret {st} zeroinitializer"));
    } else {
        cx.line("ret i64 0");
    }
    cx.raw("}");
    cx.buf
}

impl Cx<'_> {
    /// Resolve a statement to `(file_idx, line, col)`, or `None` when the
    /// span is outside the loaded file table.
    fn stmt_loc(&self, s: &Stmt) -> Option<(u32, u32, u32)> {
        let span = match s {
            Stmt::Expr(e) => &e.span,
            Stmt::Decl { span, .. }
            | Stmt::Assign { span, .. }
            | Stmt::If { span, .. }
            | Stmt::While { span, .. }
            | Stmt::ForCount { span, .. }
            | Stmt::ForIn { span, .. }
            | Stmt::Match { span, .. }
            | Stmt::Return { span, .. }
            | Stmt::Stop(span)
            | Stmt::Skip(span)
            | Stmt::SpawnBlock { span, .. }
            | Stmt::SpawnExpr { span, .. }
            | Stmt::LockBlock { span, .. }
            | Stmt::New { span, .. }
            | Stmt::GoroutineBlock { span, .. }
            | Stmt::Defer { span, .. }
            | Stmt::Asm { span, .. } => span,
            Stmt::NestedFn(_) => return None,
        };
        let f = self.files.get(span.file as usize)?;
        let (line, col) = f.loc(span.start as usize);
        Some((span.file, line, col))
    }

    /// Bump the coverage counter for this statement's line.
    fn cov_stmt(&mut self, s: &Stmt) {
        let span = match s {
            Stmt::Expr(e) => &e.span,
            Stmt::Decl { span, .. }
            | Stmt::Assign { span, .. }
            | Stmt::If { span, .. }
            | Stmt::While { span, .. }
            | Stmt::ForCount { span, .. }
            | Stmt::ForIn { span, .. }
            | Stmt::Match { span, .. }
            | Stmt::Return { span, .. }
            | Stmt::Stop(span)
            | Stmt::Skip(span)
            | Stmt::SpawnBlock { span, .. }
            | Stmt::SpawnExpr { span, .. }
            | Stmt::LockBlock { span, .. }
            | Stmt::New { span, .. }
            | Stmt::GoroutineBlock { span, .. }
            | Stmt::Defer { span, .. }
            | Stmt::Asm { span, .. } => span,
            Stmt::NestedFn(_) => return,
        };
        // Keyed by byte offset: `Cx` has no source-line table, and the driver
        // resolves offsets to lines once lowering finishes.
        let Some(slot) = cov_slot(span.file, span.start) else {
            return;
        };
        // The runtime keeps one counter per slot; nothing to allocate here.
        self.line(&format!("call void @sn_cov_hit(i64 {slot})"));
    }

    fn line(&mut self, s: &str) {
        if self.terminated {
            return;
        }
        self.buf.push_str("  ");
        self.buf.push_str(s);
        if let Some((file, ln, col)) = self.cur_loc {
            let id = self.dbg.diloc(self.cur_sub, file, ln, col);
            self.buf.push_str(&format!(", !dbg !{id}"));
        }
        self.buf.push('\n');
        if s.starts_with("ret ") || s.starts_with("br ") {
            self.terminated = true;
        }
    }
    fn raw(&mut self, s: &str) {
        if s.ends_with(':') {
            self.terminated = false;
            self.buf.push_str(s);
            self.buf.push('\n');
            return;
        }
        if s.trim() == "}" {
            self.buf.push_str(s);
            self.buf.push('\n');
            return;
        }
        if self.terminated {
            return;
        }
        self.buf.push_str(s);
        self.buf.push('\n');
    }
    /// Emit a side-effect LLVM inline-assembly call. Clobbers are
    /// rendered as `~{name}` in the constraint string so LLVM knows which
    /// CPU state the assembly may clobber.
    fn emit_asm(&mut self, template: &str, clobbers: &[String]) {
        let esc = template.replace('\\', "\\\\").replace('"', "\\22");
        let constr = if clobbers.is_empty() {
            "\"\"".to_string()
        } else {
            let mut c = String::from("\"");
            for (i, cl) in clobbers.iter().enumerate() {
                if i > 0 {
                    c.push(',');
                }
                c.push_str(&format!("~ {{{}}}", cl.trim_matches('{').trim_matches('}')).replace(' ', ""));
            }
            c.push('"');
            c
        };
        self.line(&format!("call void asm sideeffect \"{}\", {}()", esc, constr));
    }

    fn t(&mut self) -> String {
        self.tmp += 1;
        format!("%t{}", self.tmp)
    }
    fn l(&mut self, p: &str) -> String {
        self.lbl += 1;
        format!("{p}{}", self.lbl)
    }

    fn stmt(&mut self, s: &Stmt) {
        self.cur_loc = self.stmt_loc(s);
        self.cov_stmt(s);
        match s {
            Stmt::Expr(e) => {
                self.expr(e);
            }
            Stmt::Decl {
                names, value, ty, ..
            } => {
                if names.len() == 1 {
                    let name = &names[0];
                    let t = if let Some(ta) = ty {
                        resolve_type_ast(ta, self.db)
                    } else if let Some(v) = value {
                        self.lookup_ty(v)
                    } else {
                        Type::Int
                    };
                    let t = match t {
                        Type::None => Type::Optional(Box::new(Type::Int)),
                        Type::Named(_) => value.as_ref().map(|v| self.lookup_ty(v)).unwrap_or(Type::Int),
                        other => other,
                    };
                    let an = format!("%v_{}_{}", name, self.l("lv"));
                    if !self.locals.contains_key(name) {
                        match &t {
                            Type::Record(rn) => {
                                let sz = self.db.records.get(rn).map(|r| r.size).unwrap_or(8);
                                self.line(&format!("{an} = alloca i8, i64 {sz}"));
                            }
                            _ => self.line(&format!("{an} = alloca {}", llty(&t))),
                        }
                        self.locals.insert(name.clone(), (an.clone(), t.clone()));
                        if !self.loops.is_empty() {
                            self.loop_locals.insert(name.clone());
                        }
                    }
                    let an = self.locals.get(name).unwrap().0.clone();
                    if let Some(v) = value {
                        let (vr, vt) = self.expr(v);
                        if matches!(t, Type::Record(_)) {
                            let (vr, _) = self.coerce(vr, &vt, &t);
                            let sz = if let Type::Record(rn) = &t {
                                self.db.records.get(rn).map(|r| r.size).unwrap_or(8)
                            } else {
                                8
                            };
                            self.line(&format!("call void @sn_record_copy(ptr {an}, ptr {vr}, i64 {sz})"));
                        } else {
                            let (vr, _) = self.coerce(vr, &vt, &t);
                            self.line(&format!("store {} {vr}, ptr {an}", llty(&t)));
                        }
                    } else if matches!(t, Type::Record(_)) {
                        // stack record default-init to zero
                    } else if t == Type::Lock {
                        let c = self.t();
                        self.line(&format!("{c} = call ptr @sn_lock_new()"));
                        self.line(&format!("store ptr {c}, ptr {an}"));
                    } else if matches!(t, Type::Chan(_)) {
                        let c = self.t();
                        self.line(&format!("{c} = call ptr @sn_chan_new()"));
                        self.line(&format!("store ptr {c}, ptr {an}"));
                    } else if t.is_ptr() || matches!(t, Type::Optional(_)) {
                        self.line(&format!("store ptr null, ptr {an}"));
                    } else {
                        self.line(&format!("store i64 0, ptr {an}"));
                    }
                } else if let Some(v) = value {
                    let (vr, vt) = self.expr(v);
                    if let Type::Tuple(ts) = vt {
                        for (i, n) in names.iter().enumerate() {
                            let el = self.t();
                            self.line(&format!(
                                "{el} = extractvalue {} {vr}, {i}",
                                tuple_ll(&ts)
                            ));
                            let an = format!("%v_{n}");
                            let et = ts[i].clone();
                            self.line(&format!("{an} = alloca {}", llty(&et)));
                            self.line(&format!("store {} {el}, ptr {an}", llty(&et)));
                            self.locals.insert(n.clone(), (an, et));
                        }
                    }
                }
            }
            Stmt::Assign { target, op, value, .. } => {
                let (vr, vt) = self.expr(value);
                if let ExprKind::Tuple(els) = &target.kind {
                    if let Type::Tuple(ts) = &vt {
                        for (i, el) in els.iter().enumerate() {
                            if let ExprKind::Ident(n) = &el.kind {
                                let (an, lt) = self.locals.get(n).cloned().unwrap();
                                let ev = self.t();
                                self.line(&format!(
                                    "{ev} = extractvalue {} {vr}, {i}",
                                    tuple_ll(ts)
                                ));
                                self.line(&format!("store {} {ev}, ptr {an}", llty(&lt)));
                            }
                        }
                    }
                    return;
                }
                match &target.kind {
                    ExprKind::Ident(n) => {
                        let (an, lt) = self.locals.get(n).cloned().expect("assign ident");
                        if *op == AssignOp::Eq && matches!(lt, Type::Record(_)) {
                            let (vr, _) = self.coerce(vr, &vt, &lt);
                            let sz = if let Type::Record(rn) = &lt {
                                self.db.records.get(rn).map(|r| r.size).unwrap_or(8)
                            } else {
                                8
                            };
                            self.line(&format!("call void @sn_record_copy(ptr {an}, ptr {vr}, i64 {sz})"));
                            return;
                        }
                        let src = if *op == AssignOp::Eq {
                            let (vr, _) = self.coerce(vr, &vt, &lt);
                            vr
                        } else {
                            let cur = self.t();
                            self.line(&format!("{cur} = load {}, ptr {an}", llty(&lt)));
                            self.arith_op(assign_bin(*op), cur, vr, &lt)
                        };
                        self.line(&format!("store {} {src}, ptr {an}", llty(&lt)));
                    }
                    ExprKind::Index { base, index } => {
                        let (b, bt) = self.expr(base);
                        let (i, _) = self.expr(index);
                        match bt {
                            Type::List(el) if el.is_ptr() || matches!(*el, Type::Optional(_)) => {
                                self.line(&format!("call void @sn_list_set_ptr(ptr {b}, i64 {i}, ptr {vr})"));
                            }
                            Type::List(el) => {
                                let vb = self.as_bits(vr, &el);
                                self.line(&format!("call void @sn_list_set_i64(ptr {b}, i64 {i}, i64 {vb})"));
                            }
                            Type::Map(k, v) => {
                                let kb = self.as_bits(i, &k);
                                let vb = self.as_bits(vr, &v);
                                self.line(&format!("call void @sn_map_set(ptr {b}, i64 {kb}, i64 {vb})"));
                            }
                            _ => {}
                        }
                    }
                    ExprKind::Member { base, name } => {
                        let (b, bt) = self.expr(base);
                        if let Type::Record(rn) = bt {
                            if let Some(info) = self.db.records.get(&rn) {
                                if let Some(f) = info.fields.iter().find(|f| f.name == *name) {
                                    let fp = self.t();
                                    self.line(&format!("{fp} = getelementptr i8, ptr {b}, i64 {}", f.offset));
                                    if matches!(f.ty, Type::Record(_)) {
                                        let sz = if let Type::Record(inner_rn) = &f.ty {
                                            self.db.records.get(inner_rn).map(|r| r.size).unwrap_or(8)
                                        } else { 8 };
                                        self.line(&format!("call void @sn_record_copy(ptr {fp}, ptr {vr}, i64 {sz})"));
                                    } else {
                                        let (vr, _) = self.coerce(vr, &vt, &f.ty);
                                        let (ft, _) = ll_field_ty(&f.ty);
                                        if ft == "i64" {
                                            self.line(&format!("store i64 {vr}, ptr {fp}"));
                                        } else if ft == "double" {
                                            self.line(&format!("store double {vr}, ptr {fp}"));
                                        } else {
                                            let tr = self.t();
                                            self.line(&format!("{tr} = trunc i64 {vr} to {ft}"));
                                            self.line(&format!("store {ft} {tr}, ptr {fp}"));
                                        }
                                    }
                                }
                            }
                        } else if let Type::Blueprint(bp) = bt {
                            if let Some(info) = self.db.blueprints.get(&bp) {
                                if let Some(f) = info.fields.iter().find(|f| f.name == *name) {
                                    let fp = self.t();
                                    self.line(&format!("{fp} = getelementptr i8, ptr {b}, i64 {}", f.offset));
                                    let (vr, _) = self.coerce(vr, &vt, &f.ty);
                                    self.line(&format!("store {} {vr}, ptr {fp}", llty(&f.ty)));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Stmt::If {
                cond,
                then_body,
                else_ifs,
                else_body,
                ..
            } => {
                let end = self.l("ifend");
                self.if_chain(cond, then_body, else_ifs, else_body, 0, &end);
                self.raw(&format!("{end}:"));
            }
            Stmt::While { cond, body, .. } => {
                let h = self.l("wh");
                let b = self.l("wb");
                let e = self.l("we");
                self.line(&format!("br label %{h}"));
                self.raw(&format!("{h}:"));
                let (c, _) = self.expr(cond);
                let c1 = self.as_i1(c);
                self.line(&format!("br i1 {c1}, label %{b}, label %{e}"));
                self.raw(&format!("{b}:"));
                self.loops.push((h.clone(), e.clone()));
                for s in body {
                    self.stmt(s);
                }
                self.loops.pop();
                self.line(&format!("br label %{h}"));
                self.raw(&format!("{e}:"));
            }
            Stmt::ForCount {
                init,
                cond,
                step,
                body,
                ..
            } => {
                self.stmt(init);
                let h = self.l("fh");
                let b = self.l("fb");
                let st = self.l("fs");
                let e = self.l("fe");
                self.line(&format!("br label %{h}"));
                self.raw(&format!("{h}:"));
                let (c, _) = self.expr(cond);
                let c1 = self.as_i1(c);
                self.line(&format!("br i1 {c1}, label %{b}, label %{e}"));
                self.raw(&format!("{b}:"));
                self.loops.push((st.clone(), e.clone()));
                for s in body {
                    self.stmt(s);
                }
                self.loops.pop();
                self.line(&format!("br label %{st}"));
                self.raw(&format!("{st}:"));
                self.stmt(step);
                self.line(&format!("br label %{h}"));
                self.raw(&format!("{e}:"));
            }
            Stmt::ForIn { name, iter, body, .. } => {
                let (it, ity) = self.expr(iter);
                let it = if let Type::Optional(_) = ity {
                    it
                } else {
                    it
                };
                let n = self.t();
                self.line(&format!("{n} = call i64 @sn_list_len(ptr {it})"));
                let ixn = format!("%v__i_{}", self.l("fi"));
                self.line(&format!("{ixn} = alloca i64"));
                self.line(&format!("store i64 0, ptr {ixn}"));
                let elem_ty = match &ity {
                    Type::List(e) => *e.clone(),
                    Type::Optional(inner) => match inner.as_ref() {
                        Type::List(e) => *e.clone(),
                        _ => Type::Int,
                    },
                    Type::Map(_, v) => *v.clone(),
                    _ => Type::Int,
                };
                let an = format!("%v_{}_{}", name, self.l("lv"));
                self.line(&format!("{an} = alloca {}", llty(&elem_ty)));
                self.locals.insert(name.clone(), (an.clone(), elem_ty.clone()));
                self.loop_locals.insert(name.clone());
                let h = self.l("ih");
                let b = self.l("ib");
                let st = self.l("is");
                let e = self.l("ie");
                self.line(&format!("br label %{h}"));
                self.raw(&format!("{h}:"));
                let i = self.t();
                self.line(&format!("{i} = load i64, ptr {ixn}"));
                let cmp = self.t();
                self.line(&format!("{cmp} = icmp slt i64 {i}, {n}"));
                self.line(&format!("br i1 {cmp}, label %{b}, label %{e}"));
                self.raw(&format!("{b}:"));
                let ev = self.t();
                if elem_ty.is_ptr() || matches!(elem_ty, Type::Optional(_)) {
                    self.line(&format!("{ev} = call ptr @sn_list_get_ptr(ptr {it}, i64 {i})"));
                    self.line(&format!("store ptr {ev}, ptr {an}"));
                } else {
                    self.line(&format!("{ev} = call i64 @sn_list_get_i64(ptr {it}, i64 {i})"));
                    let val = self.from_bits(ev, &elem_ty);
                    self.line(&format!("store {} {val}, ptr {an}", llty(&elem_ty)));
                }
                self.loops.push((st.clone(), e.clone()));
                for s in body {
                    self.stmt(s);
                }
                self.loops.pop();
                self.line(&format!("br label %{st}"));
                self.raw(&format!("{st}:"));
                let i2 = self.t();
                self.line(&format!("{i2} = load i64, ptr {ixn}"));
                let i3 = self.t();
                self.line(&format!("{i3} = add i64 {i2}, 1"));
                self.line(&format!("store i64 {i3}, ptr {ixn}"));
                self.line(&format!("br label %{h}"));
                self.raw(&format!("{e}:"));
            }
            Stmt::Match {
                expr,
                arms,
                default,
                ..
            } => {
                let (v, vt) = self.expr(expr);
                let end = self.l("mend");
                for (i, (pat, body)) in arms.iter().enumerate() {
                    let yes = self.l("my");
                    let no = self.l("mn");
                    let (p, _) = self.expr(pat);
                    let eq = self.eq_vals(v.clone(), p, &vt);
                    let c1 = self.as_i1(eq);
                    self.line(&format!("br i1 {c1}, label %{yes}, label %{no}"));
                    self.raw(&format!("{yes}:"));
                    for s in body {
                        self.stmt(s);
                    }
                    self.line(&format!("br label %{end}"));
                    self.raw(&format!("{no}:"));
                    let _ = i;
                }
                if let Some(b) = default {
                    for s in b {
                        self.stmt(s);
                    }
                }
                self.line(&format!("br label %{end}"));
                self.raw(&format!("{end}:"));
            }
            Stmt::Return { value, .. } => {
                if let Some(v) = value {
                    let (r, t) = self.expr(v);
                    self.emit_defers();
                    if let Type::Tuple(ts) = t {
                        let mut final_r = r;
                        for (i, elem_t) in ts.iter().enumerate() {
                            if let Type::Record(rn) = elem_t {
                                let sz = self.db.records.get(rn).map(|r| r.size).unwrap_or(8);
                                let st = tuple_ll(&ts);
                                let orig_ptr = self.t();
                                self.line(&format!("{orig_ptr} = extractvalue {st} {final_r}, {i}"));
                                let mem = self.t();
                                self.line(&format!("{mem} = call ptr @sn_alloc(i64 {sz})"));
                                self.line(&format!("call void @sn_record_copy(ptr {mem}, ptr {orig_ptr}, i64 {sz})"));
                                let next_r = self.t();
                                self.line(&format!("{next_r} = insertvalue {st} {final_r}, ptr {mem}, {i}"));
                                final_r = next_r;
                            }
                        }
                        self.line(&format!("ret {} {final_r}", tuple_ll(&ts)));
                    } else if let Type::Record(rn) = &t {
                        let sz = self.db.records.get(rn).map(|r| r.size).unwrap_or(8);
                        let mem = self.t();
                        self.line(&format!("{mem} = call ptr @sn_alloc(i64 {sz})"));
                        self.line(&format!("call void @sn_record_copy(ptr {mem}, ptr {r}, i64 {sz})"));
                        self.line(&format!("ret ptr {mem}"));
                    } else {
                        let ret_ty = self.current_fn_ret.clone();
                        let (r, _) = self.coerce(r, &t, &ret_ty);
                        self.line(&format!("ret {} {r}", llty(&ret_ty)));
                    }
                } else {
                    self.emit_defers();
                    self.line("ret void");
                }
            }
            Stmt::Stop(_) => {
                if let Some((_, brk)) = self.loops.last() {
                    let b = brk.clone();
                    self.line(&format!("br label %{b}"));
                    let dead = self.l("dead");
                    self.raw(&format!("{dead}:"));
                }
            }
            Stmt::Skip(_) => {
                if let Some((cont, _)) = self.loops.last() {
                    let c = cont.clone();
                    self.line(&format!("br label %{c}"));
                    let dead = self.l("dead");
                    self.raw(&format!("{dead}:"));
                }
            }
            Stmt::SpawnBlock { body, .. } => self.spawn_block(body, false),
            Stmt::GoroutineBlock { body, .. } => self.spawn_block(body, true),
            Stmt::SpawnExpr { expr, .. } => {
                if let ExprKind::Call { .. } = &expr.kind {
                    self.spawn_block(&[Stmt::Expr(expr.clone())], false);
                } else {
                    self.expr(expr);
                }
            }
            Stmt::LockBlock { name, body, .. } => {
                let (an, _) = self.locals.get(name).cloned().unwrap();
                let lk = self.t();
                self.line(&format!("{lk} = load ptr, ptr {an}"));
                self.line(&format!("call void @sn_lock_acq(ptr {lk})"));
                for s in body {
                    self.stmt(s);
                }
                self.line(&format!("call void @sn_lock_rel(ptr {lk})"));
            }
            Stmt::New { ty, type_args, name, args, .. } => {
                let resolved = if !type_args.is_empty() {
                    let parts: Vec<String> = type_args.iter().map(|a| match a {
                        TypeAst::Int => "int".into(),
                        TypeAst::Str => "str".into(),
                        TypeAst::Bool => "bool".into(),
                        TypeAst::Named(n) => n.clone(),
                        _ => "T".into(),
                    }).collect();
                    format!("{}_{}", ty, parts.join("_"))
                } else {
                    ty.clone()
                };
                let info = self.db.blueprints.get(&resolved).cloned().unwrap();
                let tid = info.type_id;
                let obj = self.t();
                self.line(&format!(
                    "{obj} = call ptr @sn_obj_new(i64 {}, i64 {tid}, ptr @vtab_{})",
                    info.size, resolved
                ));
                for f in &info.fields {
                    if let Some(init) = &f.init {
                        let (vr, vt) = self.expr(init);
                        let (vr, _) = self.coerce(vr, &vt, &f.ty);
                        let fp = self.t();
                        self.line(&format!(
                            "{fp} = getelementptr i8, ptr {obj}, i64 {}",
                            f.offset
                        ));
                        self.line(&format!("store {} {vr}, ptr {fp}", llty(&f.ty)));
                    }
                }
                let mut pos = 0usize;
                for a in args {
                    match a {
                        Arg::Named { name: fnm, value } => {
                            if let Some(f) = info.fields.iter().find(|f| f.name == *fnm) {
                                let (vr, vt) = self.expr(value);
                                let (vr, _) = self.coerce(vr, &vt, &f.ty);
                                let fp = self.t();
                                self.line(&format!(
                                    "{fp} = getelementptr i8, ptr {obj}, i64 {}",
                                    f.offset
                                ));
                                self.line(&format!("store {} {vr}, ptr {fp}", llty(&f.ty)));
                            }
                        }
                        Arg::Pos(value) => {
                            if let Some(f) = info.fields.get(pos) {
                                let (vr, vt) = self.expr(value);
                                let (vr, _) = self.coerce(vr, &vt, &f.ty);
                                let fp = self.t();
                                self.line(&format!(
                                    "{fp} = getelementptr i8, ptr {obj}, i64 {}",
                                    f.offset
                                ));
                                self.line(&format!("store {} {vr}, ptr {fp}", llty(&f.ty)));
                            }
                            pos += 1;
                        }
                    }
                }
                let an = format!("%v_{name}");
                self.line(&format!("{an} = alloca ptr"));
                self.line(&format!("store ptr {obj}, ptr {an}"));
                self.locals
                    .insert(name.clone(), (an, Type::Blueprint(resolved.clone())));
                if !self.loops.is_empty() {
                    // Its alloca lives in the loop body and does not dominate
                    // anything after the loop.
                    self.loop_locals.insert(name.clone());
                }
                if self.db.funcs.contains_key(&format!("{resolved}::create")) {
                    self.line(&format!("call void @sn_m_{resolved}_create(ptr {obj})"));
                }
            }
            Stmt::Defer { body, .. } => {
                self.defers.push(body.clone());
            }
            Stmt::Asm { template, clobbers, .. } => {
                self.emit_asm(template, clobbers);
            }
            Stmt::NestedFn(f) => {
                let ret = f.ret.as_ref().map(ast_to_type).unwrap_or(Type::Void);
                let clos = self.lower_closure(&f.params, &ret, &f.body);
                let an = format!("%v_{}_{}", f.name, self.l("lv"));
                self.line(&format!("{an} = alloca ptr"));
                self.line(&format!("store ptr {clos}, ptr {an}"));
                let ty = Type::Fn {
                    params: f.params.iter().map(|p| resolve_type_ast(&p.ty, self.db)).collect(),
                    ret: Box::new(ret),
                };
                self.locals.insert(f.name.clone(), (an, ty));
            }
        }
    }

    fn emit_defers(&mut self) {
        let ds = self.defers.clone();
        for body in ds.iter().rev() {
            for s in body {
                self.stmt(s);
            }
        }
    }

    fn if_chain(
        &mut self,
        cond: &Expr,
        then_body: &[Stmt],
        else_ifs: &[(Expr, Vec<Stmt>)],
        else_body: &Option<Vec<Stmt>>,
        idx: usize,
        end: &str,
    ) {
        let yes = self.l("ify");
        let no = self.l("ifn");
        let (c, _) = self.expr(cond);
        let c1 = self.as_i1(c);
        self.line(&format!("br i1 {c1}, label %{yes}, label %{no}"));
        self.raw(&format!("{yes}:"));
        for s in then_body {
            self.stmt(s);
        }
        self.line(&format!("br label %{end}"));
        self.raw(&format!("{no}:"));
        if idx < else_ifs.len() {
            let (c2, b2) = &else_ifs[idx];
            self.if_chain(c2, b2, else_ifs, else_body, idx + 1, end);
        } else if let Some(b) = else_body {
            for s in b {
                self.stmt(s);
            }
            self.line(&format!("br label %{end}"));
        } else {
            self.line(&format!("br label %{end}"));
        }
    }

    fn spawn_block(&mut self, body: &[Stmt], green: bool) {
        let mut caps: Vec<String> = Vec::new();
        collect_idents(body, &mut caps);
        caps.retain(|n| self.locals.contains_key(n));
        caps.sort();
        caps.dedup();
        let id = self.l("sp");
        let fname = format!("sn_spawn_{id}");
        let env = self.t();
        let nbytes = (caps.len() * 8) as i64;
        self.line(&format!("{env} = call ptr @sn_alloc(i64 {nbytes})"));
        for (i, n) in caps.iter().enumerate() {
            let (an, t) = self.locals.get(n).cloned().unwrap();
            let v = self.t();
            self.line(&format!("{v} = load {}, ptr {an}", llty(&t)));
            let slot = self.t();
            self.line(&format!(
                "{slot} = getelementptr i8, ptr {env}, i64 {}",
                i * 8
            ));
            if t.is_ptr() || matches!(t, Type::Optional(_)) {
                self.line(&format!("store ptr {v}, ptr {slot}"));
            } else {
                self.line(&format!("store i64 {v}, ptr {slot}"));
            }
        }
        if green {
            self.line(&format!("call void @sn_go_spawn(ptr @{fname}, ptr {env})"));
        } else {
            self.line(&format!("call void @sn_spawn(ptr @{fname}, ptr {env})"));
        }
        let cap_tys: Vec<(String, Type)> = caps
            .iter()
            .map(|n| (n.clone(), self.locals.get(n).unwrap().1.clone()))
            .collect();
        let mut nested = Vec::new();
        let buf = {
            let (cf, cl, _) = self.cur_loc.unwrap_or((0, 1, 1));
            let child_sub = self.dbg.sub("<closure>", cf, cl, &fname);
            let mut child = Cx {
                db: self.db,
                buf: String::new(),
                tmp: 0,
                lbl: 0,
                locals: HashMap::new(),
                loops: Vec::new(),
        loop_locals: std::collections::HashSet::new(),
                strings: self.strings,
                extra_fns: &mut nested,
                current_bp: self.current_bp.clone(),
                files: self.files,
                cur_loc: None,
                cur_sub: child_sub,
                dbg: self.dbg,
                terminated: false,
                current_fn_ret: Type::Void,
                defers: Vec::new(),
            };
            child.raw(&format!("define void @{fname}(ptr %env) !dbg !{child_sub} {{"));
            child.raw("entry:");
            for (i, (n, t)) in cap_tys.iter().enumerate() {
                let slot = child.t();
                child.line(&format!(
                    "{slot} = getelementptr i8, ptr %env, i64 {}",
                    i * 8
                ));
                let an = format!("%v_{n}");
                child.line(&format!("{an} = alloca {}", llty(t)));
                let v = child.t();
                child.line(&format!("{v} = load {}, ptr {slot}", llty(t)));
                child.line(&format!("store {} {v}, ptr {an}", llty(t)));
                child.locals.insert(n.clone(), (an, t.clone()));
            }
            for s in body {
                child.stmt(s);
            }
            child.line("ret void");
            child.raw("}");
            child.buf
        };
        self.extra_fns.push(buf);
        self.extra_fns.extend(nested);
    }

    fn expr(&mut self, e: &Expr) -> (String, Type) {
        match &e.kind {
            ExprKind::Int(n) => (format!("{n}"), Type::Int),
            ExprKind::Float(f) => {
                let lit = if f.is_finite() { format!("{f:?}") } else if *f == f64::INFINITY { "0x7FF0000000000000".to_string() } else { "0xFFF0000000000000".to_string() };
                (lit, Type::Float)
            }
            ExprKind::Dec { scaled, scale } => (format!("{scaled}"), Type::Dec(*scale)),
            ExprKind::Bool(b) => (if *b { "1".into() } else { "0".into() }, Type::Bool),
            ExprKind::None => ("null".into(), Type::None),
            ExprKind::Str(s) => {
                let id = self.strings.len();
                self.strings.push(s.clone());
                let t = self.t();
                self.line(&format!(
                    "{t} = call ptr @sn_str_new(ptr @.str.{id}, i64 {})",
                    s.len()
                ));
                (t, Type::Str)
            }
            ExprKind::Self_ => {
                let (an, t) = self.locals.get("self").cloned().unwrap();
                if an.starts_with("%p_") {
                    return (an, t);
                }
                let v = self.t();
                self.line(&format!("{v} = load ptr, ptr {an}"));
                (v, t)
            }
            ExprKind::Ident(n) => {
                if let Some((an, t)) = self.locals.get(n).cloned() {
                    if matches!(t, Type::Record(_)) {
                        return (an, t);
                    }
                    let v = self.t();
                    self.line(&format!("{v} = load {}, ptr {an}", llty(&t)));
                    let want = self.lookup_ty(e);
                    if want != t && want != Type::Int && (t.is_fixed_int() || t == Type::Int || matches!(t, Type::Optional(_))) {
                        let (v, wt) = self.coerce_narrow(v, &t, &want);
                        (v, wt)
                    } else if matches!((&t, &want), (Type::Optional(_), _)) && t != want {
                        self.coerce_narrow(v, &t, &want)
                    } else {
                        (v, t)
                    }
                } else if let Some(sig) = self.db.funcs.get(n).cloned() {
                    let c = self.fn_as_closure(&sig);
                    (c, Type::Fn {
                        params: sig.params.iter().map(|p| p.1.clone()).collect(),
                        ret: Box::new(sig.ret.clone()),
                    })
                } else if self.db.blueprints.contains_key(n) {
                    (n.clone(), Type::Blueprint(n.clone()))
                } else {
                    (n.clone(), Type::Named(n.clone()))
                }
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let want = self.lookup_ty(e);
                self.bin(*op, lhs, rhs, &want)
            }
            ExprKind::Unary { op, expr } => {
                let (v, t) = self.expr(expr);
                match op {
                    UnOp::Neg => {
                        if t == Type::Float {
                            let r = self.t();
                            self.line(&format!("{r} = fneg double {v}"));
                            return (r, t);
                        }
                        let r = self.t();
                        self.line(&format!("{r} = sub i64 0, {v}"));
                        (self.wrap_int(r, &t), t)
                    }
                    UnOp::Not => {
                        let r = self.t();
                        self.line(&format!("{r} = xor i64 {v}, 1"));
                        (r, Type::Bool)
                    }
                    UnOp::BitNot => {
                        let r = self.t();
                        self.line(&format!("{r} = xor i64 {v}, -1"));
                        let w = self.wrap_int(r, &t);
                        (w, t)
                    }
                }
            }
            ExprKind::Call { callee, type_args, args } => self.call(callee, type_args, args, e.span),
            ExprKind::Index { base, index } => {
                let (b, bt) = self.expr(base);
                let (i, _) = self.expr(index);
                match bt {
                    Type::List(el) => {
                        let r = self.t();
                        if el.is_ptr() || matches!(*el, Type::Optional(_)) {
                            self.line(&format!("{r} = call ptr @sn_list_get_ptr(ptr {b}, i64 {i})"));
                            (r, *el)
                        } else {
                            self.line(&format!("{r} = call i64 @sn_list_get_i64(ptr {b}, i64 {i})"));
                            let val = self.from_bits(r, &el);
                            (val, *el)
                        }
                    }
                    Type::Map(k, v) => {
                        let kb = self.as_bits(i, &k);
                        let bits = self.t();
                        self.line(&format!("{bits} = call i64 @sn_map_get(ptr {b}, i64 {kb})"));
                        (self.from_bits(bits, &v), *v)
                    }
                    Type::Str => {
                        let r = self.t();
                        let n = self.t();
                        self.line(&format!("{n} = add i64 {i}, 1"));
                        self.line(&format!("{r} = call ptr @sn_str_slice(ptr {b}, i64 {i}, i64 {n})"));
                        (r, Type::Str)
                    }
                    other => ("0".into(), other),
                }
            }
            ExprKind::Member { base, name } => self.member(base, name),
            ExprKind::Interpolate { parts } => {
                let mut acc: Option<String> = None;
                for p in parts {
                    let piece = match p {
                        InterpPart::Lit(s) => {
                            let id = self.strings.len();
                            self.strings.push(s.clone());
                            let t = self.t();
                            self.line(&format!(
                                "{t} = call ptr @sn_str_new(ptr @.str.{id}, i64 {})",
                                s.len()
                            ));
                            t
                        }
                        InterpPart::Expr(ex) => {
                            let (v, t) = self.expr(ex);
                            self.to_str(v, &t)
                        }
                    };
                    acc = Some(match acc {
                        None => piece,
                        Some(a) => {
                            let r = self.t();
                            self.line(&format!("{r} = call ptr @sn_str_concat(ptr {a}, ptr {piece})"));
                            r
                        }
                    });
                }
                (acc.unwrap_or_else(|| {
                    let id = self.strings.len();
                    self.strings.push(String::new());
                    let t = self.t();
                    self.line(&format!("{t} = call ptr @sn_str_new(ptr @.str.{id}, i64 0)"));
                    t
                }), Type::Str)
            }
            ExprKind::List(els) => {
                let et = if let Some(x) = els.first() {
                    self.lookup_ty(x)
                } else {
                    Type::Int
                };
                let kind = if et.is_ptr() || matches!(et, Type::Optional(_)) { 1 } else { 0 };
                let l = self.t();
                self.line(&format!("{l} = call ptr @sn_list_new(i64 {kind})"));
                for el in els {
                    let (v, vt) = self.expr(el);
                    if kind == 1 {
                        self.line(&format!("call void @sn_list_push_ptr(ptr {l}, ptr {v})"));
                    } else {
                        let vb = self.as_bits(v, &vt);
                        self.line(&format!("call void @sn_list_push_i64(ptr {l}, i64 {vb})"));
                    }
                }
                (l, Type::List(Box::new(et)))
            }
            ExprKind::Map(entries) => {
                let (kt, vt) = if let Some((k, v)) = entries.first() {
                    (self.lookup_ty(k), self.lookup_ty(v))
                } else {
                    (Type::Str, Type::Int)
                };
                let kk = if kt.is_ptr() { 1 } else { 0 };
                let vk = if vt.is_ptr() { 1 } else { 0 };
                let m = self.t();
                self.line(&format!("{m} = call ptr @sn_map_new(i64 {kk}, i64 {vk})"));
                for (k, v) in entries {
                    let (kr, _) = self.expr(k);
                    let (vr, _) = self.expr(v);
                    let kb = self.as_bits(kr, &kt);
                    let vb = self.as_bits(vr, &vt);
                    self.line(&format!("call void @sn_map_set(ptr {m}, i64 {kb}, i64 {vb})"));
                }
                (m, Type::Map(Box::new(kt), Box::new(vt)))
            }
            ExprKind::Tuple(xs) => {
                let mut ts = Vec::new();
                let mut vs = Vec::new();
                for x in xs {
                    let (v, t) = self.expr(x);
                    vs.push(v);
                    ts.push(t);
                }
                let st = tuple_ll(&ts);
                let mut agg = "zeroinitializer".to_string();
                for (i, (v, t)) in vs.iter().zip(ts.iter()).enumerate() {
                    let n = self.t();
                    self.line(&format!("{n} = insertvalue {st} {agg}, {} {v}, {i}", llty(t)));
                    agg = n;
                }
                (agg, Type::Tuple(ts))
            }
            ExprKind::Cast { expr, ty } => {
                let (v, st) = self.expr(expr);
                let dt = ast_to_type(ty);
                (self.cast_val(v, &st, &dt), dt)
            }
            ExprKind::Await(inner) => {
                let (fut, _) = self.expr(inner);
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_future_await(ptr {fut})"));
                (r, Type::Int)
            }
            ExprKind::SuperCall { method, args } => {
                let parent = self
                    .current_bp
                    .as_ref()
                    .and_then(|bp| self.db.blueprints.get(bp))
                    .and_then(|b| b.parent.clone())
                    .unwrap_or_default();
                let llvm = format!("sn_m_{parent}_{method}");
                let mut argv = vec!["ptr %v_self".to_string()];
                let mut vs = Vec::new();
                for a in args {
                    match a {
                        Arg::Pos(e) => {
                            let (v, t) = self.expr(e);
                            vs.push((v, t));
                        }
                        Arg::Named { value, .. } => {
                            let (v, t) = self.expr(value);
                            vs.push((v, t));
                        }
                    }
                }
                for (v, t) in &vs {
                    argv.push(format!("{} {v}", llty(t)));
                }
                let sig = self.db.funcs.get(&format!("{parent}::{method}")).cloned();
                if let Some(sig) = sig {
                    if sig.ret == Type::Void {
                        self.line(&format!("call void @{llvm}({})", argv.join(", ")));
                        ("0".into(), Type::Void)
                    } else {
                        let r = self.t();
                        self.line(&format!(
                            "{r} = call {} @{llvm}({})",
                            llty_ret(&sig.ret),
                            argv.join(", ")
                        ));
                        (r, sig.ret)
                    }
                } else {
                    ("0".into(), Type::Void)
                }
            }
            ExprKind::Try(inner) => self.expr_try(inner),
            ExprKind::OptionalChain(inner) => self.expr(inner),
            ExprKind::ForceUnwrap(inner) => self.expr_unwrap(inner),
            ExprKind::Lambda { params, ret, body } => {
                let rt = ret.as_ref().map(ast_to_type).unwrap_or(Type::Void);
                let c = self.lower_closure(params, &rt, body);
                (
                    c,
                    Type::Fn {
                        params: params.iter().map(|p| resolve_type_ast(&p.ty, self.db)).collect(),
                        ret: Box::new(rt),
                    },
                )
            }
        }
    }

    fn lookup_ty(&self, e: &Expr) -> Type {
        self.db
            .expr_ty
            .get(&(e.span.file, e.span.start, e.span.end))
            .cloned()
            .unwrap_or(Type::Int)
    }

    fn bin(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, want: &Type) -> (String, Type) {
        match op {
            BinOp::And | BinOp::Or => {
                let (l, _) = self.expr(lhs);
                let yes = self.l("sc");
                let no = self.l("sc");
                let join = self.l("scj");
                let c1 = self.as_i1(l.clone());
                if op == BinOp::And {
                    self.line(&format!("br i1 {c1}, label %{yes}, label %{no}"));
                } else {
                    self.line(&format!("br i1 {c1}, label %{no}, label %{yes}"));
                }
                self.raw(&format!("{yes}:"));
                let (r, _) = self.expr(rhs);
                let rkeep = r.clone();
                self.line(&format!("br label %{join}"));
                self.raw(&format!("{no}:"));
                let constv = if op == BinOp::And { "0" } else { "1" };
                self.line(&format!("br label %{join}"));
                self.raw(&format!("{join}:"));
                let phi = self.t();
                self.line(&format!(
                    "{phi} = phi i64 [ {rkeep}, %{yes} ], [ {constv}, %{no} ]"
                ));
                (phi, Type::Bool)
            }
            BinOp::Otherwise => {
                let (l, lt) = self.expr(lhs);
                let yes = self.l("ow");
                let no = self.l("ow");
                let join = self.l("owj");
                let c = self.t();
                self.line(&format!("{c} = icmp ne ptr {l}, null"));
                self.line(&format!("br i1 {c}, label %{yes}, label %{no}"));
                self.raw(&format!("{yes}:"));
                let some = if matches!(lt, Type::Optional(ref i) if !i.is_ptr()) {
                    let u = self.t();
                    self.line(&format!("{u} = call i64 @sn_unbox_i64(ptr {l})"));
                    u
                } else {
                    l.clone()
                };
                self.line(&format!("br label %{join}"));
                self.raw(&format!("{no}:"));
                let (r, rt) = self.expr(rhs);
                self.line(&format!("br label %{join}"));
                self.raw(&format!("{join}:"));
                let inner = lt.unwrap_optional().cloned().unwrap_or(rt.clone());
                let phi = self.t();
                self.line(&format!(
                    "{phi} = phi {} [ {some}, %{yes} ], [ {r}, %{no} ]",
                    llty(&inner)
                ));
                (phi, inner)
            }
            BinOp::Add if self.lookup_ty(lhs) == Type::Str => {
                let (l, _) = self.expr(lhs);
                let (r, _) = self.expr(rhs);
                let t = self.t();
                self.line(&format!("{t} = call ptr @sn_str_concat(ptr {l}, ptr {r})"));
                (t, Type::Str)
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                let (l, lt) = self.expr(lhs);
                let (r, rt2) = self.expr(rhs);
                if lt == Type::Float || rt2 == Type::Float {
                    let pred = match op {
                        BinOp::Eq => "oeq",
                        BinOp::Ne => "one",
                        BinOp::Lt => "olt",
                        BinOp::Gt => "ogt",
                        BinOp::Le => "ole",
                        BinOp::Ge => "oge",
                        _ => "oeq",
                    };
                    let c = self.t();
                    self.line(&format!("{c} = fcmp {pred} double {l}, {r}"));
                    let z = self.t();
                    self.line(&format!("{z} = zext i1 {c} to i64"));
                    return (z, Type::Bool);
                }
                let pred = match op {
                    BinOp::Eq => "eq",
                    BinOp::Ne => "ne",
                    BinOp::Lt => "slt",
                    BinOp::Gt => "sgt",
                    BinOp::Le => "sle",
                    BinOp::Ge => "sge",
                    _ => "eq",
                };
                let c = self.t();
                if lt == Type::Str || matches!(lt, Type::Optional(_)) || lt.is_ptr() && op != BinOp::Lt {
                    if lt == Type::Str && matches!(op, BinOp::Eq | BinOp::Ne) {
                        let eq = self.t();
                        self.line(&format!("{eq} = call i64 @sn_str_eq(ptr {l}, ptr {r})"));
                        if op == BinOp::Ne {
                            let x = self.t();
                            self.line(&format!("{x} = xor i64 {eq}, 1"));
                            return (x, Type::Bool);
                        }
                        return (eq, Type::Bool);
                    }
                    if matches!(op, BinOp::Eq | BinOp::Ne) {
                        self.line(&format!("{c} = icmp {pred} ptr {l}, {r}"));
                    } else {
                        self.line(&format!("{c} = icmp {pred} i64 {l}, {r}"));
                    }
                } else {
                    self.line(&format!("{c} = icmp {pred} i64 {l}, {r}"));
                }
                let z = self.t();
                self.line(&format!("{z} = zext i1 {c} to i64"));
                (z, Type::Bool)
            }
            BinOp::In => {
                let (l, lt) = self.expr(lhs);
                let (r, rt) = self.expr(rhs);
                let d = self.t();
                match rt {
                    Type::List(el) if el.is_ptr() || matches!(*el, Type::Str) => {
                        self.line(&format!(
                            "{d} = call i64 @sn_list_contains_ptr(ptr {r}, ptr {l})"
                        ));
                    }
                    Type::List(_) => {
                        self.line(&format!(
                            "{d} = call i64 @sn_list_contains_i64(ptr {r}, i64 {l})"
                        ));
                    }
                    Type::Str => {
                        self.line(&format!("{d} = call i64 @sn_str_contains(ptr {r}, ptr {l})"));
                    }
                    _ => {
                        self.line(&format!(
                            "{d} = call i64 @sn_list_contains_i64(ptr {r}, i64 {l})"
                        ));
                    }
                }
                let _ = lt;
                (d, Type::Bool)
            }
            arith => {
                let (l, lt) = self.expr(lhs);
                let (r, _) = self.expr(rhs);
                let t = if lt == Type::Float {
                    lt.clone()
                } else if want.int_min_max().is_some() || *want == Type::Int {
                    want.clone()
                } else {
                    lt.clone()
                };
                (self.arith_op(arith, l, r, &t), t)
            }
        }
    }

    fn arith_op(&mut self, op: BinOp, l: String, r: String, t: &Type) -> String {
        if *t == Type::Float {
            let d = self.t();
            match op {
                BinOp::Add => self.line(&format!("{d} = fadd double {l}, {r}")),
                BinOp::Sub => self.line(&format!("{d} = fsub double {l}, {r}")),
                BinOp::Mul => self.line(&format!("{d} = fmul double {l}, {r}")),
                BinOp::Div => self.line(&format!("{d} = fdiv double {l}, {r}")),
                BinOp::Mod => self.line(&format!("{d} = frem double {l}, {r}")),
                BinOp::Pow => self.line(&format!("{d} = call double @llvm.pow.f64(double {l}, double {r})")),
                _ => self.line(&format!("{d} = fadd double {l}, {r}")),
            }
            return d;
        }
        let d = self.t();
        match op {
            BinOp::Add => self.line(&format!("{d} = add i64 {l}, {r}")),
            BinOp::Sub => self.line(&format!("{d} = sub i64 {l}, {r}")),
            BinOp::Mul => self.line(&format!("{d} = mul i64 {l}, {r}")),
            BinOp::Div => self.line(&format!("{d} = call i64 @sn_div_i64(i64 {l}, i64 {r})")),
            BinOp::Mod => self.line(&format!("{d} = call i64 @sn_mod_i64(i64 {l}, i64 {r})")),
            BinOp::Pow => self.line(&format!("{d} = call i64 @sn_pow_i64(i64 {l}, i64 {r})")),
            BinOp::BitAnd => self.line(&format!("{d} = and i64 {l}, {r}")),
            BinOp::BitOr => self.line(&format!("{d} = or i64 {l}, {r}")),
            BinOp::BitXor => self.line(&format!("{d} = xor i64 {l}, {r}")),
            BinOp::Shl => self.line(&format!("{d} = shl i64 {l}, {r}")),
            BinOp::Shr => self.line(&format!("{d} = ashr i64 {l}, {r}")),
            _ => self.line(&format!("{d} = add i64 {l}, {r}")),
        }
        self.wrap_int(d.clone(), t)
    }

    fn wrap_int(&mut self, v: String, t: &Type) -> String {
        match t {
            Type::U8 | Type::Byte => {
                let d = self.t();
                self.line(&format!("{d} = and i64 {v}, 255"));
                d
            }
            Type::U16 => {
                let d = self.t();
                self.line(&format!("{d} = and i64 {v}, 65535"));
                d
            }
            Type::U32 => {
                let d = self.t();
                self.line(&format!("{d} = and i64 {v}, 4294967295"));
                d
            }
            Type::I8 => {
                let s = self.t();
                let d = self.t();
                self.line(&format!("{s} = shl i64 {v}, 56"));
                self.line(&format!("{d} = ashr i64 {s}, 56"));
                d
            }
            Type::I16 => {
                let s = self.t();
                let d = self.t();
                self.line(&format!("{s} = shl i64 {v}, 48"));
                self.line(&format!("{d} = ashr i64 {s}, 48"));
                d
            }
            Type::I32 => {
                let s = self.t();
                let d = self.t();
                self.line(&format!("{s} = shl i64 {v}, 32"));
                self.line(&format!("{d} = ashr i64 {s}, 32"));
                d
            }
            _ => v,
        }
    }

    fn call(&mut self, callee: &Expr, args: &[Arg]) -> (String, Type) {
        if let ExprKind::Member { base, name } = &callee.kind {
            return self.method(base, name, args);
        }
        if let ExprKind::Ident(n) = &callee.kind {
            return self.ident_call(n, args);
        }
        let (c, ct) = self.expr(callee);
        if let Type::Fn { params, ret } = ct {
            return self.call_closure_val(c, &params, &ret, args);
        }
        ("0".into(), Type::Void)
    }

    /// Coerce a value to LLVM pointer type. Pointers pass through; integer
    /// literals (typically 0, meaning NULL) are cast.
    fn as_ptr(&mut self, v: String, t: Type) -> String {
        match t {
            Type::Ptr | Type::List(_) | Type::Map(_, _) | Type::Blueprint(_)
            | Type::Record(_) | Type::Contract(_) | Type::Any | Type::Optional(_)
            | Type::Ref(_) | Type::Chan(_) | Type::Json | Type::Named(_) => v,
            _ => {
                let p = self.t();
                self.line(&format!("{p} = inttoptr i64 {v} to ptr"));
                p
            }
        }
    }

    /// Call an `extern` C symbol, marshalling SN values to the C ABI.
    fn ffi_call(&mut self, sig: &FuncSig, vs: &[(String, Type)]) -> (String, Type) {
        let mut argv = Vec::new();
        for (i, (v, t)) in vs.iter().enumerate() {
            let want = sig.params.get(i).map(|p| p.1.clone()).unwrap_or(Type::Ptr);
            argv.push(match &want {
                // SN strings are refcounted objects; C wants a char*.
                Type::Str | Type::Error => {
                    let c = self.t();
                    self.line(&format!("{c} = call ptr @sn_str_cstr(ptr {v})"));
                    format!("ptr {c}")
                }
                Type::Ptr => match t {
                    // Already a pointer: pass through.
                    Type::Ptr | Type::List(_) | Type::Map(_, _) | Type::Blueprint(_)
                    | Type::Record(_) | Type::Contract(_) | Type::Any
                    | Type::Optional(_) | Type::Ref(_) | Type::Chan(_) | Type::Json => {
                        format!("ptr {v}")
                    }
                    // Integer literals stand in for NULL / cast addresses.
                    Type::Int | Type::U64 | Type::I32 | Type::U32 | Type::I16
                    | Type::U16 | Type::I8 | Type::U8 | Type::Bool | Type::Byte => {
                        let (vi, _) = self.coerce_narrow(v.clone(), t, &Type::Int);
                        let p = self.t();
                        self.line(&format!("{p} = inttoptr i64 {vi} to ptr"));
                        format!("ptr {p}")
                    }
                    _ => format!("ptr {v}"),
                },
                _ => {
                    let want_ty = match &want {
                        Type::Float => Type::Float,
                        Type::Byte | Type::U8 | Type::I8 | Type::Bool => Type::I8,
                        Type::U16 | Type::I16 => Type::I16,
                        Type::U32 | Type::I32 => Type::I32,
                        _ => Type::Int,
                    };
                    if want_ty == Type::Float {
                        // An integer literal must become a real double, not an
                        // integer constant annotated as one.
                        let v2 = if *t == Type::Float {
                            v.clone()
                        } else {
                            let r = self.t();
                            self.line(&format!("{r} = sitofp i64 {v} to double"));
                            r
                        };
                        format!("double {v2}")
                    } else {
                        let (v2, _) = self.coerce_narrow(v.clone(), t, &want_ty);
                        format!("{} {v2}", ffi_param_ty(&want_ty))
                    }
                }
            });
        }
        if sig.ret == Type::Void {
            self.line(&format!("call void @{}({})", sig.llvm, argv.join(", ")));
            return ("0".into(), Type::Void);
        }
        let rty = ffi_ret_ty(&sig.ret);
        let r = self.t();
        self.line(&format!(
            "{r} = call {rty} @{}({})",
            sig.llvm,
            argv.join(", ")
        ));
        // A C function returning `char *` hands back a raw string; adopt it
        // into the SN heap so the value is refcounted like any other.
        if sig.ret == Type::Str {
            let s = self.t();
            self.line(&format!("{s} = call ptr @sn_str_from_cstr(ptr {r})"));
            return (s, Type::Str);
        }
        (r, sig.ret.clone())
    }

    /// Call a user-declared function, materialising default arguments and
    /// coercing each parameter to its declared type.
    fn call_user_fn(&mut self, sig: &FuncSig, vs: &[(String, Type)]) -> (String, Type) {
        let mut argv = Vec::new();
        for (i, p) in sig.params.iter().enumerate() {
            if i < vs.len() {
                let (v2, _) = self.coerce(vs[i].0.clone(), &vs[i].1, &p.1);
                argv.push(format!("{} {}", llty(&p.1), v2));
            } else if let Some(Some(dflt)) = sig.defaults.get(i) {
                let (dv, dt) = self.expr(dflt);
                argv.push(format!("{} {dv}", llty(&dt)));
            } else if p.1.is_ptr() || matches!(p.1, Type::Optional(_)) {
                argv.push("ptr null".into());
            } else {
                argv.push(format!("{} 0", llty(&p.1)));
            }
        }
        if sig.ret == Type::Void {
            self.line(&format!("call void @{}({})", sig.llvm, argv.join(", ")));
            return ("0".into(), Type::Void);
        }
        let r = self.t();
        self.line(&format!(
            "{r} = call {} @{}({})",
            llty_ret(&sig.ret),
            sig.llvm,
            argv.join(", ")
        ));
        (r, sig.ret.clone())
    }

    fn ident_call(&mut self, n: &str, type_args: &[TypeAst], args: &[Arg], span: Span, callee_span: Span) -> (String, Type) {
        let mut vs = Vec::new();
        for a in args {
            match a {
                Arg::Pos(e) | Arg::Named { value: e, .. } => vs.push(self.expr(e)),
            }
        }
        if let Some(mono) = self.db.resolved_calls.get(&(span.file, span.start, span.end)).cloned()
            .or_else(|| self.db.resolved_calls.get(&(callee_span.file, callee_span.start, callee_span.end)).cloned())
        {
            if let Some(sig) = self.db.funcs.get(&mono).cloned() {
                return self.call_user_fn(&sig, &vs);
            }
        }
        if let Some(template) = self.db.fn_templates.get(n).cloned() {
            let mut eff_args = type_args.to_vec();
            if eff_args.is_empty() {
                let mut inferred = HashMap::new();
                let tp_names = template_param_names(&template.type_params);
                for (p, v) in template.params.iter().zip(vs.iter()) {
                    infer_type_param(&p.ty, &v.1, &tp_names, &mut inferred);
                }
                for tp in &template.type_params {
                    eff_args.push(inferred.get(&tp.name).cloned().unwrap_or(TypeAst::Int));
                }
            }
            let mono = mono_name(&template.name, &eff_args);
            if let Some(sig) = self.db.funcs.get(&mono).cloned() {
                return self.call_user_fn(&sig, &vs);
            }
        }
        // `extern` declarations are dispatched before the builtin names so a
        // C symbol like `read` or `open` is reachable.
        if let Some(sig) = self.db.funcs.get(n).cloned() {
            if sig.is_extern {
                return self.ffi_call(&sig, &vs);
            }
            // A user function shadows a builtin of the same name. Re-enter
            // through the fallback arm, which already fills default args.
            let shadowed = self.db.funcs.get(n).cloned().unwrap();
            return self.call_user_fn(&shadowed, &vs);
        }
        match n {
            "print" | "printn" => {
                let nl = n == "print";
                if let Some((v, t)) = vs.first() {
                    self.print_val(v.clone(), t, nl);
                }
                ("0".into(), Type::Void)
            }
            "input" => {
                let p = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_input(ptr {p})"));
                (r, Type::Str)
            }
            "file_read" => {
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_file_read(ptr {})", vs[0].0));
                (r, Type::Str)
            }
            "file_write" => {
                let r = self.t();
                self.line(&format!(
                    "{r} = call i64 @sn_file_write(ptr {}, ptr {})",
                    vs[0].0, vs[1].0
                ));
                (r, Type::Bool)
            }
            "file_read_ex" => {
                let body = self.t();
                let err = self.t();
                self.line(&format!("{body} = alloca ptr"));
                self.line(&format!("{err} = alloca ptr"));
                self.line(&format!(
                    "call void @sn_file_read_ex(ptr {}, ptr {body}, ptr {err})",
                    vs[0].0
                ));
                let b = self.t();
                let e = self.t();
                self.line(&format!("{b} = load ptr, ptr {body}"));
                self.line(&format!("{e} = load ptr, ptr {err}"));
                self.make_pair(b, Type::Str, e, Type::Error)
            }
            "file_write_ex" | "file_append" | "file_delete" | "file_copy" | "file_move"
            | "file_mkdir" | "file_rmdir" => {
                let r = self.t();
                let callee = match n {
                    "file_write_ex" => "sn_file_write_ex",
                    "file_append" => "sn_file_append",
                    "file_delete" => "sn_file_delete",
                    "file_copy" => "sn_file_copy",
                    "file_move" => "sn_file_move",
                    "file_mkdir" => "sn_file_mkdir",
                    _ => "sn_file_rmdir",
                };
                if vs.len() == 1 {
                    self.line(&format!("{r} = call ptr @{callee}(ptr {})", vs[0].0));
                } else {
                    self.line(&format!(
                        "{r} = call ptr @{callee}(ptr {}, ptr {})",
                        vs[0].0, vs[1].0
                    ));
                }
                (r, Type::Error)
            }
            "file_exists" => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_file_exists(ptr {})", vs[0].0));
                (r, Type::Bool)
            }
            "file_list" => {
                let body = self.t();
                let err = self.t();
                self.line(&format!("{body} = alloca ptr"));
                self.line(&format!("{err} = alloca ptr"));
                self.line(&format!(
                    "call void @sn_file_list(ptr {}, ptr {body}, ptr {err})",
                    vs[0].0
                ));
                let b = self.t();
                let e = self.t();
                self.line(&format!("{b} = load ptr, ptr {body}"));
                self.line(&format!("{e} = load ptr, ptr {err}"));
                self.make_pair(b, Type::List(Box::new(Type::Str)), e, Type::Error)
            }
            "path_join" => {
                let r = self.t();
                self.line(&format!(
                    "{r} = call ptr @sn_path_join(ptr {}, ptr {})",
                    vs[0].0, vs[1].0
                ));
                (r, Type::Str)
            }
            "path_base" | "path_dir" | "path_ext" => {
                let r = self.t();
                let callee = match n {
                    "path_base" => "sn_path_base",
                    "path_dir" => "sn_path_dir",
                    _ => "sn_path_ext",
                };
                self.line(&format!("{r} = call ptr @{callee}(ptr {})", vs[0].0));
                (r, Type::Str)
            }
            "os_getenv" => {
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_os_getenv(ptr {})", vs[0].0));
                (r, Type::Optional(Box::new(Type::Str)))
            }
            "os_exit" => {
                self.line(&format!("call void @sn_os_exit(i64 {})", vs[0].0));
                ("0".into(), Type::Void)
            }
            "os_args" => {
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_os_args()"));
                (r, Type::List(Box::new(Type::Str)))
            }
            "os_system" => {
                let code_slot = self.t();
                let out_slot = self.t();
                let err_slot = self.t();
                self.line(&format!("{code_slot} = alloca i64"));
                self.line(&format!("{out_slot} = alloca ptr"));
                self.line(&format!("{err_slot} = alloca ptr"));
                self.line(&format!(
                    "call void @sn_os_system(ptr {}, ptr {code_slot}, ptr {out_slot}, ptr {err_slot})",
                    vs[0].0
                ));
                let c = self.t();
                let o = self.t();
                let e = self.t();
                self.line(&format!("{c} = load i64, ptr {code_slot}"));
                self.line(&format!("{o} = load ptr, ptr {out_slot}"));
                self.line(&format!("{e} = load ptr, ptr {err_slot}"));
                self.make_triple(c, Type::Int, o, Type::Str, e, Type::Error)
            }
            "os_exec" => {
                let code_slot = self.t();
                let out_slot = self.t();
                let err_slot = self.t();
                self.line(&format!("{code_slot} = alloca i64"));
                self.line(&format!("{out_slot} = alloca ptr"));
                self.line(&format!("{err_slot} = alloca ptr"));
                self.line(&format!(
                    "call void @sn_os_exec(ptr {}, ptr {}, ptr {code_slot}, ptr {out_slot}, ptr {err_slot})",
                    vs[0].0, vs[1].0
                ));
                let c = self.t();
                let o = self.t();
                let e = self.t();
                self.line(&format!("{c} = load i64, ptr {code_slot}"));
                self.line(&format!("{o} = load ptr, ptr {out_slot}"));
                self.line(&format!("{e} = load ptr, ptr {err_slot}"));
                self.make_triple(c, Type::Int, o, Type::Str, e, Type::Error)
            }
            "panic" => {
                self.line(&format!("call void @sn_panic(ptr {})", vs[0].0));
                ("0".into(), Type::Void)
            }
            "error" => {
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_error_new(ptr {})", vs[0].0));
                (r, Type::Error)
            }
            "json_parse" => {
                let body = self.t();
                let err = self.t();
                self.line(&format!("{body} = alloca ptr"));
                self.line(&format!("{err} = alloca ptr"));
                self.line(&format!(
                    "call void @sn_json_parse(ptr {}, ptr {body}, ptr {err})",
                    vs[0].0
                ));
                let b = self.t();
                let e = self.t();
                self.line(&format!("{b} = load ptr, ptr {body}"));
                self.line(&format!("{e} = load ptr, ptr {err}"));
                self.make_pair(b, Type::Json, e, Type::Error)
            }
            "json_encode" => {
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_json_encode(ptr {})", vs[0].0));
                (r, Type::Str)
            }
            "http_get" => self.http_req("GET", &vs, false),
            "http_post" => self.http_req("POST", &vs, true),
            "http_serve_once" => {
                let r = self.t();
                self.line(&format!(
                    "{r} = call ptr @sn_http_serve_once(i64 {}, ptr {})",
                    vs[0].0, vs[1].0
                ));
                (r, Type::Error)
            }
            "time_now_ms" => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_time_now_ms()"));
                (r, Type::Int)
            }
            "time_sleep_ms" => {
                self.line(&format!("call void @sn_time_sleep_ms(i64 {})", vs[0].0));
                ("0".into(), Type::Void)
            }
            "time_format" => {
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_time_format(i64 {})", vs[0].0));
                (r, Type::Str)
            }
            "sleep_async" => {
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_async_sleep(i64 {})", vs[0].0));
                (r, Type::Named("future".into()))
            }
            "hton16" => { let r = self.t(); self.line(&format!("{r} = call i64 @sn_hton16(i64 {})", vs[0].0)); (r, Type::Int) }
            "ntoh16" => { let r = self.t(); self.line(&format!("{r} = call i64 @sn_ntoh16(i64 {})", vs[0].0)); (r, Type::Int) }
            "hton32" => { let r = self.t(); self.line(&format!("{r} = call i64 @sn_hton32(i64 {})", vs[0].0)); (r, Type::Int) }
            "ntoh32" => { let r = self.t(); self.line(&format!("{r} = call i64 @sn_ntoh32(i64 {})", vs[0].0)); (r, Type::Int) }
            "hton64" => { let r = self.t(); self.line(&format!("{r} = call i64 @sn_hton64(i64 {})", vs[0].0)); (r, Type::Int) }
            "ntoh64" => { let r = self.t(); self.line(&format!("{r} = call i64 @sn_ntoh64(i64 {})", vs[0].0)); (r, Type::Int) }
            "swap16" => { let r = self.t(); self.line(&format!("{r} = call i64 @sn_swap16_i64(i64 {})", vs[0].0)); (r, Type::Int) }
            "swap32" => { let r = self.t(); self.line(&format!("{r} = call i64 @sn_swap32_i64(i64 {})", vs[0].0)); (r, Type::Int) }
            "swap64" => { let r = self.t(); self.line(&format!("{r} = call i64 @sn_swap64_i64(i64 {})", vs[0].0)); (r, Type::Int) }
            "select" => {
                let mut chan_vs = Vec::new();
                let mut timeout = "-1".to_string();
                for a in args {
                    match a {
                        Arg::Pos(e) => {
                            let (v, t) = self.expr(e);
                            if t == Type::Int {
                                timeout = v;
                            } else {
                                chan_vs.push(v);
                            }
                        }
                        Arg::Named { value, .. } => {
                            let (v, t) = self.expr(value);
                            if t == Type::Int {
                                timeout = v;
                            } else {
                                chan_vs.push(v);
                            }
                        }
                    }
                }
                let n = chan_vs.len();
                let arr = self.t();
                self.line(&format!("{arr} = alloca [{n} x ptr]"));
                for (i, v) in chan_vs.iter().enumerate() {
                    let slot = self.t();
                    self.line(&format!(
                        "{slot} = getelementptr [{n} x ptr], ptr {arr}, i32 0, i32 {i}"
                    ));
                    self.line(&format!("store ptr {v}, ptr {slot}"));
                }
                let r = self.t();
                self.line(&format!(
                    "{r} = call i64 @sn_chan_select(i64 {n}, ptr {arr}, i64 {timeout})"
                ));
                (r, Type::Int)
            }
            "get_async" => {
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_async_http_get(ptr {})", vs[0].0));
                (r, Type::Named("future".into()))
            }
            "future_data" => {
                let r = self.t();
                let f = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                self.line(&format!("{r} = call ptr @sn_future_data(ptr {f})"));
                (r, Type::Str)
            }
            "tcp_listen" | "tcp_listen_any" => {
                let port = vs.first().map(|x| x.0.clone()).unwrap_or("0".into());
                let r = self.t();
                if n == "tcp_listen" {
                    self.line(&format!("{r} = call i64 @sn_tcp_listen(i64 {port})"));
                } else {
                    self.line(&format!("{r} = call i64 @sn_tcp_listen_any(i64 {port})"));
                }
                (r, Type::Int)
            }
            "tcp_local_port" => {
                let fd = vs.first().map(|x| x.0.clone()).unwrap_or("-1".into());
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_tcp_local_port(i64 {fd})"));
                (r, Type::Int)
            }
            "tcp_connect" => {
                let host_obj = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let host = self.t();
                self.line(&format!("{host} = call ptr @sn_str_cstr(ptr {host_obj})"));
                let port = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                let tmo = vs.get(2).map(|x| x.0.clone()).unwrap_or("30000".into());
                let r = self.t();
                self.line(&format!(
                    "{r} = call i64 @sn_tcp_connect(ptr {host}, i64 {port}, i64 {tmo})"
                ));
                (r, Type::Int)
            }
            "tcp_accept" => {
                let fd = vs.first().map(|x| x.0.clone()).unwrap_or("-1".into());
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_tcp_accept(i64 {fd})"));
                (r, Type::Int)
            }
            "tcp_read" => {
                let fd = vs.first().map(|x| x.0.clone()).unwrap_or("-1".into());
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_tcp_read(i64 {fd})"));
                (r, Type::Str)
            }
            "tcp_write" => {
                let fd = vs.first().map(|x| x.0.clone()).unwrap_or("-1".into());
                let data = vs.get(1).map(|x| x.0.clone()).unwrap_or("null".into());
                let len = self.t();
                self.line(&format!(
                    "{len} = call i64 @sn_str_len(ptr {data})"
                ));
                let cstr = self.t();
                self.line(&format!("{cstr} = call ptr @sn_str_cstr(ptr {data})"));
                let r = self.t();
                self.line(&format!(
                    "{r} = call i64 @sn_tcp_write(i64 {fd}, ptr {cstr}, i64 {len})"
                ));
                (r, Type::Int)
            }
            "tcp_close" => {
                let fd = vs.first().map(|x| x.0.clone()).unwrap_or("-1".into());
                self.line(&format!("call void @sn_tcp_close(i64 {fd})"));
                ("0".into(), Type::Void)
            }
            "bytearray_new" => {
                let cap = vs.first().map(|x| x.0.clone()).unwrap_or("8".into());
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_bytearray_new(i64 {cap})"));
                (r, Type::Ptr)
            }
            "bytearray_len" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_bytearray_len(ptr {p})"));
                (r, Type::Int)
            }
            "bytearray_reserve" | "bytearray_clear" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                if n == "bytearray_reserve" {
                    let amt = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                    self.line(&format!("call void @sn_bytearray_reserve(ptr {p}, i64 {amt})"));
                } else {
                    self.line(&format!("call void @sn_bytearray_clear(ptr {p})"));
                }
                ("0".into(), Type::Void)
            }
            "bytearray_push" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let b = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_bytearray_push(ptr {p}, i64 {b})"));
                (r, Type::Int)
            }
            "bytearray_get" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let i = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_bytearray_get(ptr {p}, i64 {i})"));
                (r, Type::Int)
            }
            "bytearray_set" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let i = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                let v = vs.get(2).map(|x| x.0.clone()).unwrap_or("0".into());
                self.line(&format!(
                    "call void @sn_bytearray_set(ptr {p}, i64 {i}, i64 {v})"
                ));
                ("0".into(), Type::Void)
            }
            "bytearray_append_str" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let s = vs.get(1).map(|x| x.0.clone()).unwrap_or("null".into());
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_bytearray_append_str(ptr {p}, ptr {s})"));
                (r, Type::Ptr)
            }
            "bytearray_append_bytes" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let s = vs.get(1).map(|x| x.0.clone()).unwrap_or("null".into());
                let len = self.t();
                self.line(&format!("{len} = call i64 @sn_str_len(ptr {s})"));
                let cs = self.t();
                self.line(&format!("{cs} = call ptr @sn_str_cstr(ptr {s})"));
                self.line(&format!(
                    "call void @sn_bytearray_append_bytes(ptr {p}, ptr {cs}, i64 {len})"
                ));
                ("0".into(), Type::Void)
            }
            "bytearray_slice" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let a = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                let b = vs.get(2).map(|x| x.0.clone()).unwrap_or("-1".into());
                let r = self.t();
                self.line(&format!(
                    "{r} = call ptr @sn_bytearray_slice(ptr {p}, i64 {a}, i64 {b})"
                ));
                (r, Type::Ptr)
            }
            "bytearray_write_at" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let off = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                let s = vs.get(2).map(|x| x.0.clone()).unwrap_or("null".into());
                let len = self.t();
                self.line(&format!("{len} = call i64 @sn_str_len(ptr {s})"));
                let cs = self.t();
                self.line(&format!("{cs} = call ptr @sn_str_cstr(ptr {s})"));
                let r = self.t();
                self.line(&format!(
                    "{r} = call i64 @sn_bytearray_write_at(ptr {p}, i64 {off}, ptr {cs}, i64 {len})"
                ));
                (r, Type::Int)
            }
            "bytearray_find" | "bytearray_rfind" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let b = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                let r = self.t();
                if n == "bytearray_find" {
                    let f = vs.get(2).map(|x| x.0.clone()).unwrap_or("0".into());
                    self.line(&format!(
                        "{r} = call i64 @sn_bytearray_find(ptr {p}, i64 {b}, i64 {f})"
                    ));
                } else {
                    self.line(&format!("{r} = call i64 @sn_bytearray_rfind(ptr {p}, i64 {b})"));
                }
                (r, Type::Int)
            }
            "bytearray_truncate" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let v = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                self.line(&format!("call void @sn_bytearray_truncate(ptr {p}, i64 {v})"));
                ("0".into(), Type::Void)
            }
            "bytearray_fill" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let b = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                let c = vs.get(2).map(|x| x.0.clone()).unwrap_or("0".into());
                self.line(&format!("call void @sn_bytearray_fill(ptr {p}, i64 {b}, i64 {c})"));
                ("0".into(), Type::Void)
            }
            "bytearray_data" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_bytearray_data(ptr {p})"));
                (r, Type::Ptr)
            }
            "sha256" | "sha512" | "md5" | "digest_sha256" | "digest_sha512"
            | "digest_md5" => {
                // (data, len, out) -> fills a raw buffer; returns its address.
                let s = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let len = self.t();
                self.line(&format!("{len} = call i64 @sn_str_len(ptr {s})"));
                let cs = self.t();
                self.line(&format!("{cs} = call ptr @sn_str_cstr(ptr {s})"));
                // (str, out)
                // (str, out): `out` is a bytearray; the runtime writes the
                // digest into its bytes and fixes up the length.
                let ba = vs.get(1).map(|x| x.0.clone()).unwrap_or("null".into());
                let sym = match n {
                    "sha256" | "digest_sha256" => "sn_bytearray_sha256",
                    "sha512" | "digest_sha512" => "sn_bytearray_sha512",
                    _ => "sn_bytearray_md5",
                };
                self.line(&format!("call void @{sym}(ptr {cs}, i64 {len}, ptr {ba})"));
                (ba, Type::Ptr)
            }
            "hmac_sha256" | "digest_hmac_sha256" => {
                let key = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let klen = self.t();
                self.line(&format!("{klen} = call i64 @sn_str_len(ptr {key})"));
                let kcs = self.t();
                self.line(&format!("{kcs} = call ptr @sn_str_cstr(ptr {key})"));
                let msg = vs.get(1).map(|x| x.0.clone()).unwrap_or("null".into());
                let mlen = self.t();
                self.line(&format!("{mlen} = call i64 @sn_str_len(ptr {msg})"));
                let mcs = self.t();
                self.line(&format!("{mcs} = call ptr @sn_str_cstr(ptr {msg})"));
                let ba = vs.get(2).map(|x| x.0.clone()).unwrap_or("null".into());
                self.line(&format!(
                    "call void @sn_bytearray_hmac_sha256(ptr {kcs}, i64 {klen}, ptr {mcs}, i64 {mlen}, ptr {ba})"
                ));
                (ba, Type::Ptr)
            }
            "crc32" | "crc32_update" | "digest_crc32" | "digest_crc32_update" => {
                // crc32(data) | crc32_update(seed, data)
                let (seed, d) = if n == "crc32_update" || n == "digest_crc32_update" {
                    (vs.first().map(|x| x.0.clone()).unwrap_or("0".into()),
                     vs.get(1).map(|x| x.0.clone()).unwrap_or("null".into()))
                } else {
                    ("0".to_string(), vs.first().map(|x| x.0.clone()).unwrap_or("null".into()))
                };
                let len = self.t();
                self.line(&format!("{len} = call i64 @sn_str_len(ptr {d})"));
                let cs = self.t();
                self.line(&format!("{cs} = call ptr @sn_str_cstr(ptr {d})"));
                let r = self.t();
                if n == "crc32" || n == "digest_crc32" {
                    self.line(&format!("{r} = call i64 @sn_crc32(ptr {cs}, i64 {len})"));
                } else {
                    self.line(&format!(
                        "{r} = call i64 @sn_crc32_update(i64 {seed}, ptr {cs}, i64 {len})"
                    ));
                }
                (r, Type::Int)
            }
            "consttime_eq" => {
                let a = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let b = vs.get(1).map(|x| x.0.clone()).unwrap_or("null".into());
                let alen = self.t();
                self.line(&format!("{alen} = call i64 @sn_str_len(ptr {a})"));
                let acs = self.t();
                self.line(&format!("{acs} = call ptr @sn_str_cstr(ptr {a})"));
                let blen = self.t();
                self.line(&format!("{blen} = call i64 @sn_str_len(ptr {b})"));
                let bcs = self.t();
                self.line(&format!("{bcs} = call ptr @sn_str_cstr(ptr {b})"));
                let mbit = self.t();
                self.line(&format!("{mbit} = icmp eq i64 {alen}, {blen}"));
                let m = self.t();
                self.line(&format!("{m} = zext i1 {mbit} to i64"));
                let cm = self.t();
                self.line(&format!(
                    "{cm} = call i64 @sn_consttime_eq(ptr {acs}, ptr {bcs}, i64 {alen})"
                ));
                let both = self.t();
                self.line(&format!("{both} = and i64 {m}, {cm}"));
                let bit = self.t();
                self.line(&format!("{bit} = icmp ne i64 {both}, 0"));
                let r = self.t();
                self.line(&format!("{r} = zext i1 {bit} to i64"));
                (r, Type::Bool)
            }
            "secure_zero" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let len = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                self.line(&format!("call void @sn_secure_zero(ptr {p}, i64 {len})"));
                ("0".into(), Type::Void)
            }
            "int_ptr" => {
                let v = vs.first().map(|x| x.0.clone()).unwrap_or("0".into());
                let p = self.t();
                self.line(&format!("{p} = inttoptr i64 {v} to ptr"));
                (p, Type::Ptr)
            }
            "free_checked" | "check_live" | "alloc_size" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let r = self.t();
                let sym = match n {
                    "free_checked" => "sn_free_checked",
                    "check_live" => "sn_check_live",
                    _ => "sn_alloc_size",
                };
                self.line(&format!("{r} = call i64 @{sym}(ptr {p})"));
                (r, Type::Int)
            }
            "alloc_live_count" => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_alloc_live_count()"));
                (r, Type::Int)
            }
            "alloc_freed_count" => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_alloc_freed_count()"));
                (r, Type::Int)
            }
            "alloc_guard" => {
                let v = vs.first().map(|x| x.0.clone()).unwrap_or("1".into());
                self.line(&format!("call void @sn_alloc_guard(i64 {v})"));
                ("0".into(), Type::Void)
            }
            "gc_count" => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_gc_object_count()"));
                (r, Type::Int)
            }
            "gc_collect" => {
                // Pass every GC-visible local that is live here. Taking the
                // address of a slot keeps its value in memory, so the
                // collector sees exactly what the program can still reach.
                let mut names: Vec<String> = self.locals.keys().cloned().collect();
                names.sort();
                for nm in names {
                    if self.loop_locals.contains(&nm) {
                        continue; // storage does not dominate this point
                    }
                    if let Some((an, t)) = self.locals.get(&nm).cloned() {
                        if t.is_ptr()
                            || matches!(
                                t,
                                Type::Str
                                    | Type::Error
                                    | Type::Json
                                    | Type::List(_)
                                    | Type::Map(_, _)
                                    | Type::Blueprint(_)
                                    | Type::Contract(_)
                                    | Type::Optional(_)
                                    | Type::Ref(_)
                                    | Type::Chan(_)
                                    | Type::Ptr
                                    | Type::Named(_)
                            )
                        {
                            // Root the local's *value*: the pointer the
                            // program can still reach, not the slot address.
                            let val = self.t();
                            self.line(&format!("{val} = load ptr, ptr {an}"));
                            self.line(&format!("call void @sn_gc_root_push(ptr {val})"));
                        }
                    }
                }
                self.line("call void @sn_gc_collect_roots()");
                ("0".into(), Type::Void)
            }
            "gc_set_threshold" => {
                let v = vs.first().map(|x| x.0.clone()).unwrap_or("4096".into());
                self.line(&format!("call void @sn_gc_set_threshold(i64 {v})"));
                ("0".into(), Type::Void)
            }
            "ptr_load" | "ptr_load_byte" | "ptr_load_f64" | "ptr_alloc" | "ptr_int" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let base = self.as_ptr(p0.clone(), vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let off = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                match n {
                    "ptr_int" => {
                        let r = self.t();
                        self.line(&format!("{r} = ptrtoint ptr {base} to i64"));
                        (r, Type::Int)
                    }
                    "ptr_alloc" => {
                        let r = self.t();
                        self.line(&format!("{r} = call ptr @sn_ptr_alloc(i64 {p0})"));
                        (r, Type::Ptr)
                    }
                    "ptr_load_byte" => {
                        let r = self.t();
                        self.line(&format!(
                            "{r} = call i64 @sn_ptr_load_byte(ptr {base}, i64 {off})"
                        ));
                        (r, Type::Int)
                    }
                    "ptr_load_f64" => {
                        let r = self.t();
                        self.line(&format!(
                            "{r} = call double @sn_ptr_load_f64(ptr {base}, i64 {off})"
                        ));
                        (r, Type::Float)
                    }
                    _ => {
                        let r = self.t();
                        self.line(&format!("{r} = call i64 @sn_ptr_load(ptr {base}, i64 {off})"));
                        (r, Type::Int)
                    }
                }
            }
            "ptr_store" | "ptr_store_byte" | "ptr_store_f64" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let base = self.as_ptr(p0.clone(), vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let off = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                let val = vs.get(2).map(|x| x.0.clone()).unwrap_or("0".into());
                match n {
                    "ptr_store_byte" => self.line(&format!(
                        "call void @sn_ptr_store_byte(ptr {base}, i64 {off}, i64 {val})"
                    )),
                    "ptr_store_f64" => self.line(&format!(
                        "call void @sn_ptr_store_f64(ptr {base}, i64 {off}, double {val})"
                    )),
                    _ => self.line(&format!(
                        "call void @sn_ptr_store(ptr {base}, i64 {off}, i64 {val})"
                    )),
                }
                ("0".into(), Type::Void)
            }
            "ptr_load_str" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let base = self.as_ptr(p0.clone(), vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let off = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_ptr_load_str(ptr {base}, i64 {off})"));
                (r, Type::Str)
            }
            "ptr_store_str" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let base = self.as_ptr(p0.clone(), vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let off = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                let s = vs.get(2).map(|x| x.0.clone()).unwrap_or("null".into());
                let len = self.t();
                self.line(&format!("{len} = call i64 @sn_str_len(ptr {s})"));
                let cs = self.t();
                self.line(&format!("{cs} = call ptr @sn_str_cstr(ptr {s})"));
                self.line(&format!(
                    "call void @sn_ptr_store_str(ptr {base}, i64 {off}, ptr {cs}, i64 {len})"
                ));
                ("0".into(), Type::Void)
            }
            "ptr_free" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let base = self.as_ptr(p0.clone(), vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                self.line(&format!("call void @sn_ptr_free(ptr {base})"));
                ("0".into(), Type::Void)
            }
            "select_read" => {
                // select_read(list<int> fds, timeout_ms) -> index
                let arr = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let tmo = vs.get(1).map(|x| x.0.clone()).unwrap_or("-1".into());
                let cnt = self.t();
                self.line(&format!("{cnt} = call i64 @sn_list_len(ptr {arr})"));
                let p = self.t();
                self.line(&format!("{p} = call ptr @sn_list_data(ptr {arr})"));
                let r = self.t();
                self.line(&format!(
                    "{r} = call i64 @sn_select_read(i64 {cnt}, ptr {p}, i64 {tmo})"
                ));
                (r, Type::Int)
            }
            "any_box" => {
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_any_box_i64(i64 {})", vs[0].0));
                (r, Type::Any)
            }
            "alloc" => {
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_alloc(i64 {})", vs[0].0));
                (r, Type::Ref(Box::new(Type::Byte)))
            }
            "free" => {
                self.line(&format!("call void @sn_free(ptr {})", vs[0].0));
                ("0".into(), Type::Void)
            }
            "address" => {
                if let Arg::Pos(e) = &args[0] {
                    if let ExprKind::Ident(nm) = &e.kind {
                        let an = self.locals.get(nm).unwrap().0.clone();
                        return (an, Type::Ref(Box::new(self.locals.get(nm).unwrap().1.clone())));
                    }
                }
                ("null".into(), Type::Ref(Box::new(Type::Int)))
            }
            "value" => {
                let r = self.t();
                self.line(&format!("{r} = load i64, ptr {}", vs[0].0));
                (r, Type::Int)
            }
            "set" => {
                self.line(&format!("store i64 {}, ptr {}", vs[1].0, vs[0].0));
                ("0".into(), Type::Void)
            }
            other => {
                if let Some((an, Type::Fn { params, ret })) = self.locals.get(other).cloned() {
                    let c = self.t();
                    self.line(&format!("{c} = load ptr, ptr {an}"));
                    return self.call_closure_val(c, &params, &ret, args);
                }
                if let Some(sig) = self.db.funcs.get(other).cloned() {
                    self.call_user_fn(&sig, &vs)
                } else {
                    ("0".into(), Type::Int)
                }
            }
        }
    }

    fn lookup_blueprint_method(&self, bp: &str, m: &str) -> Option<FuncSig> {
        let mut cur: Option<String> = Some(bp.to_string());
        while let Some(c) = cur {
            let key = format!("{c}::{m}");
            if let Some(sig) = self.db.funcs.get(&key) {
                return Some(sig.clone());
            }
            cur = self.db.blueprints.get(&c).and_then(|b| b.parent.clone());
        }
        None
    }

    fn lookup_contract_method(&self, c: &str, m: &str) -> Option<FuncSig> {
        if let Some(ct) = self.db.contracts.get(c) {
            for cm in &ct.methods {
                if cm.name == m {
                    return Some(cm.clone());
                }
            }
        }
        None
    }

    fn vcall(&mut self, b: &str, m: &str, sig: &FuncSig, vs: &[(String, Type)]) -> (String, Type) {
        let mut argv = vec![format!("ptr {b}")];
        for (i, p) in sig.params.iter().enumerate() {
            if i < vs.len() {
                argv.push(format!("{} {}", llty(&p.1), vs[i].0));
            }
        }
        let Some(&slot) = self.db.method_slots.get(m) else {
            // no slot known (shouldn't happen for a valid method): fall back to void-safe no-op
            return if sig.ret == Type::Void {
                ("0".into(), Type::Void)
            } else {
                ("0".into(), sig.ret.clone())
            };
        };
        let g = self.db.vtable_methods.len();
        let vp = self.t();
        self.line(&format!("{vp} = getelementptr i8, ptr {b}, i64 16"));
        let vt = self.t();
        self.line(&format!("{vt} = load ptr, ptr {vp}"));
        let sp = self.t();
        self.line(&format!(
            "{sp} = getelementptr [{g} x ptr], ptr {vt}, i32 0, i32 {slot}"
        ));
        let ff = self.t();
        self.line(&format!("{ff} = load ptr, ptr {sp}"));
        if sig.ret == Type::Void {
            self.line(&format!("call void {ff}({})", argv.join(", ")));
            ("0".into(), Type::Void)
        } else {
            let r = self.t();
            self.line(&format!(
                "{r} = call {} {ff}({})",
                llty_ret(&sig.ret),
                argv.join(", ")
            ));
            (r, sig.ret.clone())
        }
    }

    fn method(&mut self, base: &Expr, name: &str, args: &[Arg]) -> (String, Type) {
        if let ExprKind::OptionalChain(inner) = &base.kind {
            let (b, _bt) = self.expr(inner);
            let yes = self.l("omy");
            let no = self.l("omn");
            let join = self.l("omj");
            let c = self.t();
            self.line(&format!("{c} = icmp ne ptr {b}, null"));
            self.line(&format!("br i1 {c}, label %{yes}, label %{no}"));
            self.raw(&format!("{yes}:"));
            let dummy = Expr {
                kind: ExprKind::Ident("__optbase".into()),
                span: inner.span,
            };
            let an = format!("%v_optbase_{}", self.l("ob"));
            self.line(&format!("{an} = alloca ptr"));
            self.line(&format!("store ptr {b}, ptr {an}"));
            self.locals
                .insert("__optbase".into(), (an, self.lookup_ty(inner)));
            let (sv, st) = self.method(&dummy, name, args);
            self.locals.remove("__optbase");
            let some_v = if st.is_ptr() || matches!(st, Type::Optional(_) | Type::Json) {
                sv.clone()
            } else if matches!(st, Type::Void) {
                sv.clone()
            } else {
                let boxed = self.t();
                self.line(&format!("{boxed} = call ptr @sn_box_i64(i64 {sv})"));
                boxed
            };
            self.line(&format!("br label %{join}"));
            self.raw(&format!("{no}:"));
            self.line(&format!("br label %{join}"));
            self.raw(&format!("{join}:"));
            if matches!(st, Type::Void) {
                return ("0".into(), Type::Void);
            }
            let phi = self.t();
            self.line(&format!(
                "{phi} = phi ptr [ {some_v}, %{yes} ], [ null, %{no} ]"
            ));
            let rt = if matches!(st, Type::Optional(_)) {
                st
            } else {
                Type::Optional(Box::new(st))
            };
            return (phi, rt);
        }
        let (b, bt) = self.expr(base);
        let mut vs = Vec::new();
        for a in args {
            match a {
                Arg::Pos(e) | Arg::Named { value: e, .. } => vs.push(self.expr(e)),
            }
        }
        let base_ty = match &bt {
            Type::Optional(i) => i.as_ref().clone(),
            other => other.clone(),
        };
        match (&base_ty, name) {
            (Type::Str, "length") => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_str_len(ptr {b})"));
                (r, Type::Int)
            }
            (Type::Str, "slice") => {
                let r = self.t();
                self.line(&format!(
                    "{r} = call ptr @sn_str_slice(ptr {b}, i64 {}, i64 {})",
                    vs[0].0, vs[1].0
                ));
                (r, Type::Str)
            }
            (Type::Str, "contains") => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_str_contains(ptr {b}, ptr {})", vs[0].0));
                (r, Type::Bool)
            }
            (Type::Str, "split") => {
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_str_split(ptr {b}, ptr {})", vs[0].0));
                (r, Type::List(Box::new(Type::Str)))
            }
            (Type::Str, "replace") => {
                let r = self.t();
                self.line(&format!(
                    "{r} = call ptr @sn_str_replace(ptr {b}, ptr {}, ptr {})",
                    vs[0].0, vs[1].0
                ));
                (r, Type::Str)
            }
            (Type::Str, "upper") => {
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_str_upper(ptr {b})"));
                (r, Type::Str)
            }
            (Type::Str, "lower") => {
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_str_lower(ptr {b})"));
                (r, Type::Str)
            }
            (Type::List(elem), "push") => {
                let (v, vt) = vs[0].clone();
                let (v, _) = self.coerce(v, &vt, elem);
                let kind = if elem.is_ptr() || matches!(**elem, Type::Optional(_)) { "ptr" } else { "i64" };
                if kind == "ptr" {
                    self.line(&format!("call void @sn_list_push_ptr(ptr {b}, ptr {v})"));
                } else {
                    self.line(&format!("call void @sn_list_push_i64(ptr {b}, i64 {v})"));
                }
                ("0".into(), Type::Void)
            }
            (Type::List(_), "length") | (Type::Map(_, _), "length") => {
                let r = self.t();
                let fnm = if matches!(base_ty, Type::List(_)) {
                    "sn_list_len"
                } else {
                    "sn_map_len"
                };
                self.line(&format!("{r} = call i64 @{fnm}(ptr {b})"));
                (r, Type::Int)
            }
            (Type::Map(k, _), "keys") => {
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_map_keys(ptr {b})"));
                (r, Type::List(k.clone()))
            }
            (Type::Map(_, v), "values") => {
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_map_values(ptr {b})"));
                (r, Type::List(v.clone()))
            }
            (Type::Chan(_), "send") => {
                let bits = self.as_bits(vs[0].0.clone(), &vs[0].1);
                self.line(&format!("call void @sn_chan_send(ptr {b}, i64 {bits})"));
                ("0".into(), Type::Void)
            }
            (Type::Chan(t), "receive") => {
                let bits = self.t();
                self.line(&format!("{bits} = call i64 @sn_chan_recv(ptr {b})"));
                (self.from_bits(bits, t), *t.clone())
            }
            (Type::Chan(_), "close") => {
                self.line(&format!("call void @sn_chan_close(ptr {b})"));
                ("0".into(), Type::Void)
            }
            (Type::Error, "message") => {
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_error_msg(ptr {b})"));
                (r, Type::Str)
            }
            (Type::Json, "encode") | (Type::Json, "as_str") => {
                let r = self.t();
                let fnm = if name == "as_str" {
                    "sn_json_as_str"
                } else {
                    "sn_json_encode"
                };
                self.line(&format!("{r} = call ptr @{fnm}(ptr {b})"));
                (r, Type::Str)
            }
            (Type::Json, "as_int") => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_json_as_int(ptr {b})"));
                (r, Type::Int)
            }
            (Type::Json, "as_bool") => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_json_as_bool(ptr {b})"));
                (r, Type::Bool)
            }
            (Type::Json, "is_null") => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_json_is_null(ptr {b})"));
                (r, Type::Bool)
            }
            (Type::Json, "is_object") => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_json_is_object(ptr {b})"));
                (r, Type::Bool)
            }
            (Type::Json, "is_list") => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_json_is_list(ptr {b})"));
                (r, Type::Bool)
            }
            (Type::Json, "length") => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_json_len(ptr {b})"));
                (r, Type::Int)
            }
            (Type::Json, "get") => {
                let r = self.t();
                self.line(&format!(
                    "{r} = call ptr @sn_json_get(ptr {b}, ptr {})",
                    vs[0].0
                ));
                (r, Type::Json)
            }
            (Type::Json, "at") => {
                let r = self.t();
                self.line(&format!(
                    "{r} = call ptr @sn_json_at(ptr {b}, i64 {})",
                    vs[0].0
                ));
                (r, Type::Json)
            }
            (Type::Json, "keys") => {
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_json_keys(ptr {b})"));
                (r, Type::List(Box::new(Type::Str)))
            }
            (Type::Blueprint(bp), m) => {
                let sig = self.lookup_blueprint_method(bp, m);
                let Some(sig) = sig else {
                    return ("0".into(), Type::Void);
                };
                if sig.is_static {
                    let mut argv = Vec::new();
                    for (i, p) in sig.params.iter().enumerate() {
                        if i < vs.len() {
                            argv.push(format!("{} {}", llty(&p.1), vs[i].0));
                        }
                    }
                    if sig.ret == Type::Void {
                        self.line(&format!("call void @{}({})", sig.llvm, argv.join(", ")));
                        return ("0".into(), Type::Void);
                    }
                    let r = self.t();
                    self.line(&format!(
                        "{r} = call {} @{}({})",
                        llty_ret(&sig.ret),
                        sig.llvm,
                        argv.join(", ")
                    ));
                    return (r, sig.ret.clone());
                }
                self.vcall(&b, m, &sig, &vs)
            }
            (Type::Contract(c), m) => {
                let sig = self.lookup_contract_method(c, m);
                let Some(sig) = sig else {
                    return ("0".into(), Type::Void);
                };
                self.vcall(&b, m, &sig, &vs)
            }
            (Type::Str, m) if self.db.funcs.contains_key(&format!("str::{m}")) => {
                let sig = self.db.funcs.get(&format!("str::{m}")).cloned().unwrap();
                let mut argv = vec![format!("ptr {b}")];
                for (i, p) in sig.params.iter().enumerate() {
                    if i < vs.len() {
                        argv.push(format!("{} {}", llty(&p.1), vs[i].0));
                    }
                }
                if sig.ret == Type::Void {
                    self.line(&format!("call void @{}({})", sig.llvm, argv.join(", ")));
                    ("0".into(), Type::Void)
                } else {
                    let r = self.t();
                    self.line(&format!(
                        "{r} = call {} @{}({})",
                        llty_ret(&sig.ret),
                        sig.llvm,
                        argv.join(", ")
                    ));
                    (r, sig.ret.clone())
                }
            }
            (Type::Any, m) => {
                let r = self.t();
                match m {
                    "as_int" => {
                        self.line(&format!("{r} = call i64 @sn_any_as_int(ptr {b})"));
                        (r, Type::Int)
                    }
                    "as_str" => {
                        self.line(&format!("{r} = call ptr @sn_any_as_str(ptr {b})"));
                        (r, Type::Str)
                    }
                    "as_bool" => {
                        self.line(&format!("{r} = call i64 @sn_any_as_bool(ptr {b})"));
                        (r, Type::Bool)
                    }
                    "type_name" => {
                        self.line(&format!("{r} = call ptr @sn_any_type_name(ptr {b})"));
                        (r, Type::Str)
                    }
                    _ => ("0".into(), Type::Void),
                }
            }
            _ => ("0".into(), Type::Void),
        }
    }

    fn member(&mut self, base: &Expr, name: &str) -> (String, Type) {
        if let ExprKind::Ident(en) = &base.kind {
            if let Some(vars) = self.db.enums.get(en).cloned() {
                if let Some(idx) = vars.iter().position(|v| v == name) {
                    return (format!("{}", idx as i64), Type::Enum(en.clone()));
                }
            }
        }
        let (b, bt) = self.expr(base);
        match (&bt, name) {
            (Type::Str, "length") | (Type::List(_), "length") => {
                self.method(base, "length", &[])
            }
            (Type::Chan(_), "open") => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_chan_open(ptr {b})"));
                (r, Type::Bool)
            }
            (Type::Error, "message") => self.method(base, "message", &[]),
            (Type::Record(rn), field) => {
                if let Some(info) = self.db.records.get(rn) {
                    if let Some(f) = info.fields.iter().find(|f| f.name == *field) {
                        let fp = self.t();
                        self.line(&format!(
                            "{fp} = getelementptr i8, ptr {b}, i64 {}",
                            f.offset
                        ));
                        if matches!(f.ty, Type::Record(_)) {
                            return (fp, f.ty.clone());
                        }
                        let (ft, signed) = ll_field_ty(&f.ty);
                        if ft == "i64" {
                            let v = self.t();
                            self.line(&format!("{v} = load i64, ptr {fp}"));
                            return (v, f.ty.clone());
                        } else if ft == "double" {
                            let v = self.t();
                            self.line(&format!("{v} = load double, ptr {fp}"));
                            return (v, f.ty.clone());
                        } else {
                            let raw = self.t();
                            self.line(&format!("{raw} = load {ft}, ptr {fp}"));
                            let v = self.t();
                            if signed {
                                self.line(&format!("{v} = sext {ft} {raw} to i64"));
                            } else {
                                self.line(&format!("{v} = zext {ft} {raw} to i64"));
                            }
                            return (v, f.ty.clone());
                        }
                    }
                }
                (b, bt)
            }
            (Type::Blueprint(bp), field) => {
                if let Some(info) = self.db.blueprints.get(bp) {
                    if let Some(f) = info.fields.iter().find(|f| f.name == *field) {
                        let fp = self.t();
                        self.line(&format!(
                            "{fp} = getelementptr i8, ptr {b}, i64 {}",
                            f.offset
                        ));
                        let v = self.t();
                        self.line(&format!("{v} = load {}, ptr {fp}", llty(&f.ty)));
                        return (v, f.ty.clone());
                    }
                }
                (b, bt)
            }
            _ => (b, bt),
        }
    }

    fn print_val(&mut self, v: String, t: &Type, nl: bool) {
        let pi = if nl { "sn_print_i64" } else { "sn_printn_i64" };
        let pb = if nl { "sn_print_bool" } else { "sn_printn_bool" };
        let ps = if nl { "sn_print_str" } else { "sn_printn_str" };
        let pn = if nl { "sn_print_none" } else { "sn_printn_none" };
        match t {
            Type::Float => {
                let pf = if nl { "sn_print_f64" } else { "sn_printn_f64" };
                self.line(&format!("call void @{pf}(double {v})"));
            }
            Type::Dec(sc) => {
                let st = self.t();
                self.line(&format!("{st} = call ptr @sn_str_from_dec(i64 {v}, i64 {sc})"));
                self.line(&format!("call void @{ps}(ptr {st})"));
            }
            Type::Enum(_) => {
                let st = self.to_str(v, t);
                self.line(&format!("call void @{ps}(ptr {st})"));
            }
            Type::Int | Type::Byte | Type::I8 | Type::I16 | Type::I32 | Type::U8 | Type::U16 | Type::U32 | Type::U64 => {
                self.line(&format!("call void @{pi}(i64 {v})"));
            }
            Type::Bool => self.line(&format!("call void @{pb}(i64 {v})")),
            Type::Str => self.line(&format!("call void @{ps}(ptr {v})")),
            Type::None | Type::Optional(_) => {
                let c = self.t();
                self.line(&format!("{c} = icmp eq ptr {v}, null"));
                let y = self.l("pn");
                let n = self.l("ps");
                let j = self.l("pj");
                self.line(&format!("br i1 {c}, label %{y}, label %{n}"));
                self.raw(&format!("{y}:"));
                self.line(&format!("call void @{pn}()"));
                self.line(&format!("br label %{j}"));
                self.raw(&format!("{n}:"));
                if matches!(t, Type::Optional(inner) if !inner.is_ptr() && !matches!(**inner, Type::Optional(_) | Type::Json))
                {
                    let u = self.t();
                    self.line(&format!("{u} = call i64 @sn_unbox_i64(ptr {v})"));
                    if let Type::Optional(inner) = t {
                        match inner.as_ref() {
                            Type::Dec(sc) => {
                                let st = self.t();
                                self.line(&format!("{st} = call ptr @sn_str_from_dec(i64 {u}, i64 {sc})"));
                                self.line(&format!("call void @{ps}(ptr {st})"));
                            }
                            Type::Enum(_) => {
                                let st = self.to_str(u, inner.as_ref());
                                self.line(&format!("call void @{ps}(ptr {st})"));
                            }
                            Type::Bool => {
                                self.line(&format!("call void @{pb}(i64 {u})"));
                            }
                            Type::Float => {
                                let f = self.t();
                                self.line(&format!("{f} = bitcast i64 {u} to double"));
                                let pf = if nl { "sn_print_f64" } else { "sn_printn_f64" };
                                self.line(&format!("call void @{pf}(double {f})"));
                            }
                            _ => {
                                self.line(&format!("call void @{pi}(i64 {u})"));
                            }
                        }
                    }
                } else {
                    self.line(&format!("call void @{ps}(ptr {v})"));
                }
                self.line(&format!("br label %{j}"));
                self.raw(&format!("{j}:"));
            }
            Type::Tuple(ts) => {
                for i in 0..ts.len() {
                    let el = self.t();
                    self.line(&format!("{el} = extractvalue {} {v}, {i}", tuple_ll(ts)));
                    self.print_val(el, &ts[i], nl);
                }
            }
            Type::Error => {
                let m = self.t();
                self.line(&format!("{m} = call ptr @sn_error_msg(ptr {v})"));
                self.line(&format!("call void @{ps}(ptr {m})"));
            }
            Type::Json => {
                let m = self.t();
                self.line(&format!("{m} = call ptr @sn_json_encode(ptr {v})"));
                self.line(&format!("call void @{ps}(ptr {m})"));
            }
            _ => self.line(&format!("call void @{pi}(i64 0)")),
        }
    }

    fn to_str(&mut self, v: String, t: &Type) -> String {
        let r = self.t();
        match t {
            Type::Str => return v,
            Type::Float => self.line(&format!("{r} = call ptr @sn_str_from_f64(double {v})")),
            Type::Enum(en) => {
                if let Some(variants) = self.db.enums.get(en).cloned() {
                    let sw_end = self.l("enum_str_end");
                    let def_lbl = self.l("enum_str_def");
                    let mut labels = Vec::new();
                    for (idx, _name) in variants.iter().enumerate() {
                        labels.push((idx, self.l("enum_v")));
                    }
                    let cases: Vec<String> = labels
                        .iter()
                        .map(|(idx, l)| format!("i64 {idx}, label %{l}"))
                        .collect();
                    self.line(&format!("switch i64 {v}, label %{def_lbl} [ {} ]", cases.join(" ")));
                    let phi_res = self.t();
                    let mut incoming = Vec::new();
                    for (idx, lbl) in &labels {
                        self.raw(&format!("{lbl}:"));
                        let name = &variants[*idx];
                        let id = self.strings.len();
                        self.strings.push(name.clone());
                        let s_ptr = self.t();
                        self.line(&format!("{s_ptr} = call ptr @sn_str_new(ptr @.str.{id}, i64 {})", name.len()));
                        incoming.push(format!("[ {s_ptr}, %{lbl} ]"));
                        self.line(&format!("br label %{sw_end}"));
                    }
                    self.raw(&format!("{def_lbl}:"));
                    let unk_ptr = self.t();
                    self.line(&format!("{unk_ptr} = call ptr @sn_str_from_i64(i64 {v})"));
                    incoming.push(format!("[ {unk_ptr}, %{def_lbl} ]"));
                    self.line(&format!("br label %{sw_end}"));
                    self.raw(&format!("{sw_end}:"));
                    self.line(&format!("{phi_res} = phi ptr {}", incoming.join(", ")));
                    return phi_res;
                } else {
                    self.line(&format!("{r} = call ptr @sn_str_from_i64(i64 {v})"));
                }
            }
            Type::Int | Type::Byte | Type::I8 | Type::I16 | Type::I32 | Type::U8 | Type::U16 | Type::U32 | Type::U64 => self.line(&format!("{r} = call ptr @sn_str_from_i64(i64 {v})")),
            Type::Bool => self.line(&format!("{r} = call ptr @sn_str_from_bool(i64 {v})")),
            Type::Dec(s) => self.line(&format!("{r} = call ptr @sn_str_from_dec(i64 {v}, i64 {s})")),
            _ => self.line(&format!("{r} = call ptr @sn_str_from_cstr(ptr null)")),
        }
        r
    }

    fn cast_val(&mut self, v: String, from: &Type, to: &Type) -> String {
        if from == to {
            return v;
        }
        // enum <-> int: identity at runtime (type-level only)
        if matches!(from, Type::Enum(_)) && to.is_fixed_int() {
            return v;
        }
        if from.is_fixed_int() && matches!(to, Type::Enum(_)) {
            return self.wrap_int(v, from);
        }
        // int -> int: wrap to target width
        if from.is_fixed_int() && to.is_fixed_int() {
            return self.wrap_int(v, to);
        }
        // int -> float
        if from.is_fixed_int() && *to == Type::Float {
            let r = self.t();
            self.line(&format!("{r} = sitofp i64 {v} to double"));
            return r;
        }
        // float -> int (truncate)
        if *from == Type::Float && to.is_fixed_int() {
            let r = self.t();
            self.line(&format!("{r} = fptosi double {v} to i64"));
            return self.wrap_int(r, to);
        }
        // dec <-> float
        if let (Type::Dec(s), Type::Float) = (from, to) {
            let mut p = 1.0f64;
            for _ in 0..*s {
                p *= 10.0;
            }
            let raw_fp = self.t();
            self.line(&format!("{raw_fp} = sitofp i64 {v} to double"));
            let r = self.t();
            self.line(&format!("{r} = fdiv double {raw_fp}, {p:?}"));
            return r;
        }
        if let (Type::Float, Type::Dec(s)) = (from, to) {
            let mut p = 1.0f64;
            for _ in 0..*s {
                p *= 10.0;
            }
            let scaled = self.t();
            self.line(&format!("{scaled} = fmul double {v}, {p:?}"));
            let r = self.t();
            self.line(&format!("{r} = fptosi double {scaled} to i64"));
            return r;
        }
        // float -> float identity handled above
        match (from, to) {
            (_, Type::Str) => self.to_str(v, from),
            (_, Type::Float) if *from == Type::Str => {
                let r = self.t();
                self.line(&format!("{r} = call double @sn_f64_from_str(ptr {v})"));
                r
            }
            (Type::Str, Type::Int) => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_i64_from_str(ptr {v})"));
                r
            }
            (Type::Int, Type::Dec(s)) => {
                let mut p = 1i64;
                for _ in 0..*s {
                    p *= 10;
                }
                let r = self.t();
                self.line(&format!("{r} = mul i64 {v}, {p}"));
                r
            }
            (Type::Dec(_), Type::Int) => v,
            (Type::Int, Type::Bool) | (Type::Bool, Type::Int) => v,
            _ => v,
        }
    }

    fn coerce(&mut self, v: String, from: &Type, to: &Type) -> (String, Type) {
        if from != to && *to == Type::Float && (from.int_min_max().is_some() || *from == Type::Int) {
            let r = self.t();
            self.line(&format!("{r} = sitofp i64 {v} to double"));
            return (r, to.clone());
        }
        if to.assignable_from(from, &self.db.blueprints) && from != to {
            if matches!(to, Type::Optional(inner) if !inner.is_ptr() && from != &Type::None) {
                let b = self.t();
                self.line(&format!("{b} = call ptr @sn_box_i64(i64 {v})"));
                return (b, to.clone());
            }
            if from == &Type::None {
                return ("null".into(), to.clone());
            }
        }
        (v, to.clone())
    }

    fn as_i1(&mut self, v: String) -> String {
        if v == "null" {
            let t = self.t();
            self.line(&format!("{t} = add i1 0, 0"));
            return t;
        }
        let t = self.t();
        self.line(&format!("{t} = icmp ne i64 {v}, 0"));
        t
    }

    fn eq_vals(&mut self, a: String, b: String, t: &Type) -> String {
        let c = self.t();
        if t == &Type::Str {
            self.line(&format!("{c} = call i64 @sn_str_eq(ptr {a}, ptr {b})"));
            return c;
        }
        let i = self.t();
        if t == &Type::Float {
            self.line(&format!("{i} = fcmp oeq double {a}, {b}"));
        } else if t.is_ptr() {
            self.line(&format!("{i} = icmp eq ptr {a}, {b}"));
        } else {
            self.line(&format!("{i} = icmp eq i64 {a}, {b}"));
        }
        let z = self.t();
        self.line(&format!("{z} = zext i1 {i} to i64"));
        z
    }

    fn as_bits(&mut self, v: String, t: &Type) -> String {
        if t == &Type::Float {
            let r = self.t();
            self.line(&format!("{r} = bitcast double {v} to i64"));
            return r;
        }
        if t.is_ptr() || matches!(t, Type::Optional(_)) {
            let r = self.t();
            self.line(&format!("{r} = ptrtoint ptr {v} to i64"));
            r
        } else {
            v
        }
    }

    fn from_bits(&mut self, bits: String, t: &Type) -> String {
        if t == &Type::Float {
            let r = self.t();
            self.line(&format!("{r} = bitcast i64 {bits} to double"));
            return r;
        }
        if t.is_ptr() || matches!(t, Type::Optional(_)) {
            let r = self.t();
            self.line(&format!("{r} = inttoptr i64 {bits} to ptr"));
            r
        } else {
            bits
        }
    }

    fn coerce_narrow(&mut self, v: String, from: &Type, to: &Type) -> (String, Type) {
        if let Type::Optional(inner) = from {
            if to == inner.as_ref() {
                if inner.is_ptr() || matches!(inner.as_ref(), Type::Optional(_) | Type::Json) {
                    return (v, to.clone());
                }
                let u = self.t();
                self.line(&format!("{u} = call i64 @sn_unbox_i64(ptr {v})"));
                return (u, to.clone());
            }
        }
        self.coerce(v, from, to)
    }

    fn intern(&mut self, s: &str) -> (usize, usize) {
        let id = self.strings.len();
        self.strings.push(s.to_string());
        (id, s.len())
    }

    fn panic_msg(&mut self, msg: &str) {
        let (id, n) = self.intern(msg);
        let t = self.t();
        self.line(&format!(
            "{t} = call ptr @sn_str_new(ptr @.str.{id}, i64 {n})"
        ));
        self.line(&format!("call void @sn_panic(ptr {t})"));
    }

    fn expr_unwrap(&mut self, inner: &Expr) -> (String, Type) {
        let (v, t) = self.expr(inner);
        let inner_ty = t.unwrap_optional().cloned().unwrap_or(t.clone());
        let ok = self.l("uw");
        let bad = self.l("up");
        let c = self.t();
        self.line(&format!("{c} = icmp eq ptr {v}, null"));
        self.line(&format!("br i1 {c}, label %{bad}, label %{ok}"));
        self.raw(&format!("{bad}:"));
        self.panic_msg("unwrap of none");
        self.line("unreachable");
        self.raw(&format!("{ok}:"));
        if inner_ty.is_ptr() || matches!(inner_ty, Type::Optional(_) | Type::Json) {
            (v, inner_ty)
        } else {
            let u = self.t();
            self.line(&format!("{u} = call i64 @sn_unbox_i64(ptr {v})"));
            (u, inner_ty)
        }
    }

    fn expr_try(&mut self, inner: &Expr) -> (String, Type) {
        let (v, t) = self.expr(inner);
        let Type::Tuple(ts) = t.clone() else {
            return (v, t);
        };
        if ts.len() != 2 {
            return (v, t);
        }
        let val_ty = ts[0].clone();
        let st = tuple_ll(&ts);
        let val = self.t();
        let err = self.t();
        self.line(&format!("{val} = extractvalue {st} {v}, 0"));
        self.line(&format!("{err} = extractvalue {st} {v}, 1"));
        let ok = self.l("tryok");
        let fail = self.l("tryerr");
        let c = self.t();
        self.line(&format!("{c} = icmp eq ptr {err}, null"));
        self.line(&format!("br i1 {c}, label %{ok}, label %{fail}"));
        self.raw(&format!("{fail}:"));
        self.emit_defers();
        match &self.current_fn_ret.clone() {
            Type::Tuple(rts) if rts.len() == 2 && rts[1] == Type::Error => {
                let rst = tuple_ll(rts);
                let mut agg = "zeroinitializer".to_string();
                let n = self.t();
                self.line(&format!("{n} = insertvalue {rst} {agg}, ptr {err}, 1"));
                agg = n;
                if rts[0].is_ptr() || matches!(rts[0], Type::Optional(_) | Type::Json) {
                    let n2 = self.t();
                    self.line(&format!("{n2} = insertvalue {rst} {agg}, ptr null, 0"));
                    agg = n2;
                }
                self.line(&format!("ret {rst} {agg}"));
            }
            Type::Error => {
                self.line(&format!("ret ptr {err}"));
            }
            _ => {
                let m = self.t();
                self.line(&format!("{m} = call ptr @sn_error_msg(ptr {err})"));
                self.line(&format!("call void @sn_panic(ptr {m})"));
                self.line("unreachable");
            }
        }
        self.raw(&format!("{ok}:"));
        (val, val_ty)
    }

    fn make_pair(
        &mut self,
        a: String,
        at: Type,
        b: String,
        bt: Type,
    ) -> (String, Type) {
        let ts = vec![at.clone(), bt.clone()];
        let st = tuple_ll(&ts);
        let n1 = self.t();
        self.line(&format!(
            "{n1} = insertvalue {st} zeroinitializer, {} {a}, 0",
            llty(&at)
        ));
        let n2 = self.t();
        self.line(&format!(
            "{n2} = insertvalue {st} {n1}, {} {b}, 1",
            llty(&bt)
        ));
        (n2, Type::Tuple(ts))
    }

    fn make_triple(
        &mut self,
        a: String,
        at: Type,
        b: String,
        bt: Type,
        c: String,
        ct: Type,
    ) -> (String, Type) {
        let ts = vec![at.clone(), bt.clone(), ct.clone()];
        let st = tuple_ll(&ts);
        let n1 = self.t();
        self.line(&format!(
            "{n1} = insertvalue {st} zeroinitializer, {} {a}, 0",
            llty(&at)
        ));
        let n2 = self.t();
        self.line(&format!(
            "{n2} = insertvalue {st} {n1}, {} {b}, 1",
            llty(&bt)
        ));
        let n3 = self.t();
        self.line(&format!(
            "{n3} = insertvalue {st} {n2}, {} {c}, 2",
            llty(&ct)
        ));
        (n3, Type::Tuple(ts))
    }

    fn http_req(&mut self, method: &str, vs: &[(String, Type)], has_body: bool) -> (String, Type) {
        let (id, n) = self.intern(method);
        let m = self.t();
        self.line(&format!(
            "{m} = call ptr @sn_str_new(ptr @.str.{id}, i64 {n})"
        ));
        let body_arg = if has_body && vs.len() > 1 {
            vs[1].0.clone()
        } else {
            "null".into()
        };
        let ob = self.t();
        let oe = self.t();
        self.line(&format!("{ob} = alloca ptr"));
        self.line(&format!("{oe} = alloca ptr"));
        self.line(&format!(
            "call void @sn_http_request(ptr {m}, ptr {}, ptr {body_arg}, ptr {ob}, ptr {oe})",
            vs[0].0
        ));
        let b = self.t();
        let e = self.t();
        self.line(&format!("{b} = load ptr, ptr {ob}"));
        self.line(&format!("{e} = load ptr, ptr {oe}"));
        self.make_pair(b, Type::Str, e, Type::Error)
    }

    fn call_closure_val(
        &mut self,
        clos: String,
        params: &[Type],
        ret: &Type,
        args: &[Arg],
    ) -> (String, Type) {
        let mut vs = Vec::new();
        for a in args {
            match a {
                Arg::Pos(e) | Arg::Named { value: e, .. } => vs.push(self.expr(e)),
            }
        }
        let fnp = self.t();
        let env = self.t();
        self.line(&format!("{fnp} = call ptr @sn_clos_fn(ptr {clos})"));
        self.line(&format!("{env} = call ptr @sn_clos_env(ptr {clos})"));
        let mut argv = vec![format!("ptr {env}")];
        for (i, p) in params.iter().enumerate() {
            if i < vs.len() {
                argv.push(format!("{} {}", llty(p), vs[i].0));
            }
        }
        if matches!(ret, Type::Void) {
            self.line(&format!(
                "call void {fnp}({})",
                argv.join(", ")
            ));
            ("0".into(), Type::Void)
        } else {
            let r = self.t();
            self.line(&format!(
                "{r} = call {} {fnp}({})",
                llty_ret(ret),
                argv.join(", ")
            ));
            (r, ret.clone())
        }
    }

    fn fn_as_closure(&mut self, sig: &FuncSig) -> String {
        let tname = format!("sn_asclos_{}", sig.llvm);
        let already = self
            .extra_fns
            .iter()
            .any(|s| s.contains(&format!("@{tname}(")));
        if !already {
            let mut params_ll = vec!["ptr %env".to_string()];
            let mut args_ll = Vec::new();
            for (i, p) in sig.params.iter().enumerate() {
                params_ll.push(format!("{} %a{i}", llty(&p.1)));
                args_ll.push(format!("{} %a{i}", llty(&p.1)));
            }
            let rty = llty_ret(&sig.ret);
            let mut buf = format!(
                "define {rty} @{tname}({}) {{\nentry:\n",
                params_ll.join(", ")
            );
            if sig.ret == Type::Void {
                buf.push_str(&format!(
                    "  call void @{}({})\n  ret void\n}}\n",
                    sig.llvm,
                    args_ll.join(", ")
                ));
            } else {
                buf.push_str(&format!(
                    "  %r = call {rty} @{}({})\n  ret {rty} %r\n}}\n",
                    sig.llvm,
                    args_ll.join(", ")
                ));
            }
            self.extra_fns.push(buf);
        }
        let c = self.t();
        self.line(&format!(
            "{c} = call ptr @sn_clos_new(ptr @{tname}, ptr null)"
        ));
        c
    }

    fn lower_closure(&mut self, params: &[Param], ret: &Type, body: &[Stmt]) -> String {
        let mut caps: Vec<String> = Vec::new();
        collect_idents(body, &mut caps);
        let pnames: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        caps.retain(|n| self.locals.contains_key(n) && !pnames.contains(n));
        caps.sort();
        caps.dedup();
        let id = self.l("lam");
        let fname = format!("sn_lambda_{id}");
        let env = self.t();
        let nbytes = (caps.len() * 8) as i64;
        self.line(&format!("{env} = call ptr @sn_alloc(i64 {nbytes})"));
        for (i, n) in caps.iter().enumerate() {
            let (an, t) = self.locals.get(n).cloned().unwrap();
            let v = self.t();
            self.line(&format!("{v} = load {}, ptr {an}", llty(&t)));
            let slot = self.t();
            self.line(&format!(
                "{slot} = getelementptr i8, ptr {env}, i64 {}",
                i * 8
            ));
            if t.is_ptr() || matches!(t, Type::Optional(_) | Type::Fn { .. } | Type::Json) {
                self.line(&format!("store ptr {v}, ptr {slot}"));
            } else {
                self.line(&format!("store i64 {v}, ptr {slot}"));
            }
        }
        let cap_tys: Vec<(String, Type)> = caps
            .iter()
            .map(|n| (n.clone(), self.locals.get(n).unwrap().1.clone()))
            .collect();
        let mut nested = Vec::new();
        let buf = {
            let (cf, cl, _) = self.cur_loc.unwrap_or((0, 1, 1));
            let child_sub = self.dbg.sub(&format!("<lambda {id}>"), cf, cl, &fname);
            let mut child = Cx {
                db: self.db,
                buf: String::new(),
                tmp: 0,
                lbl: 0,
                locals: HashMap::new(),
                loops: Vec::new(),
        loop_locals: std::collections::HashSet::new(),
                strings: self.strings,
                extra_fns: &mut nested,
                current_bp: self.current_bp.clone(),
                files: self.files,
                cur_loc: None,
                cur_sub: child_sub,
                dbg: self.dbg,
                terminated: false,
                current_fn_ret: ret.clone(),
                defers: Vec::new(),
            };
            let mut pll = vec!["ptr %env".to_string()];
            for p in params {
                let t = resolve_type_ast(&p.ty, self.db);
                pll.push(format!("{} %p_{}", llty(&t), p.name));
            }
            child.raw(&format!(
                "define {} @{fname}({}) !dbg !{child_sub} {{",
                llty_ret(ret),
                pll.join(", ")
            ));
            child.raw("entry:");
            for (i, (n, t)) in cap_tys.iter().enumerate() {
                let slot = child.t();
                child.line(&format!(
                    "{slot} = getelementptr i8, ptr %env, i64 {}",
                    i * 8
                ));
                let an = format!("%v_{n}");
                child.line(&format!("{an} = alloca {}", llty(t)));
                let v = child.t();
                child.line(&format!("{v} = load {}, ptr {slot}", llty(t)));
                child.line(&format!("store {} {v}, ptr {an}", llty(t)));
                child.locals.insert(n.clone(), (an, t.clone()));
            }
            for p in params {
                let t = resolve_type_ast(&p.ty, self.db);
                let an = format!("%v_{}", p.name);
                child.line(&format!("{an} = alloca {}", llty(&t)));
                child.line(&format!("store {} %p_{}, ptr {an}", llty(&t), p.name));
                child.locals.insert(p.name.clone(), (an, t));
            }
            for s in body {
                child.stmt(s);
            }
            child.emit_defers();
            if matches!(ret, Type::Void) {
                child.line("ret void");
            } else if ret.is_ptr() || matches!(ret, Type::Optional(_) | Type::Fn { .. } | Type::Json)
            {
                child.line("ret ptr null");
            } else if let Type::Tuple(ts) = ret {
                child.line(&format!("ret {} zeroinitializer", tuple_ll(ts)));
            } else {
                child.line("ret i64 0");
            }
            child.raw("}");
            child.buf
        };
        self.extra_fns.push(buf);
        self.extra_fns.extend(nested);
        let c = self.t();
        self.line(&format!(
            "{c} = call ptr @sn_clos_new(ptr @{fname}, ptr {env})"
        ));
        c
    }
}

fn lower_ext_fn(
    f: &FnItem,
    db: &CheckDb,
    ext_key: &str,
    self_ty: &Type,
    strings: &mut Vec<String>,
    extra: &mut Vec<String>,
    files: &[SourceFile],
    dbg: &mut DbgInfo,
) -> String {
    let llvm_name = format!("sn_ext_{}_{}", ext_key, f.name);
    let ret_ty = f.ret.as_ref().map(|t| resolve_type_ast(t, db)).unwrap_or(Type::Void);
    let decl_line = files
        .get(f.span.file as usize)
        .map(|sf| sf.loc(f.span.start as usize).0)
        .unwrap_or(1);
    let sub_id = dbg.sub(
        &format!("{ext_key}.{}", f.name),
        f.span.file,
        decl_line,
        &llvm_name,
    );
    let mut cx = Cx {
        db,
        buf: String::new(),
        tmp: 0,
        lbl: 0,
        locals: HashMap::new(),
        loops: Vec::new(),
        loop_locals: std::collections::HashSet::new(),
        strings,
        extra_fns: extra,
        current_bp: None,
        files,
        cur_loc: None,
        cur_sub: sub_id,
        dbg,
        terminated: false,
        current_fn_ret: ret_ty.clone(),
        defers: Vec::new(),
    };
    let mut params_ll = vec!["ptr %p_self".to_string()];
    for p in &f.params {
        let t = resolve_type_ast(&p.ty, db);
        params_ll.push(format!("{} %p_{}", llty(&t), p.name));
    }
    let rty = llty_ret(&ret_ty);
    cx.raw(&format!(
        "define {rty} @{llvm_name}({}) !dbg !{sub_id} {{",
        params_ll.join(", ")
    ));
    cx.raw("entry:");
    cx.locals
        .insert("self".into(), ("%p_self".into(), self_ty.clone()));
    for p in &f.params {
        let t = resolve_type_ast(&p.ty, db);
        let an = format!("%v_{}", p.name);
        if let Type::Record(rn) = &t {
            let sz = db.records.get(rn).map(|r| r.size).unwrap_or(8);
            cx.line(&format!("{an} = alloca i8, i64 {sz}"));
            cx.line(&format!("call void @sn_record_copy(ptr {an}, ptr %p_{}, i64 {sz})", p.name));
        } else {
            cx.line(&format!("{an} = alloca {}", llty(&t)));
            cx.line(&format!("store {} %p_{}, ptr {an}", llty(&t), p.name));
        }
        cx.locals.insert(p.name.clone(), (an, t));
    }
    for s in &f.body {
        cx.stmt(s);
    }
    cx.emit_defers();
    if matches!(ret_ty, Type::Void) {
        if !cx.terminated {
            cx.line("ret void");
        }
    } else if ret_ty.is_ptr() || matches!(ret_ty, Type::Optional(_)) {
        if !cx.terminated {
            cx.line("ret ptr null");
        }
    } else if !cx.terminated {
        cx.line("ret i64 0");
    }
    cx.raw("}");
    cx.buf
}

fn ll_field_ty(t: &Type) -> (&'static str, bool) {
    match t {
        Type::Float => ("double", false),
        Type::U8 | Type::Byte | Type::Bool | Type::I8 => ("i8", matches!(t, Type::I8)),
        Type::U16 | Type::I16 => ("i16", matches!(t, Type::I16)),
        Type::U32 | Type::I32 => ("i32", matches!(t, Type::I32)),
        _ => ("i64", false),
    }
}

fn llty(t: &Type) -> &'static str {
    match t {
        Type::Record(_) => "ptr",
        Type::Named(n) if n == "future" => "ptr",
        Type::Float => "double",
        _ if t.is_ptr() || matches!(t, Type::Optional(_) | Type::None | Type::Any) => "ptr",
        _ => "i64",
    }
}

fn llty_ret(t: &Type) -> String {
    match t {
        Type::Void => "void".into(),
        Type::Tuple(ts) => tuple_ll(ts),
        _ => llty(t).into(),
    }
}

fn tuple_ll(ts: &[Type]) -> String {
    let parts: Vec<_> = ts.iter().map(llty).collect();
    format!("{{ {} }}", parts.join(", "))
}

fn assign_bin(op: AssignOp) -> BinOp {
    match op {
        AssignOp::PlusEq => BinOp::Add,
        AssignOp::MinusEq => BinOp::Sub,
        AssignOp::StarEq => BinOp::Mul,
        AssignOp::SlashEq => BinOp::Div,
        AssignOp::PercentEq => BinOp::Mod,
        AssignOp::AmpEq => BinOp::BitAnd,
        AssignOp::PipeEq => BinOp::BitOr,
        AssignOp::CaretEq => BinOp::BitXor,
        AssignOp::ShlEq => BinOp::Shl,
        AssignOp::ShrEq => BinOp::Shr,
        AssignOp::Eq => BinOp::Add,
    }
}

fn collect_idents(body: &[Stmt], out: &mut Vec<String>) {
    for s in body {
        walk_stmt(s, out);
    }
}

fn walk_stmt(s: &Stmt, out: &mut Vec<String>) {
    match s {
        Stmt::Expr(e) => walk_expr(e, out),
        Stmt::Decl { value, .. } => {
            if let Some(v) = value {
                walk_expr(v, out);
            }
        }
        Stmt::Assign { target, value, .. } => {
            walk_expr(target, out);
            walk_expr(value, out);
        }
        Stmt::If {
            cond,
            then_body,
            else_ifs,
            else_body,
            ..
        } => {
            walk_expr(cond, out);
            collect_idents(then_body, out);
            for (c, b) in else_ifs {
                walk_expr(c, out);
                collect_idents(b, out);
            }
            if let Some(b) = else_body {
                collect_idents(b, out);
            }
        }
        Stmt::While { cond, body, .. } => {
            walk_expr(cond, out);
            collect_idents(body, out);
        }
        Stmt::ForCount {
            init,
            cond,
            step,
            body,
            ..
        } => {
            walk_stmt(init, out);
            walk_expr(cond, out);
            walk_stmt(step, out);
            collect_idents(body, out);
        }
        Stmt::ForIn { iter, body, .. } => {
            walk_expr(iter, out);
            collect_idents(body, out);
        }
        Stmt::Match {
            expr,
            arms,
            default,
            ..
        } => {
            walk_expr(expr, out);
            for (p, b) in arms {
                walk_expr(p, out);
                collect_idents(b, out);
            }
            if let Some(b) = default {
                collect_idents(b, out);
            }
        }
        Stmt::Defer { body, .. } | Stmt::SpawnBlock { body, .. } => collect_idents(body, out),
        Stmt::Asm { .. } => {},
        Stmt::SpawnExpr { expr, .. } => walk_expr(expr, out),
        Stmt::NestedFn(f) => collect_idents(&f.body, out),
        Stmt::LockBlock { name, body, .. } => {
            out.push(name.clone());
            collect_idents(body, out);
        }
        _ => {}
    }
}

fn walk_expr(e: &Expr, out: &mut Vec<String>) {
    match &e.kind {
        ExprKind::Ident(n) => out.push(n.clone()),
        ExprKind::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, out);
            walk_expr(rhs, out);
        }
        ExprKind::Unary { expr, .. }
        | ExprKind::Await(expr)
        | ExprKind::Cast { expr, .. }
        | ExprKind::Try(expr)
        | ExprKind::OptionalChain(expr)
        | ExprKind::ForceUnwrap(expr) => walk_expr(expr, out),
        ExprKind::Lambda { body, .. } => collect_idents(body, out),
        ExprKind::Call { callee, args, .. } => {
            walk_expr(callee, out);
            for a in args {
                match a {
                    Arg::Pos(e) | Arg::Named { value: e, .. } => walk_expr(e, out),
                }
            }
        }
        ExprKind::Index { base, index } => {
            walk_expr(base, out);
            walk_expr(index, out);
        }
        ExprKind::Member { base, .. } => walk_expr(base, out),
        _ => {}
    }
}
