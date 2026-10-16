//! Non-strict mode's type-checking rules (#3). What non-strict mode mostly
//! changes is the HIR lowering, which turns a missing annotation and `Any`
//! into `object` (`pycc_hir`'s `NON_STRICT_MARKER`); the checker adds three
//! rules, each admitted only in non-strict mode:
//!
//! - **Run-time conversion.** A CPython object passed to a compiled
//!   function's `int`/`float`/`bool`/`str` parameter, or returned from a
//!   function declared to return one, is converted with a type check
//!   (`TypeError` when the object is not of that type) instead of being
//!   refused ([`admits_coercion`]).
//! - **Object displays.** A display with an object element, or whose
//!   elements have no native representation, is the CPython container
//!   (`foreign::arith::object_display_ty`).
//! - **Widening.** A name rebound with a value of another type (`t = 0`
//!   then `t += x` for an object `x`, or `x = 1` then `x = "a"`) cannot keep
//!   one native type, so it becomes `object`, as every Python name is: the
//!   checker records the name ([`request_widen`]), and [`with_widening`]
//!   re-checks the module with the name's first plain assignment in each
//!   scope rewritten into `name: object = value`. Later rebindings box into
//!   that slot, which the checker already admits.
//!
//! The mode is per check run and per thread: the driver sets it with
//! [`set_non_strict`] right before it links and checks a program.

use pycc_hir::{HirItem, HirModule, HirStmt, Ty};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;

thread_local! {
    static NON_STRICT: Cell<bool> = const { Cell::new(false) };
    /// Names to widen, each keyed by the function item whose local it is
    /// (`None`: every scope, the module's included), see [`WidenKey`].
    static WIDEN: RefCell<BTreeSet<WidenKey>> = const { RefCell::new(BTreeSet::new()) };
    /// `(class, attribute)` slots to widen ([`request_attr_widen`]).
    static WIDEN_ATTRS: RefCell<BTreeSet<(String, String)>> =
        const { RefCell::new(BTreeSet::new()) };
}

/// Selects (or clears) non-strict checking for this thread.
pub fn set_non_strict(non_strict: bool) {
    NON_STRICT.with(|cell| cell.set(non_strict));
}

/// Whether this thread checks in non-strict mode.
pub(crate) fn is_non_strict() -> bool {
    NON_STRICT.with(Cell::get)
}

/// Whether the program this thread last checked was checked in non-strict
/// mode ([`set_non_strict`]), for the driver's later stages.
pub fn non_strict_checked() -> bool {
    is_non_strict()
}

