//! HIR statement nodes (`HirStmt`, match patterns) and control-flow queries over bodies.

use super::*;

#[derive(Debug, Clone, PartialEq)]
pub enum HirStmt {
    ExprStmt(HirExpr),
    Assign {
        target: String,
        value: HirExpr,
    },
    AnnAssign {
        target: String,
        annotation: Ty,
        value: Option<HirExpr>,
        /// PEP 591 (#383): `true` when the original annotation was
        /// `Final[X]`, meaning this binding may not be reassigned after its
        /// initial assignment. Set at HIR-lowering time in `stmt.rs`'s
        /// `Stmt::AnnAssign` arm by inspecting the raw AST annotation
        /// before `annotation_to_ty` unwraps `Final[X]` to `X`. The type
        /// checker's `Environment.finals` set is populated from this flag,
        /// and `check_assignment` rejects a reassignment of a `Final` name
        /// with `T0045`.
        is_final: bool,
    },
    If {
        test: HirExpr,
        body: Vec<HirStmt>,
        orelse: Vec<HirStmt>,
    },
    While {
        test: HirExpr,
        body: Vec<HirStmt>,
    },
    ForRange {
        var: String,
        start: HirExpr,
        stop: HirExpr,
        step: HirExpr,
        body: Vec<HirStmt>,
    },
    /// `for var in list:`, parallel to the existing `ForRange` -- desugars
    /// to an index-counted loop starting in a later PR-10 task (D-105),
    /// not here. Also reused, unconditionally, for `for k in <bare-name
    /// dict>:` (PR-11 Task 3, D-123: iterates keys in insertion order) --
    /// this lowering step has no type information to distinguish a
    /// `list`-typed iterable from a `dict`-typed one, so `pycc_types`'
    /// own `ForList` arms resolve `list`'s real type to `Ty::List`,
    /// `Ty::Dict` (binding `var` as the key type), or reject it, not here.
    ForList {
        var: String,
        list: String,
        body: Vec<HirStmt>,
    },
    /// `for var in <expression>:` where the expression is syntactically an
    /// attribute load (`o.attr`) or a method call (`o.method(...)`), the two
    /// shapes that can produce a foreign `object` value under D-244's `ext`
    /// artifact mode (Part 3 of #1026, PR 3c of #1082). HIR carries no
    /// types, so this node records only that syntactic shape: `pycc_types`
    /// resolves `iter` and refuses it with `I0404` unless it is `Ty::Object`
    /// -- `for x in xs.copy():` over a `list` lowers here too and is
    /// rejected there, not here. Unlike `ForList`, the iterable is a full
    /// `HirExpr` rather than a name (D-105's `String` field cannot carry
    /// `o.attr`), so every pass that walks statement sub-expressions must
    /// walk `iter` as well as `body`.
    ForObject {
        var: String,
        iter: Box<HirExpr>,
        body: Vec<HirStmt>,
    },
    /// `<bare name>[key] = value`, PR-11 Task 3 (D-123 supersedes D-105's
    /// "no subscript assignment target anywhere in this file" consequence
    /// for `list[int]`; see `Stmt::Assign`'s own lowering arm below). `dict`
    /// is carried as a plain variable name, exactly like `ForList`'s `list`
    /// field and `ListAppend`'s `list` field -- this lowering step has no
    /// type information, so a `list[int]` target also lowers to this node
    /// today (`pycc_types` rejects it with `T0033`, mirroring how
    /// `ForList`'s own `list` field is resolved to `Ty::List`, `Ty::Dict`,
    /// or rejected downstream, not here). Since Part 1 of #1142 a
    /// `memoryview` target lowers here too: `pycc_types` admits it as a
    /// buffer element store and `pycc_mir` lowers *that* one to
    /// `MirStmt::BufferSet` instead of `MirStmt::DictSet`, so this node
    /// stays the single type-blind shape for every `<name>[k] = v`.
    DictSet {
        dict: String,
        key: HirExpr,
        value: HirExpr,
    },
    /// `target = [elt for var in iter [if cond]]` (PR-12, D-117). Scoped to
    /// exactly one `for` clause and at most one `if` filter; only lowered
    /// when the comprehension is the direct RHS of a bare-name
    /// `Stmt::Assign` (see that arm's own handling below). Anywhere else a
    /// comprehension appears it lowers to [`HirExpr::Comprehension`] (#1254,
    /// D-250); this statement form is built from that same
    /// [`HirComprehension`] by `comp_assign_stmt`. `var` is already the
    /// D-117 synthesized internal name, not the source spelling -- every
    /// occurrence of the source name inside `cond`/`elt` has already been
    /// rewritten by `rename_name_in_expr` before this node is constructed,
    /// so downstream crates never see the user's own loop-variable
    /// spelling at all.
    ListCompAssign {
        target: String,
        var: String,
        iter: CompIter,
        cond: Option<Box<HirExpr>>,
        elt: Box<HirExpr>,
    },
    /// `target = {key: value for var in iter [if cond]}` (PR-12, D-117).
    /// Mirrors `ListCompAssign` exactly except for the key/value split
    /// (Python's dict-comprehension grammar has no direct list-comp analog
    /// of a single `elt`).
    DictCompAssign {
        target: String,
        var: String,
        iter: CompIter,
        cond: Option<Box<HirExpr>>,
        key: Box<HirExpr>,
        value: Box<HirExpr>,
    },
    /// `target = {elt for var in iter [if cond]}` (PR-12, D-117). Mirrors
    /// `ListCompAssign` exactly -- a set comprehension's own shape is
    /// identical to a list comprehension's, differing only in which
    /// runtime constructor/insert pair `pycc_codegen` ends up calling.
    SetCompAssign {
        target: String,
        var: String,
        iter: CompIter,
        cond: Option<Box<HirExpr>>,
        elt: Box<HirExpr>,
    },
    Return(Option<HirExpr>),
    /// `base.attr = value` (D-154, Part 1 of #375). Structurally recognized
    /// for any assignment target shaped like an attribute access -- `base`
    /// is lowered generically, exactly like `HirExpr::AttrGet`'s own `base`,
    /// and for the same reason (no type information is available at this
    /// lowering step to narrow it to only `self` or only an instance-typed
    /// receiver). No slot `Ty` is carried here: `pycc_types` resolves
    /// `attr`'s declared type from the class's `HirModule::class_defs` entry
    /// (populated by `class::lower_class`'s own `__init__` pre-scan) and
    /// checks `value` against it; `pycc_mir`/`pycc_codegen` resolve `attr`
    /// to a compile-time slot index the same way. This single shape covers
    /// both an attribute's first (slot-establishing) assignment inside
    /// `__init__` and every later reassignment inside any method body --
    /// the two are distinguished only by `class::lower_class`'s own
    /// pre-scan, not by this node's own shape.
    AttrSet {
        base: HirExpr,
        attr: String,
        value: HirExpr,
    },
    /// PEP 634-636 (#381, PR-21): structural pattern matching. The subject is
    /// evaluated once and matched against each case in order; the first
    /// matching case's body executes. See `HirPattern`/`HirMatchCase` for
    /// the per-case shape.
    Match {
        subject: HirExpr,
        cases: Vec<HirMatchCase>,
    },
    /// `try`/`except`/`else`/`finally` (PEP 3110, #382, PR-22 Part 1).
    /// The body is executed; if an exception is raised, each handler is
    /// checked in order. The `orelse` runs only if no exception was raised.
    /// The `finalbody` always runs, whether the try completed normally, an
    /// exception was caught, or an exception is propagating.
    Try {
        body: Vec<HirStmt>,
        handlers: Vec<HirExceptHandler>,
        orelse: Vec<HirStmt>,
        finalbody: Vec<HirStmt>,
    },
    /// `try: ... except* T: ...` (PEP 654, Part 3 of #382, #542). Shares
    /// `HirExceptHandler`'s shape with plain `Try` -- the grammar node is
    /// identical, only the `is_star` flag on the AST differs -- but is a
    /// distinct variant rather than an `is_star: bool` field on `Try`
    /// because the two forms cannot mix (`except`/`except*` may not appear
    /// in the same `try`, enforced at parse time) and every downstream
    /// consumer (MIR lowering, `block_always_terminates`, `pycc_types`)
    /// needs its own dispatch anyway: `except*` always binds `as e` to an
    /// `ExceptionGroup`, never to the named type itself, and lowers to a
    /// partition-based dispatch rather than mutually exclusive `type_matches`
    /// branches. A bare `except*:` has no `HirExceptHandler` counterpart --
    /// PEP 654 requires every `except*` clause to name a type -- so
    /// `handlers[i].exc_type` is never `None` here: a bare `except*:` is
    /// rejected by ruff's own parser as a syntax error (`L0001`) before HIR
    /// lowering ever runs.
    TryStar {
        body: Vec<HirStmt>,
        handlers: Vec<HirExceptHandler>,
        orelse: Vec<HirStmt>,
        finalbody: Vec<HirStmt>,
    },
    /// `raise` (PEP 3110, #382). `exc` is the exception expression to raise;
    /// `None` means a bare re-raise (only valid inside an except handler).
    /// `cause` is the optional `raise ... from cause` expression (PEP 409).
    Raise {
        exc: Option<HirExpr>,
        cause: Option<HirExpr>,
    },
    /// `del name` (#1244, Part 1 of #1216). One statement per deleted name:
    /// `del a, (b, c)` is lowered left to right into three of these by
    /// `stmt::del::lower_delete`. The statement has no runtime effect -- the
    /// memory model is leak-only (D-124), so nothing is released -- and exists
    /// so the type checker can unbind `name`; every read that CPython would
    /// answer with `NameError`/`UnboundLocalError` is refused statically.
    /// `docs/TYPE_SYSTEM.md`'s "`del` statement" section is the canonical
    /// statement of the rule.
    Delete {
        name: String,
    },
    /// `del base[start:stop:step]`, any bound omitted (Part 2c of #1371),
    /// one per slice target of a `del` statement, in the statement's
    /// left-to-right target order. Unlike [`HirStmt::Delete`] it unbinds no
    /// name: it *reads* `base` and every present bound, and at run time asks
    /// the base to delete the slice. `pycc_hir` cannot see types, so every
    /// slice target lowers here; `pycc_types` admits only a CPython-object
    /// base (`PyObject_DelItem` with a `slice` key, `foreign::slice`) and
    /// refuses every other base with this node's `span` -- the slice
    /// target's own range -- so the native refusal stays located.
    /// `docs/TYPE_SYSTEM.md`'s "`del` statement" section is the canonical
    /// statement of the rule.
    DeleteSlice {
        base: Box<HirExpr>,
        start: Option<Box<HirExpr>>,
        stop: Option<Box<HirExpr>>,
        step: Option<Box<HirExpr>>,
        span: Span,
    },
    /// `del base.attr` (Part 2 of #1443, #1457), one per attribute target of
    /// a `del` statement, in the statement's left-to-right target order. Like
    /// [`HirStmt::DeleteSlice`] it unbinds no name: it *reads* `base` and at
    /// run time asks it to delete `attr`. `pycc_hir` cannot see types, so
    /// every attribute target lowers here; `pycc_types` admits only a
    /// CPython-object base (`PyObject_DelAttr`, `foreign::attr_store`) and
    /// refuses every other base with this node's `span` -- the attribute
    /// target's own range. `docs/TYPE_SYSTEM.md`'s "`del` statement" section
    /// is the canonical statement of the rule.
    DeleteAttr {
        base: Box<HirExpr>,
        attr: String,
        span: Span,
    },
    /// `del base[key]` with a non-slice key (non-strict mode, #3). Like
    /// [`HirStmt::DeleteSlice`] it unbinds no name: it reads `base`, then
    /// `key`, and asks the base to delete the item. `pycc_types` admits only
    /// a CPython-object base (`PyObject_DelItem`) and refuses every other one
    /// with this node's `span`, the subscript target's own range.
    DeleteItem {
        base: Box<HirExpr>,
        key: Box<HirExpr>,
        span: Span,
    },
    /// A CPython-backed `import X` or `import X as Y` (Part 1 of #1282,
    /// #1291), or `from X import a, b` (#1383), nested in a module-level
    /// `if`/`try` block. Each `(local_name, module_path, from)` entry, in
    /// source order, binds `local_name` at this statement's own position,
    /// so the import runs only when control reaches it: to the opaque
    /// CPython module object `pycc_ext_obj_import` returns for
    /// `module_path` when `from` is `None`, and to the attribute
    /// [`FromImport::name`] of that module, imported with the statement's
    /// whole fromlist and level, when `from` is `Some`. Produced only when
    /// every binding of the statement is foreign: each entry also sits in [`HirModule::imports`]
    /// as an [`ImportBinding::Foreign`] with [`ForeignImportSite::Block`]
    /// and this statement's `span`, which is how the driver's lock, policy
    /// and native-build gates see it. A top-level foreign import is never
    /// this node: it stays an [`ImportBinding::Foreign`] with
    /// [`ForeignImportSite::Item`], spliced into the module body by
    /// `pycc_mir`.
    ForeignImport {
        bindings: Vec<(String, String, Option<FromImport>)>,
        span: Span,
    },
    /// `break`: leaves the innermost enclosing loop (`While`, `ForRange`,
    /// `ForList`, `ForObject`), skipping that loop's `else:` clause when it
    /// has one ([`HirStmt::LoopElse`]). Lowering admits it only with a real
    /// enclosing loop and never across a `finally` clause that sits between
    /// the statement and that loop (a `C0001`: the `finally` would have to
    /// run first, which codegen does not route yet).
    Break,
    /// `continue`: starts the innermost enclosing loop's next iteration.
    /// Admitted under exactly the rules [`HirStmt::Break`] states.
    Continue,
    /// A loop with an `else:` clause (`while ...: ... else: ...` or
    /// `for ...: ... else: ...`). `loop_stmt` is always one of the loop
    /// variants (`While`, `ForRange`, `ForList`, `ForObject`); `orelse` runs
    /// after it only when the loop finished without a `break` bound to it,
    /// as in CPython. A wrapper rather than an `orelse` field on every loop
    /// variant so that a loop without `else:` keeps its exact shape.
    LoopElse {
        loop_stmt: Box<HirStmt>,
        orelse: Vec<HirStmt>,
    },
}

