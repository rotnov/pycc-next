//! Part 2a of #1371: checking a call of a subscript result
//! (`HirExpr::ExprCall`, `table[k](args)`), admitted only on a CPython
//! object.
//!
//! Kept apart from `foreign/tests.rs` (see #1314); the fixture helpers are
//! copied from `call_tests.rs` because a sibling module cannot reach them.

/// Lowers `source` with every `import` request answered `Foreign`.
fn lower_all_foreign(source: &str) -> pycc_hir::HirModule {
    let module = pycc_parser::parse(source).expect("test source must parse");
    let mut resolved = pycc_hir::ResolvedImports::default();
    for request in pycc_hir::project_import_requests(&module) {
        resolved.insert(request.span, pycc_hir::ResolvedImport::Foreign);
    }
    pycc_hir::lower_module(&module, &resolved, None)
        .unwrap_or_else(|diagnostics| panic!("{source:?} must lower: {diagnostics:#?}"))
        .hir
}

fn check(source: &str) -> Result<(), Vec<pycc_diag::Diagnostic>> {
    crate::check_all(&lower_all_foreign(source)).map(|_| ())
}

const IMPORT: &str = "from itertools import product\n";

fn admitted(snippet: &str) {
    let source = format!("{IMPORT}{snippet}");
    check(&source).unwrap_or_else(|diagnostics| panic!("{source:?}: {diagnostics:#?}"));
}

/// The single diagnostic `snippet` is refused with, by code and phrase.
fn refused(snippet: &str, code: &str, phrase: &str) {
    let source = format!("{IMPORT}{snippet}");
    let diagnostics = check(&source).expect_err(&source);
    assert_eq!(diagnostics.len(), 1, "{source:?}: {diagnostics:#?}");
    assert_eq!(diagnostics[0].code, code, "{source:?}: {diagnostics:#?}");
    assert!(
        diagnostics[0].message.contains(phrase),
        "{source:?}: {diagnostics:#?}"
    );
}

/// Scalar and object keys and arguments, in a module body, an annotated
/// function and an unannotated private helper (the constraint solver's
/// path), with the `object` result feeding a consumer that takes one.
#[test]
fn a_call_of_an_object_subscript_is_admitted_in_every_body() {
    admitted("product['a'](1, 2.5, True, 'ab')\n");
    admitted("product[product](product)\n");
    admitted("s = str(product[1]('x'))\n");
    admitted("product['a']()\n");
    admitted("def f(k: str) -> None:\n    print(str(product[k](product)))\n");
    admitted("def _h(k):\n    return product[k](1)\n\n\nx = _h('a')\nprint(x)\n");
    admitted("x = product['a']['b'](1)\n");
}

/// A callee that is not a CPython object keeps the `C0001` HIR reported for
/// this shape before Part 2a.
#[test]
fn a_subscript_callee_that_is_not_an_object_is_refused() {
    refused(
        "t = {'a': 1}\nt['a'](1)\n",
        "C0001",
        "calling a subscript expression is not supported yet",
    );
    refused(
        "def f(t: list[int]) -> None:\n    t[0](1)\n",
        "C0001",
        "or a call of a CPython object is",
    );
}

/// An argument outside the packable operands is refused with the shared
/// object-call rule's `I0404`.
#[test]
fn a_non_packable_argument_is_refused() {
    refused(
        "product['a']([1])\n",
        "I0404",
        "passing a `list[int]` argument to a CPython object's call",
    );
    // Part 8 of #1371 admits a `None` argument (`keyword_call_tests.rs`).
}

/// The callee is checked before the arguments (CPython's evaluation order):
/// a bad key reports its own diagnostic even when an argument is also bad.
#[test]
fn the_callee_is_checked_before_the_arguments() {
    refused(
        "product[[1]]([1])\n",
        "I0404",
        "indexing a CPython object with a `list[int]` key",
    );
}

/// A walrus in the key or an argument binds its name, in a module body and
/// in a function (`collect_named_expr_bindings` and the solver's
/// `bind_named_expr_targets` both walk the new node).
#[test]
fn a_walrus_in_the_callee_or_an_argument_binds() {
    admitted("product[(k := 1)]((y := 2))\nprint(k)\nprint(y)\n");
    admitted("def f() -> None:\n    product[(k := 1)]((y := 2))\n    print(k)\n    print(y)\n");
    admitted("def _h():\n    product[(k := 1)]((y := 2))\n    return y\n\n\nz = _h()\n");
}

