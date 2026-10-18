//! Compiled frames (v0.8 accept 3): each compiled function's frame
//! description, and the traceback entry an exception records as it reaches
//! a compiled function.
//!
//! Every function with source positions (`MirStmt::Pos` markers) gets a
//! `PyccFnInfo` record (layout below, mirrored by `pycc_rt::frames` and the
//! `--ext` shim's `frames.c`): its name, qualified name, file, first line
//! and position table. An `--ext` module's shim builds a code object per
//! record at import, whose instruction `k` (after its `RESUME`) is located
//! at position `k`, so CPython's own traceback machinery prints a compiled
//! frame as it prints a Python one.
//!
//! A position marker costs nothing in the emitted code by itself: it only
//! changes what codegen considers the current position. The exception
//! edges that leave a statement (a raising operation's guard, a `raise`)
//! go through a small block per (target, position) that calls
//! `pycc_rt_tb_here(info, position, activation)` before branching to the
//! target, so a
//! traceback entry is recorded only on the exception path, exactly where
//! CPython's eval loop adds one: when an exception is raised in, or
//! propagates into, a frame. A re-raise (`raise` with no operand, leaving a
//! `finally` or an unmatched handler) branches to its target directly and
//! records nothing, as CPython's `RERAISE` does.
//!
//! In an `--ext` module, a function that can run CPython code
//! (`frame_needs`) also runs with a live frame: its prologue links a
//! CPython frame for it onto the thread's frame chain
//! (`pycc_ext_frame_enter`), each statement marker points that frame at
//! its position (one store), and every return unlinks it
//! (`pycc_ext_frame_exit`), so `sys._getframe`, `inspect` and `warnings`
//! see it as they see a Python function's frame. A traceback entry of
//! such a frame holds that very frame object.

use crate::RtFns;
use inkwell::basic_block::BasicBlock;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::values::{FunctionValue, GlobalValue};
use pycc_hir::SrcPos;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

/// `PyccFnInfo.flags`: the module body (`<module>`, whose `f_locals` is its
/// globals).
pub(crate) const FN_INFO_MODULE: u32 = 1;
/// `PyccFnInfo.flags`: a generator function (`CO_GENERATOR`).
pub(crate) const FN_INFO_GENERATOR: u32 = 2;

/// The runtime functions and per-module state of compiled frames.
pub(crate) struct FrameState<'ctx> {
    /// `pycc_rt_tb_here(info: *const PyccFnInfo, position: u32,
    /// activation: *const u8)`.
    tb_here: FunctionValue<'ctx>,
    /// The display path of each source file, by `SrcPos::file`.
    files: RefCell<Vec<String>>,
    /// A native program's source block per position (`CompileOptions`).
    blocks: RefCell<HashMap<SrcPos, String>>,
    /// Each statement marker's sites (`pycc_hir::positions::position_sites`).
    sites: RefCell<Vec<Vec<pycc_hir::positions::Site>>>,
    /// The function being emitted.
    current: RefCell<Option<FnFrame<'ctx>>>,
    /// Every record made, in order (`finish_module`'s table).
    infos: RefCell<Vec<GlobalValue<'ctx>>>,
    /// Whether records are made at all (a module compiled with no file
    /// table, e.g. a codegen unit test, has none).
    enabled: Cell<bool>,
    /// An `--ext` module's functions that run with a live frame
    /// (`frame_needs`); empty elsewhere.
    framed: RefCell<HashSet<String>>,
    /// Whether the module body runs with one (an `--ext` module).
    framed_module: Cell<bool>,
    /// Whether compiled functions read published globals through the
    /// module dict (`global_sync`), and whether the module body (which
    /// does not) is being emitted.
    global_sync: Cell<bool>,
    module_body: Cell<bool>,
    /// The `--ext` shim's `pycc_ext_frame_enter(frame, info)`,
    /// `pycc_ext_frame_exit(frame)` and
    /// `pycc_ext_tb_here_live(info, position, frame)` (`frames.c`).
    frame_enter: FunctionValue<'ctx>,
    frame_exit: FunctionValue<'ctx>,
    tb_here_live: FunctionValue<'ctx>,
}

