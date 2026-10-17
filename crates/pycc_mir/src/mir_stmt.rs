//! MIR statement and item nodes.

use super::*;

#[derive(Debug, Clone, PartialEq)]
pub enum MirStmt {
    ExprStmt(MirExpr),
    Assign {
        target: String,
        value: MirExpr,
    },
    /// A statement with zero runtime effect -- currently only produced by a
    /// value-less PEP 526 annotation (`x: int`), which CPython itself does
    /// nothing observable for either (confirmed empirically during PR-9
    /// planning: no store, no allocation, nothing an oracle diff could see).
    NoOp,
    /// `del name` of a generator-typed name: releases the reference the
    /// name holds (the last one closes the generator, running its
    /// `finally` clauses, as in CPython). Every other `del name` is static
    /// only (`MirStmt::NoOp`).
    ReleaseName(String),
    /// A continuation that the validated HIR proves cannot be reached. The
    /// match lowerer uses this for the no-case fallback of an exhaustive
    /// `match`: preserving the proof in MIR keeps later fallthrough analysis
    /// aligned with the type checker instead of inventing a synthetic `NoOp`
    /// path after all exhaustive alternatives failed.
    Unreachable,
    If {
        test: MirExpr,
        body: Vec<MirStmt>,
        orelse: Vec<MirStmt>,
    },
    While {
        test: MirExpr,
        body: Vec<MirStmt>,
    },
    ForRange {
        var: String,
        start: MirExpr,
        stop: MirExpr,
        step: MirExpr,
        body: Vec<MirStmt>,
    },
    /// `for var in list:` (mirrors `HirStmt::ForList`, D-105). `list` is
    /// carried as the plain variable name, exactly like `HirStmt::ForList`
    /// itself; there is no start/stop/step to lower here, only `body`.
    /// `var_ty` is the list's element type, which the target binds.
    ForList {
        var: String,
        var_ty: Ty,
        list: String,
        body: Vec<MirStmt>,
    },
    /// `for var in <object expression>:` (mirrors `HirStmt::ForObject`,
    /// PR 3c of #1082). Unlike every other `For*` node here the iterable is
    /// a lowered expression rather than a variable name, because the two
    /// admitted source shapes (`o.attr`, `o.method(...)`) are expressions.
    /// `var` holds each item as an opaque `PyObject *` (`Ty::Object`), and
    /// `pycc_codegen` drives the loop through `PyObject_GetIter` /
    /// `PyIter_Next` behind the D-244 shim.
    ForObject {
        var: String,
        iter: MirExpr,
        body: Vec<MirStmt>,
    },
    /// `d[k] = v` (mirrors `HirStmt::DictSet`, PR-11 Task 4/D-123). `dict` is
    /// carried as the plain variable name, exactly like `ForList`'s `list`
    /// field -- there is no sub-expression to
    /// recursively lower for it, only for `key`/`value`.
    DictSet {
        dict: String,
        key: MirExpr,
        value: MirExpr,
    },
    /// `b[i] = v` on a `memoryview`-typed base (Part 1 of #1142) -- the
    /// store counterpart of [`MirExpr::BufferGet`], and a **statement**
    /// rather than an expression because `pycc_hir` lowers every
    /// `<name>[k] = v` to `HirStmt::DictSet` and never to a `Subscript`
    /// store. Being a statement is load-bearing rather than incidental:
    /// `pycc_codegen`'s `expression_can_set_exception` is a `&MirExpr`
    /// predicate, so the `IndexError` this node can leave pending is
    /// observed by `guard_statement_effects` at the statement level and
    /// by nothing else.
    ///
    /// Unlike [`MirStmt::DictSet`] immediately above, `base` is a lowered
    /// `MirExpr` and not a bare name: codegen then reuses
    /// [`MirExpr::BufferGet`]'s own `Scalar::MemoryView` extraction rather
    /// than the dict-specific by-name read. `pycc_types` admits the store
    /// only on a bare `HirExpr::Name` base, so in practice this is always
    /// a [`MirExpr::Name`] -- carrying it as an expression is what keeps
    /// the load and the store on one codegen path.
    BufferSet {
        base: MirExpr,
        index: MirExpr,
        value: MirExpr,
    },
    /// `xs[i] = v` on a `list[T]`-typed name, split from `HirStmt::DictSet`
    /// on the base's type exactly as `BufferSet` is. `base` is always a
    /// [`MirExpr::Name`]; an out-of-range index raises `IndexError`.
    ListSet {
        base: MirExpr,
        index: MirExpr,
        value: MirExpr,
    },
    /// `for var in dict:` (mirrors `HirStmt::ForList` on a dict-typed base --
    /// see `lower_stmt`'s own `HirStmt::ForList` arm for why a dict-typed
    /// base is lowered to this node instead of `MirStmt::ForList`). `dict` is
    /// carried as the plain variable name, exactly like `ForList`'s `list`
    /// field.
    ForDict {
        var: String,
        /// The dict's key type, which codegen binds `var` as.
        var_ty: Ty,
        dict: String,
        body: Vec<MirStmt>,
    },
    /// `for var in set:` (mirrors `HirStmt::ForList` on a set-typed base --
    /// see `lower_stmt`'s own `HirStmt::ForList` arm for why a set-typed
    /// base is lowered to this node instead of `MirStmt::ForList`, PR-11
    /// Task 8, D-123). `set` is carried as the plain variable name, exactly
    /// like `ForList`'s `list` field and `ForDict`'s `dict` field.
    ///
    /// #1343: `var_ty` is the set's element type, `int` or a user-class
    /// instance, which codegen binds `var` as.
    ForSet {
        var: String,
        var_ty: Ty,
        set: String,
        body: Vec<MirStmt>,
    },
    /// `target = [elt for var in <source> [if cond]]`, already fully
    /// resolved (mirrors `HirStmt::ListCompAssign`, PR-12, D-117). `var_ty`
    /// is the loop variable's own resolved type (`Ty::Int` for a `Range`
    /// source; the iterated container's element/key type for a `List`/
    /// `Dict`/`Set` source) -- carried explicitly, rather than requiring
    /// `pycc_codegen` to re-derive it from `elt`, because `elt` need not
    /// contain any `Name` reference to `var` at all in general (though in
    /// practice it usually does) and re-deriving it by walking `elt` would
    /// be a second, independent computation of a fact `resolve_comp_source`
    /// (below) already has in hand once.
    ListCompAssign {
        target: String,
        var: String,
        var_ty: Ty,
        source: CompSource,
        cond: Option<Box<MirExpr>>,
        elt: Box<MirExpr>,
    },
    /// `target = {key: value for var in <source> [if cond]}`, already fully
    /// resolved (mirrors `HirStmt::DictCompAssign`, PR-12, D-117). Mirrors
    /// `ListCompAssign` exactly except for the key/value split.
    DictCompAssign {
        target: String,
        var: String,
        var_ty: Ty,
        source: CompSource,
        cond: Option<Box<MirExpr>>,
        key: Box<MirExpr>,
        value: Box<MirExpr>,
    },
    /// `target = {elt for var in <source> [if cond]}`, already fully
    /// resolved (mirrors `HirStmt::SetCompAssign`, PR-12, D-117). Mirrors
    /// `ListCompAssign` exactly -- a set comprehension's own shape is
    /// identical to a list comprehension's, differing only in which
    /// runtime constructor/insert pair `pycc_codegen` ends up calling.
    ///
    /// #1344 (Part 2 of #1336): `ops` is `None` for an `int` element and
    /// carries the instance class's hash and equality for a `set[C]`, as
    /// [`MirExpr::SetLiteral`]'s own `ops` does.
    SetCompAssign {
        target: String,
        var: String,
        var_ty: Ty,
        source: CompSource,
        cond: Option<Box<MirExpr>>,
        elt: Box<MirExpr>,
        ops: Option<Box<SetElementOps>>,
    },
    Return(Option<MirExpr>),
    /// `return b[start:stop]` where `b` is a `memoryview` **parameter**
    /// (Part 2 of #1175, #1179) -- the one buffer sub-range egress
    /// `pycc_types::buffer::admitted_buffer_return` admits.
    ///
    /// A node of its own rather than `Return(Some(MirExpr::Slice { .. }))`,
    /// and the reason is a miscompile rather than taste:
    /// `MirExpr::Slice::ty()` passes its base's type straight through, so a
    /// buffer base would answer `Ty::MemoryView` and reach
    /// `pycc_codegen`'s list-only `MirExpr::Slice` emit arm, which calls
    /// `pycc_rt_list_slice` -- a buffer compiled as a list. Keeping the
    /// admitted shape out of `MirExpr::Slice` entirely makes that
    /// unreachable by construction instead of by a comment, and leaves
    /// every grouped `MirExpr` catch-all that lists `MirExpr::Slice { .. }`
    /// wholesale (`bigint_rc.rs`, `exception.rs`) answering for the list
    /// slice only, which is the answer they were written for.
    ///
    /// `name` is the parameter, which the compiled body still returns *as a
    /// whole view*: the wrapper's `result == &a{index}` identity test is
    /// exact and stays exact. The bounds travel out of band through the
    /// export thunk's trailing out-pointers. An absent `start` means `0`
    /// and an absent `stop` means `i64::MAX`; both defaults are CPython's
    /// own clamping, measured through `PySlice_New`/`PyObject_GetItem`.
    /// A `step` never reaches here -- `buffer_slice_step_unsupported`
    /// refuses it in both type walkers.
    ReturnBufferSlice {
        name: String,
        start: Option<MirExpr>,
        stop: Option<MirExpr>,
    },
    /// Binds the results of a call of a multi-value twin
    /// (`crate::multi_return`) to fresh temporaries; see [`AssignMulti`].
    AssignMulti(Box<AssignMulti>),
    /// In a multi-value twin (`crate::multi_return`): stores a reference of
    /// its own to `value` through the twin's out-pointer `index` (from 1:
    /// result 0 is the twin's returned value). Only ever directly before
    /// the `Return` that hands back result 0, after every conversion, so
    /// nothing between the store and the return can raise.
    SetOut {
        index: usize,
        value: MirExpr,
    },
    /// `base.attr = value` (D-154, Part 1 of #375), resolved to a
    /// compile-time slot index by `lower_stmt` against the class's
    /// `HirClassDef` -- mirrors `MirExpr::AttrGet`'s own resolution
    /// exactly.
    AttrSet {
        base: MirExpr,
        slot: usize,
        value: MirExpr,
    },
    /// `del base[start:stop:step]` where `base` is a CPython object (Part 2c
    /// of #1371, from `HirStmt::DeleteSlice`): CPython's `PyObject_DelItem`
    /// with the same `slice` key [`MirExpr::ObjSlice`] builds, an absent
    /// bound passed as `None`. A statement because it yields nothing. Its
    /// failure (an object without `__delitem__`, or a raising one) is
    /// routed by codegen's foreign-failure edge inside the statement, as
    /// [`MirStmt::ForObject`]'s `iter()` is.
    ObjDelSlice {
        base: MirExpr,
        start: Option<MirExpr>,
        stop: Option<MirExpr>,
        step: Option<MirExpr>,
    },
    /// `base[start:stop:step] = value` where `base` is a CPython object
    /// (non-strict mode, #3, from `HirStmt::SetSlice`): CPython's
    /// `PyObject_SetItem` with the `slice` key [`MirStmt::ObjDelSlice`]
    /// builds. `value` is always an object (`MirExpr::ObjectBox` boxes a
    /// native one) and is evaluated first, CPython's order. Its failure is
    /// routed as [`MirStmt::ObjDelSlice`]'s is.
    ObjSetSlice {
        base: MirExpr,
        start: Option<MirExpr>,
        stop: Option<MirExpr>,
        step: Option<MirExpr>,
        value: MirExpr,
    },
    /// `base.attr = value` where `base` is a CPython object (Part 2 of
    /// #1443, #1457, from a `HirStmt::AttrSet` whose base types as
    /// `Ty::Object`): CPython's `PyObject_SetAttr`, String-keyed like
    /// [`MirExpr::ObjAttrGet`] because a foreign object has no slot table.
    /// Codegen evaluates `value` before `base`, CPython's own order for an
    /// attribute assignment, and packs it into a new reference the shim
    /// helper consumes. Its failure (a read-only or `__slots__`-refused
    /// attribute, a raising `__setattr__`) is routed by codegen's
    /// foreign-failure edge inside the statement, as
    /// [`MirStmt::ObjDelSlice`]'s is.
    ObjAttrSet {
        base: MirExpr,
        attr: String,
        value: MirExpr,
    },
    /// `del base.attr` where `base` is a CPython object (#1457, from
    /// `HirStmt::DeleteAttr`): CPython's `PyObject_DelAttr`. A failure (a
    /// missing attribute, a raising `__delattr__`) takes the
    /// foreign-failure edge, as [`MirStmt::ObjAttrSet`]'s does.
    ObjDelAttr {
        base: MirExpr,
        attr: String,
    },
    /// `base[key] = value` where `base` is a CPython object (non-strict
    /// mode, #3): `PyObject_SetItem`. Codegen evaluates `value`, then `base`,
    /// then `key`, CPython's own order for a subscript assignment.
    ObjSetItem {
        base: MirExpr,
        key: MirExpr,
        value: MirExpr,
    },
    /// `del base[key]` where `base` is a CPython object (non-strict mode,
    /// #3): `PyObject_DelItem`.
    ObjDelItem {
        base: MirExpr,
        key: MirExpr,
    },
    /// PEP 634-636 (#381, PR-21): A sequence of statements executed in
    /// order — used by `match` lowering to pair the subject-temporary
    /// assignment with the nested `if` chain.
    Seq(Vec<MirStmt>),
    /// `break` (from `HirStmt::Break`): leaves the innermost enclosing
    /// loop statement, skipping its `else:` clause.
    Break,
    /// `continue` (from `HirStmt::Continue`): starts the innermost
    /// enclosing loop statement's next iteration.
    Continue,
    /// A loop with an `else:` clause (from `HirStmt::LoopElse`).
    /// `loop_stmt` is one of the loop statements (`While`, `ForRange`,
    /// `ForList`, `ForObject`, `ForDict`, `ForSet`); `orelse` runs after it
    /// only when it finished without a `break` bound to it.
    LoopElse {
        loop_stmt: Box<MirStmt>,
        orelse: Vec<MirStmt>,
    },
    /// `try`/`except`/`else`/`finally` (PEP 3110, #382, PR-22 Part 1).
    /// Each handler's `exc_type_tag` is the sorted set of runtime exception
    /// type tags it accepts — the named class's own tag (matching `pycc_rt`'s
    /// `EXCEPTION_TYPE_*` constants for the seven builtins, or the tag HIR
    /// lowering assigned the class for a user-defined one) plus every
    /// raisable subclass's tag — or `None` for a bare `except:`.
    Try {
        body: Vec<MirStmt>,
        handlers: Vec<MirExceptHandler>,
        orelse: Vec<MirStmt>,
        finalbody: Vec<MirStmt>,
    },
    /// `try: ... except* T: ...` (PEP 654, Part 3 of #382, #542). Shares
    /// `Try`'s shape exactly -- `MirExceptHandler`'s `exc_type_tag` still
    /// carries the matched-type tag set (always `Some`, since `except*`
    /// requires a named type; see `pycc_hir::stmt`'s comment on the
    /// matching lowering site) -- but codegen dispatches each handler with
    /// a partition (`pycc_rt_exception_group_partition`) against the raised
    /// group rather than `Try`'s mutually-exclusive `type_matches` checks,
    /// and every matched subgroup's handler can run (PEP 654 semantics: more
    /// than one `except*` clause may fire for the same raised group), unlike
    /// `Try`'s first-match-wins dispatch.
    TryStar {
        body: Vec<MirStmt>,
        handlers: Vec<MirExceptHandler>,
        orelse: Vec<MirStmt>,
        finalbody: Vec<MirStmt>,
    },
    /// `raise <exception>` (PEP 3110, #382).
    Raise {
        exception: MirExceptionValue,
        /// The enclosing function's plain Python source name (`"<module>"`
        /// at top level, e.g. `create` rather than `pycc_hir`'s mangled
        /// `C.create.classmethod` for a classmethod -- see
        /// [`source_frame_name`]), identifying where this `raise` executes
        /// for traceback rendering (#707), matching what CPython's own
        /// traceback shows for the frame. Populated by
        /// [`set_frame_function`] as a post-pass over the body
        /// [`lower_item`] just finished lowering, rather than threaded as a
        /// parameter through `lower_stmt`'s whole recursive descent: the
        /// value is constant for an entire function body, and every
        /// statement-lowering function between `lower_item` and
        /// `lower_raise` would otherwise need a new parameter purely to pass
        /// it through unchanged. `lower_raise` itself always leaves this
        /// empty; only the post-pass fills it in.
        frame_function: String,
    },
    /// `raise <exception> from <cause>` (PEP 409, #382).
    RaiseFrom {
        exception: MirExceptionValue,
        cause: MirExceptionValue,
        /// See [`MirStmt::Raise::frame_function`] -- identifies where this
        /// `raise ... from ...` executes. Only `exception` (the raised
        /// effect) carries a frame; `cause` keeps whatever frame it already
        /// had (empty, if freshly constructed, since this compiler does not
        /// track cause objects raised elsewhere), matching CPython's
        /// distinction between an exception's own traceback and an
        /// explicitly chained cause's.
        frame_function: String,
    },
    /// Bare `raise` (re-raise, #382). Only valid inside an except handler.
    Reraise,
    /// `yield value` in a generator function (`pycc_codegen::generator`).
    Yield(MirExpr),
    /// `return value` in a generator: `value` becomes `StopIteration.value`
    /// (a bare `MirStmt::Return(None)` follows, ending the body).
    GenReturn(MirExpr),
    /// `for var in gen:` over a local bound to a generator. The loop holds
    /// its own reference in the hidden local `hold` (taken over from `gen`
    /// when `consume`, a temporary bound only for this loop), released when
    /// the loop ends -- so an unfinished generator is closed there, as in
    /// CPython -- or by the frame's epilogue on a `return` or an exception.
    ForGen {
        var: String,
        var_ty: Ty,
        generator: String,
        hold: String,
        consume: bool,
        body: Vec<MirStmt>,
    },
    /// `raise value` where `value` is a CPython object (Part 9 of #1371,
    /// from a cause-less `HirStmt::Raise` whose operand is `Ty::Object`).
    /// CPython decides what is raised, as its own `raise` does: an
    /// exception instance is raised, an exception class is instantiated
    /// with no arguments, and anything else raises `TypeError`. Codegen
    /// hands the CPython exception to the foreign-operation bridge, so it
    /// propagates as a pending pycc exception whose original the host sees.
    /// Carries no frame name: the bridged original keeps CPython's own
    /// traceback, unlike [`MirStmt::Raise`]'s pycc-rendered one.
    ObjRaise {
        value: MirExpr,
        /// `raise o from cause`: the cause, a CPython object or a pycc
        /// exception, which becomes the raised exception's `__cause__`
        /// (`pycc_ext_obj_raise_from`). `None` for a plain `raise o` (and
        /// for `raise o from None`, which `pycc_hir` records as no cause).
        cause: Option<MirExceptionValue>,
    },
    /// A foreign (CPython-object) import nested in a module-level `if`/`try`
    /// block (#1291, #1383), the statement counterpart of
    /// [`MirItem::ForeignImport`]: each `(local_name, module_path, from)`
    /// entry, in order, stores into the module global `local_name`, where
    /// the statement runs, the module object
    /// `pycc_ext_obj_import(module_path)` returns when `from` is `None`, or
    /// the attribute a `pycc_ext_obj_import_from` call returns for a
    /// from-import's name (#1383; `from` as for [`MirItem::ForeignImport`]).
    /// A failed import whose exception is an `ImportError` -- a missing
    /// name of a from-import included -- becomes a pycc raise of
    /// `ImportError`/`ModuleNotFoundError` (#1293), so an enclosing handler
    /// runs; any other failure is bridged by the object bridge inside a
    /// module-level `try`, and returns `-1` from `Py_mod_exec` directly
    /// outside every one (Part 1 of #1096).
    ForeignImport {
        bindings: Vec<(String, String, Option<FromImport>)>,
    },
}