/// A generic function call inside the callee or an argument is
/// monomorphized through the new node, a generic body's generic call there
/// is still found by the recursive-instantiation check, and a generic class
/// instantiation inside an argument is collected (and, since #1435, admitted
/// as an argument: it crosses as its class's carrier).
#[test]
fn generic_calls_inside_the_callee_and_arguments_are_rewritten() {
    admitted("def ident[T](x: T) -> T:\n    return x\n\n\nproduct[ident('a')](ident(1))\n");
    refused(
        "def ident[T](x: T) -> T:\n    return x\n\n\ndef g[T](x: T) -> T:\n    \
         product[ident('a')](1)\n    return x\n\n\ng(1)\n",
        "T0042",
        "generic function `g` calls generic function `ident`",
    );
    admitted(
        "class C[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\n\n\n\
         product['a'](C[int](1))\n",
    );
}

/// A call of a subscript result inside a method of a class that another
/// class inherits from is walked by the inherited-copies facts pass.
#[test]
fn a_call_in_an_inherited_method_is_admitted() {
    admitted(
        "class A:\n    def m(self, k: str) -> None:\n        print(str(product[k](1)))\n\n\n\
         class B(A):\n    pass\n\n\nB().m('a')\n",
    );
}

/// `snippet` through the full check-and-resolve pipeline -- the one that
/// also runs the post-check passes (generic-call rejection in a generic
/// body, monomorphization, protocol specialization) that [`check`] stops
/// short of.
fn resolve(snippet: &str) -> Result<pycc_hir::HirModule, pycc_diag::Diagnostic> {
    crate::check_and_resolve(&lower_all_foreign(&format!("{IMPORT}{snippet}")))
}

const POST_CHECK_FIXTURE: &str = "from typing import Protocol\n\n\n\
    class P(Protocol):\n    def m(self) -> int: ...\n\n\n\
    class C:\n    def __init__(self) -> None:\n        self.n = 0\n\n    \
    def m(self) -> int:\n        return 1\n\n\n\
    class Box[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\n\n    \
    def size(self) -> int:\n        return 1\n\n\n\
    def ident[T](x: T) -> T:\n    return x\n\n\n\
    def use(p: P) -> int:\n    return p.m()\n\n\n";

/// The post-check passes walk a call of a subscript result: a generic call
/// in the callee and in an argument is monomorphized, a generic class
/// instantiated inside an argument is collected, and a protocol-typed call
/// in an argument is specialized -- the resolved module keeps no call of
/// the unspecialized `use` or `ident`.
#[test]
fn the_post_check_passes_reach_the_callee_and_the_arguments() {
    let resolved = resolve(&format!(
        "{POST_CHECK_FIXTURE}def h(t: product) -> None:\n    \
         print(str(t[ident('a')](ident(1), use(C()), Box[int](2).size())))\n"
    ))
    .unwrap_or_else(|diagnostic| panic!("{diagnostic:#?}"));
    let debug = format!("{resolved:?}");
    assert!(!debug.contains("callee: \"use\""), "{debug}");
    assert!(!debug.contains("callee: \"ident\""), "{debug}");
}

/// A generic function's body is walked for generic calls through the new
/// node before the body is checked: a call of a subscript result with no
/// generic call in it passes the walk and the body check, and one whose
/// argument calls a generic function is refused by the walk itself.
#[test]
fn a_generic_body_is_walked_through_a_call_of_a_subscript_result() {
    resolve(&format!(
        "{POST_CHECK_FIXTURE}def g[T](x: T, t: product) -> T:\n    t['a'](1)\n    return x\n"
    ))
    .unwrap_or_else(|diagnostic| panic!("{diagnostic:#?}"));
    let diagnostic = resolve(&format!(
        "{POST_CHECK_FIXTURE}def g[T](x: T) -> T:\n    product['a'](ident(1))\n    return x\n\n\ng(1)\n"
    ))
    .expect_err("a generic call in a generic body");
    assert_eq!(diagnostic.code, "T0042", "{diagnostic:#?}");
    assert!(
        diagnostic
            .message
            .contains("generic function `g` calls generic function `ident`"),
        "{diagnostic:#?}"
    );
}