/// A live frame: the `PyccIFrame` the prologue linked (an entry-block
/// `alloca`) and the code pointer of position 0 (`PyccFnInfo.ip0`).
#[derive(Clone, Copy)]
struct LiveFrame<'ctx> {
    frame: inkwell::values::PointerValue<'ctx>,
    ip0: inkwell::values::PointerValue<'ctx>,
    function: FunctionValue<'ctx>,
}

/// The function being emitted.
struct FnFrame<'ctx> {
    info: GlobalValue<'ctx>,
    name: String,
    qualname: String,
    flags: u32,
    /// From the first marker: the header of a `def`, the first statement
    /// of a module body.
    file: Option<u32>,
    firstlineno: u32,
    positions: Vec<SrcPos>,
    index: HashMap<SrcPos, u32>,
    current: Option<u32>,
    edges: HashMap<(BasicBlock<'ctx>, u32), BasicBlock<'ctx>>,
    live: Option<LiveFrame<'ctx>>,
    /// The current statement's marker, and how many sites of each key its
    /// emission has entered.
    statement: Option<SrcPos>,
    counters: HashMap<String, u32>,
    /// The positions [`enter_site`] replaced, innermost last (`None`: the
    /// operation had no site and changed nothing).
    site_stack: Vec<Option<Option<u32>>>,
    /// An entry-block byte whose address tells this call of the function
    /// from any other (`pycc_rt_tb_here`'s `activation`): the live frame's
    /// own storage when it has one.
    activation: Option<inkwell::values::PointerValue<'ctx>>,
}

/// Marks `function` `cold`: a call of it is unlikely, so the blocks
/// leading to it (a traceback record, a global's re-read) are laid out
/// away from the function's hot path.
pub(crate) fn mark_cold<'ctx>(context: &'ctx Context, function: FunctionValue<'ctx>) {
    let kind = inkwell::attributes::Attribute::get_named_enum_kind_id("cold");
    function.add_attribute(
        inkwell::attributes::AttributeLoc::Function,
        context.create_enum_attribute(kind, 0),
    );
}

impl<'ctx> FrameState<'ctx> {
    pub(crate) fn declare(context: &'ctx Context, module: &Module<'ctx>) -> Self {
        let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
        let tb_here = module.get_function("pycc_rt_tb_here").unwrap_or_else(|| {
            module.add_function(
                "pycc_rt_tb_here",
                context.void_type().fn_type(
                    &[ptr_type.into(), context.i32_type().into(), ptr_type.into()],
                    false,
                ),
                Some(Linkage::External),
            )
        });
        let declare = |name: &str, params: &[inkwell::types::BasicMetadataTypeEnum<'ctx>]| {
            module.get_function(name).unwrap_or_else(|| {
                module.add_function(
                    name,
                    context.void_type().fn_type(params, false),
                    Some(Linkage::External),
                )
            })
        };
        let i32_type = context.i32_type();
        let tb_here_live = declare(
            "pycc_ext_tb_here_live",
            &[ptr_type.into(), i32_type.into(), ptr_type.into()],
        );
        mark_cold(context, tb_here);
        mark_cold(context, tb_here_live);
        Self {
            tb_here,
            files: RefCell::new(Vec::new()),
            blocks: RefCell::new(HashMap::new()),
            sites: RefCell::new(Vec::new()),
            current: RefCell::new(None),
            infos: RefCell::new(Vec::new()),
            enabled: Cell::new(false),
            framed: RefCell::new(HashSet::new()),
            framed_module: Cell::new(false),
            global_sync: Cell::new(false),
            module_body: Cell::new(false),
            frame_enter: declare("pycc_ext_frame_enter", &[ptr_type.into(), ptr_type.into()]),
            frame_exit: declare("pycc_ext_frame_exit", &[ptr_type.into()]),
            tb_here_live,
        }
    }

    /// Turns `global_sync` on (a non-strict module hosted by CPython).
    pub(crate) fn set_global_sync(&self, on: bool) {
        self.global_sync.set(on);
    }

