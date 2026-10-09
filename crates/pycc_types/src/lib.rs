mod binop;
mod boolop;
mod buffer;
mod class;
mod compare_chain;
mod comprehension;
mod constraints;
mod del_stmt;
mod empty_container;
mod enum_lower;
mod env;
mod exception;
mod expr;
mod foreign;
mod frozenset;
mod hash;
mod if_exp;
mod inherited_copies;
mod local_binder;
mod module;
mod monomorphize;
mod narrow;
mod not_implemented;
mod object_box;
mod object_none;
mod redeclaration;
mod set_element;
mod solver;
mod std_receiver;
mod string_conversion;
#[cfg(test)]
mod tests;
mod unop;
mod unpack;

pub(crate) use local_binder::{bind_local_types_in_body, bind_local_types_in_stmt};
use pycc_hir::block_always_returns;

pub use buffer::{
    function_local_producer_spellings, imported_producer_spellings, is_buffer_producer_spelling,
};
use comprehension::{CompElts, CompView, check_comp_assign};
pub(crate) use enum_lower::{
    check_enum_loop_body_function, check_enum_loop_body_module, enum_member_attr_type,
    unroll_enum_loops,
};
pub(crate) use env::BindingState;
pub use env::Environment;
use exception::{
    check_raise_stmt, check_try_star_stmt, check_try_stmt, is_unshadowed_builtin_exception,
};
pub use expr::infer_expr;
pub(crate) use expr::{infer_expr_in, reject_memoryview_declaration, reject_memoryview_read};
pub(crate) use redeclaration::{
    check_incompatible_attribute_redeclarations, check_incompatible_redefinitions,
};

pub(crate) use constraints::*;
pub(crate) use module::module_function_local_names;
pub use module::{
    DiagnosticKey, KeyedDiagnostics, check, check_all, check_all_keyed, check_and_resolve,
    check_and_resolve_all, check_and_resolve_all_keyed,
};
#[cfg(test)]
pub(crate) use module::{check_with_signatures, checked_function_signatures};
pub use monomorphize::*;

use pycc_diag::{Diagnostic, Span};
#[cfg(test)]
use pycc_hir::BinOpKind;
#[cfg(test)]
use pycc_hir::CmpOpKind;
#[cfg(test)]
use pycc_hir::PropertyDef;
pub use pycc_hir::Ty;
#[cfg(test)]
use pycc_hir::UnaryOpKind;
use pycc_hir::{
    CompIter, FStringPart, HirClassDef, HirExpr, HirItem, HirMatchCase, HirModule, HirPattern,
    HirStmt,
};
use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::sync::Arc;

fn unbound_local(name: &str) -> Diagnostic {
    Diagnostic::error(
        "T0021",
        format!("local name `{name}` is not bound before this use"),
        Span::new(0, 0),
    )
}

/// Issue #118 Part 1: a name that is *maybe* bound (assigned on some but not
/// all paths reaching this use) is not safely readable. CPython raises
/// `NameError`/`UnboundLocalError` for the same control-flow shapes; the
/// strict AOT frontend rejects the read with `T0041` instead, distinguishing
/// "possibly unbound" from "never bound" (`T0021`) for actionable diagnostics.
fn possibly_unbound(name: &str) -> Diagnostic {
    Diagnostic::error(
        "T0041",
        format!("local name `{name}` may not be bound on every path reaching this use"),
        Span::new(0, 0),
    )
}

fn non_callable_binding(name: &str) -> Diagnostic {
    Diagnostic::error(
        "T0021",
        format!("name `{name}` is bound to a non-callable value"),
        Span::new(0, 0),
    )
}

fn std_scalar_to_ty(kind: pycc_std::ScalarKind) -> Ty {
    match kind {
        pycc_std::ScalarKind::Float => Ty::Float,
    }
}

/// A stdlib function symbol (e.g. `math.sqrt`) referenced without a call
/// (`print(math.sqrt)`, not `math.sqrt(x)`) has no callable `Ty` this
/// compiler's type system can express -- there is no first-class function
/// type here, matching `non_callable_binding`'s own "this compiler's
/// value types are all primitives" precedent (D-110's doc comment above).
fn std_function_used_as_a_value(name: &str) -> Diagnostic {
    Diagnostic::error(
        "T0021",
        format!("`{name}` is a stdlib function and must be called, e.g. `{name}(...)`"),
        Span::new(0, 0),
    )
    .with_help(format!("call it: `{name}(...)`"))
}

/// A stdlib constant (e.g. `math.pi`) called like a function
/// (`math.pi()`) -- `pycc_hir`'s lowering does not distinguish
/// `StdSymbolKind::Function` from `StdSymbolKind::Constant` at the
/// call-position (it resolves any registered symbol name into
/// `HirExpr::Call`), so this mismatch surfaces here instead.
fn std_constant_is_not_callable(name: &str) -> Diagnostic {
    Diagnostic::error(
        "T0021",
        format!("`{name}` is a stdlib constant, not a function, and cannot be called"),
        Span::new(0, 0),
    )
}

/// A stdlib class marker (e.g. `enum.Enum`) referenced as a first-class
/// value (`print(enum.Enum)`, not `class C(Enum):`). `Enum` is only a
/// marker for enum class detection — it has no runtime representation
/// this compiler can emit as a value.
fn enum_marker_is_not_a_value(name: &str) -> Diagnostic {
    Diagnostic::error(
        "T0021",
        format!(
            "`{name}` is a class marker, not a first-class value — use it only as a base class (`class C(Enum):`)"
        ),
        Span::new(0, 0),
    )
}

/// A stdlib marker symbol (protocol marker, ABC marker, or decorator
/// marker) referenced as a first-class value (#380, PR-20). These symbols
/// are only valid as base-class markers or decorators — they have no
/// runtime representation this compiler can emit as a value.
fn marker_is_not_a_value(name: &str) -> Diagnostic {
    Diagnostic::error(
        "T0021",
        format!(
            "`{name}` is a marker symbol, not a first-class value — use it only as a base class marker or decorator"
        ),
        Span::new(0, 0),
    )
}

/// An annotation-subscript marker (`typing.Final`, `typing.Annotated`,
/// #762) referenced as a first-class value or called directly, instead of
/// being used as an annotation subscript (`Final[int]`, `Annotated[int,
/// ...]`). Unlike the other marker kinds, `Final`/`Annotated` are never
/// valid as a base class or a decorator, so `marker_is_not_a_value`'s
/// generic guidance would be misleading here (review finding on PR #766).
fn annotation_marker_is_not_a_value(name: &str) -> Diagnostic {
    Diagnostic::error(
        "T0021",
        format!(
            "`{name}` is an annotation marker, not a first-class value — use it only as an annotation subscript (e.g. `{name}[int]`)"
        ),
        Span::new(0, 0),
    )
}

/// The `typing.cast` marker (#767) referenced as a first-class value, or
/// called through its qualified name (`typing.cast(int, x)`) instead of the
/// bare name imported with `from typing import cast`. `cast` is recognized
/// by bare callee name in `infer_expr_in`/`collect_expr_constraints` (like
/// `isinstance`/`issubclass`), so neither the qualified call form nor a
/// value reference resolves to the special case; both land here.
fn cast_marker_is_not_a_value(name: &str) -> Diagnostic {
    Diagnostic::error(
        "T0021",
        format!(
            "`{name}` is a compile-time cast marker, not a first-class value — import it with `from typing import cast` and call the bare name (e.g. `cast(int, value)`)"
        ),
        Span::new(0, 0),
    )
}

/// The `typing.TYPE_CHECKING` marker (#790) referenced as a first-class
/// value, or called, instead of being used exactly as the (possibly
/// negated-free) test of an `if`/`elif` statement. pycc constant-folds
/// `if TYPE_CHECKING: ...` in `pycc_hir` before type-checking ever sees the
/// test expression (see `pycc_std::StdSymbolKind::TypeCheckingMarker`'s own
/// doc comment) -- this diagnostic only fires for the qualified
/// `typing.TYPE_CHECKING` spelling used somewhere else, such as `x =
/// typing.TYPE_CHECKING` or `typing.TYPE_CHECKING()`.
fn type_checking_marker_is_not_a_value(name: &str) -> Diagnostic {
    Diagnostic::error(
        "T0021",
        format!(
            "`{name}` is a compile-time marker, not a first-class value — use it only as the test of an `if TYPE_CHECKING:` guard"
        ),
        Span::new(0, 0),
    )
}

/// A typing-form marker (#1378: `typing.Dict`, `List`, `Set`, `FrozenSet`,
/// `Tuple`, `Any`, `Generic`) referenced as a first-class value or called
/// directly. The container aliases are only meaningful as annotation
/// subscripts, `Any` only as an annotation, and `Generic` only as a base, so
/// no single usage example fits all seven -- this message carries none,
/// rather than borrowing `annotation_marker_is_not_a_value`'s `{name}[int]`
/// advice that is wrong for `Dict`/`Tuple` arity and false for `Any`.
fn typing_form_is_not_a_value(name: &str) -> Diagnostic {
    Diagnostic::error(
        "T0021",
        format!(
            "`{name}` is a typing construct, not a first-class value — pycc does not support using it as a value"
        ),
        Span::new(0, 0),
    )
}

/// Returns `true` if `kind` is any marker symbol kind (Enum, Protocol, ABC,
/// Decorator, Annotation, Cast, TypeChecking, or TypingForm). Used by call-site and
/// value-reference guards to reject marker symbols used as first-class
/// values with a consistent diagnostic.
fn is_marker_kind(kind: pycc_std::StdSymbolKind) -> bool {
    matches!(
        kind,
        pycc_std::StdSymbolKind::EnumMarker
            | pycc_std::StdSymbolKind::ProtocolMarker
            | pycc_std::StdSymbolKind::AbcMarker
            | pycc_std::StdSymbolKind::DecoratorMarker
            | pycc_std::StdSymbolKind::EnumAutoMarker
            | pycc_std::StdSymbolKind::AnnotationMarker
            | pycc_std::StdSymbolKind::CastMarker
            | pycc_std::StdSymbolKind::TypeCheckingMarker
            | pycc_std::StdSymbolKind::TypingFormMarker
    )
}

/// Issue #142: the sorted set of known Python 3.14 callable builtin names
/// that this compiler version does not implement. These are valid Python --
/// `ValueError("x")`, `Exception("msg")`, `int("5")`, `range(10)` (as a
/// standalone call, not a `for`-loop iterable) -- but the current pycc slice
/// only hand-recognizes `print`, `len`, and `float`. A bare-name call to any
/// name in this table is a *capability gap* (`C0001`), not a name-resolution
/// failure (`T0021`): the builtin genuinely exists in Python 3.14, this
/// compiler just does not implement it yet.
///
/// The table is the 139 callable, non-dunder, non-`site`-module names in
/// Python 3.14's `builtins` module, minus the three already implemented
/// (`print`, `len`, `float`), plus `__import__` (the one callable dunder
/// that users legitimately call), yielding 137. `range` is included
/// because it only works inside `for` loops (via `HirStmt::ForRange`), not
/// as a standalone call. `site`-module additions (`copyright`, `credits`,
/// `exit`, `help`, `license`, `quit`) are excluded because they are not
/// part of the `builtins` module proper and are host-dependent. Other
/// dunders (`__build_class__`, `__debug__`, etc.) are excluded as
/// internal/interpreter-implementation details users do not call directly.
///
/// **Invariant:** the array is kept sorted lexicographically by Rust's `str`
/// ordering (byte-wise UTF-8, which for ASCII identifiers matches ASCII
/// code-point order) so `is_known_callable_builtin` can use binary search.
/// A unit test (`known_callable_builtins_table_is_sorted`) asserts this.
const KNOWN_CALLABLE_BUILTINS: &[&str] = &[
    "ArithmeticError",
    "AssertionError",
    "AttributeError",
    "BaseException",
    "BaseExceptionGroup",
    "BlockingIOError",
    "BrokenPipeError",
    "BufferError",
    "BytesWarning",
    "ChildProcessError",
    "ConnectionAbortedError",
    "ConnectionError",
    "ConnectionRefusedError",
    "ConnectionResetError",
    "DeprecationWarning",
    "EOFError",
    "EncodingWarning",
    "EnvironmentError",
    "Exception",
    "ExceptionGroup",
    "FileExistsError",
    "FileNotFoundError",
    "FloatingPointError",
    "FutureWarning",
    "GeneratorExit",
    "IOError",
    "ImportError",
    "ImportWarning",
    "IndentationError",
    "IndexError",
    "InterruptedError",
    "IsADirectoryError",
    "KeyError",
    "KeyboardInterrupt",
    "LookupError",
    "MemoryError",
    "ModuleNotFoundError",
    "NameError",
    "NotADirectoryError",
    "NotImplementedError",
    "OSError",
    "OverflowError",
    "PendingDeprecationWarning",
    "PermissionError",
    "ProcessLookupError",
    "PythonFinalizationError",
    "RecursionError",
    "ReferenceError",
    "ResourceWarning",
    "RuntimeError",
    "RuntimeWarning",
    "StopAsyncIteration",
    "StopIteration",
    "SyntaxError",
    "SyntaxWarning",
    "SystemError",
    "SystemExit",
    "TabError",
    "TimeoutError",
    "TypeError",
    "UnboundLocalError",
    "UnicodeDecodeError",
    "UnicodeEncodeError",
    "UnicodeError",
    "UnicodeTranslateError",
    "UnicodeWarning",
    "UserWarning",
    "ValueError",
    "Warning",
    "ZeroDivisionError",
    "__import__",
    "abs",
    "aiter",
    "all",
    "anext",
    "any",
    "ascii",
    "bin",
    "bool",
    "breakpoint",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "filter",
    "format",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "hex",
    "id",
    "input",
    "int",
    "iter",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
];

/// Issue #142: returns `true` if `name` is a known Python 3.14 callable
/// builtin that this compiler version does not implement. Uses binary search
/// over the sorted [`KNOWN_CALLABLE_BUILTINS`] table. Called only after
/// user-defined function lookup (and the `print`/`len`/`float`/stdlib/class
/// special cases) have all missed, so a user `def ValueError(...)` always
/// takes priority over this classification.
fn is_known_callable_builtin(name: &str) -> bool {
    KNOWN_CALLABLE_BUILTINS.binary_search(&name).is_ok()
}

/// Issue #142: the `C0001` diagnostic for a call to a known but unsupported
/// callable builtin (e.g. `ValueError("x")`). Distinct from `T0021`'s "call
/// to undefined function" -- the builtin genuinely exists in Python 3.14,
/// this compiler just does not implement it yet.
fn unsupported_callable_builtin(name: &str) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!("call to builtin `{name}` is valid Python but not implemented yet"),
        Span::new(0, 0),
    )
}

/// Looks up a bare name's already-bound type, producing the same
/// "unbound local" vs. "not defined" distinction `HirExpr::Name` itself
/// uses in `infer_expr_in` below. `HirStmt::ForList`'s `list` field and
/// `HirExpr::ListAppend`'s `list` field are both plain `String`s rather
/// than `HirExpr::Name` nodes (D-105's HIR shape), so they can't go
/// through `infer_expr_in`'s own `Name` arm and need this helper instead.
fn lookup_bound_name(
    env: &Environment,
    local_names: &[&str],
    name: &str,
) -> Result<Ty, Diagnostic> {
    lookup_bound_name_inner(env, local_names, name, false)
}

/// [`lookup_bound_name`] with the `memoryview` read refusal suppressed, for
/// the **one** position that admits a buffer-bound name: the target of an
/// element store, `b[i] = v` (Part 1 of #1142).
///
/// A separate entry point and not a relaxation of the refusal itself. Every
/// other difference from [`lookup_bound_name`] would be a language-wide
/// regression -- `reject_object_read`, the possibly-unbound arm, the
/// unbound-local arm and the "name is not defined" `T0021` all apply to
/// `d[k] = v` for every base -- so the two share one body and differ by
/// exactly the one line this flag guards.
fn lookup_bound_name_for_store(
    env: &Environment,
    local_names: &[&str],
    name: &str,
) -> Result<Ty, Diagnostic> {
    lookup_bound_name_inner(env, local_names, name, true)
}

fn lookup_bound_name_inner(
    env: &Environment,
    local_names: &[&str],
    name: &str,
    admit_buffer: bool,
) -> Result<Ty, Diagnostic> {
    // Issue #118 Part 1: three-way distinction -- definitely bound -> ok, maybe
    // bound -> T0041, unbound -> T0021 (local) or "not defined" (global).
    match env.binding_state(name) {
        Some(BindingState::Definitely(ty)) => {
            // Part 1 of #1026, choke point 2: `for x in numpy:` and
            // `[e for x in numpy]` lower to `HirStmt::ForList` /
            // `HirExpr::Comprehension`, whose iterable is a plain `String`
            // (D-105), so they reach the binding through this helper
            // rather than through `infer_expr_in`'s `Name` arm.
            crate::foreign::reject_object_read(name, ty)?;
            // Part 1 of #1027, the same choke point for the same reason: a
            // `memoryview` parameter reached by `for x in v` bypasses the
            // `Name` arm's own guard, and without this call the iteration is
            // refused as a `T0033` type error instead of the `C0001`
            // capability gap the type actually is.
            if !admit_buffer {
                // Part 2a of #1142 (#1165): provenance comes from the same
                // environment the binding did, so an artifact-owned name
                // iterated with `for x in a` gets the owned refusal rather
                // than one that calls it a borrowed parameter.
                reject_memoryview_read(name, ty, env.owned_buffers.contains(name))?;
            }
            Ok(ty.clone())
        }
        Some(BindingState::Maybe(_)) => Err(possibly_unbound(name)),
        None => {
            if is_local(local_names, name) {
                Err(unbound_local(name))
            } else {
                Err(Diagnostic::error(
                    "T0021",
                    format!("name `{name}` is not defined"),
                    Span::new(0, 0),
                ))
            }
        }
    }
}

