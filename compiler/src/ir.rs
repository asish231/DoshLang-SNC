use crate::ast::*;
use crate::check::{CheckDb, resolve_type_ast, mono_name, template_param_names, infer_type_param};
use crate::span::{SourceFile, Span};
use crate::types::*;
use std::collections::HashMap;

thread_local! {
    static ALGEBRAIC_ENUMS: std::cell::RefCell<std::collections::HashSet<String>> = std::cell::RefCell::new(std::collections::HashSet::new());
}

fn set_algebraic_enums(set: std::collections::HashSet<String>) {
    ALGEBRAIC_ENUMS.with(|cell| {
        *cell.borrow_mut() = set;
    });
}

fn is_algebraic_enum_type(name: &str) -> bool {
    ALGEBRAIC_ENUMS.with(|cell| cell.borrow().contains(name))
}

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
    allocas: Vec<String>,
    tmp: u32,
    lbl: u32,
    locals: HashMap<String, (String, Type)>,
    concrete_types: HashMap<String, String>,
    loops: Vec<(String, String, usize)>,
    /// Locals declared inside a loop body. Their storage does not dominate
    /// code after the loop, so they must not be referenced from there.
    loop_locals: std::collections::HashSet<String>,
    strings: &'a mut Vec<String>,
    extra_fns: &'a mut Vec<String>,
    current_bp: Option<String>,
    /// SWARM RABBIT HOLE — Wave 8 (Impl-I), DIFF-1 Rule 1b: the checked
    /// function's summary key (check::fn_summary_key) — indexes
    /// CheckDb.lane_assignment, the checker's pinned-lane pre-coloring
    /// (function → local → lane L2..L6). Empty for synthetic child
    /// contexts (closures, spawn bodies, extension shims): they host no
    /// pre-colored declarations.
    fn_key: String,
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
    current_nursery: Option<String>,
    active_locks: Vec<String>,
    active_nurseries: Vec<String>,
    active_regions: Vec<String>,
    /// Lexically open `frozen` blocks (SWARM RABBIT HOLE, D0 gap #2,
    /// Impl-B): while non-zero, the inline lane bump/reset lowering keeps
    /// the checked runtime calls, preserving the loud frozen-lane errors.
    frozen_depth: usize,
    /// SWARM RABBIT HOLE — Wave 13 (Impl-R): the lexically open wave
    /// contexts' SSA values (the SnWave* returned by sn_wave_new),
    /// innermost last. read_W/contrib lower against the innermost
    /// open wave; the checker guarantees there is exactly one (nested
    /// waves are E0812), and the stack shape keeps the lowering
    /// honest if that ever relaxes.
    wave_stack: Vec<String>,
    /// SWARM RABBIT HOLE — Wave 15 (Impl-T, Cohort Delta): Salt ε —
    /// this function's degraded contract atoms, cloned from the
    /// checker's tables at lower_fn entry. The DBM engine itself is
    /// compile-time only; the runtime sees JUST these typed
    /// assertions (the boundary rule's degradation, lowered through
    /// the fail-fast convention — sn_contract_assert: fprintf +
    /// exit(1), the E-LKIND/E-SFC pattern).
    contract_entry: Vec<crate::check::ContractCheck>,
    /// The degraded ensures atoms — checked at every return (and the
    /// fall-through exit, result bound to the returned value).
    contract_exit: Vec<crate::check::ContractCheck>,
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

thread_local! {
    /// Inline lane bump/reset gate (SWARM RABBIT HOLE, D0 gap #2,
    /// Impl-B): the driver clears it for debug builds (-O0) so every lane
    /// operation keeps the checked, poisoning runtime calls there; -O1+
    /// lowers the inline load/gep/store fast path instead.
    static LANE_INLINE: std::cell::Cell<bool> = std::cell::Cell::new(true);

    /// Compile-time 0xAA poison emission (SWARM RABBIT HOLE, Wave 6,
    /// Impl-D — bench/RESULTS.md §5 "A control the inline path breaks"):
    /// SN_POISON=1 was a silent no-op on the inline fast path, so the
    /// T-FIBR debug guarantee (W4A repair 2) vanished for constant-size
    /// allocations. When the driver sets this, EVERY lane allocation the
    /// compiler lowers — inline fast path, inline slow path and runtime
    /// call — emits its own `llvm.memset(p, 0xAA, size)`, and every
    /// region-exit rewind re-poisons [watermark, old_top) inline: the
    /// guarantee no longer depends on which codegen path ran or on the
    /// runtime's env flag. Active for -O0, `snc debug` and `--poison`.
    static LANE_POISON: std::cell::Cell<bool> = std::cell::Cell::new(false);
}

/// Enable or disable the inline lane fast path for the next `lower` pass.
pub fn lane_inline(on: bool) {
    LANE_INLINE.with(|c| c.set(on));
}

fn lane_inline_on() -> bool {
    LANE_INLINE.with(|c| c.get())
}

/// Enable compile-time poison emission for the next `lower` pass.
pub fn lane_poison(on: bool) {
    LANE_POISON.with(|c| c.set(on));
}

fn lane_poison_on() -> bool {
    LANE_POISON.with(|c| c.get())
}