    /// Whether the code being emitted reads published globals through the
    /// module dict: a compiled function of a module that does.
    pub(crate) fn syncs_globals(&self) -> bool {
        self.global_sync.get() && !self.module_body.get()
    }

    /// An `--ext` module's live frames: `framed` functions and the body.
    pub(crate) fn set_framed(&self, framed: HashSet<String>) {
        *self.framed.borrow_mut() = framed;
        self.framed_module.set(true);
    }

    /// Installs the file table (`SrcPos::file` -> display path) and turns
    /// records on.
    pub(crate) fn set_files(
        &self,
        files: Vec<String>,
        blocks: HashMap<SrcPos, String>,
        sites: Vec<Vec<pycc_hir::positions::Site>>,
    ) {
        self.enabled.set(!files.is_empty());
        *self.files.borrow_mut() = files;
        *self.blocks.borrow_mut() = blocks;
        *self.sites.borrow_mut() = sites;
    }
}

/// The frame names of the compiled function `mangled`: `co_name` and
/// `co_qualname`.
pub(crate) fn frame_names(mangled: &str) -> (String, String) {
    let mut qualname = source_qualname(&pycc_hir::function_qualname(mangled));
    if let Some(lifted) = lifted_method(&qualname) {
        qualname = lifted;
    }
    let name = qualname
        .rsplit('.')
        .next()
        .unwrap_or(qualname.as_str())
        .to_string();
    (name, qualname)
}

/// `pycc_mir`'s frame-name decomposition of a mangled function name,
/// keeping the class: an inherited `super()` copy is its origin's method, a
/// `.static`/`.classmethod`/`.setter` suffix and a `0gen_` specialization
/// prefix go, and a dependency module's qualified spelling is the plain
/// name.
fn source_qualname(mangled: &str) -> String {
    let marker = format!(".{}", pycc_hir::SUPER_TARGET_MARKER);
    let m = mangled
        .split_once(marker.as_str())
        .map_or(mangled, |(copied, _)| copied);
    let m = m
        .strip_suffix(".classmethod")
        .or_else(|| m.strip_suffix(".static"))
        .or_else(|| m.strip_suffix(".setter"))
        .unwrap_or(m);
    let m = m.strip_prefix("0gen_").unwrap_or(m);
    match m.split_once('.') {
        Some((class, rest)) => format!("{}.{rest}", pycc_hir::namespace::plain_name(class)),
        None => pycc_hir::namespace::plain_name(m).to_string(),
    }
}

/// A CPython-run class's method body lifted to `__pycc_liftN_<name>__`
/// (`src/fallback/lift.rs`) is the method `name`.
fn lifted_method(name: &str) -> Option<String> {
    let rest = name.strip_prefix("__pycc_lift")?;
    let digits = rest.find(|c: char| !c.is_ascii_digit())?;
    let method = rest[digits..].strip_prefix('_')?.strip_suffix("__")?;
    (digits > 0 && !method.is_empty()).then(|| method.to_string())
}