/// True when `ty` contains a `Ty::Param` anywhere in its structure,
/// including nested inside a container (D-133/D-134). Unlike
/// `scan_signature_ty_for_param` (which additionally *rejects* a
/// container-position occurrence with `T0042`), this is a plain structural
/// predicate used only to decide whether a function needs generic
/// treatment at all -- the shape gate itself still runs via
/// `check_generic_function`/`generic_type_param_name` for a function this
/// predicate says yes to.
fn ty_contains_param(ty: &Ty) -> bool {
    match ty {
        Ty::Param(_) => true,
        Ty::List(inner) | Ty::Set(inner) | Ty::FrozenSet(inner) | Ty::Optional(inner) => {
            ty_contains_param(inner)
        }
        Ty::Dict(kv) => ty_contains_param(&kv.0) || ty_contains_param(&kv.1),
        Ty::Tuple(elems) => elems.iter().any(ty_contains_param),
        // D-154: an instance's payload is only its class's name, never a
        // `Ty::Param` (or anything else `Ty`-shaped) -- a class-typed
        // annotation isn't even resolvable yet (`annotation_to_ty` has no
        // arm for a bare class name, `pycc_hir::class`'s own doc comment),
        // so `Ty::Instance` can never carry a generic type parameter to
        // scan for.
        Ty::Int
        | Ty::Float
        | Ty::Bool
        | Ty::Str
        | Ty::None
        | Ty::Infer
        | Ty::Instance(_)
        | Ty::Protocol(_)
        | Ty::Object
        | Ty::MemoryView => false,
    }
}

/// True when a function's signature makes it a PEP 695 generic function
/// (D-133/D-134) -- i.e. `check_and_resolve`/`check` must route it through
/// `check_generic_function`/`instantiate_generic_call` instead of the
/// ordinary concrete-`Ty` path.
fn is_generic_signature(params: &[(String, Ty)], return_ty: &Ty) -> bool {
    params.iter().any(|(_, ty)| ty_contains_param(ty)) || ty_contains_param(return_ty)
}

fn function_local_names<'a>(params: &'a [(String, Ty)], body: &'a [HirStmt]) -> Vec<&'a str> {
    let mut names = params
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    collect_local_names(body, &mut names);
    names
}

/// PEP 572 (#774): walks `expr` for every `HirExpr::NamedExpr { name, .. }`
/// node at any depth and pushes each `name` into `names` (skipping one
/// already recorded, mirroring every other arm in `collect_local_names`
/// below). A walrus target binds into the *enclosing function scope*
/// (`lower_stmt`'s own placement restriction limits where a `NamedExpr` can
/// appear to an `if`/`while` test or a bare expression statement -- never
/// inside a nested function/comprehension scope), so this walk does not need
/// to worry about crossing a scope boundary the way a general free-variable
/// analysis would.
pub(crate) fn collect_named_expr_names_in_expr<'a>(expr: &'a HirExpr, names: &mut Vec<&'a str>) {
    match expr {
        HirExpr::NamedExpr { name, value } => {
            collect_named_expr_names_in_expr(value, names);
            if !is_local(names, name) {
                names.push(name);
            }
        }
        HirExpr::IntLiteral(_)
        | HirExpr::FloatLiteral(_)
        | HirExpr::BoolLiteral(_)
        | HirExpr::StringLiteral(_)
        | HirExpr::EmptyList(_)
        | HirExpr::EmptyDict(_)
        | HirExpr::NoneLiteral
        | HirExpr::NotImplemented
        | HirExpr::Name(_)
        | HirExpr::Super => {}
        HirExpr::ListPop { list } => {
            if let Some(receiver) = list.attr_expr() {
                collect_named_expr_names_in_expr(receiver, names);
            }
        }
        // #1254 (D-250): lowering refuses a walrus inside a comprehension,
        // and its loop variable is node-scoped, not a local of the function.
        HirExpr::Comprehension(_) => {}
        HirExpr::Call { args, .. } => {
            for arg in args {
                collect_named_expr_names_in_expr(arg, names);
            }
        }
        HirExpr::CompareChain { first, links } => {
            for operand in pycc_hir::compare_chain_operands(first, links) {
                collect_named_expr_names_in_expr(operand, names);
            }
        }
        HirExpr::BinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            collect_named_expr_names_in_expr(left, names);
            collect_named_expr_names_in_expr(right, names);
        }
        HirExpr::IfExp { test, body, orelse } => {
            for part in [test, body, orelse] {
                collect_named_expr_names_in_expr(part, names);
            }
        }
        HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
            collect_named_expr_names_in_expr(operand, names)
        }
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(inner) = part {
                    collect_named_expr_names_in_expr(inner, names);
                }
            }
        }
        HirExpr::ListLiteral(es)
        | HirExpr::ObjectList(es)
        | HirExpr::SetLiteral(es)
        | HirExpr::TupleLiteral(es) => {
            for e in es {
                collect_named_expr_names_in_expr(e, names);
            }
        }
        HirExpr::Subscript { base, index } => {
            collect_named_expr_names_in_expr(base, names);
            collect_named_expr_names_in_expr(index, names);
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            collect_named_expr_names_in_expr(base, names);
            for bound in [start, stop, step].into_iter().flatten() {
                collect_named_expr_names_in_expr(bound, names);
            }
        }
        HirExpr::ListAppend { list, value } => {
            if let Some(receiver) = list.attr_expr() {
                collect_named_expr_names_in_expr(receiver, names);
            }
            collect_named_expr_names_in_expr(value, names);
        }
        HirExpr::SetAdd { value, .. } => collect_named_expr_names_in_expr(value, names),
        HirExpr::DictLiteral(pairs) => {
            for (k, v) in pairs {
                collect_named_expr_names_in_expr(k, names);
                collect_named_expr_names_in_expr(v, names);
            }
        }
        HirExpr::DictGetOrDefault { dict, key, default } => {
            if let Some(receiver) = dict.attr_expr() {
                collect_named_expr_names_in_expr(receiver, names);
            }
            collect_named_expr_names_in_expr(key, names);
            collect_named_expr_names_in_expr(default, names);
        }
        HirExpr::AttrGet { base, .. } => collect_named_expr_names_in_expr(base, names),
        HirExpr::MethodCall { base, args, .. } => {
            collect_named_expr_names_in_expr(base, names);
            for arg in args {
                collect_named_expr_names_in_expr(arg, names);
            }
        }
        HirExpr::ReceiverDispatchedCall { call, .. } => {
            collect_named_expr_names_in_expr(call, names)
        }
        HirExpr::KeywordCall { call, keywords, .. } => {
            collect_named_expr_names_in_expr(call, names);
            for (_, value) in keywords {
                collect_named_expr_names_in_expr(value, names);
            }
        }
        HirExpr::GenericClassInstantiate { args, .. } => {
            for arg in args {
                collect_named_expr_names_in_expr(arg, names);
            }
        }
        HirExpr::ExprCall { callee, args } => {
            collect_named_expr_names_in_expr(callee, names);
            for arg in args {
                collect_named_expr_names_in_expr(arg, names);
            }
        }
        HirExpr::ReceiverClassCall { args } => {
            for arg in args {
                collect_named_expr_names_in_expr(arg, names);
            }
        }
    }
}

fn collect_local_names<'a>(body: &'a [HirStmt], names: &mut Vec<&'a str>) {
    for stmt in body {
        match stmt {
            HirStmt::Assign { target, .. } => {
                if !is_local(names, target) {
                    names.push(target);
                }
            }
            // #1244: `del x` makes `x` local to the function exactly as an
            // assignment does (CPython's `UnboundLocalError` when the
            // function never binds it first).
            HirStmt::AnnAssign { target, .. } | HirStmt::Delete { name: target } => {
                if !is_local(names, target) {
                    names.push(target);
                }
            }
            // An `import` binds its local names as an assignment does
            // (#1291; `pycc_hir` produces this node only at module level).
            HirStmt::ForeignImport { bindings, .. } => {
                for (target, _, _) in bindings {
                    if !is_local(names, target) {
                        names.push(target);
                    }
                }
            }
            HirStmt::If { test, body, orelse } => {
                collect_named_expr_names_in_expr(test, names);
                collect_local_names(body, names);
                collect_local_names(orelse, names);
            }
            HirStmt::While { test, body } => {
                collect_named_expr_names_in_expr(test, names);
                collect_local_names(body, names);
            }
            HirStmt::ForRange { var, body, .. } => {
                if !is_local(names, var) {
                    names.push(var);
                }
                collect_local_names(body, names);
            }
            HirStmt::ForList { var, body, .. } | HirStmt::ForObject { var, body, .. } => {
                if !is_local(names, var) {
                    names.push(var);
                }
                collect_local_names(body, names);
            }
            // PR-12 Task 3 (D-117): a comprehension introduces two new local
            // names where a plain `for` loop introduces one -- its own
            // `target` (the comprehension's result) and its synthesized
            // `var` (the loop variable, already collision-proof by
            // construction, see `pycc_hir`'s `synthesize_comp_var_name`).
            // Neither has a body to recurse into (a comprehension is not a
            // nested block of statements).
            HirStmt::ListCompAssign { target, var, .. }
            | HirStmt::SetCompAssign { target, var, .. }
            | HirStmt::DictCompAssign { target, var, .. } => {
                if !is_local(names, target) {
                    names.push(target);
                }
                if !is_local(names, var) {
                    names.push(var);
                }
            }
            // `d[k] = v` (PR-11 Task 3) reassigns an existing binding's
            // contents, not a name -- unlike `Assign`/`AnnAssign`/`ForList`
            // above, it introduces no new local name to collect.
            // `base.attr = value` (D-154) is the same shape: it mutates an
            // existing instance's attribute slot, never binds a new local
            // name.
            HirStmt::ExprStmt(expr) => collect_named_expr_names_in_expr(expr, names),
            // A slice or attribute `del` (Part 2c of #1371, #1457) binds no
            // name, and `pycc_hir` refuses a walrus in its operands.
            HirStmt::Return(_)
            | HirStmt::DictSet { .. }
            | HirStmt::AttrSet { .. }
            | HirStmt::DeleteSlice { .. }
            | HirStmt::DeleteAttr { .. }
            | HirStmt::Raise { .. } => {}
            HirStmt::Match { cases, .. } => {
                for case in cases {
                    collect_pattern_capture_names(&case.pattern, names);
                    collect_local_names(&case.body, names);
                }
            }
            // Part 3 of #382 (#542): `except*` collects local names exactly
            // like plain `try`/`except` -- its `as` binding introduces a
            // local the same way, whatever the bound type turns out to be.
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
                collect_local_names(body, names);
                for handler in handlers {
                    if let Some(name) = &handler.name
                        && !is_local(names, name)
                    {
                        names.push(name);
                    }
                    collect_local_names(&handler.body, names);
                }
                collect_local_names(orelse, names);
                collect_local_names(finalbody, names);
            }
        }
    }
}

/// PEP 634-636 (#381, PR-21): collects all capture names introduced by a
/// pattern (recursively), for `collect_local_names`'s pre-pass.
pub(crate) fn collect_pattern_capture_names<'a>(pattern: &'a HirPattern, names: &mut Vec<&'a str>) {
    match pattern {
        HirPattern::Wildcard
        | HirPattern::Literal(_)
        | HirPattern::Singleton(_)
        | HirPattern::NoneSingleton => {}
        HirPattern::Capture(name) => {
            if !is_local(names, name) {
                names.push(name);
            }
        }
        HirPattern::Sequence(subs) | HirPattern::Or(subs) => {
            for sub in subs {
                collect_pattern_capture_names(sub, names);
            }
        }
        HirPattern::SequenceStar(subs, rest) => {
            for sub in subs {
                collect_pattern_capture_names(sub, names);
            }
            if let Some(rest) = rest
                && !is_local(names, rest)
            {
                names.push(rest);
            }
        }
        HirPattern::Mapping(pairs, rest) => {
            for (_, sub) in pairs {
                collect_pattern_capture_names(sub, names);
            }
            if let Some(rest) = rest
                && !is_local(names, rest)
            {
                names.push(rest);
            }
        }
        HirPattern::Class {
            positional,
            keyword,
            ..
        } => {
            for sub in positional {
                collect_pattern_capture_names(sub, names);
            }
            for (_, sub) in keyword {
                collect_pattern_capture_names(sub, names);
            }
        }
        HirPattern::As(inner, name) => {
            collect_pattern_capture_names(inner, names);
            if !is_local(names, name) {
                names.push(name);
            }
        }
    }
}

fn is_local(local_names: &[&str], name: &str) -> bool {
    local_names.contains(&name)
}

fn is_assignable(from: Ty, to: Ty) -> bool {
    from == to
    || (from == Ty::Bool && to == Ty::Int) // bool is a subtype of int, TYPE_SYSTEM.md's representation table
    // PEP 695 (#387): a `Ty::Param` in a generic class's method signature
    // is accepted as matching any concrete scalar type during type checking,
    // because `monomorphize` will substitute the type parameter with the
    // correct concrete type before MIR/codegen. The `GenericClassInstantiate`
    // expression already validates that the type argument is a scalar
    // (int/float/bool/str) at HIR-lowering time, so this is safe.
    // Both directions are needed: `to == Ty::Param` for `self.v = arg` where
    // the attribute slot is `Ty::Param`, and `from == Ty::Param` for
    // `return self.v` where the attribute read yields `Ty::Param` and the
    // function's return type is a concrete scalar. A non-generic function
    // (e.g. `Use.fetch`) that reads a generic class instance's attribute
    // (`b.v` where `b: Box[T]`) and returns it as `int` also relies on the
    // `from == Ty::Param` direction: during type checking `b.v` is still
    // `Ty::Param` (monomorphization has not yet substituted it).
    //
    // The `from == Ty::Param` direction is, however, unsafe for a *generic
    // function's own* type parameter: `def bad[T](x: T) -> int: return x`
    // would pass here (T assignable to int) then panic at codegen once T is
    // substituted with `str` at the call site. That case is rejected
    // separately in `check_generic_function_in` / `check_function_in` via
    // the threaded `own_type_param` context (see `check_stmt_in_function`'s
    // `Return` arm), not by narrowing this clause -- `is_assignable` has no
    // call-site context to distinguish a class-owned `Ty::Param` from a
    // function-owned one.
    || matches!(to, Ty::Param(_)) && matches!(from, Ty::Int | Ty::Float | Ty::Bool | Ty::Str)
    || matches!(from, Ty::Param(_)) && matches!(to, Ty::Int | Ty::Float | Ty::Bool | Ty::Str)
    // `T | None` (PEP 604, D-197, #763, Part 1 of #747): both a bare `inner`
    // value and a bare `None` widen to `Ty::Optional(inner)` -- matching
    // D-086's "no implicit widening OR narrowing" stance means this is the
    // *only* direction: `Ty::Optional(inner)` is deliberately NOT assignable
    // back to a bare `inner` here. No flow-sensitive narrowing exists yet
    // anywhere in this crate (`is None`/`is not None` only ever produces a
    // `Ty::Bool` presence result, see `expr.rs`'s `Compare` arm) -- an
    // `Optional[int]` stays `Optional[int]` on every path, including inside
    // an `if x is not None:` branch. Narrowing is tracked as Part 2 follow-up
    // work (issue #747), deliberately deferred out of this PR: the
    // `Optional[int]` representation and `is`/`is not` presence test are
    // independently useful and verifiable (see the conformance fixture at
    // `tests/fixtures/pep_0604_union.py`, which reads the presence result
    // directly rather than an unwrapped payload) without it.
    || matches!(&to, Ty::Optional(inner) if from == Ty::None || is_assignable(from.clone(), (**inner).clone()))
}

/// The canonical `T0025`: an annotated assignment whose initializer type is
/// not assignable to the declared annotation.
///
/// One function rather than one construction per arm because three call
/// sites now raise it -- `check_stmt`'s and `check_stmt_in_function`'s
/// `AnnAssign` arms, and `constraints::reject_producer_annotation_mismatch`,
/// the solver's mirror of the second (Part 2a of #1142, issue #1165).
/// AGENTS.md's no-paraphrase rule makes that decisive: the solver's answer
/// beats the check phase's under `crate::module::merge_solver_first`, so a
/// message that drifted from this one would be the message the user sees.
pub(crate) fn annotation_initializer_mismatch(
    target: &str,
    inferred: &Ty,
    annotation: &Ty,
) -> Diagnostic {
    Diagnostic::error(
        "T0025",
        format!(
            "cannot assign `{}` to `{target}: {}`, initializer does not match the declared annotation",
            inferred.name(),
            annotation.name()
        ),
        Span::new(0, 0),
    ).with_help(foreign::object_into_scalar_help(inferred, annotation).unwrap_or_else(|| format!("change the value to `{}` (the expected/declared type), or the declaration/annotation to `{}` (the actual type)", annotation.name(), inferred.name())))
}

/// The canonical PEP 591 `T0045`: a second assignment to a `Final` name.
///
/// Shared with `constraints::reject_final_rebinding`, the solver's mirror of
/// the [`check_assignment`] refusal above, for the reason
/// [`annotation_initializer_mismatch`] is shared: the solver's answer for a
/// function displaces the check phase's, so the two must not paraphrase each
/// other apart.
pub(crate) fn final_reassignment(target: &str) -> Diagnostic {
    Diagnostic::error(
        "T0045",
        format!("cannot reassign `Final` name `{target}`"),
        Span::new(0, 0),
    )
}

fn numeric_or_bool_compatible(a: Ty, b: Ty) -> bool {
    let is_numeric_like = |t: &Ty| matches!(t, Ty::Int | Ty::Float | Ty::Bool);
    (is_numeric_like(&a) && is_numeric_like(&b)) || (a == Ty::Str && b == Ty::Str)
}

fn check_range_operand(
    env: &Environment,
    position: &str,
    expr: &HirExpr,
) -> Result<(), Diagnostic> {
    check_range_operand_in(env, &[], position, expr)
}

fn check_range_operand_in(
    env: &Environment,
    local_names: &[&str],
    position: &str,
    expr: &HirExpr,
) -> Result<(), Diagnostic> {
    let actual = infer_expr_in(env, local_names, expr)?;
    if is_assignable(actual.clone(), Ty::Int) {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "T0021",
            format!("range {position} expects `int`, got `{}`", actual.name()),
            Span::new(0, 0),
        )
        .with_help("pass an `int` value"))
    }
}

fn check_assignment(env: &mut Environment, target: &str, ty: Ty) -> Result<(), Diagnostic> {
    check_assignment_of(env, target, ty, None)
}