/// Whether `body` contains a `break` bound to the loop whose body it is:
/// one not nested inside an inner loop's own body. An inner loop's `else:`
/// clause is outside that inner loop, so a `break` there does count.
pub fn body_breaks_loop(body: &[HirStmt]) -> bool {
    body_jumps(body, &|stmt| matches!(stmt, HirStmt::Break))
}

/// Whether `body` contains a `break` or a `continue` bound to the loop whose
/// body it is, under [`body_breaks_loop`]'s binding rule.
pub fn body_jumps_in_loop(body: &[HirStmt]) -> bool {
    body_jumps(body, &|stmt| {
        matches!(stmt, HirStmt::Break | HirStmt::Continue)
    })
}

pub(crate) fn stmt_jumps(stmt: &HirStmt, is_jump: &dyn Fn(&HirStmt) -> bool) -> bool {
    let recurse = |body: &[HirStmt]| body_jumps(body, is_jump);
    match stmt {
        HirStmt::Break | HirStmt::Continue => is_jump(stmt),
        HirStmt::If { body, orelse, .. } => recurse(body) || recurse(orelse),
        HirStmt::Match { cases, .. } => cases.iter().any(|case| recurse(&case.body)),
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            recurse(body)
                || handlers.iter().any(|h| recurse(&h.body))
                || recurse(orelse)
                || recurse(finalbody)
        }
        HirStmt::LoopElse { orelse, .. } => recurse(orelse),
        _ => false,
    }
}