/// Starts the record of the function `mangled` (or the module body,
/// `module_body`), whose body is about to be emitted.
pub(crate) fn begin_function<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    mangled: &str,
    flags: u32,
) {
    let state = &rt.frames;
    state.module_body.set(flags & FN_INFO_MODULE != 0);
    // A published signature's compiled entry (`pycc_ast::ext_signatures`)
    // only passes its wrapper's arguments on: the function it calls has
    // the frame, which links past the wrapper, so the entry has none and
    // no traceback entry either.
    // So does the body of a nested `def` CPython creates
    // (`src/fallback/nested`): its Python forwarder's frame is the
    // function's.
    if !state.enabled.get()
        || mangled.starts_with("_pycc_sig_")
        || mangled.starts_with("__pycc_lift_nest")
    {
        return;
    }
    let (name, qualname) = if flags & FN_INFO_MODULE != 0 {
        ("<module>".to_string(), "<module>".to_string())
    } else {
        frame_names(mangled)
    };
    let info = module.add_global(info_type(context), None, "pycc_fninfo");
    info.set_linkage(Linkage::Private);
    // A generator's body runs across resumes from its own heap frame, so it
    // gets no live frame (its traceback entries are built when needed).
    let live = if flags & FN_INFO_MODULE != 0 {
        state.framed_module.get()
    } else {
        flags & FN_INFO_GENERATOR == 0 && state.framed.borrow().contains(mangled)
    };
    let live = if live {
        enter_frame(context, builder, state, info)
    } else {
        None
    };
    let activation = match live {
        Some(live) => Some(live.frame),
        None => builder
            .build_alloca(context.i8_type(), "pycc_activation")
            .ok(),
    };
    *state.current.borrow_mut() = Some(FnFrame {
        info,
        name,
        qualname,
        flags,
        file: None,
        firstlineno: 0,
        positions: Vec::new(),
        index: HashMap::new(),
        current: None,
        edges: HashMap::new(),
        live,
        activation,
        statement: None,
        counters: HashMap::new(),
        site_stack: Vec::new(),
    });
}

/// The prologue of a function with a live frame, at the builder's
/// position in its entry block: the frame's storage, its link onto the
/// thread's chain, and the code pointer its positions are offsets from.
fn enter_frame<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    state: &FrameState<'ctx>,
    info: GlobalValue<'ctx>,
) -> Option<LiveFrame<'ctx>> {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let function = builder.get_insert_block()?.get_parent()?;
    // `PyccIFrame` is eleven words (CPython never reads its `localsplus`,
    // where `pycc_ext_frame_enter` keeps the thread state and the top it
    // replaced).
    let frame = builder
        .build_alloca(ptr.array_type(11), "pycc_frame")
        .expect("build_alloca should not fail for a frame");
    builder
        .build_call(
            state.frame_enter,
            &[frame.into(), info.as_pointer_value().into()],
            "",
        )
        .expect("build_call should not fail for pycc_ext_frame_enter");
    let ip0_field = builder
        .build_struct_gep(info_type(context), info.as_pointer_value(), 4, "fninfo_ip0")
        .expect("build_struct_gep should not fail for PyccFnInfo.ip0");
    let ip0 = builder
        .build_load(ptr, ip0_field, "ip0")
        .expect("build_load should not fail for PyccFnInfo.ip0")
        .into_pointer_value();
    Some(LiveFrame {
        frame,
        ip0,
        function,
    })
}

/// `MirStmt::Pos`: makes `pos` the current position.
pub(crate) fn set_position<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    pos: SrcPos,
) {
    let mut current = rt.frames.current.borrow_mut();
    let Some(frame) = current.as_mut() else {
        return;
    };
    if frame.file.is_none() {
        frame.file = Some(pos.file);
        frame.firstlineno = pos.line;
    }
    if frame.file != Some(pos.file) {
        return;
    }
    frame.statement = Some(pos);
    frame.counters.clear();
    let index = frame.intern(pos);
    frame.current = Some(index);
    point_live_frame(context, builder, frame, index);
}

impl FnFrame<'_> {
    /// The index of `pos`'s location in this function's position table.
    fn intern(&mut self, pos: SrcPos) -> u32 {
        let pos = pos.location();
        let next = self.positions.len() as u32;
        let index = *self.index.entry(pos).or_insert(next);
        if index == next {
            self.positions.push(pos);
        }
        index
    }
}