/// [`check_assignment`] for a plain `name = value` rebinding, the one
/// binding seam whose MIR lowering wraps the value in
/// `MirExpr::ObjectBox` (Part 2 of #1387, #1475): rebinding a name already
/// inferred as `object` to a native value boxes it, under
/// [`object_box::admits_value`] (which also refuses a class method's own
/// `cls`, `I0404`). Every other binder -- a `for` target, a walrus, a
/// `match` capture, a comprehension variable, an annotation on a name
/// already bound -- stores its value unboxed, so it keeps the strict
/// [`check_assignment`] and a native value there stays `T0023`.
fn check_assignment_boxing(
    env: &mut Environment,
    target: &str,
    ty: Ty,
    value: &HirExpr,
) -> Result<(), Diagnostic> {
    // #1476 (Part 3 of #1387): a local's first binding fixes its type, so a
    // bare read of an `object` name narrowed by an `isinstance` guard
    // declares the local `object`, as it did before the guard narrowed
    // anything. Otherwise `if isinstance(o, int): y = o` / `else: y = o`
    // would turn a program that compiles into a `T0023`. Every other use of
    // the narrowed name keeps its native type.
    let ty = if env.lookup_any(target).is_none() && narrow::is_bare_narrowed_object_read(env, value)
    {
        Ty::Object
    } else {
        ty
    };
    check_assignment_of(env, target, ty, Some(value))
}

fn check_assignment_of(
    env: &mut Environment,
    target: &str,
    ty: Ty,
    boxed_value: Option<&HirExpr>,
) -> Result<(), Diagnostic> {
    // #1325 / Part 1 of #1333: binding a CPython object to a name is
    // admitted in both scopes, so no refusal sits here. The rest of this
    // function applies unchanged: rebinding the name to another type is
    // `T0023`, an annotation it does not satisfy is `T0025`, and a one-armed
    // `if` leaves it possibly unbound.
    //
    // At module scope the name is an ordinary module global
    // (`pycc_codegen`'s `collect_module_bindings`). Since Part 1 of #1499
    // that global owns its reference: a borrowed value (`y = x`) is retained
    // on bind and a rebinding releases the old one (`pycc_codegen`'s
    // `object_slot.rs`). In a function body the name is a function-local
    // slot, which still never releases: a rebinding leaks the old value
    // (#1092) until Part 2 of #1499 (#1502), because a parameter or alias
    // there is stored with no incref (`docs/RUNTIME.md`).
    // Part 2a of #1142 (#1165): assigning to a name bound to a buffer
    // *parameter* is refused. See `buffer::buffer_parameter_rebinding` for
    // the two independent grounds; the one that matters most here is that
    // the flat, flow-insensitive `owned_buffers` set below is sound only
    // because no name can acquire *parameter* provenance mid-function.
    // Artifact-owned provenance can be lost mid-function, which the
    // solver's `ConstraintEnvironment::rebind_over_owned_buffer` handles
    // (#1166 round-11 review finding 2); this phase needs no mirror of it,
    // because every way a name stops denoting its owned buffer is a
    // reassignment this function has already refused by the time such a
    // stale marker could be read -- `T0023` here for an incompatible
    // rebinding, and at a join for a name the branches bind differently. Reassigning an
    // artifact-*owned* buffer stays admitted -- codegen frees the previous
    // allocation before the store (D-074) -- so the guard keys on
    // provenance, not on the type alone.
    if matches!(env.lookup_any(target), Some(Ty::MemoryView)) && !env.owned_buffers.contains(target)
    {
        return Err(buffer::buffer_parameter_rebinding(target));
    }
    // PEP 591 (#383): reject reassignment of a `Final` name. The `finals`
    // set is populated *after* the initial assignment's `check_assignment`
    // call returns (in `check_stmt`/`check_stmt_in_function`'s `AnnAssign`
    // arm), so the initial binding does not trigger this check — only a
    // subsequent `Assign` or valued `AnnAssign` to the same name does. The
    // `bindings.contains_key` guard distinguishes a reassignment (the name
    // already has a runtime value) from the value-less `Final` declaration
    // case (`x: Final[int]` then `x = 1`): a value-less declaration puts the
    // name in `declared`, not `bindings`, so the first real assignment is
    // the *initial* assignment and must be allowed.
    if env.finals.contains(target) && env.bindings.contains_key(target) {
        return Err(final_reassignment(target));
    }
    // Every value assignment re-shadows a same-named `def` (D-110),
    // including a compatible-type reassignment of a name that already has a
    // representation record -- that branch below returns without calling
    // `bind()`, so clearing only inside `bind()` left the def-rebound flag
    // permanently stuck for any name with a pre-`def` binding (PR #252's
    // round-5 review caught `helper = 1; def helper() -> int: ...;
    // helper = 2; helper()` resolving the function where CPython raises
    // `TypeError`). Cleared here unconditionally, matching the solver's own
    // unconditional clearing in its Assign arm. Function bodies operate on
    // `child_for_function` clones, so a body-local assignment clears only
    // that body's view, never the module-level fact.
    env.def_rebound.remove(target);
    // Issue #769 (Part 2 of #747): any assignment to `target` kills its
    // narrowing overlay entry from this point forward -- the overlay
    // records a fact about the value `target` held at narrowing time
    // (`x is not None`), which a reassignment (even to a value that
    // happens to be compatible with the narrowed type) invalidates.
    // Unconditional and target-only: this fires for every assignment path
    // (`Assign`, `AnnAssign`, a `for` loop's own target, ...) since they all
    // route through this single function, and never touches any other
    // name's overlay entry.
    env.narrowed.remove(target);
    // Issue #118 Part 1: use `lookup_any` (not `lookup`) so a maybe-bound name
    // being reassigned on the current path becomes definite, while the
    // first-assignment-wins representation (type) from the maybe-binding is
    // retained -- `lookup` would return `None` for a `Maybe` binding, wrongly
    // treating the reassignment as a fresh first binding.
    if let Some(previous) = env.lookup_any(target) {
        let boxed = match boxed_value {
            Some(value) => object_box::admits_value(env, value, &ty, &previous)?,
            None => false,
        };
        if !class::is_assignable_env(env, &ty, &previous) && !boxed {
            // #380 (PR-20): if the mismatch involves a protocol,
            // produce a detailed T0046 conformance error.
            let diag = if matches!(previous, Ty::Protocol(_)) || matches!(ty, Ty::Protocol(_)) {
                class::assignable_error(env, &ty, &previous)
            } else {
                Diagnostic::error(
                    "T0023",
                    format!(
                        "cannot assign `{}` to `{target}`, previously inferred as `{}`",
                        ty.name(),
                        previous.name()
                    ),
                    Span::new(0, 0),
                ).with_help(format!("change the value to `{}` (the expected/declared type), or the declaration/annotation to `{}` (the actual type)", previous.name(), ty.name()))
            };
            return Err(diag);
        }
        // Issue #118 Part 1: a compatible reassignment on the current path
        // upgrades a `Maybe` binding to `Definitely` (the name is now
        // definitely assigned on this path). The first-assignment-wins
        // representation (type) is retained -- `bind` with the *existing*
        // type, not `ty`, would be wrong here because `bind` takes the
        // passed type; instead, directly insert `Definitely(previous)` to
        // keep the sticky representation while upgrading the binding state.
        if matches!(env.binding_state(target), Some(BindingState::Maybe(_))) {
            env.bind(target.to_string(), previous);
        }
        return Ok(());
    }
    // issue #245: a value-less `AnnAssign` (`x: int`) records a declared
    // type without binding one. The first real assignment/valued
    // redeclaration reaching this point is validated against that
    // declaration, and -- on success -- the *declared* type becomes the
    // sticky representation, not `ty` itself, exactly as an
    // `AnnAssign{value: Some}` already makes its own annotation (not the
    // initializer's inferred type) the sticky representation just above in
    // `check_stmt`/`check_stmt_in_function`. Worked examples: `x: int; x =
    // 1` binds `Int` (matches); `x: int; x: bool = True` binds `Int` (the
    // *earlier* declaration wins, `bool` is merely assignable to it); `x:
    // int; x = "hello"` rejects with `T0026`, distinct from `T0023`
    // (nothing was "previously inferred" here -- it was declared, never
    // assigned).
    if let Some(declared) = env.declared_ty(target) {
        if !is_assignable(ty.clone(), declared.clone()) {
            return Err(Diagnostic::error(
                "T0026",
                format!(
                    "cannot assign `{}` to `{target}`, previously declared as `{target}: {}`",
                    ty.name(),
                    declared.name()
                ),
                Span::new(0, 0),
            ).with_help(format!("change the value to `{}` (the expected/declared type), or the declaration/annotation to `{}` (the actual type)", declared.name(), ty.name())));
        }
        env.declared.remove(target);
        env.bind(target.to_string(), declared);
        return Ok(());
    }
    env.bind(target.to_string(), ty);
    Ok(())
}

/// PEP 572 (#774): walks `expr` for every `HirExpr::NamedExpr { name, value }`
/// node (an `if`/`while` test or a bare expression statement -- the only
/// placements `pycc_hir::stmt::lower_stmt`'s own placement restriction
/// allows a `NamedExpr` to survive lowering in) and binds each `name` into
/// `env` via [`check_assignment`], exactly as `HirStmt::Assign`'s own arm
/// does for an ordinary `target = value` assignment.
///
/// Walked in the expression's own left-to-right evaluation order, and each
/// binding is applied immediately (not batched) -- so a later sibling
/// sub-expression that reads an earlier walrus-bound name (e.g. `(a := 1) +
/// (b := a + 1)`) resolves correctly. `value`'s type is re-derived here via
/// `infer_expr_in` rather than threaded through from the walk that already
/// validated it (this statement's own `infer_expr`/`infer_expr_in` call,
/// made by the caller just before this one runs) -- `value` has no side
/// effects of its own beyond further nested walrus bindings, which this
/// same recursive walk also applies, so recomputing its type is
/// behavior-preserving, just not the cheapest possible implementation.
fn collect_named_expr_bindings(
    env: &mut Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<(), Diagnostic> {
    match expr {
        HirExpr::NamedExpr { name, value } => {
            collect_named_expr_bindings(env, local_names, value)?;
            let ty = infer_expr_in(env, local_names, value)?;
            check_assignment(env, name, ty)
        }
        HirExpr::IntLiteral(_)
        | HirExpr::FloatLiteral(_)
        | HirExpr::BoolLiteral(_)
        | HirExpr::StringLiteral(_)
        | HirExpr::EmptyList(_)
        | HirExpr::EmptyDict(_)
        | HirExpr::NoneLiteral
        | HirExpr::NotImplemented
        | HirExpr::Name(_)
        | HirExpr::Super => Ok(()),
        HirExpr::ListPop { list } => match list.attr_expr() {
            Some(receiver) => collect_named_expr_bindings(env, local_names, receiver),
            None => Ok(()),
        },
        // #1254 (D-250): no walrus can sit inside a comprehension (lowering
        // refuses it), so there is nothing to bind in the enclosing scope.
        HirExpr::Comprehension(_) => Ok(()),
        HirExpr::Call { args, .. } => {
            for arg in args {
                collect_named_expr_bindings(env, local_names, arg)?;
            }
            Ok(())
        }
        HirExpr::CompareChain { first, links } => {
            for operand in pycc_hir::compare_chain_operands(first, links) {
                collect_named_expr_bindings(env, local_names, operand)?;
            }
            Ok(())
        }
        HirExpr::BinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            collect_named_expr_bindings(env, local_names, left)?;
            collect_named_expr_bindings(env, local_names, right)
        }
        HirExpr::IfExp { test, body, orelse } => {
            for part in [test, body, orelse] {
                collect_named_expr_bindings(env, local_names, part)?;
            }
            Ok(())
        }
        HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
            collect_named_expr_bindings(env, local_names, operand)
        }
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(inner) = part {
                    collect_named_expr_bindings(env, local_names, inner)?;
                }
            }
            Ok(())
        }
        HirExpr::ListLiteral(es)
        | HirExpr::ObjectList(es)
        | HirExpr::SetLiteral(es)
        | HirExpr::TupleLiteral(es) => {
            for e in es {
                collect_named_expr_bindings(env, local_names, e)?;
            }
            Ok(())
        }
        HirExpr::Subscript { base, index } => {
            collect_named_expr_bindings(env, local_names, base)?;
            collect_named_expr_bindings(env, local_names, index)
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            collect_named_expr_bindings(env, local_names, base)?;
            for bound in [start, stop, step].into_iter().flatten() {
                collect_named_expr_bindings(env, local_names, bound)?;
            }
            Ok(())
        }
        HirExpr::ListAppend { list, value } => {
            if let Some(receiver) = list.attr_expr() {
                collect_named_expr_bindings(env, local_names, receiver)?;
            }
            collect_named_expr_bindings(env, local_names, value)
        }
        HirExpr::SetAdd { value, .. } => collect_named_expr_bindings(env, local_names, value),
        HirExpr::DictLiteral(pairs) => {
            for (k, v) in pairs {
                collect_named_expr_bindings(env, local_names, k)?;
                collect_named_expr_bindings(env, local_names, v)?;
            }
            Ok(())
        }
        HirExpr::DictGetOrDefault { dict, key, default } => {
            if let Some(receiver) = dict.attr_expr() {
                collect_named_expr_bindings(env, local_names, receiver)?;
            }
            collect_named_expr_bindings(env, local_names, key)?;
            collect_named_expr_bindings(env, local_names, default)
        }
        HirExpr::AttrGet { base, .. } => collect_named_expr_bindings(env, local_names, base),
        HirExpr::MethodCall { base, args, .. } => {
            collect_named_expr_bindings(env, local_names, base)?;
            for arg in args {
                collect_named_expr_bindings(env, local_names, arg)?;
            }
            Ok(())
        }
        HirExpr::ReceiverDispatchedCall { call, .. } => {
            collect_named_expr_bindings(env, local_names, call)
        }
        HirExpr::KeywordCall { call, keywords, .. } => {
            collect_named_expr_bindings(env, local_names, call)?;
            for (_, value) in keywords {
                collect_named_expr_bindings(env, local_names, value)?;
            }
            Ok(())
        }
        HirExpr::GenericClassInstantiate { args, .. } => {
            for arg in args {
                collect_named_expr_bindings(env, local_names, arg)?;
            }
            Ok(())
        }
        HirExpr::ExprCall { callee, args } => {
            collect_named_expr_bindings(env, local_names, callee)?;
            for arg in args {
                collect_named_expr_bindings(env, local_names, arg)?;
            }
            Ok(())
        }
        HirExpr::ReceiverClassCall { args } => {
            for arg in args {
                collect_named_expr_bindings(env, local_names, arg)?;
            }
            Ok(())
        }
    }
}

/// Issue #118 Part 1: joins two branch environments into `env` after an `if`
/// statement. The join lattice is: `Definitely` > `Maybe` > unbound.
///
/// - A name bound `Definitely` in **both** branches (with compatible types)
///   joins to `Definitely`.
/// - A name bound in only one branch (or `Maybe` in either) joins to `Maybe`.
/// - A name bound `Definitely` in one branch and `Maybe` in the other joins to
///   `Maybe`.
/// - A name unbound in one branch and bound in the other joins to `Maybe`.
/// - A name unbound in both branches stays unbound.
///
/// Types from both branches must be compatible (via `is_assignable`); a
/// mismatch produces `T0023`. The first-established representation (type)
/// wins, matching `check_assignment`'s first-assignment-wins rule.
fn join_if_branches(
    env: &mut Environment,
    body_env: &Environment,
    orelse_env: &Environment,
) -> Result<(), Diagnostic> {
    let mut joined: HashMap<String, BindingState> = HashMap::new();
    // Pass 1: process every name bound in the body branch. For each, look
    // up the orelse branch's state (if any) and join. This pass covers all
    // names in `body_env`; the `(None, None)` case never arises because we
    // iterate `body_env.bindings` directly (each entry is `Some` on the body
    // side by construction).
    for (name, body_state) in &body_env.bindings {
        let orelse_state = orelse_env.bindings.get(name);
        match (body_state, orelse_state) {
            // Both branches bind the name.
            (BindingState::Definitely(bt), Some(BindingState::Definitely(ot))) => {
                if bt != ot && !is_assignable(bt.clone(), ot.clone()) {
                    return Err(Diagnostic::error(
                        "T0023",
                        format!(
                            "cannot assign `{}` to `{name}`, previously inferred as `{}`",
                            ot.name(),
                            bt.name()
                        ),
                        Span::new(0, 0),
                    ).with_help(format!("change the value to `{}` (the expected/declared type), or the declaration/annotation to `{}` (the actual type)", bt.name(), ot.name())));
                }
                // First-assignment-wins: keep the body's type (matching
                // check_assignment's representation stickiness).
                joined.insert(name.clone(), BindingState::Definitely(bt.clone()));
            }
            // One or both branches have Maybe, or only one branch binds it.
            (BindingState::Definitely(ty), Some(BindingState::Maybe(_)))
            | (BindingState::Maybe(_), Some(BindingState::Definitely(ty)))
            | (BindingState::Maybe(ty), Some(BindingState::Maybe(_))) => {
                // Join of Definitely and Maybe = Maybe; Maybe and Maybe = Maybe.
                // Keep the type from whichever is available (first wins).
                joined.insert(name.clone(), BindingState::Maybe(ty.clone()));
            }
            // Body binds it, orelse does not -> Maybe.
            (BindingState::Definitely(ty), None) | (BindingState::Maybe(ty), None) => {
                joined.insert(name.clone(), BindingState::Maybe(ty.clone()));
            }
        }
    }
    // Pass 2: process names bound only in the orelse branch (not in body).
    // Each such name is Maybe (the body branch might have been the taken one).
    for (name, orelse_state) in &orelse_env.bindings {
        if body_env.bindings.contains_key(name) {
            continue; // already handled in pass 1
        }
        match orelse_state {
            BindingState::Definitely(ty) | BindingState::Maybe(ty) => {
                joined.insert(name.clone(), BindingState::Maybe(ty.clone()));
            }
        }
    }
    env.bindings = joined;
    // Part 2a of #1142 (#1165): provenance joins as a union. A name owned on
    // either path is owned for every reader after the join -- the alternative
    // (intersection) would report the *parameter* refusal for a name no
    // parameter ever bound. Codegen's epilogue is safe on the untaken path
    // because `storage_slot_at_entry` null-initializes the slot and
    // `pycc_rt_buffer_f64_free` is a documented no-op on null.
    env.owned_buffers
        .extend(body_env.owned_buffers.iter().cloned());
    env.owned_buffers
        .extend(orelse_env.owned_buffers.iter().cloned());
    // Blocker fix (D-068 review of #780): reconcile the `narrowed` overlay
    // the same way `bindings` is reconciled just above, instead of leaving
    // it as whatever it was before this `if` ran. See
    // `narrow::join_narrowed`'s doc comment for the full soundness
    // rationale -- a name stays narrowed only if both branches still narrow
    // it to the exact same type; a kill (or a narrowing established in only
    // one branch) drops out.
    env.narrowed = narrow::join_narrowed(&body_env.narrowed, &[&orelse_env.narrowed]);
    Ok(())
}