/// A loop's `else:` clause whose loop holds a `break`, as the `if` with an
/// opaque test that both `pycc_types` and `pycc_mir` treat it as: a suite
/// that runs at most once and whose bindings and narrowing join with the
/// path that skipped it, so the two passes cannot disagree on either. The
/// test is [`HirExpr::OpaqueBool`], which no narrowing recognizer, constant
/// folder or always-true check can see through.
pub fn loop_else_as_if(orelse: &[HirStmt]) -> HirStmt {
    HirStmt::If {
        test: HirExpr::OpaqueBool,
        body: orelse.to_vec(),
        orelse: Vec::new(),
    }
}

/// The body of a loop statement (`While`, `ForRange`, `ForList`,
/// `ForObject`), or `None` for any other statement.
pub fn loop_body(stmt: &HirStmt) -> Option<&[HirStmt]> {
    match stmt {
        HirStmt::While { body, .. }
        | HirStmt::ForRange { body, .. }
        | HirStmt::ForList { body, .. }
        | HirStmt::ForObject { body, .. } => Some(body),
        _ => None,
    }
}

/// Issue #769 (Part 2 of #747): the polarity of an `if`'s `name is [not]
/// None` test, as recognized by [`optional_none_test`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoneTestPolarity {
    /// `name is None` -- true in the `if` body means `name` is absent, so
    /// only the `orelse` branch can be narrowed to the Optional's inner
    /// type.
    Is,
    /// `name is not None` -- true in the `if` body means `name` is present,
    /// so only the `body` branch can be narrowed.
    IsNot,
}