pub fn lower(programs: &[Program], db: &CheckDb, files: &[SourceFile]) -> IrModule {
    let mut alg = std::collections::HashSet::new();
    for (en, _) in &db.enums {
        if db.is_algebraic_enum(en) {
            alg.insert(en.clone());
        }
    }
    set_algebraic_enums(alg);
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
    // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the global pools'
    // runtime slots. Module-level statements were check-only before
    // this wave; the global-pool discipline makes the declared pool a
    // real runtime object: `@sn_g_{name}` holds the pool pointer,
    // constructed by main's entry (sn_pool_new_mode with the declared
    // mode). Sorted for deterministic emission; the naming follows the
    // lvalue_addr convention (@sn_g_{nm}).
    {
        let mut names: Vec<&String> = db.global_pools.keys().collect();
        names.sort();
        for name in names {
            globals.push(format!("@sn_g_{name} = internal global ptr null"));
        }
    }
    // SWARM RABBIT HOLE — Wave 13 (Impl-R): the seam cells' runtime
    // slots — the same @sn_g_{name} convention (W2B §4.1: the seam
    // cells ARE the module-scope Tier-L cells). Each module-scope
    // `ptr NAME = lcell_new(k)` becomes a real runtime object
    // constructed at main entry (the DIFF-4 global-pool pattern:
    // module initializers never run as code); the carrier literal k
    // is the compile-time fact the checker recorded at collect.
    {
        let mut names: Vec<&String> = db.lcell_globals.keys().collect();
        names.sort();
        for name in names {
            globals.push(format!("@sn_g_{name} = internal global ptr null"));
        }
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

/// Returns true if any subclass of `bp` overrides `method`.
/// If false, any call to `method` on an instance of `bp` or its subclasses
/// will execute the exact same implementation, allowing direct call devirtualization.
fn has_overriding_subclasses(db: &CheckDb, bp: &str, method: &str) -> bool {
    for (name, info) in &db.blueprints {
        if name != bp && blueprint_extends(&db.blueprints, name, bp) {
            if info.concrete_methods.iter().any(|m| m == method)
                || info.methods.iter().any(|m| m == method)
            {
                return true;
            }
        }
    }
    false
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

/// List element kind fed to sn_list_new / sn_list_new_lane.
/// SWARM RABBIT HOLE — Wave 12 (Impl-Q), the push_ptr pointee retention
/// fix (closing Impl-J's Wave-8 flag): 0 = i64 elements, 1 = OWNED
/// pointer elements (str/blueprint/closure/... handles — the runtime
/// ARC's them with sn_retain at push and sn_release at list death),
/// 2 = BORROW elements (ref<T>/mut<T>/raw ptr): the element is a slot
/// or lane address, not an object header — the runtime retains NOTHING
/// (the pointee of a scalar target is not an object, and a ref into an
/// object-typed binding is kept alive by the binding itself; the
/// checker's E-ESCAPE region poset owns the target's lifetime). The old
/// blind retain treated the slot as an Obj header and clobbered the
/// target local's value (list<ref<int>> of locals was broken). No
/// owned object type is assignable to ref/mut/ptr elements, so the
/// split is exhaustive.
fn list_elem_kind(elem: &Type) -> i64 {
    if matches!(elem, Type::Ref(_) | Type::Mut(_) | Type::Ptr) {
        2
    } else if elem.is_ptr() || matches!(elem, Type::Optional(_)) {
        1
    } else {
        0
    }
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

/// SWARM RABBIT HOLE — Wave 15 (Impl-T, Cohort Delta): does the
/// degraded atom reference `result`? (Gates the result-binding slot.)
fn expr_refs_result(e: &Expr) -> bool {
    expr_refs_name(e, "result")
}

/// Wave 15 (Impl-T): does the atom reference the named binding
/// (directly, or as the receiver of a .length() term)?
fn expr_refs_name(e: &Expr, name: &str) -> bool {
    match &e.kind {
        ExprKind::Ident(n) => n == name,
        ExprKind::Binary { lhs, rhs, .. } => {
            expr_refs_name(lhs, name) || expr_refs_name(rhs, name)
        }
        ExprKind::Unary { expr, .. } => expr_refs_name(expr, name),
        ExprKind::Call { callee, args, .. } => {
            expr_refs_name(callee, name)
                || args.iter().any(|a| match a {
                    Arg::Pos(x) | Arg::Inout(x) => expr_refs_name(x, name),
                    Arg::Named { value, .. } => expr_refs_name(value, name),
                })
        }
        ExprKind::Member { base, .. } => expr_refs_name(base, name),
        _ => false,
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
        format!("sn_m_{}_{}", b, f.name.replace('.', "_"))
    } else {
        format!("sn_fn_{}", f.name.replace('.', "_"))
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
    // SWARM RABBIT HOLE — Wave 15 (Impl-T): the summary key the
    // contract tables index (the pinned-lane feed's precedent).
    let fn_key = crate::check::fn_summary_key(bp.as_deref(), &f.name);
    let mut cx = Cx {
        db,
        buf: String::new(),
        allocas: Vec::new(),
        tmp: 0,
        lbl: 0,
        locals: HashMap::new(),
        concrete_types: HashMap::new(),
        loops: Vec::new(),
        loop_locals: std::collections::HashSet::new(),
        strings,
        extra_fns: extra,
        current_bp: bp.clone(),
        // Wave 8 (Impl-I): the pinned-lane feed indexes
        // db.lane_assignment by the checker's summary key.
        fn_key: fn_key.clone(),
        files,
        cur_loc: None,
        cur_sub: sub_id,
        dbg,
        terminated: false,
        current_fn_ret: ret_ty.clone(),
        defers: Vec::new(),
        current_nursery: None,
        active_locks: Vec::new(),
        active_nurseries: Vec::new(),
        active_regions: Vec::new(),
        frozen_depth: 0,
        wave_stack: Vec::new(),
        // SWARM RABBIT HOLE — Wave 15 (Impl-T, Cohort Delta): Salt
        // ε — this function's degraded contract atoms, cloned from
        // the checker's tables (keyed by the summary key — the
        // lane_assignment precedent). The entry checks are the
        // degraded requires atoms over defaulted-out params; the
        // exit checks are the degraded ensures atoms. The DBM engine
        // is compile-time only — the runtime sees just these.
        contract_entry: db
            .contract_entry_checks
            .get(&fn_key)
            .cloned()
            .unwrap_or_default(),
        contract_exit: db.contract_exit_checks.get(&fn_key).cloned().unwrap_or_default(),
    };
    let mut params_ll = Vec::new();
    if bp.is_some() && !f.is_static {
        params_ll.push("ptr %p_self".to_string());
    }
    for p in &f.params {
        let t = resolve_type_ast(&p.ty, db);
        let ll_t = if p.is_inout { "ptr" } else { llty(&t) };
        params_ll.push(format!("{ll_t} %p_{}", p.name));
    }
    let rty = llty_ret(&ret_ty);
    if bp.is_some() && !f.is_static {
        cx.emit_alloca("%v_self = alloca ptr");
        cx.line("store ptr %p_self, ptr %v_self");
        cx.locals
            .insert("self".into(), ("%v_self".into(), Type::Blueprint(bp.clone().unwrap())));
    }
    for p in &f.params {
        let t = resolve_type_ast(&p.ty, db);
        if p.is_inout {
            cx.locals.insert(p.name.clone(), (format!("%p_{}", p.name), t));
            continue;
        }
        let an = format!("%v_{}", p.name);
        if let Type::Record(rn) = &t {
            let sz = db.records.get(rn).map(|r| r.size).unwrap_or(8);
            cx.emit_alloca(&format!("{an} = alloca i8, i64 {sz}"));
            cx.line(&format!("call void @sn_record_copy(ptr {an}, ptr %p_{}, i64 {sz})", p.name));
        } else {
            cx.emit_alloca(&format!("{an} = alloca {}", llty(&t)));
            cx.line(&format!("store {} %p_{}, ptr {an}", llty(&t), p.name));
        }
        cx.locals.insert(p.name.clone(), (an, t));
    }
    // SWARM RABBIT HOLE — Wave 15 (Impl-T, Cohort Delta): Salt ε —
    // the FN-ENTRY degraded requires assertions (the boundary rule's
    // other site: atoms over defaulted-out params, whose values exist
    // only in the callee's prologue). The params are all bound here,
    // defaults resolved by the call. And the result binding slot for
    // the exit checks, when a degraded ensures atom references
    // 'result' (the validator guarantees the fn returns an int
    // there).
    if !cx.contract_entry.is_empty() {
        for ck in cx.contract_entry.clone() {
            let (v, _) = cx.expr(&ck.expr);
            cx.contract_assert(&v, &ck.message);
        }
    }
    if !cx.contract_exit.is_empty() && cx.contract_exit.iter().any(|ck| expr_refs_result(&ck.expr)) {
        let rt = cx.current_fn_ret.clone();
        cx.emit_alloca(&format!("%v_contract_result = alloca {}", llty(&rt)));
    }
    // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): main's entry is
    // where module-level GLOBAL POOLS are constructed. Module-level
    // statements were check-only before this wave (no initializer ever
    // ran); the global-pool discipline makes the pool a real runtime
    // object: each `@sn_g_{name}` slot (emitted into IrModule.globals
    // by the module loop) is filled here with sn_pool_new_mode — the
    // mode the declaration carried (frozen/thread_local/synchronized).
    // Ordering: before any body statement so a top-of-main `unfrozen`
    // window sees a constructed pool.
    if bp.is_none() && f.name == "main" {
        let mut names: Vec<(&String, &crate::check::GlobalPoolMode)> =
            db.global_pools.iter().map(|(n, m)| (n, m)).collect();
        names.sort_by(|a, b| a.0.cmp(b.0));
        for (name, mode) in names {
            let t = db.globals.get(name).cloned().unwrap_or(Type::Pool(Box::new(Type::Int)));
            let elem = match &t {
                Type::Pool(e) => e.clone(),
                _ => Box::new(Type::Int),
            };
            let kind = if elem.is_ptr() || matches!(*elem, Type::Optional(_)) { 1i64 } else { 0i64 };
            let size = elem.abi_size();
            let mode_int = match mode {
                crate::check::GlobalPoolMode::Frozen => 0,
                crate::check::GlobalPoolMode::ThreadConfined => 1,
                crate::check::GlobalPoolMode::Synchronized => 2,
            };
            let r = cx.t();
            cx.line(&format!(
                "{r} = call ptr @sn_pool_new_mode(i64 {kind}, i64 {size}, i64 {mode_int})"
            ));
            // DIFF-4 (Impl-O): a global pool is an immortal module-scoped
            // object — register it as a permanent GC root (the pools are
            // gc_track'd, but the only reference is this LLVM global,
            // invisible to the precise-root pushes; without registration
            // an unlucky gc_collect sweeps the pool mid-program).
            cx.line(&format!("call void @sn_gc_register_global(ptr {r})"));
            cx.line(&format!("store ptr {r}, ptr @sn_g_{name}"));
        }
        // SWARM RABBIT HOLE — Wave 13 (Impl-R): the seam cells'
        // construction — the module-scope Tier-L cells (W2B §4.1),
        // built right after the global pools (sorted, so the
        // emission is deterministic). Each is gc_track'd and
        // registered as a permanent root exactly like a global pool:
        // the only reference is the LLVM global slot, invisible to
        // the precise-root pushes, and a swept seam cell would be a
        // use-after-free the tier exists to make impossible.
        {
            let mut names: Vec<(&String, &i64)> =
                db.lcell_globals.iter().map(|(n, k)| (n, k)).collect();
            names.sort_by(|a, b| a.0.cmp(b.0));
            for (name, carrier) in names {
                let r = cx.t();
                cx.line(&format!(
                    "{r} = call ptr @sn_lcell_new(i64 {carrier})"
                ));
                cx.line(&format!("call void @sn_gc_register_global(ptr {r})"));
                cx.line(&format!("store ptr {r}, ptr @sn_g_{name}"));
            }
        }
    }
    for s in &f.body {
        cx.stmt(s);
    }
    cx.exit_active_scopes();
    // SWARM RABBIT HOLE — Wave 15 (Impl-T): the FALL-THROUGH exit's
    // degraded-ensures assertions — result bound to the default
    // value the implicit exit returns (0 for int fns; a void fn's
    // degraded atoms reference params only). Dead when the body
    // always returns (line() drops after a terminator); live exactly
    // where the IR's default ret is.
    if !cx.contract_exit.is_empty() && !cx.terminated {
        cx.contract_exit_check(None);
    }
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
    let mut out = format!(
        "define {rty} @{llvm_name}({}) !dbg !{sub_id} {{\nentry:\n",
        params_ll.join(", ")
    );
    for a in &cx.allocas {
        out.push_str(a);
        out.push('\n');
    }
    out.push_str(&cx.buf);
    out.push_str("}\n\n");
    out
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
            | Stmt::ErrDefer { span, .. }
            | Stmt::Guard { span, .. }
            | Stmt::Region { span, .. }
            | Stmt::NurseryBlock { span, .. }
            | Stmt::Frozen { span, .. }
            | Stmt::Unfrozen { span, .. }
            | Stmt::Wave { span, .. }
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
            | Stmt::ErrDefer { span, .. }
            | Stmt::Guard { span, .. }
            | Stmt::Region { span, .. }
            | Stmt::NurseryBlock { span, .. }
            | Stmt::Frozen { span, .. }
            | Stmt::Unfrozen { span, .. }
            | Stmt::Wave { span, .. }
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

    fn emit_alloca(&mut self, s: &str) {
        let mut line = format!("  {s}");
        if let Some((file, ln, col)) = self.cur_loc {
            let id = self.dbg.diloc(self.cur_sub, file, ln, col);
            line.push_str(&format!(", !dbg !{id}"));
        }
        self.allocas.push(line);
    }

    fn t(&mut self) -> String {
        self.tmp += 1;
        format!("%t{}", self.tmp)
    }
    fn l(&mut self, p: &str) -> String {
        self.lbl += 1;
        format!("{p}{}", self.lbl)
    }

    /// Address of a field inside the thread-local lane array
    /// (`@g_tls_lanes[lane].field`) for the inline bump/reset fast path
    /// (SWARM RABBIT HOLE, D0 gap #2, Impl-B; W2F §1(b): TLS-reloaded, no
    /// hidden-arg ABI). Field indices mirror runtime.c's SnLane via the
    /// %sn_lane_t type in llvm.rs: 1 = top, 3 = limit, 4 = initialized,
    /// 5 = frozen_depth (offsets pinned by _Static_asserts in runtime.c).
    fn lane_field_addr(&mut self, lane: &str, field: u32) -> String {
        let g = self.t();
        self.line(&format!(
            "{g} = getelementptr [8 x %sn_lane_t], ptr @g_tls_lanes, i64 0, i64 {lane}, i32 {field}"
        ));
        g
    }

    /// Poison-fill `[p, p + bytes)` with 0xAA for a compile-time-known
    /// positive size (SWARM RABBIT HOLE, Wave 6, Impl-D — T-FIBR, W4A
    /// repair 2: the debug poison-fill guarantee must not depend on which
    /// codegen path ran). `bytes` here is the *requested* size, matching
    /// runtime.c's `sn_lane_alloc` (padding to the 8-byte round-up is
    /// never handed out, so it is not filled).
    fn emit_poison_const(&mut self, p: &str, bytes: i64) {
        self.line(&format!(
            "call void @llvm.memset.p0.i64(ptr {p}, i8 171, i64 {bytes}, i1 false)"
        ));
    }

    /// Poison-fill `[p, p + bytes)` where `bytes` is a runtime i64 value:
    /// negative sizes fill nothing (the runtime call clamps them to 0; a
    /// raw memset would instead fill ~2^64 bytes).
    fn emit_poison_dynamic(&mut self, p: &str, bytes: &str) {
        let pos = self.t();
        self.line(&format!("{pos} = icmp sgt i64 {bytes}, 0"));
        let len = self.t();
        self.line(&format!("{len} = select i1 {pos}, i64 {bytes}, i64 0"));
        self.line(&format!(
            "call void @llvm.memset.p0.i64(ptr {p}, i8 171, i64 {len}, i1 false)"
        ));
    }

    /// Poison-fill `[from, to)` — the rewind re-poison shape of
    /// runtime.c's `sn_lane_restore` (T-FIBR "rewind re-poisons": a
    /// written byte is never trusted across a rewind). An empty or
    /// inverted range (to < from — e.g. a user `lane_restore` inside the
    /// region moved top below the watermark) fills nothing, mirroring
    /// `sn_lane_poison`'s hi<lo guard.
    fn emit_poison_range(&mut self, from: &str, to: &str) {
        let neg = self.t();
        self.line(&format!("{neg} = icmp ult ptr {to}, {from}"));
        let hi = self.t();
        self.line(&format!("{hi} = ptrtoint ptr {to} to i64"));
        let lo = self.t();
        self.line(&format!("{lo} = ptrtoint ptr {from} to i64"));
        let d = self.t();
        self.line(&format!("{d} = sub i64 {hi}, {lo}"));
        let len = self.t();
        self.line(&format!("{len} = select i1 {neg}, i64 0, i64 {d}"));
        self.line(&format!(
            "call void @llvm.memset.p0.i64(ptr {from}, i8 171, i64 {len}, i1 false)"
        ));
    }

    /// SWARM RABBIT HOLE — Wave 8 (Impl-I), DIFF-1 Rule 1b: the pinned
    /// lane the checker pre-colored this function's local onto
    /// (CheckDb.lane_assignment — an scc_accum accumulator's allocation
    /// site; W2A §1.3 S6: "allocation sites flowing into scc_accum_params
    /// positions are colored onto pinned π_j at compile time").
    fn pinned_lane_of(&self, name: &str) -> Option<u8> {
        self.db
            .lane_assignment
            .get(&self.fn_key)?
            .get(name)
            .copied()
    }

    /// The pinned-lane twin of the list-literal lowering (Impl-I, Wave 8,
    /// DIFF-1 Rule 1b / W2A §1.7 K1): the growable's header is
    /// constructed ON the pinned lane (runtime sn_list_new_lane →
    /// sn_lane_alloc(π_j)) instead of the malloc'd sn_list_new. K1 —
    /// nothing else opens on π_j — so no scratch scope exit can truncate
    /// the accumulator while its SCC is live: the E0802 buried-top
    /// hazard is unrepresentable. The element backing array stays
    /// malloc/slab-backed (W2A §1.5 E-NESTED: element storage is
    /// pool-backed by design).
    fn expr_list_on_pinned_lane(
        &mut self,
        els: &[Expr],
        declared: &Type,
        lane: u8,
    ) -> (String, Type) {
        let elem = match declared {
            Type::List(e) => (**e).clone(),
            _ if let Some(x) = els.first() => self.lookup_ty(x),
            _ => Type::Int,
        };
        let kind = list_elem_kind(&elem);
        let l = self.t();
        self.line(&format!(
            "{l} = call ptr @sn_list_new_lane(i64 {kind}, i64 {lane})"
        ));
        for el in els {
            let (v, vt) = self.expr(el);
            if kind != 0 {
                self.line(&format!("call void @sn_list_push_ptr(ptr {l}, ptr {v})"));
            } else {
                let vb = self.as_bits(v, &vt);
                self.line(&format!("call void @sn_list_push_i64(ptr {l}, i64 {vb})"));
            }
        }
        (l, Type::List(Box::new(elem)))
    }

    fn stmt(&mut self, s: &Stmt) {
        self.cur_loc = self.stmt_loc(s);
        self.cov_stmt(s);
        match s {
            Stmt::Expr(e) => {
                self.expr(e);
            }
            Stmt::Decl {
                names, value, ty, mode, ..
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
                                self.emit_alloca(&format!("{an} = alloca i8, i64 {sz}"));
                            }
                            _ => self.emit_alloca(&format!("{an} = alloca {}", llty(&t))),
                        }
                        self.locals.insert(name.clone(), (an.clone(), t.clone()));
                        if !self.loops.is_empty() {
                            self.loop_locals.insert(name.clone());
                        }
                    }
                    let an = self.locals.get(name).unwrap().0.clone();
                    if let Some(v) = value {
                        if let ExprKind::Ident(rhs_name) = &v.kind {
                            if let Some(concrete) = self.concrete_types.get(rhs_name).cloned() {
                                self.concrete_types.insert(name.clone(), concrete);
                            }
                        }
                        // SWARM RABBIT HOLE — Wave 10 / DIFF-4
                        // (Impl-O): a LOCAL pool carrying a mode
                        // annotation (`pool<T> thread_local` /
                        // `synchronized`; the checker guarantees the
                        // initializer is `pool_new`) constructs with
                        // sn_pool_new_mode so the runtime object
                        // carries the mode (home-thread tag / vault
                        // lock). Bare pool_new keeps the frozen
                        // default (sn_pool_new).
                        let annotated_pool_init = matches!(t, Type::Pool(_))
                            && *mode != crate::ast::PoolMode::Frozen
                            && matches!(&v.kind, ExprKind::Call { callee, .. }
                                if matches!(&callee.kind, ExprKind::Ident(f) if f == "pool_new"));
                        if annotated_pool_init {
                            let elem = match &t {
                                Type::Pool(e) => e.clone(),
                                _ => Box::new(Type::Int),
                            };
                            let kind = if elem.is_ptr() || matches!(*elem, Type::Optional(_)) { 1i64 } else { 0i64 };
                            let size = elem.abi_size();
                            let mode_int = match mode {
                                crate::ast::PoolMode::ThreadConfined => 1,
                                crate::ast::PoolMode::Synchronized => 2,
                                crate::ast::PoolMode::Frozen => 0,
                            };
                            let r = self.t();
                            self.line(&format!(
                                "{r} = call ptr @sn_pool_new_mode(i64 {kind}, i64 {size}, i64 {mode_int})"
                            ));
                            self.line(&format!("store ptr {r}, ptr {an}"));
                        } else {
                        // SWARM RABBIT HOLE — Wave 8 (Impl-I), DIFF-1
                        // Rule 1b / W2A §1.3: the pinned-lane feed. When
                        // the checker pre-colored this local onto a
                        // pinned lane L2..L6 (an scc_accum accumulator's
                        // allocation site — CheckDb.lane_assignment),
                        // the list-literal construction routes onto the
                        // pinned lane; every other value keeps the
                        // existing lowering.
                        let (vr, vt) = match (self.pinned_lane_of(name), &v.kind) {
                            (Some(lane), ExprKind::List(els)) => {
                                self.expr_list_on_pinned_lane(els, &t, lane)
                            }
                            _ => self.expr(v),
                        };
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
                        } // (the DIFF-4 annotated-pool init's else arm)
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
                            let an = format!("%v_{}_{}", n, self.l("lv"));
                            let et = ts[i].clone();
                            self.emit_alloca(&format!("{an} = alloca {}", llty(&et)));
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
                                let (an, lt) = match self.locals.get(n).cloned() {
                                    Some(pair) => pair,
                                    None => {
                                        let an = format!("%v_{}_{}", n, self.l("lv"));
                                        let et = ts[i].clone();
                                        self.emit_alloca(&format!("{an} = alloca {}", llty(&et)));
                                        self.locals.insert(n.clone(), (an.clone(), et.clone()));
                                        (an, et)
                                    }
                                };
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
                        if *op == AssignOp::Eq {
                            if let ExprKind::Ident(rhs_n) = &value.kind {
                                if let Some(concrete) = self.concrete_types.get(rhs_n).cloned() {
                                    self.concrete_types.insert(n.clone(), concrete);
                                } else {
                                    self.concrete_types.remove(n);
                                }
                            } else {
                                self.concrete_types.remove(n);
                            }
                        }
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
                                        let (ft, signed) = ll_field_ty(&f.ty);
                                        let src = if *op == AssignOp::Eq {
                                            let (vr, _) = self.coerce(vr, &vt, &f.ty);
                                            vr
                                        } else {
                                            let cur = if ft == "i64" {
                                                let v = self.t();
                                                self.line(&format!("{v} = load i64, ptr {fp}"));
                                                v
                                            } else if ft == "double" {
                                                let v = self.t();
                                                self.line(&format!("{v} = load double, ptr {fp}"));
                                                v
                                            } else {
                                                let raw = self.t();
                                                self.line(&format!("{raw} = load {ft}, ptr {fp}"));
                                                let v = self.t();
                                                if signed {
                                                    self.line(&format!("{v} = sext {ft} {raw} to i64"));
                                                } else {
                                                    self.line(&format!("{v} = zext {ft} {raw} to i64"));
                                                }
                                                v
                                            };
                                            self.arith_op(assign_bin(*op), cur, vr, &f.ty)
                                        };
                                        if ft == "i64" {
                                            self.line(&format!("store i64 {src}, ptr {fp}"));
                                        } else if ft == "double" {
                                            self.line(&format!("store double {src}, ptr {fp}"));
                                        } else {
                                            let tr = self.t();
                                            self.line(&format!("{tr} = trunc i64 {src} to {ft}"));
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
                                    let src = if *op == AssignOp::Eq {
                                        let (vr, _) = self.coerce(vr, &vt, &f.ty);
                                        vr
                                    } else {
                                        let cur = self.t();
                                        self.line(&format!("{cur} = load {}, ptr {fp}", llty(&f.ty)));
                                        self.arith_op(assign_bin(*op), cur, vr, &f.ty)
                                    };
                                    self.line(&format!("store {} {src}, ptr {fp}", llty(&f.ty)));
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
                self.loops.push((h.clone(), e.clone(), self.active_locks.len()));
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
                self.loops.push((st.clone(), e.clone(), self.active_locks.len()));
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
                self.emit_alloca(&format!("{ixn} = alloca i64"));
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
                self.emit_alloca(&format!("{an} = alloca {}", llty(&elem_ty)));
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
                self.loops.push((st.clone(), e.clone(), self.active_locks.len()));
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
                if let Type::Enum(en) = &vt {
                    if self.db.is_algebraic_enum(en) {
                        let tag_ptr = self.t();
                        let tag_val = self.t();
                        self.line(&format!("{tag_ptr} = getelementptr inbounds i8, ptr {v}, i64 16"));
                        self.line(&format!("{tag_val} = load i64, ptr {tag_ptr}"));
                        let end = self.l("mend");
                        for (pat, body) in arms {
                            let yes = self.l("my");
                            let no = self.l("mn");
                            let mut matched_idx = None;
                            let mut bindings: Vec<(String, Type, i64)> = Vec::new();
                            match &pat.kind {
                                ExprKind::Call { callee, args, .. } => {
                                    if let ExprKind::Member { base, name } = &callee.kind {
                                        if let ExprKind::Ident(pen) = &base.kind {
                                            if pen == en {
                                                if let Some(vars) = self.db.enum_variants.get(en) {
                                                    if let Some((idx, var)) = vars.iter().enumerate().find(|(_, v)| &v.name == name) {
                                                        matched_idx = Some(idx);
                                                        for (i, a) in args.iter().enumerate() {
                                                            let arg_e = match a {
                                                                Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => e,
                                                            };
                                                            if let ExprKind::Ident(var_name) = &arg_e.kind {
                                                                if var_name != "_" {
                                                                    bindings.push((var_name.clone(), var.fields[i].1.clone(), 24 + 8 * i as i64));
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                ExprKind::Member { base, name } => {
                                    if let ExprKind::Ident(pen) = &base.kind {
                                        if pen == en {
                                            if let Some(vars) = self.db.enums.get(en) {
                                                if let Some(idx) = vars.iter().position(|v| v == name) {
                                                    matched_idx = Some(idx);
                                                }
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }

                            if let Some(target_idx) = matched_idx {
                                let c = self.t();
                                self.line(&format!("{c} = icmp eq i64 {tag_val}, {target_idx}"));
                                self.line(&format!("br i1 {c}, label %{yes}, label %{no}"));
                                self.raw(&format!("{yes}:"));
                                let saved_locals = self.locals.clone();
                                for (b_name, b_ty, b_offset) in bindings {
                                    let fp = self.t();
                                    let fval = self.t();
                                    self.line(&format!("{fp} = getelementptr inbounds i8, ptr {v}, i64 {b_offset}"));
                                    self.line(&format!("{fval} = load {}, ptr {fp}", llty(&b_ty)));
                                    let an = format!("%v_{}_{}", b_name, self.l("lv"));
                                    self.emit_alloca(&format!("{an} = alloca {}", llty(&b_ty)));
                                    self.line(&format!("store {} {fval}, ptr {an}", llty(&b_ty)));
                                    self.locals.insert(b_name, (an, b_ty));
                                }
                                for s in body {
                                    self.stmt(s);
                                }
                                self.locals = saved_locals;
                                self.line(&format!("br label %{end}"));
                                self.raw(&format!("{no}:"));
                            }
                        }
                        if let Some(b) = default {
                            for s in b {
                                self.stmt(s);
                            }
                        }
                        self.line(&format!("br label %{end}"));
                        self.raw(&format!("{end}:"));
                        return;
                    }
                }
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
                    // SWARM RABBIT HOLE — Wave 15 (Impl-T): the fn-exit
                    // degraded-ensures assertions — evaluated at the
                    // return point, result bound to the (coerced)
                    // return value, BEFORE the deferred statements
                    // run (the ensures holds of the returned state).
                    if !self.contract_exit.is_empty() {
                        let rt = self.current_fn_ret.clone();
                        let (rc, _) = self.coerce(r.clone(), &t, &rt);
                        self.contract_exit_check(Some((rc, rt)));
                    }
                    self.exit_active_scopes();
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
                    // SWARM RABBIT HOLE — Wave 15 (Impl-T): a bare
                    // return in a void fn — the degraded ensures
                    // atoms reference params only (the validator
                    // rejects 'result' without an int return).
                    if !self.contract_exit.is_empty() {
                        self.contract_exit_check(None);
                    }
                    self.exit_active_scopes();
                    self.line("ret void");
                }
            }
            Stmt::Stop(_) => {
                if let Some((_, brk, lock_depth)) = self.loops.last().cloned() {
                    self.release_locks_from(lock_depth);
                    self.line(&format!("br label %{brk}"));
                    let dead = self.l("dead");
                    self.raw(&format!("{dead}:"));
                }
            }
            Stmt::Skip(_) => {
                if let Some((cont, _, lock_depth)) = self.loops.last().cloned() {
                    self.release_locks_from(lock_depth);
                    self.line(&format!("br label %{cont}"));
                    let dead = self.l("dead");
                    self.raw(&format!("{dead}:"));
                }
            }
            Stmt::SpawnBlock { body, .. } => self.spawn_block(body, false),
            Stmt::GoroutineBlock { body, .. } => self.spawn_block(body, true),
            Stmt::NurseryBlock { body, .. } => {
                let nursery = self.t();
                self.line(&format!("{nursery} = call ptr @sn_nursery_new()"));
                self.active_nurseries.push(nursery.clone());
                let prev_nursery = self.current_nursery.replace(nursery.clone());
                for s in body {
                    self.stmt(s);
                }
                if !self.terminated {
                    self.line(&format!("call void @sn_nursery_wait(ptr {nursery})"));
                }
                self.active_nurseries.pop();
                self.current_nursery = prev_nursery;
            }
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
                self.active_locks.push(lk.clone());
                for s in body {
                    self.stmt(s);
                }
                self.active_locks.pop();
                if !self.terminated {
                    self.line(&format!("call void @sn_lock_rel(ptr {lk})"));
                }
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
                        Arg::Pos(value) | Arg::Inout(value) => {
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
                self.emit_alloca(&format!("{an} = alloca ptr"));
                self.line(&format!("store ptr {obj}, ptr {an}"));
                self.locals
                    .insert(name.clone(), (an, Type::Blueprint(resolved.clone())));
                self.concrete_types.insert(name.clone(), resolved.clone());
                if !self.loops.is_empty() {
                    // Its alloca lives in the loop body and does not dominate
                    // anything after the loop.
                    self.loop_locals.insert(name.clone());
                }
                if let Some(sig) = self.db.funcs.get(&format!("{resolved}::create")) {
                    if !sig.is_static {
                        self.line(&format!("call void @sn_m_{resolved}_create(ptr {obj})"));
                    }
                }
            }
            Stmt::Defer { body, .. } => {
                self.defers.push(body.clone());
            }
            Stmt::ErrDefer { body, .. } => {
                self.defers.push(body.clone());
            }
            Stmt::Region { body, .. } => {
                // SWARM RABBIT HOLE — D0 gap #2/#3 (Impl-B, Wave 5): the
                // region pair becomes two memory ops on the fast path
                // (W2F §2: load `top` at entry, one store at exit) instead
                // of two checked runtime calls. Lane 0 is the only lane the
                // region lowering uses (D0: ir.rs region arm). The 1-store
                // restore skips @sn_lane_restore's frozen/bounds checks —
                // licensed by Theorem A / W2A §4.3(iii): the watermark is
                // this region's own entry save and no frozen block is
                // lexically open at the exit. Debug builds (-O0) keep the
                // runtime calls.
                let wm = if lane_inline_on() {
                    let top_addr = self.lane_field_addr("0", 1);
                    // The old @sn_lane_save call lazily initialized the
                    // lane; the inline load needs that too — one predicted
                    // branch at region entry, with the initialization left
                    // to the old call on the cold path.
                    let init_addr = self.lane_field_addr("0", 4);
                    let init = self.t();
                    self.line(&format!("{init} = load i32, ptr {init_addr}"));
                    let uninit = self.t();
                    self.line(&format!("{uninit} = icmp eq i32 {init}, 0"));
                    let l_init = self.l("laneinit");
                    let l_go = self.l("lanego");
                    self.line(&format!("br i1 {uninit}, label %{l_init}, label %{l_go}"));
                    self.raw(&format!("{l_init}:"));
                    let cold = self.t();
                    self.line(&format!("{cold} = call ptr @sn_lane_save(i64 0)"));
                    self.line(&format!("br label %{l_go}"));
                    self.raw(&format!("{l_go}:"));
                    let wm = self.t();
                    self.line(&format!("{wm} = load ptr, ptr {top_addr}"));
                    wm
                } else {
                    let wm = self.t();
                    self.line(&format!("{wm} = call ptr @sn_lane_save(i64 0)"));
                    wm
                };
                self.active_regions.push(wm.clone());
                for s in body {
                    self.stmt(s);
                }
                if !self.terminated {
                    // Wave 6 (Impl-D, T-FIBR "rewind re-poisons"): in poison
                    // emission mode the freed range [watermark, old_top) is
                    // re-filled with 0xAA at the rewind, on BOTH lowerings —
                    // the inline 1-store restore used to skip it entirely
                    // (Impl-B's documented residual), and the runtime call
                    // path only re-poisons under SN_POISON.
                    if lane_inline_on() && self.frozen_depth == 0 {
                        // 1-store reset (W2F §2: "1 store per lane").
                        let top_addr = self.lane_field_addr("0", 1);
                        if lane_poison_on() {
                            let old_top = self.t();
                            self.line(&format!("{old_top} = load ptr, ptr {top_addr}"));
                            self.emit_poison_range(&wm, &old_top);
                        }
                        self.line(&format!("store ptr {wm}, ptr {top_addr}"));
                    } else {
                        if lane_poison_on() {
                            let top_addr = self.lane_field_addr("0", 1);
                            let old_top = self.t();
                            self.line(&format!("{old_top} = load ptr, ptr {top_addr}"));
                            self.line(&format!("call void @sn_lane_restore(i64 0, ptr {wm})"));
                            self.emit_poison_range(&wm, &old_top);
                        } else {
                            self.line(&format!("call void @sn_lane_restore(i64 0, ptr {wm})"));
                        }
                    }
                }
                self.active_regions.pop();
            }
            Stmt::Frozen { body, .. } => {
                self.line("call void @sn_lane_freeze(i64 0)");
                // SWARM RABBIT HOLE — D0 gap #2 (Impl-B, Wave 5): track the
                // lexically open frozen depth so the inline bump/reset
                // lowering keeps the checked runtime calls wherever a
                // frozen block is open, preserving the loud runtime errors
                // for alloc/rewind on a frozen lane.
                self.frozen_depth += 1;
                for s in body {
                    self.stmt(s);
                }
                self.frozen_depth -= 1;
                if !self.terminated {
                    self.line("call void @sn_lane_unfreeze(i64 0)");
                }
            }
            // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the
            // `unfrozen (P) { ... }` window's lowering — Stmt::Frozen's
            // shape (ir.rs:1618) with the POOL's runtime freeze
            // discipline instead of the lane's: enter =
            // sn_pool_unfreeze(P) (the additive frozen_depth++ on
            // SnPool; debug-mode enforcement of the window), body,
            // exit = sn_pool_freeze(P). `self.terminated` handles the
            // early-return unwind (the Frozen lowering's guarantee);
            // panic unwinds re-freeze in the runtime's fatal path — a
            // panicked program never resumes, so the only escape the
            // window must close is the structured one. The pool
            // pointer is LOADED from its slot (the alloca for a local
            // pool, @sn_g_{n} for a global one — lvalue_addr resolves
            // both, and each is a ptr-to-slot).
            Stmt::Unfrozen { pool, body, .. } => {
                let (pa, _) = self.lvalue_addr(&Expr {
                    kind: ExprKind::Ident(pool.clone()),
                    span: Span::dummy(),
                });
                let pv = self.t();
                self.line(&format!("{pv} = load ptr, ptr {pa}"));
                self.line(&format!("call void @sn_pool_unfreeze(ptr {pv})"));
                for s in body {
                    self.stmt(s);
                }
                if !self.terminated {
                    self.line(&format!("call void @sn_pool_freeze(ptr {pv})"));
                }
            }
            // SWARM RABBIT HOLE — Wave 13 (Impl-R, Cohort Delta): the
            // WAVE REGION's lowering (W2B §4.2's event structure,
            // literalized):
            //   (e1) wave entry: allocate the SnWave context and
            //        CAPTURE the seam cells — one sn_wave_capture per
            //        cell the checker registered (CheckDb.
            //        wave_captures, sorted names), each cell loaded
            //        from its module slot @sn_g_{name} (the DIFF-4
            //        global-pool convention; lvalue_addr resolves the
            //        global). The capture registers pointer identity —
            //        the runtime's slot table is the read_W/contrib
            //        lookup, so aliases resolve to the same slot.
            //   (e2) the body: read_W/contrib lower against the
            //        innermost open wave (wave_stack.last()) — the
            //        checker guarantees exactly one (nested waves are
            //        E0812).
            //   (e3) wave exit = J(W): sn_wave_join — the J2 batched
            //        canonical fold (one atomic ⊔ per seam cell; the
            //        join frees the context). The checker rejects
            //        early returns in the body (the exit edge
            //        contrib→exit→J(W) is structural), so the join is
            //        reached on every path that leaves the block.
            Stmt::Wave { body, span } => {
                let key = (span.file, span.start, span.end);
                let captures = self
                    .db
                    .wave_captures
                    .get(&key)
                    .cloned()
                    .unwrap_or_default();
                let w = self.t();
                self.line(&format!(
                    "{w} = call ptr @sn_wave_new(i64 {})",
                    captures.len()
                ));
                for (i, name) in captures.iter().enumerate() {
                    let (ca, _) = self.lvalue_addr(&Expr {
                        kind: ExprKind::Ident(name.clone()),
                        span: Span::dummy(),
                    });
                    let cv = self.t();
                    self.line(&format!("{cv} = load ptr, ptr {ca}"));
                    self.line(&format!(
                        "call void @sn_wave_capture(ptr {w}, i64 {i}, ptr {cv})"
                    ));
                }
                self.wave_stack.push(w.clone());
                for s in body {
                    self.stmt(s);
                }
                self.wave_stack.pop();
                if !self.terminated {
                    self.line(&format!("call void @sn_wave_join(ptr {w})"));
                }
            }
            Stmt::Guard { cond, else_body, .. } => {
                let (c, _) = self.expr(cond);
                let c1 = self.as_i1(c);
                let l_ok = self.l("guard_ok");
                let l_else = self.l("guard_else");
                self.line(&format!("br i1 {c1}, label %{l_ok}, label %{l_else}"));
                self.raw(&format!("{l_else}:"));
                for s in else_body {
                    self.stmt(s);
                }
                self.raw(&format!("{l_ok}:"));
            }
            Stmt::Asm { template, clobbers, .. } => {
                self.emit_asm(template, clobbers);
            }
            Stmt::NestedFn(f) => {
                let ret = f.ret.as_ref().map(ast_to_type).unwrap_or(Type::Void);
                let clos = self.lower_closure(&f.params, &ret, &f.body);
                let an = format!("%v_{}_{}", f.name, self.l("lv"));
                self.emit_alloca(&format!("{an} = alloca ptr"));
                self.line(&format!("store ptr {clos}, ptr {an}"));
                let ty = Type::Fn {
                    params: f.params.iter().map(|p| resolve_type_ast(&p.ty, self.db)).collect(),
                    ret: Box::new(ret),
                };
                self.locals.insert(f.name.clone(), (an, ty));
            }
        }
    }

    fn release_locks_from(&mut self, depth: usize) {
        if depth >= self.active_locks.len() {
            return;
        }
        let locks: Vec<String> = self.active_locks[depth..].to_vec();
        for lk in locks.iter().rev() {
            self.line(&format!("call void @sn_lock_rel(ptr {lk})"));
        }
    }

    fn release_active_locks(&mut self) {
        self.release_locks_from(0);
    }

    fn exit_active_scopes(&mut self) {
        self.emit_defers();
        self.release_active_locks();
        let regions = self.active_regions.clone();
        for wm in regions.iter().rev() {
            self.line(&format!("call void @sn_lane_restore(i64 0, ptr {wm})"));
        }
        let nurseries = self.active_nurseries.clone();
        for n in nurseries.iter().rev() {
            self.line(&format!("call void @sn_nursery_wait(ptr {n})"));
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

    fn spawn_block(&mut self, body: &[Stmt], _green: bool) {
        let active_nursery = self.current_nursery.clone();
        if let Some(ref nursery) = active_nursery {
            self.line(&format!("call void @sn_nursery_enter_task(ptr {nursery})"));
        }
        let mut caps: Vec<String> = Vec::new();
        collect_idents(body, &mut caps);
        caps.retain(|n| self.locals.contains_key(n));
        caps.sort();
        caps.dedup();
        let id = self.l("sp");
        let fname = format!("sn_spawn_{id}");
        let env = self.t();
        let nursery_extra = if active_nursery.is_some() { 1 } else { 0 };
        let nbytes = ((caps.len() + nursery_extra) * 8) as i64;
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
        if let Some(ref nursery) = active_nursery {
            let slot = self.t();
            self.line(&format!(
                "{slot} = getelementptr i8, ptr {env}, i64 {}",
                caps.len() * 8
            ));
            self.line(&format!("store ptr {nursery}, ptr {slot}"));
        }
        self.line(&format!("call void @sn_go_spawn(ptr @{fname}, ptr {env})"));
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
                allocas: Vec::new(),
                tmp: 0,
                lbl: 0,
                locals: HashMap::new(),
                concrete_types: HashMap::new(),
                loops: Vec::new(),
                loop_locals: std::collections::HashSet::new(),
                strings: self.strings,
                extra_fns: &mut nested,
                current_bp: self.current_bp.clone(),
                fn_key: String::new(),
                files: self.files,
                cur_loc: None,
                cur_sub: child_sub,
                dbg: self.dbg,
                terminated: false,
                current_fn_ret: Type::Void,
                defers: Vec::new(),
                current_nursery: None,
                active_locks: Vec::new(),
                active_nurseries: Vec::new(),
                active_regions: Vec::new(),
                frozen_depth: 0,
                wave_stack: Vec::new(),
                // Wave 15 (Impl-T): synthetic child context — no
                // fn-level contract lowering (a closure/spawn body is
                // not the contracted function).
                contract_entry: Vec::new(),
                contract_exit: Vec::new(),
            };
            for (i, (n, t)) in cap_tys.iter().enumerate() {
                let slot = child.t();
                child.line(&format!(
                    "{slot} = getelementptr i8, ptr %env, i64 {}",
                    i * 8
                ));
                let an = format!("%v_{n}");
                child.emit_alloca(&format!("{an} = alloca {}", llty(t)));
                let v = child.t();
                child.line(&format!("{v} = load {}, ptr {slot}", llty(t)));
                child.line(&format!("store {} {v}, ptr {an}", llty(t)));
                child.locals.insert(n.clone(), (an, t.clone()));
            }
            let child_nursery = if active_nursery.is_some() {
                let slot = child.t();
                child.line(&format!(
                    "{slot} = getelementptr i8, ptr %env, i64 {}",
                    caps.len() * 8
                ));
                let cn = child.t();
                child.line(&format!("{cn} = load ptr, ptr {slot}"));
                child.current_nursery = Some(cn.clone());
                Some(cn)
            } else {
                None
            };
            for s in body {
                child.stmt(s);
            }
            child.emit_defers();
            child.release_active_locks();
            child.release_active_locks();
            if !child.terminated {
                if let Some(ref cn) = child_nursery {
                    child.line(&format!("call void @sn_nursery_leave_task(ptr {cn})"));
                }
                child.line("ret void");
            }
            let mut out = format!("define void @{fname}(ptr %env) !dbg !{child_sub} {{\nentry:\n");
            for a in &child.allocas {
                out.push_str(a);
                out.push('\n');
            }
            out.push_str(&child.buf);
            out.push_str("}\n\n");
            out
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
                } else if let (Some(t), true) = (
                    self.db.globals.get(n).cloned(),
                    // SWARM RABBIT HOLE — Wave 13 (Impl-R): the global
                    // POOL objects and the SEAM CELLS (module-scope
                    // Tier-L lcells) are the real runtime globals —
                    // each @sn_g_{n} slot (constructed at main's
                    // entry) holds the object pointer; every use
                    // loads it. Module-level reads of anything else
                    // remain check-only (the pre-DIFF-4 behavior).
                    self.db.global_pools.contains_key(n)
                        || self.db.lcell_globals.contains_key(n),
                ) {
                    let v = self.t();
                    self.line(&format!("{v} = load ptr, ptr @sn_g_{n}"));
                    (v, t)
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
                let kind = list_elem_kind(&et);
                let l = self.t();
                self.line(&format!("{l} = call ptr @sn_list_new(i64 {kind})"));
                for el in els {
                    let (v, vt) = self.expr(el);
                    if kind != 0 {
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
                    let want = self.lookup_ty(e);
                    match want {
                        Type::Map(k, v) => (*k, *v),
                        _ => (Type::Str, Type::Int),
                    }
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
                        Arg::Pos(e) | Arg::Inout(e) => {
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
                    let (l, _) = self.coerce(l, &lt, &Type::Float);
                    let (r, _) = self.coerce(r, &rt2, &Type::Float);
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
                if let Type::Enum(en) = &lt {
                    if self.db.is_algebraic_enum(en) && matches!(op, BinOp::Eq | BinOp::Ne) {
                        let eq = self.t();
                        self.line(&format!("{eq} = call i64 @sn_enum_eq(ptr {l}, ptr {r})"));
                        if op == BinOp::Ne {
                            let x = self.t();
                            self.line(&format!("{x} = xor i64 {eq}, 1"));
                            return (x, Type::Bool);
                        }
                        return (eq, Type::Bool);
                    }
                }
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
                    Type::Map(_, _) => {
                        let kb = self.as_bits(l, &lt);
                        self.line(&format!("{d} = call i64 @sn_map_has(ptr {r}, i64 {kb})"));
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
                let (r, rt) = self.expr(rhs);
                let t = if lt == Type::Float || rt == Type::Float {
                    Type::Float
                } else if matches!(want, Type::Dec(_)) {
                    want.clone()
                } else if want.int_min_max().is_some() || *want == Type::Int {
                    want.clone()
                } else {
                    lt.clone()
                };
                let (l, _) = if t == Type::Float { self.coerce(l, &lt, &t) } else { (l, lt) };
                let (r, _) = if t == Type::Float { self.coerce(r, &rt, &t) } else { (r, rt) };
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
        if let Type::Dec(scale) = t {
            let mut scale_factor = 1i64;
            for _ in 0..*scale {
                scale_factor *= 10;
            }
            let d = self.t();
            match op {
                BinOp::Add => self.line(&format!("{d} = add i64 {l}, {r}")),
                BinOp::Sub => self.line(&format!("{d} = sub i64 {l}, {r}")),
                BinOp::Mul => {
                    let prod = self.t();
                    self.line(&format!("{prod} = mul i64 {l}, {r}"));
                    self.line(&format!("{d} = call i64 @sn_div_i64(i64 {prod}, i64 {scale_factor})"));
                }
                BinOp::Div => {
                    let scaled = self.t();
                    self.line(&format!("{scaled} = mul i64 {l}, {scale_factor}"));
                    self.line(&format!("{d} = call i64 @sn_div_i64(i64 {scaled}, i64 {r})"));
                }
                BinOp::Mod => self.line(&format!("{d} = call i64 @sn_mod_i64(i64 {l}, i64 {r})")),
                BinOp::Pow => self.line(&format!("{d} = call i64 @sn_pow_i64(i64 {l}, i64 {r})")),
                _ => self.line(&format!("{d} = add i64 {l}, {r}")),
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

    fn call(&mut self, callee: &Expr, type_args: &[TypeAst], args: &[Arg], span: Span) -> (String, Type) {
        if let ExprKind::Member { base, name } = &callee.kind {
            if let ExprKind::Ident(en) = &base.kind {
                if self.db.is_algebraic_enum(en) {
                    if let Some(vars) = self.db.enum_variants.get(en) {
                        if let Some((idx, var)) = vars.iter().enumerate().find(|(_, v)| &v.name == name) {
                            let nfields = args.len();
                            let sz = 24 + 8 * nfields as i64;
                            let obj = self.t();
                            self.line(&format!("{obj} = call ptr @sn_obj_new(i64 {sz}, i64 0, ptr inttoptr (i64 {idx} to ptr))"));
                            for (i, a) in args.iter().enumerate() {
                                let arg_expr = match a {
                                    Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => e,
                                };
                                let (ar, at) = self.expr(arg_expr);
                                let expected_ty = &var.fields[i].1;
                                let (ar, _) = self.coerce(ar, &at, expected_ty);
                                let fp = self.t();
                                let offset = 24 + 8 * i as i64;
                                self.line(&format!("{fp} = getelementptr inbounds i8, ptr {obj}, i64 {offset}"));
                                self.line(&format!("store {} {ar}, ptr {fp}", llty(expected_ty)));
                            }
                            return (obj, Type::Enum(en.clone()));
                        }
                    }
                }
            }
            return self.method(base, name, args, span);
        }
        if let ExprKind::Ident(n) = &callee.kind {
            return self.ident_call(n, type_args, args, span, callee.span);
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
            | Type::Ref(_) | Type::Chan(_) | Type::Json | Type::Named(_)
            // Wave 14 (Impl-S): the SFC layouts are pointer-like
            // values (the lane-backed header pointer) — pass through.
            | Type::Morton(_, _) | Type::Hilbert(_, _) => v,
            Type::Enum(en) if self.db.is_algebraic_enum(&en) => v,
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

    fn lvalue_addr(&mut self, e: &Expr) -> (String, Type) {
        match &e.kind {
            ExprKind::Ident(nm) => {
                if let Some((an, ty)) = self.locals.get(nm).cloned() {
                    return (an, ty);
                }
                if let Some(gt) = self.db.globals.get(nm).cloned() {
                    return (format!("@sn_g_{nm}"), gt);
                }
            }
            ExprKind::Index { base, index } => {
                let (b, bt) = self.expr(base);
                let (i, _) = self.expr(index);
                if let Type::List(elem) = bt {
                    let r = self.t();
                    self.line(&format!("{r} = call ptr @sn_list_elem_addr(ptr {b}, i64 {i})"));
                    return (r, *elem);
                }
            }
            ExprKind::Member { base, name } => {
                let (b, bt) = self.expr(base);
                match bt {
                    Type::Record(rn) => {
                        if let Some(info) = self.db.records.get(&rn) {
                            if let Some(f) = info.fields.iter().find(|f| f.name == *name) {
                                let fp = self.t();
                                self.line(&format!("{fp} = getelementptr i8, ptr {b}, i64 {}", f.offset));
                                return (fp, f.ty.clone());
                            }
                        }
                    }
                    Type::Blueprint(bp) => {
                        if let Some(info) = self.db.blueprints.get(&bp) {
                            if let Some(f) = info.fields.iter().find(|f| f.name == *name) {
                                let fp = self.t();
                                self.line(&format!("{fp} = getelementptr i8, ptr {b}, i64 {}", f.offset));
                                return (fp, f.ty.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        let (v, t) = self.expr(e);
        (v, t)
    }

    fn eval_args_for_sig(&mut self, sig: Option<&FuncSig>, args: &[Arg]) -> Vec<(String, Type)> {
        let is_method = sig
            .map(|s| s.is_method && !s.is_static && s.params.first().map(|p| p.0 == "self").unwrap_or(false))
            .unwrap_or(false);
        let offset = if is_method { 1 } else { 0 };
        let mut vs = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let is_inout_param = sig
                .and_then(|s| s.inout_params.get(i + offset))
                .copied()
                .unwrap_or(false);
            let is_ref_param = sig
                .and_then(|s| s.params.get(i + offset))
                .map(|p| matches!(p.1, Type::Ref(_) | Type::Mut(_)))
                .unwrap_or(false);
            match a {
                Arg::Inout(e) => {
                    vs.push(self.lvalue_addr(e));
                }
                Arg::Pos(e) | Arg::Named { value: e, .. } => {
                    // ARC Borrow Elision:
                    // If the parameter is passed by ref<T> or inout, or if the argument value
                    // is guaranteed live by the caller stack frame, skip emitting redundant
                    // sn_retain/sn_release pairs.
                    let _borrow_elided = is_ref_param || is_inout_param || self.is_caller_stack_live(e);
                    if is_inout_param {
                        vs.push(self.lvalue_addr(e));
                    } else {
                        vs.push(self.expr(e));
                    }
                }
            }
        }
        vs
    }

    /// Call a user-declared function, materialising default arguments and
    /// coercing each parameter to its declared type.
    fn call_user_fn(&mut self, sig: &FuncSig, vs: &[(String, Type)]) -> (String, Type) {
        let mut argv = Vec::new();
        for (i, p) in sig.params.iter().enumerate() {
            let is_inout = sig.inout_params.get(i).copied().unwrap_or(false);
            if i < vs.len() {
                if is_inout {
                    argv.push(format!("ptr {}", vs[i].0));
                } else {
                    let (v2, _) = self.coerce(vs[i].0.clone(), &vs[i].1, &p.1);
                    argv.push(format!("{} {}", llty(&p.1), v2));
                }
            } else if let Some(Some(dflt)) = sig.defaults.get(i) {
                let (dv, dt) = self.expr(dflt);
                argv.push(format!("{} {dv}", llty(&dt)));
            } else if is_inout || p.1.is_ptr() || matches!(p.1, Type::Optional(_)) {
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
        if let Some(mono) = self.db.resolved_calls.get(&(span.file, span.start, span.end)).cloned()
            .or_else(|| self.db.resolved_calls.get(&(callee_span.file, callee_span.start, callee_span.end)).cloned())
        {
            if let Some(sig) = self.db.funcs.get(&mono).cloned() {
                let vs = self.eval_args_for_sig(Some(&sig), args);
                return self.call_user_fn(&sig, &vs);
            }
        }
        if let Some(sig) = self.db.funcs.get(n).cloned() {
            if sig.is_extern {
                let vs = self.eval_args_for_sig(Some(&sig), args);
                return self.ffi_call(&sig, &vs);
            }
            let vs = self.eval_args_for_sig(Some(&sig), args);
            // SWARM RABBIT HOLE — Wave 15 (Impl-T, Cohort Delta):
            // Salt ε — this call site's degraded requires atoms
            // (span-keyed, exactly the resolved_calls precedent). The
            // arguments are already evaluated once; the assertions
            // bind the callee's params to those values — the
            // argument expressions are never re-evaluated.
            if let Some(checks) = self
                .db
                .contract_site_checks
                .get(&(span.file, span.start, span.end))
                .cloned()
            {
                self.contract_site_asserts(&sig, &vs, &checks);
            }
            return self.call_user_fn(&sig, &vs);
        }
        let vs = self.eval_args_for_sig(None, args);
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
                self.emit_alloca(&format!("{body} = alloca ptr"));
                self.emit_alloca(&format!("{err} = alloca ptr"));
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
                self.emit_alloca(&format!("{body} = alloca ptr"));
                self.emit_alloca(&format!("{err} = alloca ptr"));
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
                self.emit_alloca(&format!("{code_slot} = alloca i64"));
                self.emit_alloca(&format!("{out_slot} = alloca ptr"));
                self.emit_alloca(&format!("{err_slot} = alloca ptr"));
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
                self.emit_alloca(&format!("{code_slot} = alloca i64"));
                self.emit_alloca(&format!("{out_slot} = alloca ptr"));
                self.emit_alloca(&format!("{err_slot} = alloca ptr"));
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
                self.emit_alloca(&format!("{body} = alloca ptr"));
                self.emit_alloca(&format!("{err} = alloca ptr"));
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
                        Arg::Pos(e) | Arg::Inout(e) => {
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
                self.emit_alloca(&format!("{arr} = alloca [{n} x ptr]"));
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
            "sb_new" => {
                let cap = vs.first().map(|x| x.0.clone()).unwrap_or("64".into());
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_sb_new(i64 {cap})"));
                (r, Type::Ptr)
            }
            "sb_append" | "sb_append_str" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let s = vs.get(1).map(|x| x.0.clone()).unwrap_or("null".into());
                self.line(&format!("call void @sn_sb_append_str(ptr {p}, ptr {s})"));
                ("0".into(), Type::Void)
            }
            "sb_append_cstr" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let cs = self.as_ptr(
                    vs.get(1).map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.get(1).map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let len = vs.get(2).map(|x| x.0.clone()).unwrap_or("0".into());
                self.line(&format!("call void @sn_sb_append_cstr(ptr {p}, ptr {cs}, i64 {len})"));
                ("0".into(), Type::Void)
            }
            "sb_append_int" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let v = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                self.line(&format!("call void @sn_sb_append_i64(ptr {p}, i64 {v})"));
                ("0".into(), Type::Void)
            }
            "sb_append_byte" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let b = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                self.line(&format!("call void @sn_sb_append_byte(ptr {p}, i64 {b})"));
                ("0".into(), Type::Void)
            }
            "sb_to_str" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_sb_to_str(ptr {p})"));
                (r, Type::Str)
            }
            "sb_len" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_sb_len(ptr {p})"));
                (r, Type::Int)
            }
            "sb_clear" => {
                let p = self.as_ptr(
                    vs.first().map(|x| x.0.clone()).unwrap_or("null".into()),
                    vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr),
                );
                self.line(&format!("call void @sn_sb_clear(ptr {p})"));
                ("0".into(), Type::Void)
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
            "lane_alloc" => {
                // SWARM RABBIT HOLE — D0 gap #2 (Impl-B, Wave 5): inline
                // constant-size bump on lane 0 — W2F §1(b)'s TLS-reloaded
                // load/gep/store core. `top` is 8-aligned by construction
                // (page-aligned base, every bump rounded to 8), so the
                // constant size is pre-rounded and no alignment math is
                // emitted (W2F §6: alignment by construction). The
                // limit+frozen guard routes overflow, an uninitialized
                // lane (top == limit == null ⇒ ugt) and a frozen lane to
                // @sn_lane_alloc, which errors loudly (W2F §3: check-based
                // POSIX design, no trap handlers). Dynamic sizes, other
                // lanes and lexically frozen bodies keep the runtime call.
                //
                // Wave 6 (Impl-D, bench/RESULTS.md §5): in poison emission
                // mode every path this arm can take fills 0xAA itself —
                // the T-FIBR debug guarantee no longer depends on which
                // codegen path ran (the inline bump used to silently skip
                // the runtime's SN_POISON fill).
                let lane0_const = matches!(
                    args.get(0),
                    Some(Arg::Pos(e)) if matches!(e.kind, ExprKind::Int(0))
                );
                let size_const: Option<i64> = match args.get(1) {
                    Some(Arg::Pos(e)) => match &e.kind {
                        ExprKind::Int(n) if *n > 0 => Some(*n),
                        _ => None,
                    },
                    _ => None,
                };
                let inline_ok = lane_inline_on()
                    && self.frozen_depth == 0
                    && lane0_const
                    && size_const.is_some();
                if inline_ok {
                    let sz = size_const.unwrap();
                    let sz8 = (sz + 7) & !7;
                    let top_addr = self.lane_field_addr("0", 1);
                    let old = self.t();
                    self.line(&format!("{old} = load ptr, ptr {top_addr}"));
                    let new_top = self.t();
                    self.line(&format!("{new_top} = getelementptr i8, ptr {old}, i64 {sz8}"));
                    let lim_addr = self.lane_field_addr("0", 3);
                    let lim = self.t();
                    self.line(&format!("{lim} = load ptr, ptr {lim_addr}"));
                    let frz_addr = self.lane_field_addr("0", 5);
                    let frz = self.t();
                    self.line(&format!("{frz} = load i32, ptr {frz_addr}"));
                    let oob = self.t();
                    self.line(&format!("{oob} = icmp ugt ptr {new_top}, {lim}"));
                    let frozen_bad = self.t();
                    self.line(&format!("{frozen_bad} = icmp ne i32 {frz}, 0"));
                    let bad = self.t();
                    self.line(&format!("{bad} = or i1 {oob}, {frozen_bad}"));
                    let l_fast = self.l("lanefast");
                    let l_slow = self.l("laneslow");
                    let l_join = self.l("lanejoin");
                    self.line(&format!("br i1 {bad}, label %{l_slow}, label %{l_fast}"));
                    self.raw(&format!("{l_fast}:"));
                    self.line(&format!("store ptr {new_top}, ptr {top_addr}"));
                    if lane_poison_on() {
                        self.emit_poison_const(&old, sz);
                    }
                    self.line(&format!("br label %{l_join}"));
                    self.raw(&format!("{l_slow}:"));
                    let slow_r = self.t();
                    self.line(&format!(
                        "{slow_r} = call ptr @sn_lane_alloc(i64 0, i64 {sz})"
                    ));
                    if lane_poison_on() {
                        self.emit_poison_const(&slow_r, sz);
                    }
                    self.line(&format!("br label %{l_join}"));
                    self.raw(&format!("{l_join}:"));
                    let r = self.t();
                    self.line(&format!(
                        "{r} = phi ptr [ {old}, %{l_fast} ], [ {slow_r}, %{l_slow} ]"
                    ));
                    (r, Type::Ptr)
                } else {
                    let (lane_id, bytes) = if vs.len() >= 2 {
                        (vs[0].0.clone(), vs[1].0.clone())
                    } else if vs.len() == 1 {
                        ("0".into(), vs[0].0.clone())
                    } else {
                        ("0".into(), "0".into())
                    };
                    let r = self.t();
                    self.line(&format!("{r} = call ptr @sn_lane_alloc(i64 {lane_id}, i64 {bytes})"));
                    if lane_poison_on() {
                        self.emit_poison_dynamic(&r, &bytes);
                    }
                    (r, Type::Ptr)
                }
            }
            "lane_save" => {
                let lane_id = vs.first().map(|x| x.0.clone()).unwrap_or("0".into());
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_lane_save(i64 {lane_id})"));
                (r, Type::Ptr)
            }
            "lane_restore" => {
                let (lane_id, wm) = if vs.len() >= 2 {
                    (vs[0].0.clone(), self.as_ptr(vs[1].0.clone(), vs[1].1.clone()))
                } else if vs.len() == 1 {
                    ("0".into(), self.as_ptr(vs[0].0.clone(), vs[0].1.clone()))
                } else {
                    ("0".into(), "null".into())
                };
                self.line(&format!("call void @sn_lane_restore(i64 {lane_id}, ptr {wm})"));
                ("0".into(), Type::Void)
            }
            "lane_used" => {
                let lane_id = vs.first().map(|x| x.0.clone()).unwrap_or("0".into());
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_lane_used(i64 {lane_id})"));
                (r, Type::Int)
            }
            "lane_freeze" => {
                let lane_id = vs.first().map(|x| x.0.clone()).unwrap_or("0".into());
                self.line(&format!("call void @sn_lane_freeze(i64 {lane_id})"));
                ("0".into(), Type::Void)
            }
            "lane_unfreeze" => {
                let lane_id = vs.first().map(|x| x.0.clone()).unwrap_or("0".into());
                self.line(&format!("call void @sn_lane_unfreeze(i64 {lane_id})"));
                ("0".into(), Type::Void)
            }
            "pool_new" => {
                let (elem_kind, elem_size) = if !type_args.is_empty() {
                    let elem = resolve_type_ast(&type_args[0], self.db);
                    let kind = if elem.is_ptr() || matches!(elem, Type::Optional(_)) { 1i64 } else { 0i64 };
                    let size = elem.abi_size();
                    (format!("{kind}"), format!("{size}"))
                } else if vs.len() >= 2 {
                    (vs[0].0.clone(), vs[1].0.clone())
                } else if vs.len() == 1 {
                    ("0".into(), vs[0].0.clone())
                } else {
                    ("0".into(), "8".into())
                };
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_pool_new(i64 {elem_kind}, i64 {elem_size})"));
                let ret_ty = if !type_args.is_empty() {
                    Type::Pool(Box::new(resolve_type_ast(&type_args[0], self.db)))
                } else {
                    Type::Ptr
                };
                (r, ret_ty)
            }
            "pool_insert" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let r = self.t();
                if let Some(val) = vs.get(1) {
                    if let Type::Record(rn) = &val.1 {
                        let sz = self.db.records.get(rn).map(|rec| rec.size).unwrap_or(8);
                        self.line(&format!("{r} = call i64 @sn_pool_insert_rec(ptr {p}, ptr {}, i64 {sz})", val.0));
                    } else {
                        let vb = self.as_bits(val.0.clone(), &val.1);
                        self.line(&format!("{r} = call i64 @sn_pool_insert_i64(ptr {p}, i64 {vb})"));
                    }
                    (r, Type::Int)
                } else {
                    self.line(&format!("{r} = call i64 @sn_pool_insert_i64(ptr {p}, i64 0)"));
                    (r, Type::Int)
                }
            }
            "pool_get" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let h = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_pool_get_i64(ptr {p}, i64 {h})"));
                if let Some((_, Type::Pool(elem))) = vs.first() {
                    let val = self.from_bits(r, elem);
                    (val, (**elem).clone())
                } else {
                    (r, Type::Int)
                }
            }
            "pool_set" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let h = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                let val = vs.get(2).cloned().unwrap_or(("0".into(), Type::Int));
                let vb = self.as_bits(val.0, &val.1);
                self.line(&format!("call void @sn_pool_set_i64(ptr {p}, i64 {h}, i64 {vb})"));
                ("0".into(), Type::Void)
            }
            "pool_remove" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let h = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                self.line(&format!("call void @sn_pool_remove(ptr {p}, i64 {h})"));
                ("0".into(), Type::Void)
            }
            "pool_is_valid" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let h = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_pool_is_valid(ptr {p}, i64 {h})"));
                (r, Type::Bool)
            }
            "pool_len" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_pool_len(ptr {p})"));
                (r, Type::Int)
            }
            /* Tier-G Z-set builtins (SWARM RABBIT HOLE, P5 slice 1; W2C §1.1):
               count += delta / count -= delta (exact retraction, O(1)/key),
               merge = pointwise ⊕, delta = the 𝒟 (dst − prev), apply = ⊕. */
            "zset_new" => {
                let kk = vs.first().map(|x| x.0.clone()).unwrap_or("0".into());
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_zset_new(i64 {kk})"));
                (r, Type::Ptr)
            }
            "zset_add" | "zset_remove" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let key = vs.get(1).cloned().unwrap_or(("0".into(), Type::Int));
                let kb = self.as_bits(key.0, &key.1);
                let d = vs.get(2).map(|x| x.0.clone()).unwrap_or("0".into());
                if n == "zset_add" {
                    self.line(&format!("call void @sn_zset_add(ptr {p}, i64 {kb}, i64 {d})"));
                } else {
                    self.line(&format!("call void @sn_zset_remove(ptr {p}, i64 {kb}, i64 {d})"));
                }
                ("0".into(), Type::Void)
            }
            "zset_merge" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let dst = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let s1 = vs.get(1).map(|x| x.0.clone()).unwrap_or("null".into());
                let src = self.as_ptr(s1, vs.get(1).map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                self.line(&format!("call void @sn_zset_merge(ptr {dst}, ptr {src})"));
                ("0".into(), Type::Void)
            }
            "zset_delta" | "zset_apply" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let a = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let p1 = vs.get(1).map(|x| x.0.clone()).unwrap_or("null".into());
                let b = self.as_ptr(p1, vs.get(1).map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let r = self.t();
                if n == "zset_delta" {
                    self.line(&format!("{r} = call ptr @sn_zset_delta(ptr {a}, ptr {b})"));
                } else {
                    self.line(&format!("{r} = call ptr @sn_zset_apply(ptr {a}, ptr {b})"));
                }
                (r, Type::Ptr)
            }
            "zset_copy" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_zset_copy(ptr {p})"));
                (r, Type::Ptr)
            }
            "zset_count" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let key = vs.get(1).cloned().unwrap_or(("0".into(), Type::Int));
                let kb = self.as_bits(key.0, &key.1);
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_zset_count(ptr {p}, i64 {kb})"));
                (r, Type::Int)
            }
            "zset_total" | "zset_len" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let r = self.t();
                if n == "zset_total" {
                    self.line(&format!("{r} = call i64 @sn_zset_total(ptr {p})"));
                } else {
                    self.line(&format!("{r} = call i64 @sn_zset_len(ptr {p})"));
                }
                (r, Type::Int)
            }
            "zset_keys" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_zset_keys(ptr {p})"));
                (r, Type::List(Box::new(Type::Int)))
            }
            "zset_set_prune" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let f = vs.get(1).map(|x| x.0.clone()).unwrap_or("1".into());
                self.line(&format!("call void @sn_zset_set_prune(ptr {p}, i64 {f})"));
                ("0".into(), Type::Void)
            }
            /* Tier-L join-semilattice builtins (SWARM RABBIT HOLE, Wave 9,
               Impl-M; W2C §1.1 Tier L): inflate is the only mutator (the
               LVar/CRDT monotone discipline), merge = the ⊔ join (mutating),
               delta = the mutator-law fragment m^δ (pure, a new cell),
               apply = base ⊔ delta (pure). NO lcell_retract arm exists:
               the checker ALWAYS rejects it (E-LRET — the (L, exact)
               capability is impossible on the idempotent carrier), so
               lowering never sees the name. */
            "lcell_new" => {
                let carrier = vs.first().map(|x| x.0.clone()).unwrap_or("0".into());
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_lcell_new(i64 {carrier})"));
                (r, Type::Ptr)
            }
            "lcell_inflate" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let a = vs.get(1).cloned().unwrap_or(("0".into(), Type::Int));
                let ab = self.as_bits(a.0, &a.1);
                let b = vs.get(2).map(|x| x.0.clone()).unwrap_or("0".into());
                self.line(&format!("call void @sn_lcell_inflate(ptr {p}, i64 {ab}, i64 {b})"));
                ("0".into(), Type::Void)
            }
            "lcell_merge" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let dst = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let s1 = vs.get(1).map(|x| x.0.clone()).unwrap_or("null".into());
                let src = self.as_ptr(s1, vs.get(1).map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                self.line(&format!("call void @sn_lcell_merge(ptr {dst}, ptr {src})"));
                ("0".into(), Type::Void)
            }
            "lcell_delta" | "lcell_apply" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let a = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let p1 = vs.get(1).map(|x| x.0.clone()).unwrap_or("null".into());
                let b = self.as_ptr(p1, vs.get(1).map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let r = self.t();
                if n == "lcell_delta" {
                    self.line(&format!("{r} = call ptr @sn_lcell_delta(ptr {a}, ptr {b})"));
                } else {
                    self.line(&format!("{r} = call ptr @sn_lcell_apply(ptr {a}, ptr {b})"));
                }
                (r, Type::Ptr)
            }
            "lcell_len" | "lcell_value" | "lcell_carrier" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let r = self.t();
                let f = if n == "lcell_len" {
                    "sn_lcell_len"
                } else if n == "lcell_value" {
                    "sn_lcell_value"
                } else {
                    "sn_lcell_carrier"
                };
                self.line(&format!("{r} = call i64 @{f}(ptr {p})"));
                (r, Type::Int)
            }
            "lcell_has" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let e = vs.get(1).cloned().unwrap_or(("0".into(), Type::Int));
                let eb = self.as_bits(e.0, &e.1);
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_lcell_has(ptr {p}, i64 {eb})"));
                (r, Type::Bool)
            }
            /* SWARM RABBIT HOLE — Wave 13 (Impl-R): the wave seam's
               read-mode surface (W2B §3), lowered. read_W/contrib
               run against the ENCLOSING wave context (wave_stack's
               innermost entry — the checker guarantees it exists:
               outside a wave both are E-R1 refusals, and nested waves
               are E0812). read_J lowers to the same sn_lcell_value
               the fallback read uses — the fold-valued MODE is the
               checked temporal claim (E-R1 at the call), the value
               is the fold J(W) left in the cell. */
            "read_W" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let cell = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let w = self.wave_stack.last().cloned().unwrap_or("null".into());
                let r = self.t();
                self.line(&format!(
                    "{r} = call i64 @sn_wave_read_w(ptr {w}, ptr {cell})"
                ));
                (r, Type::Int)
            }
            "contrib" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let cell = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let a = vs.get(1).cloned().unwrap_or(("0".into(), Type::Int));
                let ab = self.as_bits(a.0, &a.1);
                let b = vs.get(2).map(|x| x.0.clone()).unwrap_or("0".into());
                let w = self.wave_stack.last().cloned().unwrap_or("null".into());
                self.line(&format!(
                    "call void @sn_wave_contrib(ptr {w}, ptr {cell}, i64 {ab}, i64 {b})"
                ));
                ("0".into(), Type::Void)
            }
            "read_J" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_lcell_value(ptr {p})"));
                (r, Type::Int)
            }
            /* SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the
               SFC layouts, lowered (A2 §2.4's decision table
               executable). Every intrinsic is ONE runtime call — the
               addr arithmetic (interleave) is feature-dispatched ONCE
               PER PROCESS inside the runtime (C2.8: PDEP/PEXT on
               x86-64 BMI2, the software shift-or path elsewhere),
               never inlined into the IR, so the dispatch knob
               (SN_SFC_FORCE_PATH) and the equivalence probes see the
               one definition. The collection args lower as ptr (the
               lane-backed header pointer); the dims/k ride as i64
               immediates from the checker-validated literals. */
            "morton_new" => {
                let d1 = vs.first().map(|x| x.0.clone()).unwrap_or("0".into());
                let d2 = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_morton_new(i64 {d1}, i64 {d2})"));
                (r, Type::Morton(Box::new(Type::Int), vec![]))
            }
            "morton_from_lex" => {
                let s0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let src = self.as_ptr(s0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let d1 = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                let d2 = vs.get(2).map(|x| x.0.clone()).unwrap_or("0".into());
                let r = self.t();
                self.line(&format!(
                    "{r} = call ptr @sn_morton_from_lex(ptr {src}, i64 {d1}, i64 {d2})"
                ));
                (r, Type::Morton(Box::new(Type::Int), vec![]))
            }
            "morton_from_bytes" => {
                let b0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let bytes = self.as_ptr(b0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let d1 = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                let d2 = vs.get(2).map(|x| x.0.clone()).unwrap_or("0".into());
                let r = self.t();
                self.line(&format!(
                    "{r} = call ptr @sn_morton_from_bytes(ptr {bytes}, i64 {d1}, i64 {d2})"
                ));
                (r, Type::Morton(Box::new(Type::Int), vec![]))
            }
            "morton_get" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let i = self.sfc_i(&vs, 1);
                let j = self.sfc_i(&vs, 2);
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_morton_get(ptr {p}, i64 {i}, i64 {j})"));
                (r, Type::Int)
            }
            "morton_set" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let i = self.sfc_i(&vs, 1);
                let j = self.sfc_i(&vs, 2);
                let v = self.sfc_i(&vs, 3);
                self.line(&format!(
                    "call void @sn_morton_set(ptr {p}, i64 {i}, i64 {j}, i64 {v})"
                ));
                ("0".into(), Type::Void)
            }
            "morton_index" => {
                let i = self.sfc_i(&vs, 0);
                let j = self.sfc_i(&vs, 1);
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_morton_index(i64 {i}, i64 {j})"));
                (r, Type::Int)
            }
            "morton_x" | "morton_y" => {
                let k = self.sfc_i(&vs, 0);
                let r = self.t();
                let f = if n == "morton_x" { "sn_morton_x" } else { "sn_morton_y" };
                self.line(&format!("{r} = call i64 @{f}(i64 {k})"));
                (r, Type::Int)
            }
            "morton_len" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_morton_len(ptr {p})"));
                (r, Type::Int)
            }
            "morton_scan_get" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let k = self.sfc_i(&vs, 1);
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_morton_scan_get(ptr {p}, i64 {k})"));
                (r, Type::Int)
            }
            "morton_to_bytes" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_morton_to_bytes(ptr {p})"));
                (r, Type::Ptr)
            }
            "hilbert_new" => {
                let k = vs.first().map(|x| x.0.clone()).unwrap_or("0".into());
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_hilbert_new(i64 {k})"));
                (r, Type::Hilbert(Box::new(Type::Int), Box::new(Type::Int)))
            }
            "hilbert_code" => {
                let x = self.sfc_i(&vs, 0);
                let y = self.sfc_i(&vs, 1);
                let k = self.sfc_i(&vs, 2);
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_hilbert_code(i64 {x}, i64 {y}, i64 {k})"));
                (r, Type::Int)
            }
            "hilbert_dx" | "hilbert_dy" => {
                let c = self.sfc_i(&vs, 0);
                let k = self.sfc_i(&vs, 1);
                let r = self.t();
                let f = if n == "hilbert_dx" { "sn_hilbert_dx" } else { "sn_hilbert_dy" };
                self.line(&format!("{r} = call i64 @{f}(i64 {c}, i64 {k})"));
                (r, Type::Int)
            }
            "hilbert_insert" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let x = self.sfc_i(&vs, 1);
                let y = self.sfc_i(&vs, 2);
                let v = self.sfc_i(&vs, 3);
                self.line(&format!(
                    "call void @sn_hilbert_insert(ptr {p}, i64 {x}, i64 {y}, i64 {v})"
                ));
                ("0".into(), Type::Void)
            }
            "hilbert_get" | "hilbert_has" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let x = self.sfc_i(&vs, 1);
                let y = self.sfc_i(&vs, 2);
                let r = self.t();
                if n == "hilbert_get" {
                    self.line(&format!("{r} = call i64 @sn_hilbert_get(ptr {p}, i64 {x}, i64 {y})"));
                    (r, Type::Int)
                } else {
                    self.line(&format!("{r} = call i64 @sn_hilbert_has(ptr {p}, i64 {x}, i64 {y})"));
                    (r, Type::Bool)
                }
            }
            "hilbert_len" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_hilbert_len(ptr {p})"));
                (r, Type::Int)
            }
            "hilbert_entry_key" | "hilbert_entry_x" | "hilbert_entry_y" | "hilbert_entry_val" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let idx = self.sfc_i(&vs, 1);
                let r = self.t();
                let f = match n {
                    "hilbert_entry_key" => "sn_hilbert_entry_key",
                    "hilbert_entry_x" => "sn_hilbert_entry_x",
                    "hilbert_entry_y" => "sn_hilbert_entry_y",
                    _ => "sn_hilbert_entry_val",
                };
                self.line(&format!("{r} = call i64 @{f}(ptr {p}, i64 {idx})"));
                (r, Type::Int)
            }
            "hilbert_box_first" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let x0 = self.sfc_i(&vs, 1);
                let y0 = self.sfc_i(&vs, 2);
                let x1 = self.sfc_i(&vs, 3);
                let y1 = self.sfc_i(&vs, 4);
                let r = self.t();
                self.line(&format!(
                    "{r} = call i64 @sn_hilbert_box_first(ptr {p}, i64 {x0}, i64 {y0}, i64 {x1}, i64 {y1})"
                ));
                (r, Type::Int)
            }
            "hilbert_box_next" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let x0 = self.sfc_i(&vs, 1);
                let y0 = self.sfc_i(&vs, 2);
                let x1 = self.sfc_i(&vs, 3);
                let y1 = self.sfc_i(&vs, 4);
                let idx = self.sfc_i(&vs, 5);
                let r = self.t();
                self.line(&format!(
                    "{r} = call i64 @sn_hilbert_box_next(ptr {p}, i64 {x0}, i64 {y0}, i64 {x1}, i64 {y1}, i64 {idx})"
                ));
                (r, Type::Int)
            }
            "sfc_path" => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_sfc_path()"));
                (r, Type::Int)
            }
            /* Salt-ζ fixpoint builtins (SWARM RABBIT HOLE, Wave 9,
               Impl-N; W2C §3.3–§3.5): the admitted closure bracket over
               Tier-G (zfix_new) and Tier-L (zfix_new_l) carriers, the
               semi-naïve round/run primitives, and the readouts.
               NO zfix_count / zfix_negate arms exist: the admission
               checker ALWAYS rejects them (E-ZADM — the two named
               divergence classes: infinite-height carrier;
               non-monotone body), so lowering never sees the names. */
            "zfix_new" | "zfix_new_l" => {
                let e0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let edges = self.as_ptr(e0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let s1 = vs.get(1).map(|x| x.0.clone()).unwrap_or("null".into());
                let seed = self.as_ptr(s1, vs.get(1).map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let r = self.t();
                let f = if n == "zfix_new" {
                    "sn_zfix_new"
                } else {
                    "sn_zfix_new_l"
                };
                self.line(&format!("{r} = call ptr @{f}(ptr {edges}, ptr {seed})"));
                (r, Type::Ptr)
            }
            "zfix_round" | "zfix_run" | "zfix_rounds" | "zfix_len" | "zfix_probes" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let r = self.t();
                let f = match n {
                    "zfix_round" => "sn_zfix_round",
                    "zfix_run" => "sn_zfix_run",
                    "zfix_rounds" => "sn_zfix_rounds",
                    "zfix_len" => "sn_zfix_len",
                    _ => "sn_zfix_probes",
                };
                self.line(&format!("{r} = call i64 @{f}(ptr {p})"));
                (r, Type::Int)
            }
            "zfix_has" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let e = vs.get(1).cloned().unwrap_or(("0".into(), Type::Int));
                let eb = self.as_bits(e.0, &e.1);
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_zfix_has(ptr {p}, i64 {eb})"));
                (r, Type::Bool)
            }
            "zfix_quiescent_g" | "zfix_quiescent_l" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let r = self.t();
                let f = if n == "zfix_quiescent_g" {
                    "sn_zfix_quiescent_g"
                } else {
                    "sn_zfix_quiescent_l"
                };
                self.line(&format!("{r} = call i64 @{f}(ptr {p})"));
                (r, Type::Bool)
            }
            "zfix_state" => {
                let p0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                let p = self.as_ptr(p0, vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr));
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_zfix_state(ptr {p})"));
                (r, Type::Ptr)
            }
            /* SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): the
               self-relative link intrinsics (kills G1 §1 A2 → S4) — the
               runtime half of the E0801 seam closure. The lowering
               resolves the record's layout (size + the link fields'
               offsets/names/target labels) from the explicit type
               argument and emits the runtime calls the verbatim
               diagnostics name; the source/destination labels come from
               the call's argument expressions. relocate_node /
               relocate_range re-encode every delta against the new
               field address (|δ'| ≤ 32767 else the E0801 (relocate)
               fail-fast); link_to_bytes runs the O(N) sender-side
               self-containedness scan before packing (a cross-slice
               link → the E0801 (serialize) fail-fast); the _raw twin is
               the bench-only comparator that prices the scan. */
            "link_block_new" | "link_set" | "link_get" | "link_elem" | "relocate_node"
            | "relocate_range" | "link_to_bytes" | "link_to_bytes_raw" => {
                // The record layout from the explicit type argument.
                let Some(ta) = type_args.first() else {
                    return ("0".into(), Type::Ptr);
                };
                let t = resolve_type_ast(ta, self.db);
                let Type::Record(rn) = &t else {
                    return ("0".into(), Type::Ptr);
                };
                let (sz, links) = match self.db.records.get(rn) {
                    Some(r) => (
                        r.size,
                        r.fields
                            .iter()
                            .filter(|f| matches!(f.ty, Type::Link(_)))
                            .map(|f| {
                                let target = match &f.ty {
                                    Type::Link(inner) => inner.to_string(),
                                    _ => String::new(),
                                };
                                (f.offset, f.name.clone(), target)
                            })
                            .collect::<Vec<_>>(),
                    ),
                    None => return ("0".into(), Type::Ptr),
                };
                let offs: Vec<i64> = (0..4)
                    .map(|j| links.get(j).map(|x| x.0).unwrap_or(-1))
                    .collect();
                // The fixed 4-slot name/target windows the runtime
                // diagnostics read (empty slots are padded with "").
                let mut names = Vec::new();
                for j in 0..4 {
                    let s = links.get(j).map(|x| x.1.clone()).unwrap_or_default();
                    let (id, _) = self.intern(&s);
                    names.push(format!("@.str.{id}"));
                }
                let mut targets = Vec::new();
                for j in 0..4 {
                    let s = links.get(j).map(|x| x.2.clone()).unwrap_or_default();
                    let (id, _) = self.intern(&s);
                    targets.push(format!("@.str.{id}"));
                }
                // The labels of an argument expression, for the
                // diagnostics' {src}/{dst}/{xs} fills.
                let lbl = |k: usize| link_arg_label(args, k);
                match n {
                    "link_block_new" => {
                        let cnt = vs.first().map(|x| x.0.clone()).unwrap_or("0".into());
                        let r = self.t();
                        self.line(&format!(
                            "{r} = call ptr @sn_link_block_new(i64 {cnt}, i64 {sz})"
                        ));
                        (r, Type::Ptr)
                    }
                    "link_elem" => {
                        let b = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                        let bt = vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr);
                        let p = self.as_ptr(b, bt);
                        let i = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                        let r = self.t();
                        self.line(&format!(
                            "{r} = call ptr @sn_link_elem(ptr {p}, i64 {i}, i64 {sz})"
                        ));
                        (r, Type::Ptr)
                    }
                    "link_set" | "link_get" => {
                        // The link-field ordinal is a checker-validated
                        // integer literal: the offset is an immediate.
                        let ord = match args.get(2) {
                            Some(Arg::Pos(e)) => match &e.kind {
                                ExprKind::Int(v) => (*v).clamp(0, 3),
                                _ => 0,
                            },
                            _ => 0,
                        };
                        let off = offs[ord as usize];
                        let fname = names[ord as usize].clone();
                        let tname = targets[ord as usize].clone();
                        let b0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                        let bt0 = vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr);
                        let p0 = self.as_ptr(b0, bt0);
                        let i0 = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                        if n == "link_set" {
                            let b1 = vs.get(3).map(|x| x.0.clone()).unwrap_or("null".into());
                            let bt1 = vs.get(3).map(|x| x.1.clone()).unwrap_or(Type::Ptr);
                            let p1 = self.as_ptr(b1, bt1);
                            let i1 = vs.get(4).map(|x| x.0.clone()).unwrap_or("0".into());
                            self.line(&format!(
                                "call void @sn_link_set(ptr {p0}, i64 {i0}, i64 {off}, \
                                 ptr {p1}, i64 {i1}, i64 {sz}, ptr {fname}, ptr {tname})"
                            ));
                            ("0".into(), Type::Void)
                        } else {
                            let r = self.t();
                            self.line(&format!(
                                "{r} = call i64 @sn_link_get(ptr {p0}, i64 {i0}, i64 {off}, i64 {sz})"
                            ));
                            (r, Type::Int)
                        }
                    }
                    "relocate_node" => {
                        let nl = links.len() as i64;
                        let src_label = format!("{}[{}]", lbl(0), lbl(1));
                        let dst_label = format!("{}[{}]", lbl(2), lbl(3));
                        let (sid, _) = self.intern(&src_label);
                        let (did, _) = self.intern(&dst_label);
                        let b0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                        let bt0 = vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr);
                        let p0 = self.as_ptr(b0, bt0);
                        let i0 = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                        let b1 = vs.get(2).map(|x| x.0.clone()).unwrap_or("null".into());
                        let bt1 = vs.get(2).map(|x| x.1.clone()).unwrap_or(Type::Ptr);
                        let p1 = self.as_ptr(b1, bt1);
                        let i1 = vs.get(3).map(|x| x.0.clone()).unwrap_or("0".into());
                        self.line(&format!(
                            "call void @sn_relocate_node(ptr {p0}, i64 {i0}, ptr {p1}, i64 {i1}, \
                             i64 {sz}, i64 {nl}, i64 {}, i64 {}, i64 {}, i64 {}, \
                             ptr {}, ptr {}, ptr {}, ptr {}, \
                             ptr {}, ptr {}, ptr {}, ptr {}, \
                             ptr @.str.{sid}, ptr @.str.{did})",
                            offs[0], offs[1], offs[2], offs[3],
                            names[0], names[1], names[2], names[3],
                            targets[0], targets[1], targets[2], targets[3],
                        ));
                        ("0".into(), Type::Void)
                    }
                    "relocate_range" => {
                        let nl = links.len() as i64;
                        let src_label = lbl(0);
                        let dst_label = lbl(1);
                        let (sid, _) = self.intern(&src_label);
                        let (did, _) = self.intern(&dst_label);
                        let b0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                        let bt0 = vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr);
                        let p0 = self.as_ptr(b0, bt0);
                        let b1 = vs.get(1).map(|x| x.0.clone()).unwrap_or("null".into());
                        let bt1 = vs.get(1).map(|x| x.1.clone()).unwrap_or(Type::Ptr);
                        let p1 = self.as_ptr(b1, bt1);
                        let cnt = vs.get(2).map(|x| x.0.clone()).unwrap_or("0".into());
                        self.line(&format!(
                            "call void @sn_relocate_range(ptr {p0}, ptr {p1}, i64 {cnt}, \
                             i64 {sz}, i64 {nl}, i64 {}, i64 {}, i64 {}, i64 {}, \
                             ptr {}, ptr {}, ptr {}, ptr {}, \
                             ptr {}, ptr {}, ptr {}, ptr {}, \
                             ptr @.str.{sid}, ptr @.str.{did})",
                            offs[0], offs[1], offs[2], offs[3],
                            names[0], names[1], names[2], names[3],
                            targets[0], targets[1], targets[2], targets[3],
                        ));
                        ("0".into(), Type::Void)
                    }
                    _ => {
                        // link_to_bytes / link_to_bytes_raw: the raw twin
                        // is the bench-only comparator that prices the
                        // sender-side scan (raw=1 skips the scan).
                        let nl = links.len() as i64;
                        let xs_label = lbl(0);
                        let (xid, _) = self.intern(&xs_label);
                        let b0 = vs.first().map(|x| x.0.clone()).unwrap_or("null".into());
                        let bt0 = vs.first().map(|x| x.1.clone()).unwrap_or(Type::Ptr);
                        let p0 = self.as_ptr(b0, bt0);
                        let a = vs.get(1).map(|x| x.0.clone()).unwrap_or("0".into());
                        let b = vs.get(2).map(|x| x.0.clone()).unwrap_or("0".into());
                        let raw = if n == "link_to_bytes_raw" { 1 } else { 0 };
                        let r = self.t();
                        self.line(&format!(
                            "{r} = call ptr @sn_link_to_bytes(ptr {p0}, i64 {a}, i64 {b}, \
                             i64 {sz}, i64 {nl}, i64 {}, i64 {}, i64 {}, i64 {}, \
                             ptr @.str.{xid}, \
                             ptr {}, ptr {}, ptr {}, ptr {}, \
                             ptr {}, ptr {}, ptr {}, ptr {}, \
                             i64 {raw})",
                            offs[0], offs[1], offs[2], offs[3],
                            names[0], names[1], names[2], names[3],
                            targets[0], targets[1], targets[2], targets[3],
                        ));
                        (r, Type::Ptr)
                    }
                }
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
                if let Some(Arg::Pos(e)) = args[0..]
                    .first()
                {
                    match &e.kind {
                        ExprKind::Ident(nm) => {
                            if let Some((an, ty)) = self.locals.get(nm).cloned() {
                                return (an, Type::Ref(Box::new(ty)));
                            }
                            if self.db.globals.contains_key(nm) {
                                let gt = self.db.globals.get(nm).cloned().unwrap_or(Type::Int);
                                return (format!("@sn_g_{nm}"), Type::Ref(Box::new(gt)));
                            }
                        }
                        ExprKind::Index { base, index } => {
                            let (b, bt) = self.expr(base);
                            let (i, _) = self.expr(index);
                            if let Type::List(elem) = bt {
                                let r = self.t();
                                self.line(&format!("{r} = call ptr @sn_list_elem_addr(ptr {b}, i64 {i})"));
                                return (r, Type::Ref(elem));
                            }
                        }
                        ExprKind::Member { base, name } => {
                            let (b, bt) = self.expr(base);
                            match bt {
                                Type::Record(rn) => {
                                    if let Some(info) = self.db.records.get(&rn) {
                                        if let Some(f) = info.fields.iter().find(|f| f.name == *name) {
                                            let fp = self.t();
                                            self.line(&format!("{fp} = getelementptr i8, ptr {b}, i64 {}", f.offset));
                                            return (fp, Type::Ref(Box::new(f.ty.clone())));
                                        }
                                    }
                                }
                                Type::Blueprint(bp) => {
                                    if let Some(info) = self.db.blueprints.get(&bp) {
                                        if let Some(f) = info.fields.iter().find(|f| f.name == *name) {
                                            let fp = self.t();
                                            self.line(&format!("{fp} = getelementptr i8, ptr {b}, i64 {}", f.offset));
                                            return (fp, Type::Ref(Box::new(f.ty.clone())));
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
                ("null".into(), Type::Ref(Box::new(Type::Int)))
            }
            "value" => {
                let inner_ty = match &vs[0].1 {
                    Type::Ref(t) | Type::Mut(t) => (**t).clone(),
                    _ => Type::Int,
                };
                if let Type::Record(rn) = &inner_ty {
                    let sz = self.db.records.get(rn).map(|r| r.size).unwrap_or(8);
                    let dst = self.t();
                    self.emit_alloca(&format!("{dst} = alloca i8, i64 {sz}"));
                    self.line(&format!("call void @sn_record_copy(ptr {dst}, ptr {}, i64 {sz})", vs[0].0));
                    return (dst, inner_ty);
                }
                let lt = llty(&inner_ty);
                let r = self.t();
                self.line(&format!("{r} = load {lt}, ptr {}", vs[0].0));
                (r, inner_ty)
            }
            "set" => {
                let inner_ty = match &vs[0].1 {
                    Type::Ref(t) | Type::Mut(t) => (**t).clone(),
                    _ => vs[1].1.clone(),
                };
                if let Type::Record(rn) = &inner_ty {
                    let sz = self.db.records.get(rn).map(|r| r.size).unwrap_or(8);
                    self.line(&format!("call void @sn_record_copy(ptr {}, ptr {}, i64 {sz})", vs[0].0, vs[1].0));
                } else {
                    let (v_coerced, _) = self.coerce(vs[1].0.clone(), &vs[1].1, &inner_ty);
                    let lt = llty(&inner_ty);
                    self.line(&format!("store {lt} {v_coerced}, ptr {}", vs[0].0));
                }
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

    fn direct_method_call(
        &mut self,
        b: &str,
        sig: &FuncSig,
        vs: &[(String, Type)],
        is_static: bool,
    ) -> (String, Type) {
        let mut argv = Vec::new();
        if !is_static {
            argv.push(format!("ptr {b}"));
        }
        for (i, p) in sig.params.iter().enumerate() {
            let is_inout = sig.inout_params.get(i).copied().unwrap_or(false);
            if i < vs.len() {
                if is_inout {
                    argv.push(format!("ptr {}", vs[i].0));
                } else {
                    let (v2, _) = self.coerce(vs[i].0.clone(), &vs[i].1, &p.1);
                    argv.push(format!("{} {}", llty(&p.1), v2));
                }
            } else if let Some(Some(dflt)) = sig.defaults.get(i) {
                let (dv, dt) = self.expr(dflt);
                argv.push(format!("{} {dv}", llty(&dt)));
            } else if is_inout || p.1.is_ptr() || matches!(p.1, Type::Optional(_)) {
                argv.push("ptr null".into());
            } else {
                argv.push(format!("{} 0", llty(&p.1)));
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

    /// Checks whether an expression's value is guaranteed to stay live across a call
    /// by the caller's stack frame (e.g. local variable alloca, parameter, or immutable borrow).
    /// Used for ARC Borrow Elision: skip redundant sn_retain/sn_release pairs.
    fn is_caller_stack_live(&self, e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Ident(name) => self.locals.contains_key(name),
            ExprKind::Self_ => true,
            ExprKind::Member { base, .. } => self.is_caller_stack_live(base),
            ExprKind::Index { base, .. } => self.is_caller_stack_live(base),
            _ => false,
        }
    }

    fn method(&mut self, base: &Expr, name: &str, args: &[Arg], call_span: Span) -> (String, Type) {
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
            self.emit_alloca(&format!("{an} = alloca ptr"));
            self.line(&format!("store ptr {b}, ptr {an}"));
            self.locals
                .insert("__optbase".into(), (an, self.lookup_ty(inner)));
            let (sv, st) = self.method(&dummy, name, args, call_span);
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
                Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => vs.push(self.expr(e)),
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
                    let vb = self.as_bits(v, elem);
                    self.line(&format!("call void @sn_list_push_i64(ptr {b}, i64 {vb})"));
                }
                ("0".into(), Type::Void)
            }
            (Type::List(elem), "push_ref") => {
                let idx = self.t();
                self.line(&format!("{idx} = call i64 @sn_list_len(ptr {b})"));
                let (v, vt) = vs[0].clone();
                let (v, _) = self.coerce(v, &vt, elem);
                let kind = if elem.is_ptr() || matches!(**elem, Type::Optional(_)) { "ptr" } else { "i64" };
                if kind == "ptr" {
                    self.line(&format!("call void @sn_list_push_ptr(ptr {b}, ptr {v})"));
                } else {
                    let vb = self.as_bits(v, elem);
                    self.line(&format!("call void @sn_list_push_i64(ptr {b}, i64 {vb})"));
                }
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_list_elem_addr(ptr {b}, i64 {idx})"));
                (r, Type::Ref(elem.clone()))
            }
            (Type::List(elem), "pop") => {
                let kind = if elem.is_ptr() || matches!(**elem, Type::Optional(_)) { "ptr" } else { "i64" };
                if kind == "ptr" {
                    let r = self.t();
                    self.line(&format!("{r} = call ptr @sn_list_pop_ptr(ptr {b})"));
                    (r, (**elem).clone())
                } else {
                    let raw = self.t();
                    self.line(&format!("{raw} = call i64 @sn_list_pop_i64(ptr {b})"));
                    let r = self.from_bits(raw, elem);
                    (r, (**elem).clone())
                }
            }
            (Type::List(_), "clear") => {
                self.line(&format!("call void @sn_list_clear(ptr {b})"));
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
            (Type::Map(_, _), "contains") | (Type::Map(_, _), "has") => {
                let kb = self.as_bits(vs[0].0.clone(), &vs[0].1);
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_map_has(ptr {b}, i64 {kb})"));
                (r, Type::Bool)
            }
            (Type::Chan(_), "send") => {
                let bits = self.as_bits(vs[0].0.clone(), &vs[0].1);
                self.line(&format!("call void @sn_chan_send(ptr {b}, i64 {bits})"));
                ("0".into(), Type::Void)
            }
            (Type::Chan(t), "receive") => {
                let want_tuple = match self.db.expr_ty.get(&(call_span.file, call_span.start, call_span.end)) {
                    Some(Type::Tuple(ts)) if ts.len() == 2 => true,
                    _ => false,
                };
                if want_tuple {
                    let out_val = self.t();
                    self.emit_alloca(&format!("{out_val} = alloca i64"));
                    let ok_i64 = self.t();
                    self.line(&format!("{ok_i64} = call i64 @sn_chan_recv_ok(ptr {b}, ptr {out_val})"));
                    let raw = self.t();
                    self.line(&format!("{raw} = load i64, ptr {out_val}"));
                    let val = self.from_bits(raw, t);
                    let ts = vec![*t.clone(), Type::Bool];
                    let st = tuple_ll(&ts);
                    let agg = self.t();
                    self.line(&format!("{agg} = insertvalue {st} zeroinitializer, {} {val}, 0", llty(t)));
                    let res = self.t();
                    self.line(&format!("{res} = insertvalue {st} {agg}, i64 {ok_i64}, 1"));
                    (res, Type::Tuple(ts))
                } else {
                    let bits = self.t();
                    self.line(&format!("{bits} = call i64 @sn_chan_recv(ptr {b})"));
                    (self.from_bits(bits, t), *t.clone())
                }
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
            (Type::Pool(elem), "insert") => {
                let (v, vt) = vs[0].clone();
                let r = self.t();
                if let Type::Record(rn) = &vt {
                    let sz = self.db.records.get(rn).map(|rec| rec.size).unwrap_or(8);
                    self.line(&format!("{r} = call i64 @sn_pool_insert_rec(ptr {b}, ptr {v}, i64 {sz})"));
                } else {
                    let vb = self.as_bits(v, elem);
                    self.line(&format!("{r} = call i64 @sn_pool_insert_i64(ptr {b}, i64 {vb})"));
                }
                (r, Type::Handle(elem.clone()))
            }
            (Type::Pool(elem), "get") => {
                let h = vs[0].0.clone();
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_pool_get_i64(ptr {b}, i64 {h})"));
                let val = self.from_bits(r, elem);
                (val, (**elem).clone())
            }
            (Type::Pool(elem), "get_ref") => {
                let h = vs[0].0.clone();
                let r = self.t();
                self.line(&format!("{r} = call ptr @sn_pool_get_ptr(ptr {b}, i64 {h})"));
                (r, Type::Ref(elem.clone()))
            }
            (Type::Pool(elem), "set") => {
                let h = vs[0].0.clone();
                let (v, _) = vs[1].clone();
                let vb = self.as_bits(v, elem);
                self.line(&format!("call void @sn_pool_set_i64(ptr {b}, i64 {h}, i64 {vb})"));
                ("0".into(), Type::Void)
            }
            (Type::Pool(_), "remove") => {
                let h = vs[0].0.clone();
                self.line(&format!("call void @sn_pool_remove(ptr {b}, i64 {h})"));
                ("0".into(), Type::Void)
            }
            (Type::Pool(_), "is_valid") => {
                let h = vs[0].0.clone();
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_pool_is_valid(ptr {b}, i64 {h})"));
                (r, Type::Bool)
            }
            (Type::Pool(_), "length") | (Type::Pool(_), "len") => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_pool_len(ptr {b})"));
                (r, Type::Int)
            }
            (Type::Blueprint(bp), m) => {
                let sig = self.lookup_blueprint_method(bp, m);
                let Some(sig) = sig else {
                    return ("0".into(), Type::Void);
                };
                if sig.is_static {
                    return self.direct_method_call(&b, &sig, &vs, true);
                }

                // Method Devirtualization:
                // Detect when receiver's concrete type is known at compile time:
                // 1. From local variable type/instantiation (`new Name(...)`), or
                // 2. Blueprint has no overriding subclasses for method `m`.
                let known_concrete_bp = match &base.kind {
                    ExprKind::Ident(id) => self.concrete_types.get(id).cloned(),
                    ExprKind::Self_ => self.current_bp.clone(),
                    _ => None,
                };

                let devirtualize = if known_concrete_bp.is_some() {
                    true
                } else {
                    !has_overriding_subclasses(self.db, bp, m)
                };

                if devirtualize {
                    let target_bp = known_concrete_bp.as_deref().unwrap_or(bp);
                    if let Some(target_sig) = self.lookup_blueprint_method(target_bp, m) {
                        let is_abstract = target_sig
                            .blueprint
                            .as_ref()
                            .and_then(|owner| self.db.blueprints.get(owner))
                            .map(|info| info.abstract_methods.iter().any(|am| am == m))
                            .unwrap_or(false);
                        if !is_abstract {
                            return self.direct_method_call(&b, &target_sig, &vs, false);
                        }
                    }
                }

                self.vcall(&b, m, &sig, &vs)
            }
            (Type::Contract(c), m) => {
                let sig = self.lookup_contract_method(c, m);
                let Some(sig) = sig else {
                    return ("0".into(), Type::Void);
                };
                if let ExprKind::Ident(id) = &base.kind {
                    if let Some(concrete_bp) = self.concrete_types.get(id).cloned() {
                        if let Some(target_sig) = self.lookup_blueprint_method(&concrete_bp, m) {
                            let is_abstract = target_sig
                                .blueprint
                                .as_ref()
                                .and_then(|owner| self.db.blueprints.get(owner))
                                .map(|info| info.abstract_methods.iter().any(|am| am == m))
                                .unwrap_or(false);
                            if !is_abstract {
                                return self.direct_method_call(&b, &target_sig, &vs, false);
                            }
                        }
                    }
                }
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
            if self.db.is_algebraic_enum(en) {
                if let Some(vars) = self.db.enums.get(en).cloned() {
                    if let Some(idx) = vars.iter().position(|v| v == name) {
                        let obj = self.t();
                        self.line(&format!("{obj} = call ptr @sn_obj_new(i64 24, i64 0, ptr inttoptr (i64 {idx} to ptr))"));
                        return (obj, Type::Enum(en.clone()));
                    }
                }
            } else if let Some(vars) = self.db.enums.get(en).cloned() {
                if let Some(idx) = vars.iter().position(|v| v == name) {
                    return (format!("{}", idx as i64), Type::Enum(en.clone()));
                }
            }
        }
        let (b, bt) = self.expr(base);
        match (&bt, name) {
            (Type::Str, "length") | (Type::List(_), "length") => {
                self.method(base, "length", &[], base.span)
            }
            (Type::Chan(_), "open") => {
                let r = self.t();
                self.line(&format!("{r} = call i64 @sn_chan_open(ptr {b})"));
                (r, Type::Bool)
            }
            (Type::Error, "message") => self.method(base, "message", &[], base.span),
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
                let tag_val = if self.db.is_algebraic_enum(en) {
                    let tag_ptr = self.t();
                    let tv = self.t();
                    self.line(&format!("{tag_ptr} = getelementptr inbounds i8, ptr {v}, i64 16"));
                    self.line(&format!("{tv} = load i64, ptr {tag_ptr}"));
                    tv
                } else {
                    v.clone()
                };
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
                    self.line(&format!("switch i64 {tag_val}, label %{def_lbl} [ {} ]", cases.join(" ")));
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
                    self.line(&format!("{unk_ptr} = call ptr @sn_str_from_i64(i64 {tag_val})"));
                    incoming.push(format!("[ {unk_ptr}, %{def_lbl} ]"));
                    self.line(&format!("br label %{sw_end}"));
                    self.raw(&format!("{sw_end}:"));
                    self.line(&format!("{phi_res} = phi ptr {}", incoming.join(", ")));
                    return phi_res;
                } else {
                    self.line(&format!("{r} = call ptr @sn_str_from_i64(i64 {tag_val})"));
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
        if from != to && *to == Type::Float {
            if let Type::Dec(s) = from {
                let mut p = 1.0f64;
                for _ in 0..*s {
                    p *= 10.0;
                }
                let raw_fp = self.t();
                self.line(&format!("{raw_fp} = sitofp i64 {v} to double"));
                let r = self.t();
                self.line(&format!("{r} = fdiv double {raw_fp}, {p:?}"));
                return (r, to.clone());
            }
            if from.int_min_max().is_some() || *from == Type::Int {
                let r = self.t();
                self.line(&format!("{r} = sitofp i64 {v} to double"));
                return (r, to.clone());
            }
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
        if let Type::Enum(en) = t {
            if self.db.is_algebraic_enum(en) {
                self.line(&format!("{c} = call i64 @sn_enum_eq(ptr {a}, ptr {b})"));
                return c;
            }
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

    /* SWARM RABBIT HOLE — Wave 14 (Impl-S, Cohort Delta): the SFC
       intrinsics' int-arg reader — the coordinates/keys/codes ride as
       i64 (the as_bits pass-through; the collection args go through
       as_ptr instead — the lane-backed header pointer). */
    fn sfc_i(&mut self, vs: &[(String, Type)], i: usize) -> String {
        let a = vs.get(i).cloned().unwrap_or(("0".into(), Type::Int));
        self.as_bits(a.0, &a.1)
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

    // -----------------------------------------------------------------
    // SWARM RABBIT HOLE — Wave 15 (Impl-T, Cohort Delta): Salt ε —
    // the degraded contract assertions' lowering. The engine is
    // compile-time; these are the ONLY contract artifacts the runtime
    // ever sees (the boundary rule's typed runtime assertions, each
    // carrying its atom text and classification, lowered through the
    // fail-fast convention: sn_contract_assert = fprintf + exit(1),
    // the E-LKIND/E-SFC pattern — never a silent drop).
    // -----------------------------------------------------------------

    /// One assertion: the condition value (an i64 bool per the
    /// comparison lowering) + the interned fail-fast message (the
    /// raw @.str global is already NUL-terminated — no SN string
    /// needed for a fprintf).
    fn contract_assert(&mut self, cond_val: &str, msg: &str) {
        let (id, _n) = self.intern(msg);
        self.line(&format!(
            "call void @sn_contract_assert(i64 {cond_val}, ptr @.str.{id})"
        ));
    }

    /// The fn-exit check: bind `result` (when the fn returns a value
    /// and some degraded atom references it), evaluate each degraded
    /// ensures atom, assert. Called at every return and at the
    /// fall-through exit.
    fn contract_exit_check(&mut self, ret_value: Option<(String, Type)>) {
        let checks = self.contract_exit.clone();
        if checks.is_empty() {
            return;
        }
        let mut saved_result: Option<Option<(String, Type)>> = None;
        if checks.iter().any(|ck| expr_refs_result(&ck.expr)) {
            let rt = self.current_fn_ret.clone();
            match ret_value {
                Some((v, t)) => {
                    let (vc, _) = self.coerce(v, &t, &rt);
                    self.line(&format!(
                        "store {} {vc}, ptr %v_contract_result",
                        llty(&rt)
                    ));
                }
                None => {
                    // the fall-through default (int fns: 0)
                    self.line(&format!(
                        "store {} 0, ptr %v_contract_result",
                        llty(&rt)
                    ));
                }
            }
            saved_result = Some(
                self.locals
                    .insert("result".into(), ("%v_contract_result".into(), rt)),
            );
        }
        for ck in checks {
            let (v, _) = self.expr(&ck.expr);
            self.contract_assert(&v, &ck.message);
        }
        if let Some(old) = saved_result {
            match old {
                Some((a, t)) => {
                    self.locals.insert("result".into(), (a, t));
                }
                None => {
                    self.locals.remove("result");
                }
            }
        }
    }

    /// The call-site check: bind each atom-referenced callee param to
    /// its ALREADY-EVALUATED argument value (one alloca per param —
    /// the argument expression is never re-evaluated), evaluate each
    /// degraded requires atom, assert, restore the caller's bindings.
    fn contract_site_asserts(
        &mut self,
        sig: &FuncSig,
        vs: &[(String, Type)],
        checks: &[crate::check::ContractCheck],
    ) {
        if checks.is_empty() {
            return;
        }
        let mut saved: Vec<(String, Option<(String, Type)>)> = Vec::new();
        for (i, p) in sig.params.iter().enumerate() {
            if !checks.iter().any(|ck| expr_refs_name(&ck.expr, &p.0)) {
                continue;
            }
            let Some((v, t)) = vs.get(i) else { continue };
            // the alloca name carries the temp counter: two call
            // sites binding the same param name in one function must
            // not collide (each site's alloca is its own slot).
            self.tmp += 1;
            let uniq = self.tmp;
            let an = format!("%v_contract_p_{}_{uniq}", p.0);
            self.emit_alloca(&format!("{an} = alloca {}", llty(&p.1)));
            let (vc, _) = self.coerce(v.clone(), t, &p.1);
            self.line(&format!("store {} {vc}, ptr {an}", llty(&p.1)));
            saved.push((p.0.clone(), self.locals.insert(p.0.clone(), (an, p.1.clone()))));
        }
        for ck in checks {
            let (v, _) = self.expr(&ck.expr);
            self.contract_assert(&v, &ck.message);
        }
        for (name, old) in saved {
            match old {
                Some((a, t)) => {
                    self.locals.insert(name, (a, t));
                }
                None => {
                    self.locals.remove(&name);
                }
            }
        }
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
        self.exit_active_scopes();
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
        self.emit_alloca(&format!("{ob} = alloca ptr"));
        self.emit_alloca(&format!("{oe} = alloca ptr"));
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
                Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => vs.push(self.expr(e)),
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
                allocas: Vec::new(),
                tmp: 0,
                lbl: 0,
                locals: HashMap::new(),
                concrete_types: HashMap::new(),
                loops: Vec::new(),
                loop_locals: std::collections::HashSet::new(),
                strings: self.strings,
                extra_fns: &mut nested,
                current_bp: self.current_bp.clone(),
                fn_key: String::new(),
                files: self.files,
                cur_loc: None,
                cur_sub: child_sub,
                dbg: self.dbg,
                terminated: false,
                current_fn_ret: ret.clone(),
                defers: Vec::new(),
                current_nursery: None,
                active_locks: Vec::new(),
                active_nurseries: Vec::new(),
                active_regions: Vec::new(),
                frozen_depth: 0,
                wave_stack: Vec::new(),
                // Wave 15 (Impl-T): synthetic child context — no
                // fn-level contract lowering (a closure/spawn body is
                // not the contracted function).
                contract_entry: Vec::new(),
                contract_exit: Vec::new(),
            };
            let mut pll = vec!["ptr %env".to_string()];
            for p in params {
                let t = resolve_type_ast(&p.ty, self.db);
                pll.push(format!("{} %p_{}", llty(&t), p.name));
            }
            for (i, (n, t)) in cap_tys.iter().enumerate() {
                let slot = child.t();
                child.line(&format!(
                    "{slot} = getelementptr i8, ptr %env, i64 {}",
                    i * 8
                ));
                let an = format!("%v_{n}");
                child.emit_alloca(&format!("{an} = alloca {}", llty(t)));
                let v = child.t();
                child.line(&format!("{v} = load {}, ptr {slot}", llty(t)));
                child.line(&format!("store {} {v}, ptr {an}", llty(t)));
                child.locals.insert(n.clone(), (an, t.clone()));
            }
            for p in params {
                let t = resolve_type_ast(&p.ty, self.db);
                let an = format!("%v_{}", p.name);
                child.emit_alloca(&format!("{an} = alloca {}", llty(&t)));
                child.line(&format!("store {} %p_{}, ptr {an}", llty(&t), p.name));
                child.locals.insert(p.name.clone(), (an, t));
            }
            for s in body {
                child.stmt(s);
            }
            child.emit_defers();
            child.release_active_locks();
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
            let mut out = format!(
                "define {} @{fname}({}) !dbg !{child_sub} {{\nentry:\n",
                llty_ret(ret),
                pll.join(", ")
            );
            for a in &child.allocas {
                out.push_str(a);
                out.push('\n');
            }
            out.push_str(&child.buf);
            out.push_str("}\n\n");
            out
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
        allocas: Vec::new(),
        tmp: 0,
        lbl: 0,
        locals: HashMap::new(),
        concrete_types: HashMap::new(),
        loops: Vec::new(),
        loop_locals: std::collections::HashSet::new(),
        strings,
        extra_fns: extra,
        current_bp: None,
        fn_key: String::new(),
        files,
        cur_loc: None,
        cur_sub: sub_id,
        dbg,
        terminated: false,
        current_fn_ret: ret_ty.clone(),
        defers: Vec::new(),
        current_nursery: None,
        active_locks: Vec::new(),
        active_nurseries: Vec::new(),
        active_regions: Vec::new(),
        frozen_depth: 0,
        wave_stack: Vec::new(),
        // Wave 15 (Impl-T): the contract lowering tables — filled
        // per function in lower_fn (and empty for the synthetic
        // child contexts; the call-site checks ride ident_call via
        // db.contract_site_checks on whichever context lowers them).
        contract_entry: Vec::new(),
        contract_exit: Vec::new(),
    };
    let mut params_ll = vec!["ptr %p_self".to_string()];
    for p in &f.params {
        let t = resolve_type_ast(&p.ty, db);
        params_ll.push(format!("{} %p_{}", llty(&t), p.name));
    }
    let rty = llty_ret(&ret_ty);
    cx.locals
        .insert("self".into(), ("%p_self".into(), self_ty.clone()));
    for p in &f.params {
        let t = resolve_type_ast(&p.ty, db);
        let an = format!("%v_{}", p.name);
        if let Type::Record(rn) = &t {
            let sz = db.records.get(rn).map(|r| r.size).unwrap_or(8);
            cx.emit_alloca(&format!("{an} = alloca i8, i64 {sz}"));
            cx.line(&format!("call void @sn_record_copy(ptr {an}, ptr %p_{}, i64 {sz})", p.name));
        } else {
            cx.emit_alloca(&format!("{an} = alloca {}", llty(&t)));
            cx.line(&format!("store {} %p_{}, ptr {an}", llty(&t), p.name));
        }
        cx.locals.insert(p.name.clone(), (an, t));
    }
    for s in &f.body {
        cx.stmt(s);
    }
    cx.exit_active_scopes();
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
    let mut out = format!(
        "define {rty} @{llvm_name}({}) !dbg !{sub_id} {{\nentry:\n",
        params_ll.join(", ")
    );
    for a in &cx.allocas {
        out.push_str(a);
        out.push('\n');
    }
    out.push_str(&cx.buf);
    out.push_str("}\n\n");
    out
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

/// SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): the label of a link
/// intrinsic's argument expression — the {src}/{dst}/{xs} fills of the
/// verbatim E0801 diagnostics carry the call site's own names
/// (`a[0]`, `b[0]`, `xs`).
fn link_arg_label(args: &[Arg], k: usize) -> String {
    fn go(e: &Expr) -> String {
        match &e.kind {
            ExprKind::Int(v) => v.to_string(),
            ExprKind::Ident(s) => s.clone(),
            ExprKind::Index { base, index } => format!("{}[{}]", go(base), go(index)),
            ExprKind::Member { base, name } => format!("{}.{}", go(base), name),
            _ => "expr".to_string(),
        }
    }
    match args.get(k) {
        Some(Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. }) => go(e),
        None => "expr".to_string(),
    }
}

fn llty(t: &Type) -> &'static str {
    match t {
        Type::Record(_) => "ptr",
        Type::Named(n) if n == "future" => "ptr",
        Type::Float => "double",
        Type::Enum(en) if is_algebraic_enum_type(en) => "ptr",
        // SWARM RABBIT HOLE — Wave 11 / DIFF-5 (Impl-P): link<T, u16> is
        // the 2-byte delta field itself.
        Type::Link(_) => "i16",
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
        Stmt::Defer { body, .. }
        | Stmt::ErrDefer { body, .. }
        | Stmt::Region { body, .. }
        | Stmt::NurseryBlock { body, .. }
        | Stmt::Frozen { body, .. }
        // SWARM RABBIT HOLE — Wave 10 / DIFF-4 (Impl-O): the window
        // body's idents are captured (the pool itself is a global —
        // not in `locals`, so spawn_block never boxes it; the body's
        // locals are).
        | Stmt::Unfrozen { body, .. }
        | Stmt::SpawnBlock { body, .. } => collect_idents(body, out),
        Stmt::Guard { cond, else_body, .. } => {
            walk_expr(cond, out);
            collect_idents(else_body, out);
        }
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
        ExprKind::Self_ => out.push("self".into()),
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
                    Arg::Pos(e) | Arg::Inout(e) | Arg::Named { value: e, .. } => walk_expr(e, out),
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