/// Issue #118 Part 1: joins a loop body environment back into `env` after a
/// `while` or `for` loop. The loop body may execute zero times, so every
/// body-only binding joins back as `Maybe`. A name that was `Definitely`
/// bound before the loop stays `Definitely` (it was bound regardless of
/// whether the loop ran), unless the body leaves it `Maybe` -- a `del`
/// (#1244). A name that was `Maybe` before the loop and is also bound in the
/// body stays `Maybe`.
pub(crate) fn join_loop_body(env: &mut Environment, body_env: &Environment) {
    // For each name bound in the body but not already Definitely bound in env,
    // downgrade to Maybe. Names already Definitely bound in env are unchanged.
    for (name, state) in &body_env.bindings {
        match env.bindings.get(name) {
            Some(BindingState::Definitely(_)) => {
                // Already definite before the loop -- stays definite. But if
                // the body assigned a different (incompatible) type,
                // check_assignment already caught that inside the body check.
                // Keep the existing definite binding -- unless the body left
                // it `Maybe`, which only a `del` can do (#1244): the loop
                // (or the `try` body this also joins) may have deleted it.
                if let BindingState::Maybe(ty) = state {
                    env.bindings
                        .insert(name.clone(), BindingState::Maybe(ty.clone()));
                }
            }
            _ => {
                // Not bound in env, or maybe-bound: the body may or may not
                // have run, so this is Maybe.
                let ty = match state {
                    BindingState::Definitely(ty) | BindingState::Maybe(ty) => ty.clone(),
                };
                env.bindings.insert(name.clone(), BindingState::Maybe(ty));
            }
        }
    }
    // Part 2a of #1142 (#1165): see `join_if_branches` -- a buffer allocated
    // in a loop body is owned after the loop, whether or not the body ran.
    env.owned_buffers
        .extend(body_env.owned_buffers.iter().cloned());
    // Blocker fix (D-068 review of #780): a loop may run zero or more
    // times, so a name stays narrowed after the loop only if the body
    // still narrows it to the same type it had going in -- the loop
    // running zero times is exactly "env's own narrowed map", and the loop
    // body having run is "body_env's narrowed map after the body executed
    // once", so intersecting the two covers both cases. A kill inside the
    // body (e.g. `while flag: x = None`) drops `x` out, matching
    // `join_if_branches`'s identical fix. See `narrow::join_narrowed`.
    env.narrowed = narrow::join_narrowed(&env.narrowed, &[&body_env.narrowed]);
}

/// PEP 634-636 (#381, PR-21): joins N case environments from a `match`
/// statement back into `env`. If `exhaustive`, a binding present in all
/// case envs is `Definitely` (one case always runs); a binding present in
/// only some is `Maybe`. If not exhaustive, there is an implicit "no match"
/// path, so every case-only binding is `Maybe`. Pre-existing bindings are
/// preserved (first-assignment-wins, matching `join_if_branches`).
fn join_match_branches(env: &mut Environment, case_envs: &[Environment], exhaustive: bool) {
    let mut joined: HashMap<String, BindingState> = HashMap::new();
    let all_names: HashSet<&String> = case_envs.iter().flat_map(|ce| ce.bindings.keys()).collect();
    for name in all_names {
        let states: Vec<&BindingState> = case_envs
            .iter()
            .filter_map(|ce| ce.bindings.get(name))
            .collect();
        let ty = states[0].ty().clone();
        let all_definite = !exhaustive
            || states.len() == case_envs.len()
                && states
                    .iter()
                    .all(|s| matches!(*s, BindingState::Definitely(_)));
        if all_definite {
            joined.insert(name.clone(), BindingState::Definitely(ty));
        } else {
            joined.insert(name.clone(), BindingState::Maybe(ty));
        }
    }
    env.bindings = joined;
    // Part 2a of #1142 (#1165): see `join_if_branches` -- provenance is a
    // union over every case environment.
    for case_env in case_envs {
        env.owned_buffers
            .extend(case_env.owned_buffers.iter().cloned());
    }
    // Blocker fix (D-068 review of #780): reconcile `narrowed` the same
    // conservative way as `join_if_branches`/`join_loop_body`. `env` itself
    // (pre-match) stands in for the implicit "no case matched" path -- safe
    // to include unconditionally (exhaustive or not): it never *adds* a
    // name to the intersection, since it can only narrow the result set
    // further, so an exhaustive match that never actually needed the
    // implicit path is unaffected wherever every case agrees anyway.
    let case_narrowed_maps: Vec<&HashMap<String, Ty>> =
        case_envs.iter().map(|ce| &ce.narrowed).collect();
    env.narrowed = narrow::join_narrowed(&env.narrowed, &case_narrowed_maps);
}

/// PEP 634-636 (#381, PR-21): checks a `match` statement. The subject is
/// inferred once; each case is checked in an independent env clone (like
/// `if` arms). Pattern captures are bound before the guard and body are
/// checked. Exhaustiveness is verified (`T0030` if not exhaustive, but the
/// match is still accepted — Python allows non-exhaustive match).
fn check_match(
    env: &mut Environment,
    local_names: &[&str],
    subject: &HirExpr,
    cases: &[HirMatchCase],
    return_ty: Option<&Ty>,
) -> Result<(), Diagnostic> {
    let subject_ty = infer_expr_in(env, local_names, subject)?;
    // Part 2 of #1026 (#1081): `match numpy.pi:` stays refused. A
    // `HirPattern::Wildcard` returns no bindings and satisfies
    // `check_exhaustive`, so before this guard the statement type-checked
    // and reached `pycc_mir`'s `lower_match`, which has no `Ty::Object`
    // handling at all. This guard refuses the whole subject before any
    // pattern binds, so it covers `HirPattern::Capture` too: since Part 1
    // of #1333 (#1362) `check_assignment` admits an object binding in a
    // function body, so the `bindings` loop below no longer refuses one.
    foreign::reject_object_operand(&subject_ty, "matching on a CPython object")?;
    let mut case_envs = Vec::with_capacity(cases.len());
    for case in cases {
        let mut case_env = env.clone();
        let bindings = check_pattern(&case_env, local_names, &case.pattern, &subject_ty)?;
        for (name, ty) in &bindings {
            check_assignment(&mut case_env, name, ty.clone())?;
        }
        if let Some(guard) = &case.guard {
            let guard_ty = infer_expr_in(&case_env, local_names, guard)?;
            if guard_ty != Ty::Bool {
                return Err(Diagnostic::error(
                    "T0021",
                    format!("match guard must be `bool`, got `{}`", guard_ty.name()),
                    Span::new(0, 0),
                ));
            }
        }
        // D-068 re-review of #780 (third round, warning finding): route
        // each case body through `narrow::check_stmt_sequence[_in_function]`
        // instead of a raw per-statement loop, so a nested early-return
        // guard inside a `match` case narrows the rest of that same case
        // body -- the identical fast-path-bypass defect the `if`/`while`
        // fast-path helpers already had fixed for finding 2, but which
        // `check_match`'s own always-raw loop had never been routed
        // through in the first place.
        match return_ty {
            Some(rt) => narrow::check_stmt_sequence_in_function(
                &mut case_env,
                local_names,
                &case.body,
                rt.clone(),
            )?,
            None => narrow::check_stmt_sequence(&mut case_env, &case.body)?,
        }
        case_envs.push(case_env);
    }
    let exhaustive = check_exhaustive(env, &subject_ty, cases);
    join_match_branches(env, &case_envs, exhaustive);
    if !exhaustive {
        return Err(Diagnostic::error(
            "T0030",
            format!(
                "non-exhaustive `match`: not every value of `{}` is covered (add a `case _:` or \
                 cover all cases)",
                subject_ty.name()
            ),
            Span::new(0, 0),
        ));
    }
    Ok(())
}

/// PEP 634-636 (#381, PR-21): checks a pattern against a subject type,
/// returning the list of `(capture_name, type)` bindings it introduces.
fn check_pattern(
    env: &Environment,
    local_names: &[&str],
    pattern: &HirPattern,
    subject_ty: &Ty,
) -> Result<Vec<(String, Ty)>, Diagnostic> {
    match pattern {
        HirPattern::Wildcard => Ok(vec![]),
        HirPattern::Capture(name) => Ok(vec![(name.clone(), subject_ty.clone())]),
        HirPattern::Literal(expr) => {
            let lit_ty = infer_expr_in(env, local_names, expr)?;
            if !is_assignable(lit_ty.clone(), subject_ty.clone())
                && !is_assignable(subject_ty.clone(), lit_ty.clone())
            {
                return Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "literal pattern `{}` does not match subject type `{}`",
                        lit_ty.name(),
                        subject_ty.name()
                    ),
                    Span::new(0, 0),
                ));
            }
            Ok(vec![])
        }
        HirPattern::Singleton(b) => {
            if *subject_ty != Ty::Bool {
                return Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "singleton pattern `True`/`False` requires a `bool` subject, got `{}`",
                        subject_ty.name()
                    ),
                    Span::new(0, 0),
                ));
            }
            let _ = b;
            Ok(vec![])
        }
        HirPattern::NoneSingleton => {
            if *subject_ty != Ty::None {
                return Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "`None` pattern requires a `None` subject, got `{}`",
                        subject_ty.name()
                    ),
                    Span::new(0, 0),
                ));
            }
            Ok(vec![])
        }
        HirPattern::Sequence(subs) => {
            let elt_ty = check_sequence_subject(subject_ty)?;
            let mut bindings = Vec::new();
            for sub in subs {
                bindings.extend(check_pattern(env, local_names, sub, &elt_ty)?);
            }
            Ok(bindings)
        }
        HirPattern::SequenceStar(subs, rest) => {
            let elt_ty = check_sequence_subject(subject_ty)?;
            let mut bindings = Vec::new();
            for sub in subs {
                bindings.extend(check_pattern(env, local_names, sub, &elt_ty)?);
            }
            if let Some(rest) = rest {
                bindings.push((rest.clone(), Ty::List(Box::new(elt_ty))));
            }
            Ok(bindings)
        }
        HirPattern::Mapping(pairs, rest) => {
            let Ty::Dict(kv) = subject_ty else {
                return Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "mapping pattern requires a `dict` subject, got `{}`",
                        subject_ty.name()
                    ),
                    Span::new(0, 0),
                ));
            };
            let (key_ty, val_ty) = kv.as_ref();
            let mut bindings = Vec::new();
            for (key_expr, val_pat) in pairs {
                let inferred_key = infer_expr_in(env, local_names, key_expr)?;
                if inferred_key != *key_ty {
                    return Err(Diagnostic::error(
                        "T0021",
                        format!(
                            "mapping pattern key `{}` does not match dict key type `{}`",
                            inferred_key.name(),
                            key_ty.name()
                        ),
                        Span::new(0, 0),
                    ));
                }
                bindings.extend(check_pattern(env, local_names, val_pat, val_ty)?);
            }
            if let Some(rest) = rest {
                bindings.push((rest.clone(), subject_ty.clone()));
            }
            Ok(bindings)
        }
        HirPattern::Class {
            class_name,
            positional,
            keyword,
        } => check_class_pattern(
            env,
            local_names,
            class_name,
            positional,
            keyword,
            subject_ty,
        ),
        HirPattern::Or(subs) => {
            let mut all_bindings: Option<Vec<(String, Ty)>> = None;
            for sub in subs {
                let sub_bindings = check_pattern(env, local_names, sub, subject_ty)?;
                match &all_bindings {
                    None => all_bindings = Some(sub_bindings),
                    Some(existing) => {
                        let existing_names: HashSet<&String> =
                            existing.iter().map(|(n, _)| n).collect();
                        let new_names: HashSet<&String> =
                            sub_bindings.iter().map(|(n, _)| n).collect();
                        if existing_names != new_names {
                            return Err(Diagnostic::error(
                                "T0021",
                                "or-pattern alternatives must bind the same set of names"
                                    .to_string(),
                                Span::new(0, 0),
                            ));
                        }
                    }
                }
            }
            Ok(all_bindings.unwrap_or_default())
        }
        HirPattern::As(inner, name) => {
            let mut bindings = check_pattern(env, local_names, inner, subject_ty)?;
            bindings.push((name.clone(), subject_ty.clone()));
            Ok(bindings)
        }
    }
}

/// Helper for sequence pattern checking: extracts the element type from a
/// `list` or `tuple` subject.
fn check_sequence_subject(subject_ty: &Ty) -> Result<Ty, Diagnostic> {
    match subject_ty {
        Ty::List(elt) => Ok((**elt).clone()),
        Ty::Tuple(elems) if !elems.is_empty() => Ok(elems[0].clone()),
        _ => Err(Diagnostic::error(
            "T0021",
            format!(
                "sequence pattern requires a `list` or `tuple` subject, got `{}`",
                subject_ty.name()
            ),
            Span::new(0, 0),
        )),
    }
}

/// Helper for class pattern checking.
fn check_class_pattern(
    env: &Environment,
    local_names: &[&str],
    class_name: &str,
    positional: &[HirPattern],
    keyword: &[(String, HirPattern)],
    subject_ty: &Ty,
) -> Result<Vec<(String, Ty)>, Diagnostic> {
    let class_def = env.lookup_class(class_name).ok_or_else(|| {
        Diagnostic::error(
            "T0021",
            format!("class `{class_name}` is not defined"),
            Span::new(0, 0),
        )
    })?;
    let subject_is_match = match subject_ty {
        Ty::Instance(name) => name.as_str() == class_name || class_def.mro.contains(name),
        _ => false,
    };
    if !subject_is_match {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "class pattern `{class_name}` does not match subject type `{}`",
                subject_ty.name()
            ),
            Span::new(0, 0),
        ));
    }
    let mut bindings = Vec::new();
    let init_params: Vec<(String, Ty)> = class_def
        .methods
        .iter()
        .find(|(name, _)| name == "__init__")
        .and_then(|(_, mangled)| env.lookup_function(mangled))
        .map(|(params, _)| {
            params
                .iter()
                .skip(1)
                .enumerate()
                .map(|(i, ty)| (format!("__pos_{i}"), ty.clone()))
                .collect()
        })
        .unwrap_or_default();
    for (i, pat) in positional.iter().enumerate() {
        let param_ty = init_params
            .get(i)
            .map(|(_, ty)| ty.clone())
            .unwrap_or(Ty::Infer);
        bindings.extend(check_pattern(env, local_names, pat, &param_ty)?);
    }
    for (attr, pat) in keyword {
        // #911 (Part 1 of #885): a class-level attribute is a compile-time
        // constant with no instance storage, so `case W(MIN_WIDTH=x)` has no
        // per-instance value to match against -- every instance of `W` would
        // bind the same constant, making the sub-pattern either always or
        // never matching. Reject it rather than silently binding `Ty::Infer`
        // through the `unwrap_or` below.
        if class_def
            .class_attrs
            .iter()
            .any(|(name, _, _)| name == attr)
        {
            return Err(Diagnostic::error(
                "T0044",
                format!(
                    "`{attr}` is a class-level attribute of class `{}`, not an instance \
                     attribute -- it is a compile-time constant and cannot be matched as a \
                     class-pattern keyword sub-pattern",
                    class_def.name
                ),
                Span::new(0, 0),
            ));
        }
        let attr_ty = class_def
            .attrs
            .iter()
            .find(|(name, _)| name == attr)
            .map(|(_, ty)| ty.clone())
            .unwrap_or(Ty::Infer);
        bindings.extend(check_pattern(env, local_names, pat, &attr_ty)?);
    }
    Ok(bindings)
}