/// Issue #769 (Part 2 of #747): recognizes an `HirStmt::If`'s `test` as a
/// top-level `name is None` / `name is not None` shape and extracts the
/// bare name and polarity, or `None` for every other test shape --
/// including a compound `and`/`or` test, a test where neither side is a
/// bare `HirExpr::Name`, or a test that is not an `is`/`is not` comparison
/// at all. Deliberately does not inspect the name's *type* -- callers
/// combine this syntactic recognition with their own scope's current type
/// for `name` (checker: `Environment`; MIR: the `scopes` stack) to decide
/// whether narrowing actually applies.
///
/// This is the single shared recognizer `pycc_types` (the checker's
/// narrowing overlay) and `pycc_mir` (the `OptionalUnwrap` lowering) both
/// call, so the syntactic pattern-match lives in exactly one place instead
/// of drifting between two independent copies -- `pycc_mir` does not
/// depend on `pycc_types` (see `crates/pycc_mir/Cargo.toml`), so this
/// recognizer lives here in `pycc_hir`, which both already depend on.
///
/// Since Part 1 of #1371 an `Is`/`IsNot` node need not have a `None`
/// operand (identity between two CPython objects), so this function
/// re-derives which side is which, and answers `None` for any other shape,
/// rather than assuming an operand order or a `None` side.
pub fn optional_none_test(test: &HirExpr) -> Option<(&str, NoneTestPolarity)> {
    let HirExpr::Compare { op, left, right } = test else {
        return None;
    };
    let polarity = match op {
        CmpOpKind::Is => NoneTestPolarity::Is,
        CmpOpKind::IsNot => NoneTestPolarity::IsNot,
        _ => return None,
    };
    let name_side = match (left.as_ref(), right.as_ref()) {
        (HirExpr::Name(name), HirExpr::NoneLiteral) => name,
        (HirExpr::NoneLiteral, HirExpr::Name(name)) => name,
        _ => return None,
    };
    Some((name_side.as_str(), polarity))
}