thread_local! {
    static EMPTY_CONTAINER_PASS: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f` as the empty-container pre-pass: an empty display does not
/// infer as the CPython container there. That rule is for a `[]` the pass
/// left unresolved; inside the pass it would bind `xs = []` to an object
/// and make the pass type `xs` from that binding instead of from
/// `xs.append(v)`.
pub(crate) fn in_empty_container_pass<T>(f: impl FnOnce() -> T) -> T {
    let previous = EMPTY_CONTAINER_PASS.with(|cell| cell.replace(true));
    let result = f();
    EMPTY_CONTAINER_PASS.with(|cell| cell.set(previous));
    result
}

/// Whether the empty-container pre-pass is running ([`in_empty_container_pass`]).
pub(crate) fn empty_display_untyped() -> bool {
    EMPTY_CONTAINER_PASS.with(Cell::get)
}

/// Whether a value of type `from` reaches a parameter or return slot of type
/// `to` through the run-time conversion: only in non-strict mode, and only
/// from `object` to one of the four scalars or a `frozenset` of `str` or
/// `int` ([`is_coercible_frozenset`]).
pub(crate) fn admits_coercion(from: &Ty, to: &Ty) -> bool {
    is_non_strict()
        && *from == Ty::Object
        && (matches!(to, Ty::Int | Ty::Float | Ty::Bool | Ty::Str) || is_coercible_frozenset(to))
}

/// Whether an object reaches a slot of type `ty` through the run-time
/// conversion as a copy: a `frozenset[str]` or `frozenset[int]`, or a
/// `tuple[str, ...]`, whose immutability makes a native copy of a CPython
/// `set`/`frozenset` or `tuple` indistinguishable from it (the tuple copy
/// even crosses back as the same object, `pycc_rt_tuple_foreign`).
pub(crate) fn is_coercible_frozenset(ty: &Ty) -> bool {
    matches!(ty, Ty::FrozenSet(element) if matches!(**element, Ty::Str | Ty::Int))
        || matches!(ty, Ty::VarTuple(element) if **element == Ty::Str)
}

/// Whether `value`, a tuple display some element of which is a CPython
/// object (so the display is the CPython tuple), is returned from a
/// function declared to return the native tuple `to` of scalars: each
/// object element is converted with a run-time type check, each other
/// element must already have its slot's type (`pycc_mir`'s
/// `coerce_returned_tuple`).
pub(crate) fn admits_tuple_display_return(
    env: &crate::Environment,
    local_names: &[&str],
    value: &pycc_hir::HirExpr,
    to: &Ty,
) -> bool {
    let (pycc_hir::HirExpr::TupleLiteral(elements), Ty::Tuple(tys)) = (value, to) else {
        return false;
    };
    is_non_strict()
        && elements.len() == tys.len()
        && elements.iter().zip(tys.iter()).all(|(element, ty)| {
            match crate::infer_expr_in(env, local_names, element) {
                Ok(Ty::Object) => matches!(ty, Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::Object),
                // A native element of its slot's type (a fixed-arity tuple
                // of `T`s is a `tuple[T, ...]`), or boxed into an `object`
                // slot.
                Ok(actual) => {
                    crate::is_assignable(actual.clone(), ty.clone())
                        || crate::object_box::admits(env, &actual, ty)
                }
                Err(_) => false,
            }
        })
}

/// Whether an object rebinding the parameter `name` of scalar type `to`
/// is converted with a run-time type check, as an object argument for
/// that parameter is ([`admits_coercion`]): in non-strict mode, for an
/// `int`, `float` or `bool` parameter (`pos = parse(src, pos)` where the
/// callee returns an object). A local is widened instead, which keeps
/// CPython's behaviour for any value; a parameter has no first binding to
/// widen, and its annotation already promises the type.
pub(crate) fn admits_rebinding_coercion(env: &crate::Environment, name: &str, from: &Ty, to: &Ty) -> bool {
    admits_coercion(from, to)
        && matches!(to, Ty::Int | Ty::Float | Ty::Bool)
        && env.params.contains(name)
}

/// Whether an index (a list, tuple or `str` subscript, a `range()`
/// argument) of type `ty` is read through `__index__` at run time: an
/// object, in non-strict mode only (`pycc_mir::MirExpr::ObjectIndex`).
pub(crate) fn admits_index(ty: &Ty) -> bool {
    is_non_strict() && *ty == Ty::Object
}

/// A name to widen: `(Some(i), name)` is the local `name` of function item
/// `i` only, `(None, name)` the name in every scope. A function's locals
/// are its own (a nested function is lifted into a function of its own
/// that receives the values it reads as parameters), so a name one
/// function rebinds with another type stays native in every other
/// function that binds it once.
type WidenKey = (Option<usize>, String);

mod container_widen;
mod fresh_returns;
pub(crate) use container_widen::request as request_container_widen;
pub(crate) use fresh_returns::is_fresh_returning;

/// Records that `name` was rebound with a value of another type, when both
/// types fit an `object` slot (`boxable`). The caller still reports its
/// diagnostic; [`with_widening`] decides whether a retry removes it.
pub(crate) fn request_widen(name: &str, previous: &Ty, new: &Ty, boxable: impl Fn(&Ty) -> bool) {
    if is_non_strict() && boxable(previous) && boxable(new) {
        let item = crate::locate::current_item();
        WIDEN.with(|widen| widen.borrow_mut().insert((item, name.to_string())));
    }
}

/// Records that attribute `attr` of `class` (or of the base that declares
/// it) was stored a value of another type, when both fit an `object` slot:
/// the slot becomes `object`, as every Python attribute is
/// ([`with_widening`]).
pub(crate) fn request_attr_widen(
    env: &crate::Environment,
    class: &str,
    attr: &str,
    slot: &Ty,
    value: &Ty,
) {
    let boxable = |ty: &Ty| matches!(ty, Ty::Object | Ty::None) || crate::foreign::is_object_operand(env, ty);
    if is_non_strict() && boxable(slot) && boxable(value) {
        WIDEN_ATTRS.with(|widen| {
            widen
                .borrow_mut()
                .insert((class.to_string(), attr.to_string()))
        });
    }
}

thread_local! {
    /// The names some statement of the program being checked unbinds: a
    /// `del` target or an `except ... as` name (CPython deletes it when the
    /// handler ends); `None` outside a whole-program check
    /// ([`with_widening`]). See [`admits_maybe_read`].
    static UNBOUND_BY_STATEMENT: RefCell<Option<crate::fx::HashSet<String>>> =
        const { RefCell::new(None) };
}

/// Whether a read of `name` (of type `ty`), bound on some paths reaching it
/// only, is admitted: in a program's check, in both modes, it is, as every
/// global and non-parameter local carries a run-time initialized flag
/// (`pycc_codegen`'s `StorageSlot::initialized`) and an unbound read raises
/// CPython's `NameError` / `UnboundLocalError`. A name some statement
/// unbinds again (`del`, an `except ... as` name) keeps the static refusal
/// (`T0041`): its flag stays set after the unbinding. So does a buffer,
/// whose ownership is tracked statically, and so does every read the
/// definite-assignment analysis is asked about outside a program's check.
pub(crate) fn admits_maybe_read(name: &str, ty: &Ty) -> bool {
    !matches!(ty, Ty::MemoryView | Ty::NdArray(_))
        && !falls_back_when_unbound(name)
        && UNBOUND_BY_STATEMENT.with(|names| {
            names
                .borrow()
                .as_ref()
                .is_some_and(|names| !names.contains(name))
        })
}

/// Whether CPython reads something else for `name` when the module leaves
/// it unbound -- a builtin, or a module attribute such as `__name__` -- so a
/// possibly-unbound read of it cannot raise `NameError` and keeps the
/// static refusal ([`admits_maybe_read`]).
fn falls_back_when_unbound(name: &str) -> bool {
    const BUILTINS: &[&str] = &[
        "ArithmeticError", "AssertionError", "AttributeError", "BaseException",
        "BaseExceptionGroup", "BlockingIOError", "BrokenPipeError", "BufferError",
        "BytesWarning", "ChildProcessError", "ConnectionAbortedError", "ConnectionError",
        "ConnectionRefusedError", "ConnectionResetError", "DeprecationWarning", "EOFError",
        "Ellipsis", "EncodingWarning", "EnvironmentError", "Exception", "ExceptionGroup",
        "False", "FileExistsError", "FileNotFoundError", "FloatingPointError", "FutureWarning",
        "GeneratorExit", "IOError", "ImportError", "ImportWarning", "IndentationError",
        "IndexError", "InterruptedError", "IsADirectoryError", "KeyError", "KeyboardInterrupt",
        "LookupError", "MemoryError", "ModuleNotFoundError", "NameError", "None",
        "NotADirectoryError", "NotImplemented", "NotImplementedError", "OSError",
        "OverflowError", "PendingDeprecationWarning", "PermissionError", "ProcessLookupError",
        "PythonFinalizationError", "RecursionError", "ReferenceError", "ResourceWarning",
        "RuntimeError", "RuntimeWarning", "StopAsyncIteration", "StopIteration", "SyntaxError",
        "SyntaxWarning", "SystemError", "SystemExit", "TabError", "TimeoutError", "True",
        "TypeError", "UnboundLocalError", "UnicodeDecodeError", "UnicodeEncodeError",
        "UnicodeError", "UnicodeTranslateError", "UnicodeWarning", "UserWarning", "ValueError",
        "Warning", "ZeroDivisionError", "abs", "aiter", "all", "anext", "any", "ascii", "bin",
        "bool", "breakpoint", "bytearray", "bytes", "callable", "chr", "classmethod", "compile",
        "complex", "copyright", "credits", "delattr", "dict", "dir", "divmod", "enumerate",
        "eval", "exec", "exit", "filter", "float", "format", "frozenset", "getattr", "globals",
        "hasattr", "hash", "help", "hex", "id", "input", "int", "isinstance", "issubclass",
        "iter", "len", "license", "list", "locals", "map", "max", "memoryview", "min", "next",
        "object", "oct", "open", "ord", "pow", "print", "property", "quit", "range", "repr",
        "reversed", "round", "set", "setattr", "slice", "sorted", "staticmethod", "str", "sum",
        "super", "tuple", "type", "vars", "zip",
    ];
    (name.starts_with("__") && name.ends_with("__")) || BUILTINS.binary_search(&name).is_ok()
}

/// Records the names `hir`'s statements unbind ([`admits_maybe_read`]).
fn record_unbinding_statements(hir: &HirModule) {
    fn handler_names(body: &[HirStmt], out: &mut crate::fx::HashSet<String>) {
        for stmt in body {
            match stmt {
                HirStmt::If { body, orelse, .. } => {
                    handler_names(body, out);
                    handler_names(orelse, out);
                }
                HirStmt::While { body, .. }
                | HirStmt::ForRange { body, .. }
                | HirStmt::ForList { body, .. }
                | HirStmt::ForObject { body, .. } => handler_names(body, out),
                HirStmt::LoopElse { loop_stmt, orelse } => {
                    handler_names(std::slice::from_ref(loop_stmt.as_ref()), out);
                    handler_names(orelse, out);
                }
                HirStmt::Match { cases, .. } => {
                    for case in cases {
                        handler_names(&case.body, out);
                    }
                }
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
                    handler_names(body, out);
                    for handler in handlers {
                        out.extend(handler.name.iter().cloned());
                        handler_names(&handler.body, out);
                    }
                    handler_names(orelse, out);
                    handler_names(finalbody, out);
                }
                _ => {}
            }
        }
    }
    let mut names = crate::fx::HashSet::default();
    for item in &hir.items {
        let body = match item {
            HirItem::Function { body, .. } => body.as_slice(),
            HirItem::TopLevelStmt(stmt) => std::slice::from_ref(stmt),
        };
        names.extend(pycc_hir::deleted_names(body));
        handler_names(body, &mut names);
    }
    UNBOUND_BY_STATEMENT.with(|cell| *cell.borrow_mut() = Some(names));
}

fn take_widen_requests() -> BTreeSet<WidenKey> {
    WIDEN.with(|widen| std::mem::take(&mut *widen.borrow_mut()))
}

/// `requests` with every key that is not a local of an ordinary function
/// item of `hir` made module-wide: a name of the module scope, a name the
/// module scope binds too (a function may declare it `global`), or a
/// request from no item or from an item the check added (an inherited
/// method's copy).
fn scope_keys(hir: &HirModule, requests: BTreeSet<WidenKey>) -> BTreeSet<WidenKey> {
    let module_names = module_scope_names(hir);
    requests
        .into_iter()
        .map(|(item, name)| {
            let local = item.is_some_and(|index| {
                matches!(hir.items.get(index), Some(HirItem::Function { .. }))
                    && !module_names.contains(&name)
            });
            (if local { item } else { None }, name)
        })
        .collect()
}

/// The names a module-level statement of `hir` binds.
fn module_scope_names(hir: &HirModule) -> BTreeSet<String> {
    fn walk(body: &[HirStmt], out: &mut BTreeSet<String>) {
        for stmt in body {
            match stmt {
                HirStmt::Assign { target, .. }
                | HirStmt::AnnAssign { target, .. }
                | HirStmt::ListCompAssign { target, .. }
                | HirStmt::SetCompAssign { target, .. } => {
                    out.insert(target.clone());
                }
                HirStmt::ForRange { var, body, .. }
                | HirStmt::ForList { var, body, .. }
                | HirStmt::ForObject { var, body, .. } => {
                    out.insert(var.clone());
                    walk(body, out);
                }
                HirStmt::If { body, orelse, .. } => {
                    walk(body, out);
                    walk(orelse, out);
                }
                HirStmt::While { body, .. } => walk(body, out),
                HirStmt::LoopElse { loop_stmt, orelse } => {
                    walk(std::slice::from_ref(loop_stmt.as_ref()), out);
                    walk(orelse, out);
                }
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
                    walk(body, out);
                    for handler in handlers {
                        out.extend(handler.name.iter().cloned());
                        walk(&handler.body, out);
                    }
                    walk(orelse, out);
                    walk(finalbody, out);
                }
                HirStmt::Match { cases, .. } => {
                    for case in cases {
                        walk(&case.body, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = BTreeSet::new();
    for item in &hir.items {
        if let HirItem::TopLevelStmt(stmt) = item {
            walk(std::slice::from_ref(stmt), &mut out);
        }
    }
    out
}

/// Runs `check` over `hir`, and in non-strict mode re-runs it with every
/// requested name widened to `object` until it passes or a run requests no
/// new name (its diagnostics are then the answer).
pub(crate) fn with_widening<T, E>(
    hir: &HirModule,
    check: impl Fn(&HirModule) -> Result<T, E>,
) -> Result<T, E> {
    pycc_hir::int_enum::set_int_enums(hir.class_defs.iter().map(|(_, class)| class));
    record_unbinding_statements(hir);
    if !is_non_strict() {
        return check(hir);
    }
    // A name first bound to `None` and rebound to another value in the same
    // scope is widened up front: a read reached before any rebinding in
    // the checker's single pass (`last = None` before a loop that sets
    // `last = s` in one branch and reads `len(last)` in another) would
    // otherwise see `None` and never request the widening.
    fresh_returns::record(hir);
    let mut widened = none_first_names(hir);
    let mut widened_attrs = BTreeSet::new();
    let mut current = (!widened.is_empty()).then(|| widen(hir, &widened, &widened_attrs));
    loop {
        take_widen_requests();
        container_widen::take();
        let result = check(current.as_ref().unwrap_or(hir));
        let mut requested = retarget_temporaries(hir, scope_keys(hir, take_widen_requests()));
        requested.extend(container_widen::granted(
            hir,
            scope_keys(hir, container_widen::take()),
        ));
        let requested_attrs = WIDEN_ATTRS.with(|widen| std::mem::take(&mut *widen.borrow_mut()));
        if result.is_ok()
            || (requested.is_subset(&widened) && requested_attrs.is_subset(&widened_attrs))
        {
            return result;
        }
        widened.extend(requested);
        widened_attrs.extend(requested_attrs);
        current = Some(widen(hir, &widened, &widened_attrs));
    }
}

/// Each requested desugaring temporary (`_pycc_N`) that holds an item of a
/// named container (`_pycc_1 = rows[i]`, for `rows[i][j] = v`) is replaced
/// by that container's name: widening the temporary alone would copy the
/// item out of its container, while a widened container hands out its own
/// CPython items, so a store through one is seen through the other.
fn retarget_temporaries(hir: &HirModule, requested: BTreeSet<WidenKey>) -> BTreeSet<WidenKey> {
    requested
        .into_iter()
        .map(|(key, name)| {
            if !name.starts_with("_pycc_") {
                return (key, name);
            }
            let mut root = None;
            for (index, item) in hir.items.iter().enumerate() {
                if key.is_some_and(|key| key != index) {
                    continue;
                }
                let body = match item {
                    HirItem::Function { body, .. } => body.as_slice(),
                    HirItem::TopLevelStmt(stmt) => std::slice::from_ref(stmt),
                };
                if root.is_none() {
                    root = item_root(body, &name);
                }
            }
            (key, root.unwrap_or(name))
        })
        .collect()
}

/// The container a temporary's first assignment reads an item of.
fn item_root(body: &[HirStmt], name: &str) -> Option<String> {
    for stmt in body {
        let found = match stmt {
            HirStmt::Assign { target, value } if target == name => {
                let mut expr = value;
                let mut subscripted = false;
                while let pycc_hir::HirExpr::Subscript { base, .. } = expr {
                    expr = base;
                    subscripted = true;
                }
                return match expr {
                    pycc_hir::HirExpr::Name(root) if subscripted => Some(root.clone()),
                    _ => None,
                };
            }
            HirStmt::If { body, orelse, .. } => {
                item_root(body, name).or_else(|| item_root(orelse, name))
            }
            HirStmt::While { body, .. }
            | HirStmt::ForRange { body, .. }
            | HirStmt::ForList { body, .. }
            | HirStmt::ForObject { body, .. } => item_root(body, name),
            HirStmt::LoopElse { loop_stmt, orelse } => {
                item_root(std::slice::from_ref(loop_stmt.as_ref()), name)
                    .or_else(|| item_root(orelse, name))
            }
            HirStmt::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => item_root(body, name)
                .or_else(|| handlers.iter().find_map(|h| item_root(&h.body, name)))
                .or_else(|| item_root(orelse, name))
                .or_else(|| item_root(finalbody, name)),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

/// The names some scope of `hir` first binds to `None` with a plain
/// assignment and binds again, in the same scope, to anything else.
fn none_first_names(hir: &HirModule) -> BTreeSet<WidenKey> {
    fn walk(
        body: &[HirStmt],
        first_none: &mut std::collections::BTreeMap<String, bool>,
        rebound: &mut BTreeSet<String>,
    ) {
        for stmt in body {
            match stmt {
                HirStmt::Assign { target, value } => {
                    let is_none = matches!(value, pycc_hir::HirExpr::NoneLiteral);
                    let first = *first_none.entry(target.clone()).or_insert(is_none);
                    if first && !is_none {
                        rebound.insert(target.clone());
                    }
                }
                HirStmt::AnnAssign { target, .. } => {
                    first_none.entry(target.clone()).or_insert(false);
                }
                HirStmt::If { body, orelse, .. } => {
                    walk(body, first_none, rebound);
                    walk(orelse, first_none, rebound);
                }
                HirStmt::While { body, .. }
                | HirStmt::ForRange { body, .. }
                | HirStmt::ForList { body, .. }
                | HirStmt::ForObject { body, .. } => walk(body, first_none, rebound),
                HirStmt::LoopElse { loop_stmt, orelse } => {
                    walk(std::slice::from_ref(loop_stmt.as_ref()), first_none, rebound);
                    walk(orelse, first_none, rebound);
                }
                HirStmt::Match { cases, .. } => {
                    for case in cases {
                        walk(&case.body, first_none, rebound);
                    }
                }
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
                    walk(body, first_none, rebound);
                    for handler in handlers {
                        walk(&handler.body, first_none, rebound);
                    }
                    walk(orelse, first_none, rebound);
                    walk(finalbody, first_none, rebound);
                }
                _ => {}
            }
        }
    }
    let mut keyed = BTreeSet::new();
    let mut top_level = std::collections::BTreeMap::new();
    let mut module_rebound = BTreeSet::new();
    for (index, item) in hir.items.iter().enumerate() {
        match item {
            HirItem::Function { params, body, .. } => {
                let mut first_none = params
                    .iter()
                    .map(|(name, _)| (name.clone(), false))
                    .collect();
                let mut rebound = BTreeSet::new();
                walk(body, &mut first_none, &mut rebound);
                keyed.extend(rebound.into_iter().map(|name| (Some(index), name)));
            }
            HirItem::TopLevelStmt(stmt) => {
                walk(std::slice::from_ref(stmt), &mut top_level, &mut module_rebound);
            }
        }
    }
    keyed.extend(module_rebound.into_iter().map(|name| (None, name)));
    scope_keys(hir, keyed)
}

/// `hir` with the first plain assignment of each name in `names`, in every
/// function body and in the module body, rewritten into `name: object =
/// value`.
fn widen(
    hir: &HirModule,
    keys: &BTreeSet<WidenKey>,
    attrs: &BTreeSet<(String, String)>,
) -> HirModule {
    let everywhere: BTreeSet<String> = keys
        .iter()
        .filter(|(item, _)| item.is_none())
        .map(|(_, name)| name.clone())
        .collect();
    let mut module = hir.clone();
    // Each widened attribute slot is `object` in the class that declares
    // it and in every class of the store's MRO that redeclares it.
    for (class, attr) in attrs {
        let mro = module
            .class_defs
            .iter()
            .find(|(name, _)| name == class)
            .map(|(_, def)| def.mro.clone())
            .unwrap_or_default();
        for (name, def) in &mut module.class_defs {
            if !mro.contains(name) && name != class {
                continue;
            }
            for (slot, ty) in &mut def.attrs {
                if slot == attr {
                    *ty = Ty::Object;
                }
            }
        }
    }
    let mut top_level_seen = BTreeSet::new();
    for (index, item) in module.items.iter_mut().enumerate() {
        match item {
            HirItem::Function { params, body, .. } => {
                // A parameter already has its declared slot.
                let mut seen: BTreeSet<String> =
                    params.iter().map(|(name, _)| name.clone()).collect();
                let mut names = everywhere.clone();
                names.extend(
                    keys.iter()
                        .filter(|(item, _)| *item == Some(index))
                        .map(|(_, name)| name.clone()),
                );
                widen_block(body, &names, &mut seen);
            }
            HirItem::TopLevelStmt(stmt) => {
                widen_block(std::slice::from_mut(stmt), &everywhere, &mut top_level_seen);
            }
        }
    }
    module
}

fn widen_block(body: &mut [HirStmt], names: &BTreeSet<String>, seen: &mut BTreeSet<String>) {
    for stmt in body {
        let is_list_comp = matches!(stmt, HirStmt::ListCompAssign { .. });
        match stmt {
            HirStmt::Assign { target, value }
                if names.contains(target) && !seen.contains(target) =>
            {
                seen.insert(target.clone());
                *stmt = HirStmt::AnnAssign {
                    target: target.clone(),
                    annotation: Ty::Object,
                    value: Some(value.clone()),
                    is_final: false,
                };
            }
            HirStmt::AnnAssign { target, .. } => {
                seen.insert(target.clone());
            }
            // A comprehension's statement form, as its expression.
            HirStmt::ListCompAssign {
                target,
                var,
                iter,
                cond,
                elt,
            }
            | HirStmt::SetCompAssign {
                target,
                var,
                iter,
                cond,
                elt,
            } if names.contains(target) && !seen.contains(target) => {
                seen.insert(target.clone());
                let elt = if is_list_comp {
                    pycc_hir::CompElt::List(elt.as_ref().clone())
                } else {
                    pycc_hir::CompElt::Set(elt.as_ref().clone())
                };
                let value =
                    pycc_hir::HirExpr::Comprehension(Box::new(pycc_hir::HirComprehension {
                        var: var.clone(),
                        iter: iter.clone(),
                        cond: cond.as_deref().cloned(),
                        elt,
                    }));
                *stmt = HirStmt::AnnAssign {
                    target: target.clone(),
                    annotation: Ty::Object,
                    value: Some(value),
                    is_final: false,
                };
            }
            // Each branch is checked from the environment before the
            // statement, so each declares the slot of a name it binds
            // first.
            HirStmt::If { body, orelse, .. } => {
                let mut orelse_seen = seen.clone();
                widen_block(body, names, seen);
                widen_block(orelse, names, &mut orelse_seen);
                seen.extend(orelse_seen);
            }
            HirStmt::While { body, .. }
            | HirStmt::ForRange { body, .. }
            | HirStmt::ForList { body, .. }
            | HirStmt::ForObject { body, .. } => widen_block(body, names, seen),
            HirStmt::LoopElse { loop_stmt, orelse } => {
                widen_block(std::slice::from_mut(loop_stmt.as_mut()), names, seen);
                widen_block(orelse, names, seen);
            }
            HirStmt::Match { cases, .. } => {
                for case in cases {
                    widen_block(&mut case.body, names, seen);
                }
            }
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
                widen_block(body, names, seen);
                for handler in handlers {
                    widen_block(&mut handler.body, names, seen);
                }
                widen_block(orelse, names, seen);
                widen_block(finalbody, names, seen);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pycc_hir::HirExpr;

    #[test]
    fn only_non_strict_converts_an_object_into_a_scalar_slot() {
        set_non_strict(false);
        assert!(!admits_coercion(&Ty::Object, &Ty::Int));
        set_non_strict(true);
        for to in [Ty::Int, Ty::Float, Ty::Bool, Ty::Str] {
            assert!(admits_coercion(&Ty::Object, &to), "{to:?}");
        }
        assert!(!admits_coercion(&Ty::Object, &Ty::List(Box::new(Ty::Int))));
        assert!(!admits_coercion(&Ty::Int, &Ty::Str));
        set_non_strict(false);
    }

    #[test]
    fn widening_rewrites_the_first_assignment_in_each_scope_only() {
        let assign = |value| HirStmt::Assign {
            target: "t".to_string(),
            value: HirExpr::IntLiteral(value),
        };
        let mut body = vec![
            HirStmt::If {
                test: HirExpr::BoolLiteral(true),
                body: vec![assign(1)],
                orelse: vec![],
            },
            assign(2),
        ];
        let names = BTreeSet::from(["t".to_string()]);
        widen_block(&mut body, &names, &mut BTreeSet::new());
        let HirStmt::If { body: inner, .. } = &body[0] else {
            unreachable!()
        };
        assert!(matches!(
            &inner[0],
            HirStmt::AnnAssign {
                annotation: Ty::Object,
                ..
            }
        ));
        assert_eq!(body[1], assign(2));
    }
}