/// The site key of the operation `expr` (`pycc_hir::positions`), if it is
/// one a statement records: a call by the callee's name, a subscript, an
/// attribute load by name, or a division.
pub(crate) fn site_key(expr: &pycc_mir::MirExpr) -> Option<String> {
    use pycc_mir::{BinOpKind, MirExpr};
    match expr {
        MirExpr::Call { callee, .. } => Some(frame_names(callee).0),
        MirExpr::Instantiate(inst) => Some(inst.class_name.clone()),
        MirExpr::ObjMethodCall { method, .. } => Some(method.clone()),
        MirExpr::ObjCall { callee, .. } => match callee.as_ref() {
            MirExpr::Name { name, .. } => Some(pycc_hir::namespace::plain_name(name).to_string()),
            _ => None,
        },
        MirExpr::ObjKeywordCall(call) => site_key(&call.call),
        MirExpr::Subscript { .. } | MirExpr::ObjSubscript { .. } | MirExpr::DictGet { .. } => {
            Some("[]".to_string())
        }
        MirExpr::ObjAttrGet { attr, .. } => Some(format!(".{attr}")),
        MirExpr::BinOp {
            op: BinOpKind::Div | BinOpKind::FloorDiv | BinOpKind::Mod,
            ..
        }
        | MirExpr::ObjBinOp {
            op: BinOpKind::Div | BinOpKind::FloorDiv | BinOpKind::Mod,
            ..
        } => Some("/".to_string()),
        _ => None,
    }
}

/// Before emitting the operation `key` names: makes the current
/// statement's next site with that key the current position, until the
/// matching [`leave_site`].
pub(crate) fn enter_site<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    key: Option<String>,
) {
    let mut current = rt.frames.current.borrow_mut();
    let Some(frame) = current.as_mut() else {
        return;
    };
    let site = key.and_then(|key| {
        let statement = frame.statement?;
        let count = frame.counters.entry(key.clone()).or_insert(0);
        let nth = *count as usize;
        *count += 1;
        let sites = rt.frames.sites.borrow();
        sites
            .get(statement.sites as usize)?
            .iter()
            .filter(|(k, _)| *k == key)
            .nth(nth)
            .map(|(_, pos)| *pos)
    });
    let Some(site) = site else {
        frame.site_stack.push(None);
        return;
    };
    frame.site_stack.push(Some(frame.current));
    let index = frame.intern(site);
    let moved = frame.statement.is_some_and(|s| s.line != site.line);
    frame.current = Some(index);
    if moved {
        point_live_frame(context, builder, frame, index);
    }
}

/// After the operation [`enter_site`] entered (its guard included): the
/// statement's own position again.
pub(crate) fn leave_site<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
) {
    let mut current = rt.frames.current.borrow_mut();
    let Some(frame) = current.as_mut() else {
        return;
    };
    let Some(Some(saved)) = frame.site_stack.pop() else {
        return;
    };
    let moved = frame.current.map(|i| frame.positions[i as usize].line)
        != saved.map(|i| frame.positions[i as usize].line);
    frame.current = saved;
    if moved && let Some(index) = saved {
        point_live_frame(context, builder, frame, index);
    }
}

/// Points a live frame at position `index` (one store), so `f_lineno`
/// reads it while the function calls out.
fn point_live_frame<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    frame: &FnFrame<'ctx>,
    index: u32,
) {
    if let Some(live) = frame.live
        && builder
            .get_insert_block()
            .is_some_and(|block| block.get_terminator().is_none())
    {
        // `frame.instr_ptr = ip0 + index` (`PyccIFrame.instr_ptr`, word 7).
        let ip = unsafe {
            builder.build_in_bounds_gep(
                context.i16_type(),
                live.ip0,
                &[context.i64_type().const_int(u64::from(index), false)],
                "frame_ip",
            )
        }
        .expect("build_in_bounds_gep should not fail for a code pointer");
        let slot = unsafe {
            builder.build_in_bounds_gep(
                context.ptr_type(inkwell::AddressSpace::default()),
                live.frame,
                &[context.i64_type().const_int(7, false)],
                "frame_instr_ptr",
            )
        }
        .expect("build_in_bounds_gep should not fail for a frame field");
        builder
            .build_store(slot, ip)
            .expect("build_store should not fail for a frame's instr_ptr");
    }
}