/// PEP 634-636 (#381, PR-21): returns `true` if the patterns across all
/// cases cover every value of `subject_ty`. See D-169 for the algorithm.
fn check_exhaustive(env: &Environment, subject_ty: &Ty, cases: &[HirMatchCase]) -> bool {
    for case in cases {
        if case.guard.is_some() {
            continue;
        }
        if is_irrefutable_pattern(&case.pattern) {
            return true;
        }
    }
    match subject_ty {
        Ty::Bool => {
            let mut has_true = false;
            let mut has_false = false;
            for case in cases {
                if case.guard.is_some() {
                    continue;
                }
                collect_bool_patterns(&case.pattern, &mut has_true, &mut has_false);
            }
            has_true && has_false
        }
        Ty::Instance(name) => {
            if let Some(class_def) = env.lookup_class(name)
                && !class_def.enum_members.is_empty()
            {
                let mut covered: HashSet<&str> = HashSet::new();
                for case in cases {
                    if case.guard.is_some() {
                        continue;
                    }
                    collect_enum_member_patterns(
                        &case.pattern,
                        name,
                        &class_def.enum_members,
                        &mut covered,
                    );
                }
                class_def
                    .enum_members
                    .iter()
                    .all(|(member, _)| covered.contains(member.as_str()))
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Returns `true` if `pattern` is irrefutable (matches every value of any
/// type): wildcard, bare capture, or an or-pattern containing an irrefutable
/// sub-pattern.
fn is_irrefutable_pattern(pattern: &HirPattern) -> bool {
    match pattern {
        HirPattern::Wildcard | HirPattern::Capture(_) => true,
        HirPattern::Or(subs) => subs.iter().any(is_irrefutable_pattern),
        HirPattern::As(inner, _) => is_irrefutable_pattern(inner),
        _ => false,
    }
}

/// Collects `True`/`False` singleton coverage from a pattern (recursing into
/// or-patterns).
fn collect_bool_patterns(pattern: &HirPattern, has_true: &mut bool, has_false: &mut bool) {
    match pattern {
        HirPattern::Singleton(true) => *has_true = true,
        HirPattern::Singleton(false) => *has_false = true,
        HirPattern::Or(subs) => {
            for sub in subs {
                collect_bool_patterns(sub, has_true, has_false);
            }
        }
        _ => {}
    }
}

/// Collects enum member coverage from class patterns like `Color.RED`.
fn collect_enum_member_patterns<'a>(
    pattern: &HirPattern,
    class_name: &str,
    members: &'a [(String, pycc_hir::EnumMemberValue)],
    covered: &mut HashSet<&'a str>,
) {
    match pattern {
        HirPattern::Class { class_name: cn, .. } => {
            if cn == class_name {
                for (member, _) in members {
                    covered.insert(member.as_str());
                }
            }
        }
        HirPattern::Or(subs) => {
            for sub in subs {
                collect_enum_member_patterns(sub, class_name, members, covered);
            }
        }
        _ => {}
    }
}

/// Issue #118 Part 1: fast-path helper for module-scope `if` statements
/// where neither branch introduces new bindings. Checks both branches
/// in-place without cloning env, matching the pre-#118 behavior.
fn check_if_branches_in_place(
    env: &mut Environment,
    body: &[HirStmt],
    orelse: &[HirStmt],
) -> Result<(), Diagnostic> {
    // Warning fix (D-068 review of #780): route through the
    // narrowing-aware sequence checker, not a raw per-statement loop.
    // `introduces_bindings` gates this fast path on "no new bindings", but
    // says nothing about whether a *nested* statement recognizes an
    // early-return narrowing guard (`narrow::apply_post_if_narrowing`) that
    // needs to propagate to later statements in this same body/orelse --
    // skipping that propagation here silently rejected an otherwise valid
    // nested guard shape. See `crates/pycc_types/src/narrow.rs`.
    narrow::check_stmt_sequence(env, body)?;
    narrow::check_stmt_sequence(env, orelse)
}

/// Issue #118 Part 1: fast-path helper for module-scope `while` loops
/// where the body introduces no new bindings. Checks the body in-place
/// without cloning env.
fn check_while_body_in_place(env: &mut Environment, body: &[HirStmt]) -> Result<(), Diagnostic> {
    // Warning fix (D-068 review of #780): see `check_if_branches_in_place`'s
    // identical comment.
    narrow::check_stmt_sequence(env, body)
}

pub fn check_stmt(env: &mut Environment, stmt: &HirStmt) -> Result<(), Diagnostic> {
    match stmt {
        HirStmt::Assign { target, value } => {
            // Part 2a of #1142 (#1165): this is `check_stmt`, the
            // *module-scope* statement walk, so it deliberately carries no
            // admitting seam for the buffer producer -- the function-scope
            // walk (`check_stmt_in_function`) is the only one that admits it.
            // A module-level `a = ndarray(n)` reaches `infer_expr` below and
            // is refused by `expr.rs`'s `Call` arm with
            // `buffer::producer_at_module_scope`, on its own ground: a
            // module-level frame gets no owned-slot epilogue, so the
            // free-at-exit lifetime has no exit to run at.
            let ty = infer_expr(env, value)?;
            check_assignment_boxing(env, target, ty, value)
        }
        HirStmt::AnnAssign {
            target,
            annotation,
            value,
            is_final,
        } => {
            // Part 1 of #1027: a `memoryview` annotation is admitted only in
            // a signature, never on a declaration -- see
            // `expr::reject_memoryview_declaration`. Checked ahead of the
            // value/no-value split so both shapes route through the one
            // contract.
            //
            // Part 2a of #1142 (#1165) admits one shape this refusal used to
            // cover: `a: NDArray = ndarray(n)`. The call is *gated* rather
            // than the contract widened, so the value-less shape the refusal
            // exists for is unchanged. Its message is not byte-identical: the
            // tail claiming a buffer is admitted only as a parameter was
            // corrected once Part 2a gave the type a second provenance.
            let produced = value
                .as_ref()
                .and_then(|value| buffer::producer_assignment_ty(env, &[], value));
            if produced.is_none() {
                reject_memoryview_declaration(target, annotation)?;
            }
            if let Some(value) = value {
                let inferred = match produced {
                    Some(produced) => produced?,
                    None => infer_expr(env, value)?,
                };
                // Part 4 of #1026 (PR 4c of #1083): a foreign CPython
                // object under a fixed-arity all-`float` tuple annotation
                // is admitted here, ahead of the assignability test, rather
                // than by widening `is_assignable`. The distinction is the
                // whole design: `is_assignable` is consulted wherever a
                // value flows into a declared type -- a parameter, a
                // `return`, a class attribute -- and widening it would
                // admit the pair at every one of them, while what Part 4
                // supports is exactly this statement, in exactly a module
                // body (an in-function occurrence is already `I0404` at the
                // read of the foreign name, so that arm needs no branch).
                // `foreign::is_object_float_tuple_annotation` owns the rule
                // and records what stays refused.
                //
                // Nothing below needs a second branch: `bind_ty` already
                // resolves to the annotation for a `Ty::Tuple`, so the name
                // binds to the tuple type the unpack really produces.
                let unpacks_into_float_tuple = matches!(inferred, Ty::Object)
                    && foreign::is_object_float_tuple_annotation(annotation);
                if !unpacks_into_float_tuple
                    && !class::is_assignable_env(env, &inferred, annotation)
                    && !object_box::admits_value(env, value, &inferred, annotation)?
                {
                    // #380 (PR-20): if the mismatch involves a protocol,
                    // produce a detailed T0046 conformance error.
                    let diag = if matches!(annotation, Ty::Protocol(_))
                        || matches!(inferred, Ty::Protocol(_))
                    {
                        class::assignable_error(env, &inferred, annotation)
                    } else {
                        annotation_initializer_mismatch(target, &inferred, annotation)
                    };
                    return Err(diag);
                }
                // Route through `check_assignment` (not a raw `env.bind`) so a
                // name's first-established representation stays sticky across
                // an annotated re-declaration, exactly as it already does for
                // plain `Assign` -- `pycc_mir`'s own `bind_variable` (D-040's
                // "first assignment fixes a binding's representation"
                // invariant) keeps the *first* recorded MIR type regardless of
                // a later compatible reassignment, so the checker's `env` must
                // agree or a later annotated reassignment (e.g. `x = 1` then
                // `x: str = "s"`, where `is_assignable(Str, Str)` alone would
                // wrongly accept it) could diverge from what codegen actually
                // stores.
                // #380 (PR-20): when the annotation is a protocol type, bind
                // with the concrete (inferred) type instead — the protocol
                // type is a compile-time-only interface, and the MIR needs
                // the concrete type for method/attribute resolution (static
                // dispatch). The conformance check above already validated
                // that the inferred type conforms to the protocol.
                let bind_ty = if matches!(annotation, Ty::Protocol(_)) {
                    inferred.clone()
                } else {
                    annotation.clone()
                };
                check_assignment(env, target, bind_ty)?;
                // Part 2a of #1142 (#1165): no owned-buffer provenance is
                // recorded here, for the same reason the plain `Assign` arm
                // above admits no producer -- at module scope `inferred` is
                // never `Ty::MemoryView`, because the only expression that
                // could produce one is refused by `buffer::
                // producer_at_module_scope` before this point.
            } else {
                // No initializer: register no *binding* (a premature read
                // still raises the existing T0021 -- collect_local_names
                // (Step 1) already marked `target` local, and `declare`
                // never touches `bindings`), but do retain the declared
                // type (issue #245) so a later plain or annotated
                // assignment is checked against it instead of silently
                // treating the first later assignment as the initial,
                // unconstrained binding.
                env.declare(target.clone(), annotation.clone())?;
            }
            // PEP 591 (#383): record the name as `Final` *after*
            // `check_assignment` returns, so the initial assignment's own
            // `check_assignment` call does not yet see the name in `finals`
            // and is not rejected. A subsequent plain `Assign` or valued
            // `AnnAssign` to the same name will see it in both `finals` and
            // `bindings`, and is rejected with `T0045`. A value-less `Final`
            // declaration (`x: Final[int]` with no `= ...`) also inserts into
            // `finals` — the first real assignment to it is allowed (the name
            // is in `declared`, not `bindings`, so `check_assignment`'s
            // `finals` check does not fire), but a second assignment is
            // rejected.
            if *is_final {
                env.finals.insert(target.clone());
            }
            Ok(())
        }
        HirStmt::ExprStmt(expr) => {
            // PEP 572 (#774): bind any walrus target the expression
            // introduces (`x := value`) *before* the full `infer_expr`
            // validation pass below -- a walrus value can itself reference
            // an earlier walrus bound within the very same expression
            // (`(a := 1) + (b := a + 1)`), and `infer_expr`'s own read-only
            // walk never binds anything, so `b`'s value would otherwise see
            // `a` as never bound. `collect_named_expr_bindings` performs its
            // own `infer_expr_in` call on each walrus's `value` as it binds
            // it (in the expression's true left-to-right evaluation order),
            // so by the time the full-expression `infer_expr` call below
            // runs, every walrus name it may reference is already bound;
            // `infer_expr` still does the real work of enforcing T0050 on
            // each `NamedExpr` node and type-checking the expression as a
            // whole (a nested walrus's own `check_assignment` inside this
            // first pass can also fail, e.g. T0045/T0023, which aborts
            // before `infer_expr` ever runs -- fine, since either error
            // aborts the same statement).
            collect_named_expr_bindings(env, &[], expr)?;
            infer_expr(env, expr).map(|_| ())
        }
        HirStmt::If { test, body, orelse } => {
            // PEP 572 (#774): bind before validating, mirroring the
            // `ExprStmt` arm's own ordering rationale above -- the test
            // always executes, so this binding is unconditional relative to
            // the branch join below.
            collect_named_expr_bindings(env, &[], test)?;
            infer_expr(env, test)?;
            // Issue #118 Part 1: check each branch in an independent clone of
            // env, then join the results. A no-else `if` makes all body-only
            // bindings `Maybe` (the orelse clone is empty, so every body
            // binding is "one branch only" -> Maybe).
            // Issue #769 (Part 2 of #747): a narrowing-eligible test needs
            // per-branch overlay state (`narrow::apply_branch_narrowing`),
            // which only exists on a branch-local `env` clone -- force the
            // slow/cloning path even when neither branch introduces a
            // binding, so the fast path below never silently skips
            // narrowing.
            let narrowing = narrow::narrowing_target(env, test);
            // Fast path: if neither branch introduces any new bindings and
            // no narrowing applies, skip the clone+join and check both
            // branches in-place (matching the pre-#118 behavior for
            // guard-only ifs).
            if narrowing.is_none() && !introduces_bindings(body) && !introduces_bindings(orelse) {
                check_if_branches_in_place(env, body, orelse)
            } else {
                let mut body_env = env.clone();
                let mut orelse_env = env.clone();
                if let Some(target) = &narrowing {
                    narrow::apply_branch_narrowing(&mut body_env, &mut orelse_env, target);
                }
                narrow::check_stmt_sequence(&mut body_env, body)?;
                narrow::check_stmt_sequence(&mut orelse_env, orelse)?;
                if let Some(target) = &narrowing {
                    narrow::end_branch_narrowing(
                        env,
                        &mut body_env,
                        &mut orelse_env,
                        target,
                        body,
                        orelse,
                    );
                }
                join_if_branches(env, &body_env, &orelse_env)
            }
        }
        HirStmt::While { test, body } => {
            // Issue #769 follow-up (D-068 re-review round 3): a `while`
            // body can be re-entered, and `test` itself re-executes on
            // every iteration too -- prescan and drop any name `body`
            // kills *before* checking `test`, so both the test and the
            // body (fast in-place path or slow clone+join path, which
            // clones `env` after this line and so inherits the pruning)
            // see it. See `narrow::apply_kill_prescan`'s doc comment.
            narrow::apply_kill_prescan(env, body);
            narrow::apply_delete_prescan(env, body, None);
            // PEP 572 (#774): bind before validating -- see the `ExprStmt`
            // arm's doc comment above for why this order is required.
            collect_named_expr_bindings(env, &[], test)?;
            infer_expr(env, test)?;
            // Issue #118 Part 1: the loop body may execute zero times, so
            // every body-only binding joins back as `Maybe`.
            // Fast path: if the body introduces no bindings, check in-place.
            if !introduces_bindings(body) {
                check_while_body_in_place(env, body)
            } else {
                let mut body_env = env.clone();
                narrow::check_stmt_sequence(&mut body_env, body)?;
                join_loop_body(env, &body_env);
                Ok(())
            }
        }
        HirStmt::ForRange {
            var,
            start,
            stop,
            step,
            body,
        } => {
            check_range_operand(env, "start", start)?;
            check_range_operand(env, "stop", stop)?;
            check_range_operand(env, "step", step)?;
            // Issue #118 Part 1: track whether the loop variable was already
            // definitely bound before the loop. If so, it stays definite
            // after the loop (the variable was bound regardless of whether
            // the loop ran). If not, it is `Maybe` after the loop (the loop
            // may execute zero times).
            let was_definite = matches!(env.binding_state(var), Some(BindingState::Definitely(_)));
            check_assignment(env, var, Ty::Int)?;
            let mut body_env = env.clone();
            // Issue #769 follow-up (D-068 re-review round 3): the loop
            // body can re-run, so prescan-drop any name it kills before
            // checking it. See `narrow::apply_kill_prescan`.
            narrow::apply_kill_prescan(&mut body_env, body);
            narrow::apply_delete_prescan(&mut body_env, body, Some(var));
            narrow::check_stmt_sequence(&mut body_env, body)?;
            join_loop_body(env, &body_env);
            // Issue #118 Part 1: if the loop variable was not definitely bound
            // before the loop, downgrade it to Maybe (the loop may execute
            // zero times). A pre-bound variable stays Definitely bound.
            if !was_definite && let Some(ty) = env.lookup_any(var) {
                env.bind_maybe(var.to_string(), ty);
            }
            Ok(())
        }
        HirStmt::ForList { var, list, body } => {
            // #379 (PR-19): `for c in Color:` — iterating an enum class's
            // members. A class name is not a value binding (classes live in
            // `env.classes`, not `env.bindings`), so `lookup_bound_name`
            // fails. Check `env.lookup_class` first: if `list` is an enum
            // class (has non-empty `enum_members`), bind `var` to
            // `Ty::Instance(list)` and check the body. The actual unrolling
            // (expanding the loop into N sequential copies) is done by a
            // separate HIR→HIR rewrite pass that runs after
            // `check_and_resolve` (see `unroll_enum_loops`), so MIR never
            // sees an enum iterable.
            if let Some(class_def) = env.lookup_class(list)
                && !class_def.enum_members.is_empty()
            {
                return check_enum_loop_body_module(env, var, list, body);
            }
            // #1325: a bare name bound to a CPython object iterates exactly
            // like `for x in o.attr:` does -- `pycc_hir` has no type
            // information, so the bare name arrives here rather than as a
            // `HirStmt::ForObject`, and `pycc_mir` routes it to
            // `MirStmt::ForObject`. Keyed on the type alone: a bare imported
            // module iterates too, and fails at run time with CPython's own
            // `TypeError`, which import provenance could not rule out anyway
            // (`from os import path` binds a module, `from sys import path` a
            // list). A `Maybe` binding falls through to `lookup_bound_name`
            // and keeps its possibly-unbound diagnostic.
            if let Some(BindingState::Definitely(Ty::Object)) = env.binding_state(list) {
                return foreign::for_loop::check_module_object_loop(env, var, body);
            }
            // Module (top-level) scope has no "local before assignment"
            // concept the way a function body does -- every other arm here
            // (e.g. `ExprStmt` via `infer_expr`) resolves names with an
            // empty `local_names` slice too, so an unresolved `list` is
            // simply "not defined," never `unbound_local`.
            let list_ty = lookup_bound_name(env, &[], list)?;
            // PR-11 Task 3 (D-123): `for k in d:` iterates a dict's own keys
            // in insertion order, mirroring `len()`'s own relaxation to
            // accept `Ty::Dict` alongside `Ty::List` at this crate's other
            // hand-recognized dispatch points. `HirStmt::ForList` itself is
            // reused unconditionally for any bare-name iterable, dict or
            // list alike (`pycc_hir`'s own lowering has no type information
            // to pick a different node) -- this is the point where the real
            // type is resolved. PR-11 Task 7 (D-123): `for x in s:` iterates
            // a set's own elements (order is this implementation's own
            // insertion order, not a CPython-matching guarantee -- see
            // D-123's own iteration-order caveat), so `Ty::Set` is accepted
            // here too, binding the loop variable as the set's element type.
            let var_ty = match list_ty {
                Ty::List(elem_ty) => *elem_ty,
                Ty::Dict(kv) => kv.0,
                Ty::Set(elem_ty) | Ty::FrozenSet(elem_ty) => *elem_ty,
                other => {
                    return Err(Diagnostic::error(
                        "T0033",
                        format!(
                            "`{}` cannot be iterated with `for ... in ...` (only list[T]/dict[K, V]/set[T]/frozenset[T] supports this)",
                            other.name()
                        ),
                        Span::new(0, 0),
                    ));
                }
            };
            // Issue #118 Part 1: track whether the loop variable was already
            // definitely bound before the loop (see ForRange above).
            let was_definite = matches!(env.binding_state(var), Some(BindingState::Definitely(_)));
            check_assignment(env, var, var_ty)?;
            let mut body_env = env.clone();
            // Issue #769 follow-up (D-068 re-review round 3): see
            // `narrow::apply_kill_prescan`.
            narrow::apply_kill_prescan(&mut body_env, body);
            narrow::apply_delete_prescan(&mut body_env, body, Some(var));
            narrow::check_stmt_sequence(&mut body_env, body)?;
            join_loop_body(env, &body_env);
            // Issue #118 Part 1: if the loop variable was not definitely bound
            // before the loop, downgrade it to Maybe (the loop may execute
            // zero times). A pre-bound variable stays Definitely bound.
            if !was_definite && let Some(ty) = env.lookup_any(var) {
                env.bind_maybe(var.to_string(), ty);
            }
            Ok(())
        }
        // PR 3c of #1082 (Part 3 of #1026): `for x in o.attr:` and
        // `for x in o.method(...):`, the two iterable shapes `pycc_hir`
        // admits without type information. This is the point where the
        // iterable's real type decides, and the dispatch is on that
        // resolved type and nothing else.
        HirStmt::ForObject { var, iter, body } => {
            let iter_ty = infer_expr(env, iter)?;
            if !matches!(iter_ty, Ty::Object) {
                // Reachable: an attribute load whose base is not a foreign
                // object lowers to this node too, so `for x in C.value:`
                // over an `int` class attribute lands here and reports
                // `got `int``. (`for x in xs.copy():` over a `list` also
                // lowers to this node, but its receiver is rejected with
                // `T0043` while `iter` is inferred above, so it never
                // reaches this arm.) Iterating any of those is not
                // supported by another arm either, so the refusal is
                // correct -- only its wording is specific to this shape.
                return Err(Diagnostic::error(
                    "I0404",
                    format!(
                        "`for ... in <attribute or method call>` is only supported when the \
                         iterable is a CPython object (Part 3 of #1026), got `{}`",
                        iter_ty.name()
                    ),
                    Span::new(0, 0),
                ));
            }
            foreign::for_loop::check_module_object_loop(env, var, body)
        }
        // PR-12 Task 3 (D-117): `target = <comp>` at module scope, checked
        // by the shared `comprehension::check_comp_assign` helper.
        HirStmt::ListCompAssign {
            target,
            var,
            iter,
            cond,
            elt,
        } => check_comp_assign(
            env,
            &[],
            target,
            CompView {
                var,
                iter,
                cond: cond.as_deref(),
                elts: CompElts::List(elt),
            },
        ),
        HirStmt::SetCompAssign {
            target,
            var,
            iter,
            cond,
            elt,
        } => check_comp_assign(
            env,
            &[],
            target,
            CompView {
                var,
                iter,
                cond: cond.as_deref(),
                elts: CompElts::Set(elt),
            },
        ),
        HirStmt::DictCompAssign {
            target,
            var,
            iter,
            cond,
            key,
            value,
        } => check_comp_assign(
            env,
            &[],
            target,
            CompView {
                var,
                iter,
                cond: cond.as_deref(),
                elts: CompElts::Dict(key, value),
            },
        ),
        HirStmt::Return(_) => Err(Diagnostic::error(
            "T0024",
            "'return' outside a function is not allowed".to_string(),
            Span::new(0, 0),
        )),
        HirStmt::DictSet { dict, key, value } => check_dict_set(env, &[], dict, key, value),
        HirStmt::AttrSet { base, attr, value } => {
            class::check_attr_set(env, &[], base, attr, value)
        }
        HirStmt::Match { subject, cases } => check_match(env, &[], subject, cases, None),
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => check_try_stmt(env, &[], body, handlers, orelse, finalbody, None),
        HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => check_try_star_stmt(env, &[], body, handlers, orelse, finalbody, None),
        HirStmt::Raise { exc, cause } => check_raise_stmt(env, &[], exc, cause),
        HirStmt::Delete { name } => del_stmt::check_delete(env, name),
        HirStmt::DeleteSlice {
            base,
            start,
            stop,
            step,
            span,
        } => foreign::slice::check_delete_slice(
            env,
            &[],
            base,
            [start.as_deref(), stop.as_deref(), step.as_deref()],
            *span,
        ),
        HirStmt::DeleteAttr { base, span, .. } => {
            foreign::attr_store::check_delete_attr(env, &[], base, *span)
        }
        HirStmt::ForeignImport { bindings, .. } => {
            foreign::bind_block_import(env, bindings);
            Ok(())
        }
    }
}

/// `d[k] = v` (PR-11 Task 3, D-123): insert-or-update. Shared between
/// module (`check_stmt`, `local_names = &[]`) and function-body
/// (`check_stmt_in_function`) scope, mirroring how `check_range_operand`/
/// `check_range_operand_in` are already split for `ForRange`. Reuses
/// `T0033` for a non-`dict` base (matching `ListAppend`'s own "does not
/// support X" shape) and `T0021` for a key or value type mismatch
/// (matching `Subscript`'s and `ListAppend`'s own reuse of `T0021` for an
/// operand/assignment constraint mismatch).
fn check_dict_set(
    env: &Environment,
    local_names: &[&str],
    dict: &str,
    key: &HirExpr,
    value: &HirExpr,
) -> Result<(), Diagnostic> {
    // The buffer element store is admitted *before* the `Ty::Dict`
    // destructure below, because that destructure's `else` is the `T0033`
    // refusal -- and before it, through `lookup_bound_name_for_store`,
    // because `lookup_bound_name`'s own `reject_memoryview_read` would
    // otherwise report `C0001` one line earlier still.
    let dict_ty = lookup_bound_name_for_store(env, local_names, dict)?;
    if dict_ty == Ty::MemoryView {
        return check_buffer_set(env, local_names, key, value);
    }
    let Ty::Dict(kv) = &dict_ty else {
        return Err(Diagnostic::error(
            "T0033",
            format!("`{}` does not support item assignment", dict_ty.name()),
            Span::new(0, 0),
        ));
    };
    let (key_ty, val_ty) = kv.as_ref();
    // Exact `Ty` equality on the key, not `is_assignable` -- same reasoning
    // as `Subscript`'s own `Ty::Dict` read arm.
    let key_expr_ty = infer_expr_in(env, local_names, key)?;
    if key_expr_ty != *key_ty {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "dict key type mismatch: expected `{}`, found `{}`",
                key_ty.name(),
                key_expr_ty.name()
            ),
            Span::new(0, 0),
        )
        .with_help(format!("use a `{}` value here", key_ty.name())));
    }
    let value_ty = infer_expr_in(env, local_names, value)?;
    if !is_assignable(value_ty.clone(), val_ty.clone()) {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "cannot assign `{}` to a dict value of `{}`",
                value_ty.name(),
                val_ty.name()
            ),
            Span::new(0, 0),
        ).with_help(format!("change the value to `{}` (the expected/declared type), or the declaration/annotation to `{}` (the actual type)", val_ty.name(), value_ty.name())));
    }
    Ok(())
}