/// Issue #769 (Part 2 of #747): true only when `body`'s control flow
/// unconditionally terminates the enclosing function on every path through
/// it -- the strict predicate the early-return-narrows-the-continuation
/// shape needs (`if name is None: <body>` narrows every read after the
/// `if` only when `<body>` is provably inescapable).
///
/// Deliberately **not** the same thing as "a `return` occurs somewhere in
/// `body`" (an existing, separately-named unsound check elsewhere in this
/// codebase treats those as equivalent, which is wrong: `if flag: return
/// 0` contains a `return` but does not terminate on every path). This
/// predicate is true only when `body`'s *last* statement is itself
/// unconditionally terminating -- a bare `return`, or an `if` whose `body`
/// **and** non-empty `orelse` both recursively terminate. `raise` is
/// deliberately not a terminator (a program that raises still guarantees
/// the same soundness a `return` does, but correctly recognizing that
/// would additionally have to account for an enclosing `try`/`except`
/// catching it before it propagates out of the function -- out of scope
/// here), and `match` is not analyzed at all (also out of scope, kept
/// sound by omission rather than by an exhaustiveness heuristic that could
/// be wrong). Sound-by-omission for any statement shape not explicitly
/// handled below: an unhandled last statement makes the whole body report
/// `false`, never `true`.
///
/// Shared between `pycc_types::narrow` (the checker's own narrowing
/// overlay) and `pycc_mir` (its `OptionalUnwrap` lowering) for the
/// identical reason [`optional_none_test`] is shared: `pycc_mir` does not
/// depend on `pycc_types`, so the one predicate both need lives here, in
/// their common dependency, rather than as two independently-maintained
/// copies (D-205).
pub fn definitely_terminates(body: &[HirStmt]) -> bool {
    match body.last() {
        Some(HirStmt::Return(_)) => true,
        Some(HirStmt::If { body, orelse, .. }) => {
            !orelse.is_empty() && definitely_terminates(body) && definitely_terminates(orelse)
        }
        _ => false,
    }
}

