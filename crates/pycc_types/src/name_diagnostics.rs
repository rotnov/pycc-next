//! Diagnostics for names that are not values or not callable: unbound locals,
//! stdlib/typing markers used as values, and known-but-unsupported builtins.

use super::*;

pub(crate) fn unbound_local(name: &str) -> Diagnostic {
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
pub(crate) fn possibly_unbound(name: &str) -> Diagnostic {
    Diagnostic::error(
        "T0041",
        format!("local name `{name}` may not be bound on every path reaching this use"),
        Span::new(0, 0),
    )
}

pub(crate) fn non_callable_binding(name: &str) -> Diagnostic {
    Diagnostic::error(
        "T0021",
        format!("name `{name}` is bound to a non-callable value"),
        Span::new(0, 0),
    )
}

pub(crate) fn std_scalar_to_ty(kind: pycc_std::ScalarKind) -> Ty {
    match kind {
        pycc_std::ScalarKind::Float => Ty::Float,
        pycc_std::ScalarKind::Int => Ty::Int,
    }
}

/// A stdlib function symbol (e.g. `math.sqrt`) referenced without a call
/// (`print(math.sqrt)`, not `math.sqrt(x)`) has no callable `Ty` this
/// compiler's type system can express -- there is no first-class function
/// type here, matching `non_callable_binding`'s own "this compiler's
/// value types are all primitives" precedent (D-110's doc comment above).
pub(crate) fn std_function_used_as_a_value(name: &str) -> Diagnostic {
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
pub(crate) fn std_constant_is_not_callable(name: &str) -> Diagnostic {
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
pub(crate) fn enum_marker_is_not_a_value(name: &str) -> Diagnostic {
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
pub(crate) fn marker_is_not_a_value(name: &str) -> Diagnostic {
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
pub(crate) fn annotation_marker_is_not_a_value(name: &str) -> Diagnostic {
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
pub(crate) fn cast_marker_is_not_a_value(name: &str) -> Diagnostic {
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
pub(crate) fn type_checking_marker_is_not_a_value(name: &str) -> Diagnostic {
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
pub(crate) fn typing_form_is_not_a_value(name: &str) -> Diagnostic {
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
pub(crate) fn is_marker_kind(kind: pycc_std::StdSymbolKind) -> bool {
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
pub(crate) const KNOWN_CALLABLE_BUILTINS: &[&str] = &[
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
pub(crate) fn is_known_callable_builtin(name: &str) -> bool {
    KNOWN_CALLABLE_BUILTINS.binary_search(&name).is_ok()
}

/// Issue #142: the `C0001` diagnostic for a call to a known but unsupported
/// callable builtin (e.g. `ValueError("x")`). Distinct from `T0021`'s "call
/// to undefined function" -- the builtin genuinely exists in Python 3.14,
/// this compiler just does not implement it yet.
pub(crate) fn unsupported_callable_builtin(name: &str) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!("call to builtin `{name}` is valid Python but not implemented yet"),
        Span::new(0, 0),
    )
}