/// Where an exception edge leaving the current statement branches to reach
/// `target`: a block recording this frame's traceback entry first, or
/// `target` itself outside any record.
pub(crate) fn exception_edge<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    target: BasicBlock<'ctx>,
) -> BasicBlock<'ctx> {
    let mut current = rt.frames.current.borrow_mut();
    let Some(frame) = current.as_mut() else {
        return target;
    };
    let Some(position) = frame.current else {
        return target;
    };
    if let Some(&edge) = frame.edges.get(&(target, position)) {
        return edge;
    }
    let Some(function) = target.get_parent() else {
        return target;
    };
    let resume = builder.get_insert_block();
    let edge = context.append_basic_block(function, "tb_here");
    builder.position_at_end(edge);
    let info = frame.info.as_pointer_value();
    let position_value = context.i32_type().const_int(u64::from(position), false);
    match frame.live {
        // The live frame itself goes into the traceback.
        Some(live) => builder
            .build_call(
                rt.frames.tb_here_live,
                &[info.into(), position_value.into(), live.frame.into()],
                "",
            )
            .expect("build_call should not fail for pycc_ext_tb_here_live"),
        None => {
            let activation = frame.activation.unwrap_or_else(|| {
                context
                    .ptr_type(inkwell::AddressSpace::default())
                    .const_null()
            });
            builder
                .build_call(
                    rt.frames.tb_here,
                    &[info.into(), position_value.into(), activation.into()],
                    "",
                )
                .expect("build_call should not fail for pycc_rt_tb_here")
        }
    };
    builder
        .build_unconditional_branch(target)
        .expect("build_unconditional_branch should not fail for a fresh block");
    if let Some(resume) = resume {
        builder.position_at_end(resume);
    }
    frame.edges.insert((target, position), edge);
    edge
}

/// Adds this frame's traceback entry, at the current position, to the
/// CPython exception set now: an `--ext` module body's failed CPython
/// operation, which fails the import directly with that exception
/// (`pycc_ext_tb_here_raised` in the shim's `frames.c`).
pub(crate) fn record_cpython_raise<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
) {
    let current = rt.frames.current.borrow();
    let Some(frame) = current.as_ref() else {
        return;
    };
    let Some(position) = frame.current else {
        return;
    };
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    let hook = module
        .get_function("pycc_ext_tb_here_raised")
        .unwrap_or_else(|| {
            module.add_function(
                "pycc_ext_tb_here_raised",
                context
                    .void_type()
                    .fn_type(&[ptr_type.into(), context.i32_type().into()], false),
                Some(Linkage::External),
            )
        });
    builder
        .build_call(
            hook,
            &[
                frame.info.as_pointer_value().into(),
                context
                    .i32_type()
                    .const_int(u64::from(position), false)
                    .into(),
            ],
            "",
        )
        .expect("build_call should not fail for pycc_ext_tb_here_raised");
}

/// `struct PyccFnInfo` (`pycc_rt::frames::FnInfo`):
/// `{ code, func, globals, builtins, ip0, name, qualname, filename,
/// positions: *const [u32; 4], npos: u32, firstlineno: u32, flags: u32,
/// file: u32, texts: *const *const c_char }`. The first five fields are
/// the `--ext` shim's, filled at import; positions hold `line, end_line,
/// col, end_col`; `texts` (null, or one per position, each possibly null)
/// are a native program's source blocks.
fn info_type(context: &Context) -> inkwell::types::StructType<'_> {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i32_type = context.i32_type();
    context.struct_type(
        &[
            ptr.into(),
            ptr.into(),
            ptr.into(),
            ptr.into(),
            ptr.into(),
            ptr.into(),
            ptr.into(),
            ptr.into(),
            ptr.into(),
            i32_type.into(),
            i32_type.into(),
            i32_type.into(),
            i32_type.into(),
            ptr.into(),
        ],
        false,
    )
}

fn c_string<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    text: &str,
) -> inkwell::values::PointerValue<'ctx> {
    let bytes = text.as_bytes();
    let global = module.add_global(
        context.i8_type().array_type(bytes.len() as u32 + 1),
        None,
        "pycc_fninfo_str",
    );
    global.set_initializer(&context.const_string(bytes, true));
    global.set_constant(true);
    global.set_linkage(Linkage::Private);
    global.as_pointer_value()
}