/// `b[i] = v` on a name bound to a `memoryview` (Part 1 of #1142): the
/// store counterpart of the `Subscript` load `expr.rs` intercepts, and the
/// second operation on such a name this compiler admits after `b[i]` and
/// `len(b)`.
///
/// The index rule is the load's rule verbatim -- `T0021` and `is_assignable`
/// against `Ty::Int`, so `bool` is accepted under D-086 -- because a load
/// and a store that disagreed about `b[True]` would be a defect on its own.
/// The value rule is `check_dict_set`'s own: `T0021` and `is_assignable`
/// against the element type, which for the one admitted buffer format
/// (`'d'`) is `Ty::Float`. `is_assignable` is not a widening here -- D-086
/// admits no implicit `int` -> `float` -- so `b[0] = 1` is refused and the
/// generated code has exactly one scalar shape to store.
///
/// This admits the store's *target name* and nothing else: every other read
/// of a buffer-bound name stays `reject_memoryview_read`'s `C0001`, which is
/// what keeps D-244's wholly-wrapper-owned lifetime true by construction.
fn check_buffer_set(
    env: &Environment,
    local_names: &[&str],
    key: &HirExpr,
    value: &HirExpr,
) -> Result<(), Diagnostic> {
    let index_ty = infer_expr_in(env, local_names, key)?;
    if !is_assignable(index_ty.clone(), Ty::Int) {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "`memoryview` index must be `int`, found `{}`",
                index_ty.name()
            ),
            Span::new(0, 0),
        )
        .with_help("use an `int` value"));
    }
    let value_ty = infer_expr_in(env, local_names, value)?;
    if !is_assignable(value_ty.clone(), Ty::Float) {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "cannot assign `{}` to a `memoryview` element of `float`",
                value_ty.name()
            ),
            Span::new(0, 0),
        )
        .with_help("use a `float` value here"));
    }
    Ok(())
}

/// Checks `function` in complete isolation from any module -- no sibling
/// function signatures, no module-level globals, and (D-154, Part 1 of
/// #375) no class table either, since this entry point takes a bare
/// `&HirItem` with no enclosing `&HirModule` to source `class_defs` from.
/// A body that calls a sibling function or reads a module global already
/// couldn't check here before D-154 existed; a body that instantiates a
/// class, or reads/writes/calls a method on an instance, is the same kind
/// of gap, not a new one -- `Environment::classes` is simply empty.
/// Production compilation never reaches this function: `check`/
/// `check_and_resolve` always build their `Environment` from a real
/// `HirModule` (via `class::bind_classes`), so this isolation only affects
/// a caller that deliberately checks one function outside any module
/// context -- this crate's own unit tests, and the workspace's own
/// direct-API integration tests (e.g. `tests/slice0.rs`), today.
pub fn check_function(function: &HirItem) -> Result<(), Diagnostic> {
    let local_names = match function {
        HirItem::Function { params, body, .. } => function_local_names(params, body),
        HirItem::TopLevelStmt(_) => Vec::new(),
    };
    check_function_in(&Environment::new(), function, &local_names)
}

/// Checks one function's body, resolving sibling calls and module-level
/// global reads against a clone of `module_env` (see D-040/D-041/D-055) instead
/// of an isolated, self-only scope. Lexically local binding targets are removed
/// from that clone before the body is checked. The clone owns independent value
/// bindings while sharing the immutable function registry through copy-on-write
/// storage, so a function's parameters and local assignments never leak back
/// into the module scope or into any other function's check.
fn check_function_in(
    module_env: &Environment,
    function: &HirItem,
    local_names: &[&str],
) -> Result<(), Diagnostic> {
    let HirItem::Function {
        name,
        params,
        return_ty,
        body,
    } = function
    else {
        panic!("check_function called with a non-Function HirItem");
    };
    let standalone_params;
    let (resolved_params, resolved_return, signature_was_registered) =
        if let Some((param_tys, return_ty)) = module_env.lookup_function(name) {
            (param_tys.as_slice(), return_ty.clone(), true)
        } else {
            standalone_params = params.iter().map(|(_, ty)| ty.clone()).collect::<Vec<_>>();
            (standalone_params.as_slice(), return_ty.clone(), false)
        };
    if resolved_params.contains(&Ty::Infer) || resolved_return == Ty::Infer {
        return Err(Diagnostic::error(
            "T0021",
            format!("cannot check private helper `{name}` before its signature is inferred"),
            Span::new(0, 0),
        ));
    }
    let mut env = module_env.child_for_function(local_names);
    // PEP 695 (#387): record the function's own type-parameter name (if any)
    // so `check_stmt_in_function`'s `Return` arm can reject a generic
    // function returning its own `Ty::Param` as a concrete scalar. Errors
    // from `generic_type_param_name` (multiple params, container-position
    // occurrence) are suppressed here with `.ok()`: a generic function
    // reaching `check_function_in` through `check_generic_function_in` has
    // already been validated by that function's own
    // `generic_type_param_name` call, and a non-generic function returns
    // `Ok(None)` unconditionally.
    // Part 2b of #1142 (#1164), review round 5: the single pending-return
    // record's precondition, computed once for the whole body. Read only by
    // the buffer-egress admission in `check_stmt_in_function`'s
    // `HirStmt::Return` arm; see `crate::buffer::buffer_return_inside_finally`.
    env.returns_inside_finally = pycc_hir::body_returns_inside_finally(body);
    env.return_inferred = *return_ty == Ty::Infer;
    env.in_classmethod = name.ends_with(".classmethod");
    env.own_type_param = generic_type_param_name(params, return_ty).ok().flatten();
    // #433: extract the class name from a mangled `<ClassName>.<method>`
    // name so `infer_expr_in`'s `HirExpr::Super` arm can resolve the next
    // class in the MRO. A top-level function name contains no `.`, so
    // `current_class` stays `None` for those (and for the module-level
    // environment, which never goes through `check_function_in`).
    //
    // #1337 (D-254): a receiver-exact copy of an inherited body is spelled
    // with its *receiver* prefix, but `super()` anchors at the class that
    // defines the body -- the copy's origin class. The receiver the
    // `super()` search follows is `self`'s type (`class::super_call`).
    env.current_class = match pycc_hir::inherited_copy_origin(name, &|n| module_env.classes.get(n))
    {
        Some(copy) => Some(copy.origin_class),
        None => name
            .split('.')
            .next()
            .filter(|prefix| *prefix != name)
            .map(String::from),
    };
    if !signature_was_registered {
        env.bind_function(
            name.clone(),
            resolved_params.to_vec(),
            resolved_return.clone(),
        );
    }
    for ((param_name, _), param_ty) in params.iter().zip(resolved_params.iter().cloned()) {
        env.bind(param_name.clone(), param_ty);
    }
    // #380 (PR-20): determine whether this function is an abstract method.
    // An abstract method has a declaration-style body (`...` or `pass`)
    // that is not lowered — its HIR body is just `Return(None)`. The
    // function name is mangled as `<ClassName>.<method>`; we check whether
    // the class lists it in `abstract_methods`. Skip body checking and
    // the return-contract check for abstract methods — the body is never
    // executed (a concrete subclass overrides it).
    let is_abstract_method = name
        .split('.')
        .next()
        .filter(|class_name| *class_name != name)
        .and_then(|class_name| module_env.classes.get(class_name))
        .is_some_and(|class_def| {
            let method_name = name.split('.').nth(1).unwrap_or("");
            class_def.abstract_methods.iter().any(|m| m == method_name)
        });
    if !is_abstract_method {
        narrow::check_stmt_sequence_in_function(
            &mut env,
            local_names,
            body,
            resolved_return.clone(),
        )?;
    }
    if !is_abstract_method && resolved_return != Ty::None && !block_always_returns(body) {
        return Err(Diagnostic::error(
            "T0022",
            format!(
                "function `{name}` can exit without returning `{}`",
                resolved_return.name()
            ),
            Span::new(0, 0),
        )
        .with_help(format!("return a `{}` value", resolved_return.name())));
    }
    Ok(())
}

/// Issue #118 Part 1: fast-path helper for function-scope `if` statements
/// where neither branch introduces new bindings.
fn check_if_branches_in_place_in_function(
    env: &mut Environment,
    local_names: &[&str],
    body: &[HirStmt],
    orelse: &[HirStmt],
    return_ty: Ty,
) -> Result<(), Diagnostic> {
    // Warning fix (D-068 review of #780): see the module-scope
    // `check_if_branches_in_place`'s identical comment -- route through the
    // narrowing-aware sequence checker so a nested early-return guard's
    // narrowing propagates to later statements even on this fast path.
    narrow::check_stmt_sequence_in_function(env, local_names, body, return_ty.clone())?;
    narrow::check_stmt_sequence_in_function(env, local_names, orelse, return_ty)
}

/// Issue #118 Part 1: fast-path helper for function-scope `while` loops
/// where the body introduces no new bindings.
fn check_while_body_in_place_in_function(
    env: &mut Environment,
    local_names: &[&str],
    body: &[HirStmt],
    return_ty: Ty,
) -> Result<(), Diagnostic> {
    // Warning fix (D-068 review of #780): see
    // `check_if_branches_in_place_in_function`'s identical comment.
    narrow::check_stmt_sequence_in_function(env, local_names, body, return_ty)
}