/// PEP 634-636 (#381, PR-21): a single `match` case's pattern, lowered from
/// `ruff_python_ast::Pattern`. The type checker verifies each pattern against
/// the subject's known type and collects capture bindings; the MIR pass
/// desugars each pattern into a match condition plus binding assignments.
#[derive(Debug, Clone, PartialEq)]
pub enum HirPattern {
    /// `case _:` — matches anything, binds nothing.
    Wildcard,
    /// `case x:` — matches anything, binds `x` to the subject.
    Capture(String),
    /// `case 42:` / `case "hello":` / `case 3.14:` — a literal value pattern.
    Literal(HirExpr),
    /// `case True:` / `case False:` — a bool singleton pattern.
    Singleton(bool),
    /// `case None:` — the `None` singleton pattern.
    NoneSingleton,
    /// `case [a, b, c]:` — a fixed-length sequence pattern.
    Sequence(Vec<HirPattern>),
    /// `case [a, *rest]:` — a sequence pattern with a star-capture. The
    /// leading `Vec` are the fixed patterns before the star; `rest` captures
    /// the remaining elements as a list.
    SequenceStar(Vec<HirPattern>, Option<String>),
    /// `case {"k": v}:` — a mapping pattern. Each pair is `(key_expr,
    /// value_pattern)`; `rest` captures the remaining dict.
    Mapping(Vec<(HirExpr, HirPattern)>, Option<String>),
    /// `case Point(0, 0):` / `case Point(x=0, y=0):` — a class pattern.
    /// `positional` matches `__init__`'s parameters (after `self`);
    /// `keyword` matches named attributes.
    Class {
        class_name: String,
        positional: Vec<HirPattern>,
        keyword: Vec<(String, HirPattern)>,
    },
    /// `case 1 | 2 | 3:` — an or-pattern. All sub-patterns must bind the
    /// same set of capture names (PEP 634).
    Or(Vec<HirPattern>),
    /// `case [a, b] as pair:` — an as-pattern. The inner pattern must
    /// match, and `name` is bound to the subject.
    As(Box<HirPattern>, String),
}

/// PEP 634-636 (#381, PR-21): a single `case` clause in a `match`
/// statement.
#[derive(Debug, Clone, PartialEq)]
pub struct HirMatchCase {
    pub pattern: HirPattern,
    pub guard: Option<HirExpr>,
    pub body: Vec<HirStmt>,
}