/// Finishes the current function's record: writes its initializer, or
/// drops it when the body had no position.
pub(crate) fn end_function<'ctx>(context: &'ctx Context, module: &Module<'ctx>, rt: &RtFns<'ctx>) {
    let Some(frame) = rt.frames.current.borrow_mut().take() else {
        return;
    };
    if let Some(live) = frame.live {
        exit_frame(context, rt, live);
    }
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let i32_type = context.i32_type();
    let filename = frame
        .file
        .and_then(|file| rt.frames.files.borrow().get(file as usize).cloned())
        .unwrap_or_default();
    let words: Vec<_> = frame
        .positions
        .iter()
        .flat_map(|p| [p.line, p.end_line, p.col, p.end_col])
        .map(|w| i32_type.const_int(u64::from(w), false))
        .collect();
    let positions = module.add_global(
        i32_type.array_type(words.len() as u32),
        None,
        "pycc_fninfo_pos",
    );
    positions.set_initializer(&i32_type.const_array(&words));
    positions.set_constant(true);
    positions.set_linkage(Linkage::Private);
    let null = ptr.const_null();
    let texts = {
        let blocks = rt.frames.blocks.borrow();
        let entries: Vec<_> = frame
            .positions
            .iter()
            .map(|p| match blocks.get(p) {
                Some(text) => c_string(context, module, text),
                None => null,
            })
            .collect();
        if entries.iter().all(|entry| entry.is_null()) {
            null
        } else {
            let table = module.add_global(
                ptr.array_type(entries.len() as u32),
                None,
                "pycc_fninfo_texts",
            );
            table.set_initializer(&ptr.const_array(&entries));
            table.set_constant(true);
            table.set_linkage(Linkage::Private);
            table.as_pointer_value()
        }
    };
    let init = info_type(context).const_named_struct(&[
        null.into(),
        null.into(),
        null.into(),
        null.into(),
        null.into(),
        c_string(context, module, &frame.name).into(),
        c_string(context, module, &frame.qualname).into(),
        c_string(context, module, &filename).into(),
        positions.as_pointer_value().into(),
        i32_type
            .const_int(frame.positions.len() as u64, false)
            .into(),
        i32_type
            .const_int(u64::from(frame.firstlineno), false)
            .into(),
        i32_type.const_int(u64::from(frame.flags), false).into(),
        i32_type
            .const_int(u64::from(frame.file.unwrap_or(0)), false)
            .into(),
        texts.into(),
    ]);
    frame.info.set_initializer(&init);
    rt.frames.infos.borrow_mut().push(frame.info);
}

/// The epilogue of a function with a live frame: unlinks it before every
/// `ret` of the function, whichever path reaches it.
fn exit_frame<'ctx>(context: &'ctx Context, rt: &RtFns<'ctx>, live: LiveFrame<'ctx>) {
    let builder = context.create_builder();
    for block in live.function.get_basic_block_iter() {
        let Some(terminator) = block.get_terminator() else {
            continue;
        };
        if terminator.get_opcode() != inkwell::values::InstructionOpcode::Return {
            continue;
        }
        builder.position_before(&terminator);
        builder
            .build_call(rt.frames.frame_exit, &[live.frame.into()], "")
            .expect("build_call should not fail for pycc_ext_frame_exit");
    }
}

/// Emits the module's record table, `pycc_fn_info_table` (pointers) and
/// `pycc_fn_info_count`, which an `--ext` shim walks at import.
pub(crate) fn finish_module<'ctx>(context: &'ctx Context, module: &Module<'ctx>, rt: &RtFns<'ctx>) {
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let infos = rt.frames.infos.borrow();
    let entries: Vec<_> = infos.iter().map(|g| g.as_pointer_value()).collect();
    let table = module.add_global(
        ptr.array_type(entries.len() as u32),
        None,
        "pycc_fn_info_table",
    );
    table.set_initializer(&ptr.const_array(&entries));
    table.set_constant(true);
    let count = module.add_global(context.i64_type(), None, "pycc_fn_info_count");
    count.set_initializer(&context.i64_type().const_int(entries.len() as u64, false));
    count.set_constant(true);
}