/// A comprehension's already-resolved iterable source (PR-12, D-117) --
/// the MIR-level counterpart to `pycc_hir::CompIter`, but with a
/// bare-name iterable already split into its concrete container kind,
/// mirroring `HirStmt::ForList`'s own split into `MirStmt::ForList`/
/// `ForDict`/`ForSet` at this exact lowering stage. Kept as a field on
/// each `*CompAssign` variant (not exploded into a full cross-product of
/// top-level `MirStmt` variants per comprehension-kind x source-kind
/// combination) -- mirrors the precedent `MirExpr::Subscript` already
/// established (one node, internal branching on the resolved type in
/// `pycc_codegen`), avoiding a 3x4 combinatorial explosion for no
/// benefit.
#[derive(Debug, Clone, PartialEq)]
pub enum CompSource {
    Range {
        start: MirExpr,
        stop: MirExpr,
        step: MirExpr,
    },
    List(String),
    Dict(String),
    Set(String),
    /// A CPython object iterated through the iterator protocol (Part 1 of
    /// #1255): the iterable expression, evaluated once in the enclosing
    /// scope. Its loop variable is a CPython object, and the comprehension
    /// produces a CPython `list` or `set` ([`MirComprehension::ty`]). Only
    /// the expression form carries it: `pycc_hir` lowers the statement form
    /// of such a comprehension to a plain assignment.
    Object(MirExpr),
    /// A list-valued expression, evaluated once before the loop (a `str`
    /// iterable arrives here converted by `pycc_rt_str_to_list`).
    ListExpr(MirExpr),
}