fn check_stmt_in_function(
    env: &mut Environment,
    local_names: &[&str],
    stmt: &HirStmt,
    return_ty: Ty,
) -> Result<(), Diagnostic> {
    match stmt {
        HirStmt::Return(None) => {
            if return_ty != Ty::None && !object_none::admits_none_return(&return_ty, None) {
                return Err(Diagnostic::error(
                    "T0022",
                    format!(
                        "expected a return value of type `{}`, got none",
                        return_ty.name()
                    ),
                    Span::new(0, 0),
                )
                .with_help(format!("return a `{}` value", return_ty.name())));
            }
            Ok(())
        }
        HirStmt::Return(Some(expr)) if object_none::admits_none_return(&return_ty, Some(expr)) => {
            // #1387: `return None` into an `object` slot; see `object_none`.
            Ok(())
        }
        HirStmt::Return(Some(expr)) => {
            // Part 2b of #1142 (#1164): the one position an artifact-owned
            // buffer name is admitted as a whole value. Intercepted here,
            // before `infer_expr_in` reaches its `Name` arm's
            // `reject_memoryview_read`, on the model that arm's own doc
            // comment records for `b[i]` and `len(b)` -- the refusal is not
            // weakened, the set of expressions that reach it narrows by one.
            //
            // Part 1 of #1175 adds the **second** provenance: the name a
            // `memoryview` parameter binds. The provenance split itself now
            // lives in `crate::buffer::admits_buffer_egress`, which both
            // walkers call with their own spelling of the same three facts,
            // so the solver's admission cannot drift wider than this one.
            // `owned_buffers` membership is this environment's answer to
            // "artifact-owned"; a `Ty::MemoryView` binding outside that set
            // is a parameter and nothing else.
            //
            // An early-return admission owes an account of *every* check the
            // ordinary path would have run, not only the one it was designed
            // to bypass. Two run below, and the two answers differ.
            //
            // The **assignability** check is bypassed deliberately: the
            // operand's type is `Ty::MemoryView` by construction of
            // `owned_buffers` on the owned arm and by the `buffer_bound`
            // conjunct itself on the caller-owned one, and the declared type
            // is `Ty::MemoryView` by `admitted_buffer_return`'s own test, so
            // the two agree on both arms.
            //
            // The **definite-assignment** check is *not* bypassed, and the
            // third conjunct below is what keeps it. `owned_buffers` joins as
            // a union across control flow while the binding joins on the
            // `Definitely`/`Maybe`/unbound lattice, so the two sets disagree
            // for exactly the name that is owned on some path and unbound on
            // another (review round 3 of #1164). Consulting `owned_buffers`
            // alone admitted `if c: a = ndarray(4)` / `return a`, whose false
            // path loads the null-initialized slot and hands the host an
            // internal `SystemError` instead of the `T0041` this contract
            // owes -- see `docs/TYPE_SYSTEM.md`'s definite-assignment clause.
            //
            // The conjunct tests the *result* of a join rather than any
            // particular join, so it closes every join form at once -- `if`
            // without `else`, an `if`/`else` binding on one side only, a
            // `while`, `for`-`range` or `for`-list body, a non-exhaustive
            // `match`, and a `try` body -- rather than the one
            // counter-example that prompted it. It also fails closed on
            // `None`: a name that is not bound at all falls through to the
            // ordinary `T0021`.
            //
            // The `returns_inside_finally` test is review round 5's, and
            // unlike the conjuncts above it is a *refusal* rather than a
            // decline: see `crate::buffer::buffer_return_inside_finally` for
            // the host crash it closes and why the mechanism's cardinality
            // assumption, not the set of paths reaching it, is what had to
            // change. It is a whole-function property, so it is computed
            // once in `check_function_in` rather than re-walked here.
            //
            // Part 1 of #1175 keeps it for the caller-owned provenance too,
            // and runs it **once**, after the provenance verdict rather than
            // inside either arm -- a duplicated check is a check that drifts.
            // A caller-owned return transfers no ownership and so needs no
            // pending-return record of its own, but a function that also
            // allocates owned storage still has one, and Part 1 does not
            // carry the argument about that record's state on the parameter
            // path. The narrowing is deliberate and conservative, not a
            // necessity; the D-244 amendment records it as revisitable with
            // #1173.
            if let Some((name, shape)) =
                crate::buffer::admitted_buffer_return(expr, Some(&return_ty))
            {
                let state = env.binding_state(name);
                if crate::buffer::admits_buffer_egress(
                    env.owned_buffers.contains(name),
                    matches!(state, Some(BindingState::Definitely(_))),
                    state.is_some_and(|state| matches!(state.ty(), Ty::MemoryView)),
                    shape,
                ) {
                    // The post-admission block, kept line-for-line parallel
                    // with the solver's own (`crate::constraints`) so a
                    // future divergence is visible in review. Every refusal
                    // here exists *because* a buffer boundary was admitted;
                    // the `!owned` gate is not one of them and lives in
                    // `admits_buffer_egress`'s formula alone (#1179).
                    if let crate::buffer::AdmittedBufferReturn::Slice {
                        start,
                        stop,
                        has_step,
                    } = shape
                    {
                        if has_step {
                            return Err(crate::buffer::buffer_slice_step_unsupported(name));
                        }
                        // D1: this branch exits before `infer_expr_in` ever
                        // sees the operand, so the bound type check the
                        // ordinary `HirExpr::Slice` arm performs has to be
                        // repeated here. Without it `b[1.5:3]` would reach
                        // an `i64` out-slot write undiagnosed and `b[x:3]`
                        // with `x` unbound would reach `pycc_mir::lookup`'s
                        // "check should have rejected this HIR" panic.
                        for (label, bound) in [("start", start), ("stop", stop)] {
                            if let Some(bound) = bound {
                                let bound_ty = infer_expr_in(env, local_names, bound)?;
                                if !is_assignable(bound_ty.clone(), Ty::Int) {
                                    return Err(crate::buffer::buffer_slice_bound_not_an_int(
                                        label, &bound_ty,
                                    ));
                                }
                            }
                        }
                    }
                    if env.returns_inside_finally {
                        return Err(crate::buffer::buffer_return_inside_finally(name));
                    }
                    return Ok(());
                }
            }
            // Part 1 of #1333: returning a CPython object is admitted; the
            // ordinary assignability check below governs it, so `-> str`
            // returning an object stays the `T0022` mismatch.
            let actual = infer_expr_in(env, local_names, expr)?;
            if !class::is_assignable_env(env, &actual, &return_ty)
                && !object_box::admits_value(env, expr, &actual, &return_ty)?
            {
                // #1344: an inferred `set[int]` return the solver could not
                // widen to the `set[C]` the body builds is a compiler limit,
                // not a user error.
                if env.return_inferred
                    && let Some(diag) =
                        crate::comprehension::inferred_set_return_limit(&return_ty, &actual)
                {
                    return Err(diag);
                }
                // #380 (PR-20): if the mismatch involves a protocol,
                // produce a detailed T0046 conformance error.
                let diag = if matches!(return_ty, Ty::Protocol(_))
                    || matches!(actual, Ty::Protocol(_))
                {
                    class::assignable_error(env, &actual, &return_ty)
                } else {
                    Diagnostic::error(
                        "T0022",
                        format!(
                            "expected return type `{}`, got `{}`",
                            return_ty.name(),
                            actual.name()
                        ),
                        Span::new(0, 0),
                    )
                    .with_help(
                        foreign::object_into_scalar_help(&actual, &return_ty)
                            .unwrap_or_else(|| format!("return a `{}` value", return_ty.name())),
                    )
                };
                return Err(diag);
            }
            // PEP 695 (#387): `is_assignable`'s `from == Ty::Param` clause
            // lets a generic function's own `Ty::Param` pass as any concrete
            // scalar (the clause is needed for non-generic functions reading
            // generic class instance attributes). Narrow that here: if the
            // returned value is the *current function's own* `Ty::Param` and
            // the declared return type is a concrete scalar, reject -- after
            // monomorphization `T` is substituted with the call-site scalar,
            // which may not match the declared return type (e.g.
            // `def bad[T](x: T) -> int: return x` called as `bad("s")`).
            // A class-owned `Ty::Param` (the function has no own type param)
            // is not rejected here -- monomorphization of the class
            // substitutes it before codegen.
            if let (Some(own), Ty::Param(actual_name)) = (&env.own_type_param, &actual)
                && own.as_str() == actual_name.as_ref()
                && matches!(return_ty, Ty::Int | Ty::Float | Ty::Bool | Ty::Str)
            {
                return Err(Diagnostic::error(
                    "T0022",
                    format!(
                        "expected return type `{}`, got `{}`",
                        return_ty.name(),
                        actual.name()
                    ),
                    Span::new(0, 0),
                ).with_help(format!(
                    "a generic function's type parameter `{own}` is not guaranteed to be `{}` at every call site",
                    return_ty.name()
                )));
            }
            Ok(())
        }
        HirStmt::If { test, body, orelse } => {
            // PEP 572 (#774): bind before validating -- see the module-scope
            // `check_stmt`'s `ExprStmt` arm's doc comment for why a walrus
            // value that references an earlier walrus in the same
            // expression (`(a := 1) + (b := a + 1)`) requires the binding
            // pass to run before the full-expression `infer_expr_in` check.
            // `collect_local_names` (Step 1) already added the name to
            // `local_names`. The test always executes, so this binding is
            // unconditional relative to the branch join below.
            collect_named_expr_bindings(env, local_names, test)?;
            infer_expr_in(env, local_names, test)?;
            // Issue #118 Part 1: check each branch in an independent clone of
            // env, then join the results. A no-else `if` makes all body-only
            // bindings `Maybe`.
            // Issue #769 (Part 2 of #747): see the module-scope `If` arm's
            // identical comment -- a narrowing-eligible test forces the
            // slow/cloning path.
            let narrowing = narrow::narrowing_target(env, test);
            // Fast path: if neither branch introduces any new bindings and
            // no narrowing applies, skip the clone+join and check both
            // branches in-place.
            if narrowing.is_none() && !introduces_bindings(body) && !introduces_bindings(orelse) {
                check_if_branches_in_place_in_function(
                    env,
                    local_names,
                    body,
                    orelse,
                    return_ty.clone(),
                )
            } else {
                let mut body_env = env.clone();
                let mut orelse_env = env.clone();
                if let Some(target) = &narrowing {
                    narrow::apply_branch_narrowing(&mut body_env, &mut orelse_env, target);
                }
                narrow::check_stmt_sequence_in_function(
                    &mut body_env,
                    local_names,
                    body,
                    return_ty.clone(),
                )?;
                narrow::check_stmt_sequence_in_function(
                    &mut orelse_env,
                    local_names,
                    orelse,
                    return_ty.clone(),
                )?;
                if let Some(target) = &narrowing {
                    narrow::end_branch_narrowing(
                        env,
                        &mut body_env,
                        &mut orelse_env,
                        target,
                        body,
                        orelse,
                    );
                }
                join_if_branches(env, &body_env, &orelse_env)
            }
        }
        HirStmt::While { test, body } => {
            // Issue #769 follow-up (D-068 re-review round 3): see the
            // module-scope `While` arm's identical comment and
            // `narrow::apply_kill_prescan`'s doc comment.
            narrow::apply_kill_prescan(env, body);
            narrow::apply_delete_prescan(env, body, None);
            // PEP 572 (#774): bind before validating, mirroring the `If`
            // arm just above.
            collect_named_expr_bindings(env, local_names, test)?;
            infer_expr_in(env, local_names, test)?;
            // Issue #118 Part 1: the loop body may execute zero times, so
            // every body-only binding joins back as `Maybe`.
            // Fast path: if the body introduces no bindings, check in-place.
            if !introduces_bindings(body) {
                check_while_body_in_place_in_function(env, local_names, body, return_ty.clone())
            } else {
                let mut body_env = env.clone();
                narrow::check_stmt_sequence_in_function(
                    &mut body_env,
                    local_names,
                    body,
                    return_ty.clone(),
                )?;
                join_loop_body(env, &body_env);
                Ok(())
            }
        }
        HirStmt::ForRange {
            var,
            start,
            stop,
            step,
            body,
        } => {
            check_range_operand_in(env, local_names, "start", start)?;
            check_range_operand_in(env, local_names, "stop", stop)?;
            check_range_operand_in(env, local_names, "step", step)?;
            // Issue #118 Part 1: track whether the loop variable was already
            // definitely bound before the loop (see check_stmt's ForRange).
            let was_definite = matches!(env.binding_state(var), Some(BindingState::Definitely(_)));
            check_assignment(env, var, Ty::Int)?;
            let mut body_env = env.clone();
            // Issue #769 follow-up (D-068 re-review round 3): see
            // `narrow::apply_kill_prescan`.
            narrow::apply_kill_prescan(&mut body_env, body);
            narrow::apply_delete_prescan(&mut body_env, body, Some(var));
            narrow::check_stmt_sequence_in_function(
                &mut body_env,
                local_names,
                body,
                return_ty.clone(),
            )?;
            join_loop_body(env, &body_env);
            // Issue #118 Part 1: if the loop variable was not definitely bound
            // before the loop, downgrade it to Maybe (the loop may execute
            // zero times). A pre-bound variable stays Definitely bound.
            if !was_definite && let Some(ty) = env.lookup_any(var) {
                env.bind_maybe(var.to_string(), ty);
            }
            Ok(())
        }
        HirStmt::ForList { var, list, body } => {
            // #379 (PR-19): `for c in Color:` inside a function body —
            // same enum iteration intercept as the module-scope `check_stmt`
            // arm above. A class name is not a value binding, so
            // `lookup_bound_name` fails; check `env.lookup_class` first.
            if let Some(class_def) = env.lookup_class(list)
                && !class_def.enum_members.is_empty()
            {
                return check_enum_loop_body_function(
                    env,
                    var,
                    list,
                    body,
                    local_names,
                    return_ty.clone(),
                );
            }
            let list_ty = lookup_bound_name(env, local_names, list)?;
            // See the module-scope `check_stmt` arm's own comment (PR-11
            // Task 3, D-123): `for k in d:` iterates a dict's keys. PR-11
            // Task 7 (D-123): `for x in s:` iterates a set's elements.
            let var_ty = match list_ty {
                Ty::List(elem_ty) => *elem_ty,
                Ty::Dict(kv) => kv.0,
                Ty::Set(elem_ty) | Ty::FrozenSet(elem_ty) => *elem_ty,
                other => {
                    return Err(Diagnostic::error(
                        "T0033",
                        format!(
                            "`{}` cannot be iterated with `for ... in ...` (only list[T]/dict[K, V]/set[T]/frozenset[T] supports this)",
                            other.name()
                        ),
                        Span::new(0, 0),
                    ));
                }
            };
            // Issue #118 Part 1: track whether the loop variable was already
            // definitely bound before the loop (see check_stmt's ForList).
            let was_definite = matches!(env.binding_state(var), Some(BindingState::Definitely(_)));
            check_assignment(env, var, var_ty)?;
            let mut body_env = env.clone();
            // Issue #769 follow-up (D-068 re-review round 3): see
            // `narrow::apply_kill_prescan`.
            narrow::apply_kill_prescan(&mut body_env, body);
            narrow::apply_delete_prescan(&mut body_env, body, Some(var));
            narrow::check_stmt_sequence_in_function(
                &mut body_env,
                local_names,
                body,
                return_ty.clone(),
            )?;
            join_loop_body(env, &body_env);
            // Issue #118 Part 1: if the loop variable was not definitely bound
            // before the loop, downgrade it to Maybe (the loop may execute
            // zero times). A pre-bound variable stays Definitely bound.
            if !was_definite && let Some(ty) = env.lookup_any(var) {
                env.bind_maybe(var.to_string(), ty);
            }
            Ok(())
        }
        // PR 3c of #1082: refused unconditionally inside a function body.
        // A function-body `for` over a CPython object needs its loop and
        // iterator emission moved onto the function failure edge, which is
        // Part 2 of #1333 (#1363); the module body's own loop runs on the
        // module-exec edge instead. The iterable is therefore never inferred
        // here -- there is no shape of it this arm could accept yet.
        //
        // The message states that bound and stops there. `pycc_hir` routes
        // *every* attribute and attribute-callee-call iterable to
        // `ForObject`, so this arm also fires for `d.keys()` and
        // `xs.copy()`, which a module body refuses too (with `T0036` and
        // `T0043`). A message promising that a module body implements the
        // construct would send those callers to a scope where their program
        // fails differently.
        HirStmt::ForObject { .. } => Err(Diagnostic::error(
            "I0404",
            "`for ... in <attribute or method call>` is not supported inside a function body \
             yet, so the statement is refused here whatever the iterable turns out to be"
                .to_string(),
            Span::new(0, 0),
        )),
        // PR-12 Task 3 (D-117): `target = <comp>` in a function body,
        // checked by the shared `comprehension::check_comp_assign` helper.
        HirStmt::ListCompAssign {
            target,
            var,
            iter,
            cond,
            elt,
        } => check_comp_assign(
            env,
            local_names,
            target,
            CompView {
                var,
                iter,
                cond: cond.as_deref(),
                elts: CompElts::List(elt),
            },
        ),
        HirStmt::SetCompAssign {
            target,
            var,
            iter,
            cond,
            elt,
        } => check_comp_assign(
            env,
            local_names,
            target,
            CompView {
                var,
                iter,
                cond: cond.as_deref(),
                elts: CompElts::Set(elt),
            },
        ),
        HirStmt::DictCompAssign {
            target,
            var,
            iter,
            cond,
            key,
            value,
        } => check_comp_assign(
            env,
            local_names,
            target,
            CompView {
                var,
                iter,
                cond: cond.as_deref(),
                elts: CompElts::Dict(key, value),
            },
        ),
        HirStmt::Assign { target, value } => {
            // #1021: `name_binding` is a no-op for every code but `T0003`,
            // and for a `T0003` it substitutes the binding's name only when
            // `value` is itself the empty literal that failed -- the only
            // locator a `T0003` gets, since `HirStmt::Assign` carries no span
            // and every container diagnostic in this crate renders at `1:1`.
            // A `T0003` from a nested element position (`[[]]`) keeps the
            // generic wording; see `name_binding`'s own documentation.
            // Part 2a of #1142 (#1165): the buffer producer is admitted at
            // exactly this position -- an assignment's whole right-hand side
            // -- and `buffer::producer_assignment_ty` is the one seam that
            // admits it. `None` means the value is not a producer here
            // (including when the program's own `class`/`def`/binding of the
            // spelling wins, D-244 #1129 statement (h)) and the ordinary
            // inference below runs unchanged.
            if let Some(produced) = buffer::producer_assignment_ty(env, local_names, value) {
                let ty = produced?;
                check_assignment(env, target, ty)?;
                env.owned_buffers.insert(target.clone());
                return Ok(());
            }
            let ty = infer_expr_in(env, local_names, value)
                .map_err(|d| empty_container::name_binding(d, target, value))?;
            check_assignment_boxing(env, target, ty, value)
        }
        HirStmt::AnnAssign {
            target,
            annotation,
            value,
            is_final,
        } => {
            // Part 1 of #1027: a `memoryview` annotation is admitted only in
            // a signature, never on a declaration -- see
            // `expr::reject_memoryview_declaration`. Checked ahead of the
            // value/no-value split so both shapes route through the one
            // contract.
            //
            // Part 2a of #1142 (#1165): see the module-scope arm's comment --
            // the refusal is gated, not widened, so `a: NDArray = ndarray(n)`
            // is admitted while a value-less `a: NDArray` is refused exactly
            // as before.
            let produced = value
                .as_ref()
                .and_then(|value| buffer::producer_assignment_ty(env, local_names, value));
            if produced.is_none() {
                reject_memoryview_declaration(target, annotation)?;
            }
            if let Some(value) = value {
                let inferred = match produced {
                    Some(produced) => produced?,
                    None => infer_expr_in(env, local_names, value)
                        .map_err(|d| empty_container::name_binding(d, target, value))?,
                };
                if !class::is_assignable_env(env, &inferred, annotation)
                    && !object_box::admits_value(env, value, &inferred, annotation)?
                {
                    // #380 (PR-20): if the mismatch involves a protocol,
                    // produce a detailed T0046 conformance error.
                    let diag = if matches!(annotation, Ty::Protocol(_))
                        || matches!(inferred, Ty::Protocol(_))
                    {
                        class::assignable_error(env, &inferred, annotation)
                    } else {
                        annotation_initializer_mismatch(target, &inferred, annotation)
                    };
                    return Err(diag);
                }
                // See the module-scope `check_stmt` arm's comment: route
                // through `check_assignment` so a name's first-established
                // representation stays sticky, matching `pycc_mir`'s own
                // `bind_variable` invariant.
                // #380 (PR-20): when the annotation is a protocol type,
                // bind with the concrete (inferred) type instead — see
                // the module-scope arm's comment for the rationale.
                let bind_ty = if matches!(annotation, Ty::Protocol(_)) {
                    inferred.clone()
                } else {
                    annotation.clone()
                };
                check_assignment(env, target, bind_ty)?;
                // Part 2a of #1142 (#1165): record the provenance the read
                // seams dispatch on. A `Ty::MemoryView` can only reach this
                // point from the producer -- the gated declaration refusal
                // above rejects every other initializer under a buffer
                // annotation -- so this needs no second look at `value`.
                if matches!(inferred, Ty::MemoryView) {
                    env.owned_buffers.insert(target.clone());
                }
            } else {
                // See the module-scope `check_stmt` arm's comment (issue
                // #245): retain the declared type via `env.declare` without
                // binding it, so a premature read still raises T0021 and a
                // later assignment is checked against the declaration.
                env.declare(target.clone(), annotation.clone())?;
            }
            // PEP 591 (#383): see the module-scope `check_stmt` arm's
            // comment — record the name as `Final` *after*
            // `check_assignment` returns so the initial assignment is not
            // rejected, only a subsequent reassignment.
            if *is_final {
                env.finals.insert(target.clone());
            }
            Ok(())
        }
        HirStmt::ExprStmt(expr) => {
            // PEP 572 (#774): bind before validating, mirroring the
            // module-scope `check_stmt`'s `ExprStmt` arm's doc comment.
            collect_named_expr_bindings(env, local_names, expr)?;
            infer_expr_in(env, local_names, expr).map(|_| ())
        }
        HirStmt::DictSet { dict, key, value } => check_dict_set(env, local_names, dict, key, value),
        HirStmt::AttrSet { base, attr, value } => {
            class::check_attr_set(env, local_names, base, attr, value)
        }
        HirStmt::Match { subject, cases } => {
            check_match(env, local_names, subject, cases, Some(&return_ty))
        }
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => check_try_stmt(
            env,
            local_names,
            body,
            handlers,
            orelse,
            finalbody,
            Some(&return_ty),
        ),
        HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => check_try_star_stmt(
            env,
            local_names,
            body,
            handlers,
            orelse,
            finalbody,
            Some(&return_ty),
        ),
        HirStmt::Raise { exc, cause } => check_raise_stmt(env, local_names, exc, cause),
        HirStmt::Delete { name } => del_stmt::check_delete(env, name),
        HirStmt::DeleteSlice {
            base,
            start,
            stop,
            step,
            span,
        } => foreign::slice::check_delete_slice(
            env,
            local_names,
            base,
            [start.as_deref(), stop.as_deref(), step.as_deref()],
            *span,
        ),
        HirStmt::DeleteAttr { base, span, .. } => {
            foreign::attr_store::check_delete_attr(env, local_names, base, *span)
        }
        // `pycc_hir` never produces this node in a function body; binding
        // it here keeps the two statement checkers in step (#1291).
        HirStmt::ForeignImport { bindings, .. } => {
            foreign::bind_block_import(env, bindings);
            Ok(())
        }
    }
}

fn t0042(message: impl Into<String>) -> Diagnostic {
    Diagnostic::error("T0042", message.into(), Span::new(0, 0))
}

/// Recursively scans one signature-position `Ty` for `Ty::Param` occurrences
/// (D-133/D-134), threading the type parameter name found so far through
/// every call. `is_top_level` distinguishes a parameter/return type's own
/// top-level position (where a bare `Ty::Param` is the one shape v0.2
/// instantiates) from any position nested inside a container (`list[T]`,
/// `dict[str, T]`, ...), which D-134 rejects outright regardless of whether
/// the type parameter is otherwise consistent -- container-of-type-parameter
/// is out of scope independently of this defense-in-depth pass, matching
/// D-105's own pre-existing "container element type is fixed, not generic"
/// restriction.
///
/// Defense in depth, not a reachable frontend path: `crates/pycc_hir/src/func.rs`'s
/// `lower_function` already enforces at most one PEP 695 `TypeVar` per
/// function (Task 1), and since D-228 (issue #918) `annotation_to_ty`
/// rejects a `Ty::Param` element inside the container annotations it does
/// lower -- with this same `T0042` code and wording, but a real span -- so a
/// real `def f[T](x: list[T])` or `def f[T, U](...)` still cannot reach
/// `pycc_types` from parsed source. (Before #918 the reason was blunter:
/// `annotation_to_ty` lowered no subscripted container annotation at all.)
/// This only fires for a hand-constructed `HirItem` (a future frontend
/// regression, or a unit test exercising this function directly, mirroring
/// how this file's other "defense in depth" checks are exercised
/// elsewhere).
fn scan_signature_ty_for_param(
    ty: &Ty,
    is_top_level: bool,
    found: &mut Option<String>,
) -> Result<(), Diagnostic> {
    match ty {
        Ty::Param(name) => {
            if !is_top_level {
                return Err(t0042(format!(
                    "type parameter `{name}` used inside a container position is not supported yet -- v0.2 only instantiates a bare type-parameter position, matching D-105's own fixed-container-element-type restriction"
                )));
            }
            match found {
                Some(existing) if existing != name.as_ref() => {
                    return Err(t0042(format!(
                        "generic functions with more than one type parameter are not supported yet (found both `{existing}` and `{name}`)"
                    )));
                }
                _ => *found = Some(name.to_string()),
            }
            Ok(())
        }
        Ty::List(elem) | Ty::Set(elem) | Ty::FrozenSet(elem) | Ty::Optional(elem) => {
            scan_signature_ty_for_param(elem, false, found)
        }
        Ty::Dict(kv) => {
            scan_signature_ty_for_param(&kv.0, false, found)?;
            scan_signature_ty_for_param(&kv.1, false, found)
        }
        Ty::Tuple(elems) => {
            for elem in elems.iter() {
                scan_signature_ty_for_param(elem, false, found)?;
            }
            Ok(())
        }
        // D-154: `Ty::Instance` can never carry a `Ty::Param` -- see
        // `ty_contains_param`'s own identical arm/reasoning above.
        Ty::Int
        | Ty::Float
        | Ty::Bool
        | Ty::Str
        | Ty::None
        | Ty::Infer
        | Ty::Instance(_)
        | Ty::Protocol(_)
        | Ty::Object
        | Ty::MemoryView => Ok(()),
    }
}

/// Finds the single PEP 695 type-parameter name used across a function's
/// parameter and return types, or `None` if the function isn't generic at
/// all. Returns `T0042` if it finds two distinct names or any
/// container-position occurrence (see `scan_signature_ty_for_param`'s own
/// doc comment for why this is defense-in-depth rather than a reachable
/// frontend path).
fn generic_type_param_name(
    params: &[(String, Ty)],
    return_ty: &Ty,
) -> Result<Option<String>, Diagnostic> {
    let mut found = None;
    for (_, ty) in params {
        scan_signature_ty_for_param(ty, true, &mut found)?;
    }
    scan_signature_ty_for_param(return_ty, true, &mut found)?;
    Ok(found)
}

/// D-133/D-134: type-checks a generic function's body exactly once,
/// symbolically. `Ty::Param(name)` participates in the existing
/// `collect_expr_constraints`/`infer_expr_in` traversal (via `check_function`
/// below) as an ordinary, opaque `Ty` value: `merge_inferred_types`,
/// `is_assignable`, and `numeric_result_type` already compare `Ty` by
/// structural equality (plus the one `Bool`/`Int` special case), so
/// `Ty::Param("T") == Ty::Param("T")` unifies exactly like any other
/// self-consistent type, while `Ty::Param("T")` used where an `int`-only
/// operator expects a numeric operand (e.g. `x + 1` where `x: T`) already
/// falls through those functions' existing "no match" arms and produces the
/// same `T0021` a real type mismatch would -- no changes to either function
/// were needed for this. This function's own job is the shape gate
/// `check_function` cannot express on its own: reject more than one
/// distinct type-parameter name or any container-position occurrence
/// (`T0042`, defense in depth per `generic_type_param_name`'s doc comment)
/// before the body is checked at all.
///
/// Same module-isolation caveat as `check_function` (D-154, Part 1 of
/// #375): this entry point's own `Environment` has no class table either,
/// for the identical reason -- no `&HirModule` is available here to source
/// `class_defs` from.
pub fn check_generic_function(func: &HirItem) -> Result<(), Diagnostic> {
    let local_names = match func {
        HirItem::Function { params, body, .. } => function_local_names(params, body),
        HirItem::TopLevelStmt(_) => Vec::new(),
    };
    check_generic_function_in(&Environment::new(), func, &local_names)
}

/// Module-environment-aware counterpart to [`check_generic_function`],
/// mirroring `check_function`/`check_function_in`'s own existing split.
///
/// PR-13 final review (I3): a generic function's body is checked against the
/// module's function-signature environment exactly like an ordinary
/// function's body is, so a call to a *non-generic* sibling function
/// resolves normally instead of producing a factually false "call to
/// undefined function" `T0021`. The earlier env-less scoping was not a
/// design requirement -- a generic function's own type parameter is
/// resolved by call-site substitution, which is orthogonal to whether its
/// body can see its siblings.
///
/// PR-13 final review (Critical): before the body is checked at all, every
/// call in it is scanned and rejected with `T0042` when its callee is this
/// function itself or any other generic function registered in
/// `module_env`. `monomorphize` never rewrites calls that appear inside a
/// generic body (it drops the original generic item wholesale and emits
/// only substituted specializations), so such a call would survive into the
/// emitted specialization as a reference to a function that no longer
/// exists, and `pycc_mir` would panic on it. D-134's thin slice is
/// single-call-site monomorphization, not general recursive generic
/// instantiation, so this shape is rejected pre-codegen with a clear
/// diagnostic instead of being accepted by `check` and crashing `build`.
fn check_generic_function_in(
    module_env: &Environment,
    func: &HirItem,
    local_names: &[&str],
) -> Result<(), Diagnostic> {
    if let HirItem::Function {
        name,
        params,
        return_ty,
        body,
    } = func
    {
        generic_type_param_name(params, return_ty)?;
        reject_generic_calls_in_block(module_env, name, body)?;
    }
    check_function_in(module_env, func, local_names)
}

/// `T0042` for one rejected call inside a generic function's own body
/// (see `check_generic_function_in`).
fn reject_generic_call(own_name: &str, callee: &str) -> Diagnostic {
    if own_name == callee {
        t0042(format!(
            "generic function `{own_name}` calls itself -- a generic function cannot call itself or another generic function (recursive generic instantiation is not supported yet)"
        ))
    } else {
        t0042(format!(
            "generic function `{own_name}` calls generic function `{callee}` -- a generic function cannot call itself or another generic function (recursive generic instantiation is not supported yet)"
        ))
    }
}

/// Walks every statement in a generic function's body, rejecting any call
/// whose callee is `own_name` or a generic function registered in
/// `module_env`. Structurally mirrors `rewrite_generic_calls_in_stmt` --
/// every statement position that can hold an expression is visited.
fn reject_generic_calls_in_block(
    module_env: &Environment,
    own_name: &str,
    body: &[HirStmt],
) -> Result<(), Diagnostic> {
    for stmt in body {
        reject_generic_calls_in_stmt(module_env, own_name, stmt)?;
    }
    Ok(())
}

/// Pushes every expression position a comprehension's iterable can hold
/// (`CompIter::Range`'s three bounds, `CompIter::Iterable`'s expression;
/// `CompIter::Name` holds none).
fn comp_iter_exprs<'a>(iter: &'a CompIter, exprs: &mut Vec<&'a HirExpr>) {
    match iter {
        CompIter::Range { start, stop, step } => {
            exprs.push(start);
            exprs.push(stop);
            exprs.push(step);
        }
        CompIter::Iterable(iterable) => exprs.push(iterable),
        CompIter::Name(_) => {}
    }
}

fn reject_generic_calls_in_stmt(
    module_env: &Environment,
    own_name: &str,
    stmt: &HirStmt,
) -> Result<(), Diagnostic> {
    let mut exprs: Vec<&HirExpr> = Vec::new();
    let mut blocks: Vec<&[HirStmt]> = Vec::new();
    match stmt {
        HirStmt::ExprStmt(expr) | HirStmt::Assign { value: expr, .. } => exprs.push(expr),
        HirStmt::AnnAssign { value, .. } => exprs.extend(value.iter()),
        HirStmt::Return(value) => exprs.extend(value.iter()),
        HirStmt::Delete { .. } | HirStmt::ForeignImport { .. } => {}
        HirStmt::DeleteSlice {
            base,
            start,
            stop,
            step,
            ..
        } => {
            exprs.push(base);
            exprs.extend([start, stop, step].into_iter().flatten().map(|b| &**b));
        }
        HirStmt::DeleteAttr { base, .. } => exprs.push(base),
        HirStmt::If { test, body, orelse } => {
            exprs.push(test);
            blocks.push(body);
            blocks.push(orelse);
        }
        HirStmt::While { test, body } => {
            exprs.push(test);
            blocks.push(body);
        }
        HirStmt::ForRange {
            start,
            stop,
            step,
            body,
            ..
        } => {
            exprs.push(start);
            exprs.push(stop);
            exprs.push(step);
            blocks.push(body);
        }
        HirStmt::ForList { body, .. } => blocks.push(body),
        // Unlike `ForList`, a `ForObject`'s iterable is a real expression
        // (`o.m(gen(1))`), so it must be walked here too or a generic call
        // inside the iterable escapes this pass (PR 3c of #1082).
        HirStmt::ForObject { iter, body, .. } => {
            exprs.push(iter);
            blocks.push(body);
        }
        HirStmt::DictSet { key, value, .. } => {
            exprs.push(key);
            exprs.push(value);
        }
        HirStmt::AttrSet { base, value, .. } => {
            exprs.push(base);
            exprs.push(value);
        }
        HirStmt::ListCompAssign {
            iter, cond, elt, ..
        }
        | HirStmt::SetCompAssign {
            iter, cond, elt, ..
        } => {
            comp_iter_exprs(iter, &mut exprs);
            exprs.extend(cond.iter().map(|c| c.as_ref()));
            exprs.push(elt);
        }
        HirStmt::DictCompAssign {
            iter,
            cond,
            key,
            value,
            ..
        } => {
            comp_iter_exprs(iter, &mut exprs);
            exprs.extend(cond.iter().map(|c| c.as_ref()));
            exprs.push(key);
            exprs.push(value);
        }
        HirStmt::Match { subject, cases } => {
            exprs.push(subject);
            for case in cases {
                if let Some(guard) = &case.guard {
                    exprs.push(guard);
                }
                blocks.push(&case.body);
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
            blocks.push(body);
            for handler in handlers {
                blocks.push(&handler.body);
            }
            blocks.push(orelse);
            blocks.push(finalbody);
        }
        HirStmt::Raise { exc, cause } => {
            exprs.extend(exc.iter());
            exprs.extend(cause.iter());
        }
    }
    for expr in exprs {
        reject_generic_calls_in_expr(module_env, own_name, expr)?;
    }
    for block in blocks {
        reject_generic_calls_in_block(module_env, own_name, block)?;
    }
    Ok(())
}

fn reject_generic_calls_in_expr(
    module_env: &Environment,
    own_name: &str,
    expr: &HirExpr,
) -> Result<(), Diagnostic> {
    match expr {
        HirExpr::Call { callee, args } => {
            if callee == own_name || module_env.lookup_generic(callee).is_some() {
                return Err(reject_generic_call(own_name, callee));
            }
            for arg in args {
                reject_generic_calls_in_expr(module_env, own_name, arg)?;
            }
            Ok(())
        }
        HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
            reject_generic_calls_in_expr(module_env, own_name, operand)
        }
        HirExpr::CompareChain { first, links } => {
            for operand in pycc_hir::compare_chain_operands(first, links) {
                reject_generic_calls_in_expr(module_env, own_name, operand)?;
            }
            Ok(())
        }
        HirExpr::BinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            reject_generic_calls_in_expr(module_env, own_name, left)?;
            reject_generic_calls_in_expr(module_env, own_name, right)
        }
        HirExpr::IfExp { test, body, orelse } => {
            for part in [test, body, orelse] {
                reject_generic_calls_in_expr(module_env, own_name, part)?;
            }
            Ok(())
        }
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(inner) = part {
                    reject_generic_calls_in_expr(module_env, own_name, inner)?;
                }
            }
            Ok(())
        }
        HirExpr::ListLiteral(elements)
        | HirExpr::ObjectList(elements)
        | HirExpr::SetLiteral(elements)
        | HirExpr::TupleLiteral(elements) => {
            for element in elements {
                reject_generic_calls_in_expr(module_env, own_name, element)?;
            }
            Ok(())
        }
        HirExpr::DictLiteral(pairs) => {
            for (key, value) in pairs {
                reject_generic_calls_in_expr(module_env, own_name, key)?;
                reject_generic_calls_in_expr(module_env, own_name, value)?;
            }
            Ok(())
        }
        HirExpr::Subscript { base, index } => {
            reject_generic_calls_in_expr(module_env, own_name, base)?;
            reject_generic_calls_in_expr(module_env, own_name, index)
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            reject_generic_calls_in_expr(module_env, own_name, base)?;
            for bound in [start, stop, step].into_iter().flatten() {
                reject_generic_calls_in_expr(module_env, own_name, bound)?;
            }
            Ok(())
        }
        HirExpr::ListAppend { list, value } => {
            if let Some(receiver) = list.attr_expr() {
                reject_generic_calls_in_expr(module_env, own_name, receiver)?;
            }
            reject_generic_calls_in_expr(module_env, own_name, value)
        }
        HirExpr::SetAdd { value, .. } => reject_generic_calls_in_expr(module_env, own_name, value),
        HirExpr::DictGetOrDefault { dict, key, default } => {
            if let Some(receiver) = dict.attr_expr() {
                reject_generic_calls_in_expr(module_env, own_name, receiver)?;
            }
            reject_generic_calls_in_expr(module_env, own_name, key)?;
            reject_generic_calls_in_expr(module_env, own_name, default)
        }
        HirExpr::AttrGet { base, .. } => reject_generic_calls_in_expr(module_env, own_name, base),
        HirExpr::MethodCall { base, args, .. } => {
            reject_generic_calls_in_expr(module_env, own_name, base)?;
            for arg in args {
                reject_generic_calls_in_expr(module_env, own_name, arg)?;
            }
            Ok(())
        }
        HirExpr::ReceiverDispatchedCall { call, .. } => {
            reject_generic_calls_in_expr(module_env, own_name, call)
        }
        HirExpr::KeywordCall { call, keywords, .. } => {
            reject_generic_calls_in_expr(module_env, own_name, call)?;
            for (_, value) in keywords {
                reject_generic_calls_in_expr(module_env, own_name, value)?;
            }
            Ok(())
        }
        // PEP 695 (#387): `C[type_arg](args)` — recurse into args only.
        // `class` is a bare name (not an expression), and `type_arg` is a
        // compile-time `Ty`, so neither needs generic-call rejection.
        // #1411: `type(self)(args)` walks its arguments the same way.
        HirExpr::GenericClassInstantiate { args, .. } | HirExpr::ReceiverClassCall { args } => {
            for arg in args {
                reject_generic_calls_in_expr(module_env, own_name, arg)?;
            }
            Ok(())
        }
        HirExpr::ExprCall { callee, args } => {
            reject_generic_calls_in_expr(module_env, own_name, callee)?;
            for arg in args {
                reject_generic_calls_in_expr(module_env, own_name, arg)?;
            }
            Ok(())
        }
        // PEP 572 (#774): `target := value` — recurse into `value` only,
        // mirroring `AttrGet`'s own single-sub-expression shape just above.
        HirExpr::NamedExpr { name: _, value } => {
            reject_generic_calls_in_expr(module_env, own_name, value)
        }
        // #1254: every sub-expression, as the statement form's arm does.
        HirExpr::Comprehension(comp) => {
            for sub in comp.sub_exprs() {
                reject_generic_calls_in_expr(module_env, own_name, sub)?;
            }
            Ok(())
        }
        HirExpr::IntLiteral(_)
        | HirExpr::FloatLiteral(_)
        | HirExpr::BoolLiteral(_)
        | HirExpr::StringLiteral(_)
        | HirExpr::EmptyList(_)
        | HirExpr::EmptyDict(_)
        | HirExpr::NoneLiteral
        | HirExpr::NotImplemented
        | HirExpr::Name(_)
        | HirExpr::Super => Ok(()),
        HirExpr::ListPop { list } => match list.attr_expr() {
            Some(receiver) => reject_generic_calls_in_expr(module_env, own_name, receiver),
            None => Ok(()),
        },
    }
}