#[derive(Debug, PartialEq)]
pub enum MirItem {
    Function {
        name: String,
        params: Vec<(String, Ty)>,
        return_ty: Ty,
        body: Vec<MirStmt>,
    },
    TopLevelStmt(MirStmt),
    /// Part 1 of #1026: a foreign (CPython-object) import, spliced into
    /// `MirModule::items` at the position the corresponding module-body
    /// statement would have occupied, so codegen's single source-order
    /// item loop emits the `pycc_ext_obj_import` call exactly where the
    /// `import` statement stood.
    ///
    /// It is an *item*, not a side table, precisely so that ordering is
    /// structural rather than a convention codegen has to re-derive: PR 1b
    /// carried the same information as a separate `foreign_imports` vector,
    /// which could only be emitted as a hoisted prologue and would have run
    /// a module's imports ahead of a preceding statement's observable
    /// effects -- a divergence from CPython that D-244 rule 3 does not
    /// admit.
    ///
    /// `from` is `None` for `import X`, and for one name of a top-level
    /// `from X import a, b` (#1278) it is that binding's name, its index,
    /// the statement's whole fromlist and its relative `level`, which the
    /// `pycc_ext_obj_import_from` call passes to CPython's `__import__`.
    /// `level` is `0` except for the entry module's relative
    /// `from .x import a, b` under `--ext --foreign-relative-imports`
    /// (#1366).
    ForeignImport {
        local_name: String,
        module_path: String,
        from: Option<FromImport>,
    },
}

/// Whether `body` (a function's) contains a `yield` of its own: whether the
/// function is a generator, which a function merely returning a generator
/// (`def f() -> Iterator[int]: return g()`) is not.
#[must_use]
pub fn body_yields(body: &[MirStmt]) -> bool {
    fn expr_yields(expr: &MirExpr) -> bool {
        matches!(expr, MirExpr::GenYield { .. } | MirExpr::GenYieldFrom { .. })
            || expr.children().into_iter().any(expr_yields)
    }
    body.iter().any(|stmt| {
        matches!(stmt, MirStmt::Yield(_) | MirStmt::GenReturn(_))
            || stmt.exprs().into_iter().any(expr_yields)
            || stmt.bodies().into_iter().any(body_yields)
    })
}
