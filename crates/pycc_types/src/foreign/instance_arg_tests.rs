//! #1435: an instance of a regular user class as a positional argument of
//! a call on a CPython object -- a method call, a direct call and a call of
//! a subscript result -- and the positions and classes that stay refused.
//! Since #1470 it also pins the same instances as a subscript key and a
//! rich-comparison operand, and the enum member, exception instance and
//! class method's `cls` those positions still refuse.
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

const PRELUDE: &str = "from itertools import product\n\n\nclass Q:\n    \
                       def __init__(self, n: int) -> None:\n        self.n = n\n\n\n";

fn admitted(snippet: &str) {
    let source = format!("{PRELUDE}{snippet}");
    check(&source).unwrap_or_else(|diagnostics| panic!("{source:?}: {diagnostics:#?}"));
}

/// The single diagnostic `snippet` is refused with, by code and phrase.
fn refused(snippet: &str, code: &str, phrase: &str) {
    let source = format!("{PRELUDE}{snippet}");
    let diagnostics = check(&source).expect_err(&source);
    assert_eq!(diagnostics.len(), 1, "{source:?}: {diagnostics:#?}");
    assert_eq!(diagnostics[0].code, code, "{source:?}: {diagnostics:#?}");
    assert!(
        diagnostics[0].message.contains(phrase),
        "{source:?}: {diagnostics:#?}"
    );
}

/// All three object-call shapes take an instance, in a module body, a
/// function and a method passing its own `self`, beside the scalar and
/// object arguments they already took.
#[test]
fn an_instance_argument_is_admitted_in_every_call_shape() {
    admitted("product(Q(1))\n");
    admitted("product.attr(Q(1), 2, product)\n");
    admitted("product['k'](Q(1))\n");
    admitted("def f(q: Q) -> None:\n    print(str(product(q)))\n");
    admitted(
        "class P:\n    def __init__(self, c: Q) -> None:\n        self.c = c\n\n    \
         def go(self) -> None:\n        print(str(product(self, self.c)))\n",
    );
}

/// An unannotated private helper's parameter is inferred through the
/// constraint solver, which defers the argument check to this checker.
#[test]
fn an_instance_through_an_unannotated_helper_is_admitted() {
    admitted("def _h(n):\n    return product(Q(n))\n\n\nx = _h(1)\nprint(x)\n");
}

/// A value typed by a `Protocol` has no single run-time class the checker
/// can name, so it stays refused.
#[test]
fn a_protocol_typed_argument_is_refused() {
    refused(
        "from typing import Protocol\n\n\nclass H(Protocol):\n    def size(self) -> int: ...\n\n\n\
         def g(h: H) -> None:\n    product(h)\n",
        "I0404",
        "passing a `H` argument to a CPython object's call",
    );
}

/// A subclass instance and a generic class instantiation are regular
/// classes too.
#[test]
fn subclass_and_generic_instances_are_admitted() {
    admitted("class R(Q):\n    pass\n\n\nproduct(R(1))\n");
    admitted(
        "class C[T]:\n    def __init__(self, x: T) -> None:\n        self.x = x\n\n\nproduct(C[int](1))\n",
    );
}

/// An enum member and an exception instance have no carrier (module doc of
/// `foreign.rs`'s `is_carriable_instance`), so they stay refused.
#[test]
fn enum_and_exception_instances_are_refused() {
    refused(
        "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n\n\nproduct(Color.RED)\n",
        "I0404",
        "passing a `Color` argument to a CPython object's call",
    );
    refused(
        "class E(Exception):\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n\n\
         product(E(1))\n",
        "I0404",
        "passing a `E` argument to a CPython object's call",
    );
}

/// #1470: an instance is a subscript key and a rich-comparison operand on
/// either side of a CPython object, in a module body, a function and a
/// method passing its own `self`. (The list-display element is pinned in
/// `foreign/list_display/tests.rs`, whose display needs an `--ext`
/// annotation.)
#[test]
fn an_instance_key_and_comparison_operand_are_admitted() {
    admitted("x = product[Q(1)]\n");
    admitted("x = product == Q(1)\n");
    admitted("x = Q(1) != product\n");
    admitted("x = product < Q(1)\n");
    admitted("def f(q: Q) -> None:\n    print(product[q], q == product, product >= q)\n");
    admitted(
        "class P:\n    def go(self) -> None:\n        print(product[self], self == product)\n",
    );
    admitted("class R(Q):\n    pass\n\n\nx = product[R(1)] == R(2)\n");
}

/// The key and comparison positions keep every refusal the call argument
/// keeps: an enum member, an exception instance and a class method's `cls`.
#[test]
fn a_refused_instance_key_or_comparison_operand_is_refused() {
    const COLOR: &str = "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n\n\n";
    const EXC: &str =
        "class E(Exception):\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n\n";
    refused(
        &format!("{COLOR}x = product[Color.RED]\n"),
        "I0404",
        "indexing a CPython object with a `Color` key",
    );
    refused(
        &format!("{EXC}x = product[E(1)]\n"),
        "I0404",
        "indexing a CPython object with a `E` key",
    );
    refused(
        &format!("{COLOR}x = Color.RED == product\n"),
        "I0404",
        "comparing a CPython object with a `Color` value",
    );
    refused(
        &format!("{EXC}x = product == E(1)\n"),
        "I0404",
        "comparing a CPython object with a `E` value",
    );
    for (expr, phrase) in [
        (
            "product[cls]",
            "indexing a CPython object with a class method's `cls`",
        ),
        (
            "product == cls",
            "comparing a CPython object with a class method's `cls`",
        ),
        (
            "cls != product",
            "comparing a CPython object with a class method's `cls`",
        ),
    ] {
        refused(
            &format!(
                "class K:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
                 @classmethod\n    def make(cls) -> None:\n        print({expr})\n"
            ),
            "I0404",
            phrase,
        );
    }
    admitted("def g(cls: Q) -> None:\n    print(product[cls], product == cls)\n");
}

/// A class method's `cls` is a null instance (`Environment::in_classmethod`)
/// and is refused by name in every call shape; an ordinary parameter that
/// happens to be named `cls` is not.
#[test]
fn a_class_methods_cls_is_refused() {
    for call in ["product(cls)", "product.m(1, cls)", "product['k'](cls)"] {
        refused(
            &format!(
                "class K:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
                 @classmethod\n    def make(cls) -> None:\n        {call}\n"
            ),
            "I0404",
            "passing a class method's `cls` argument to a CPython object's",
        );
    }
    admitted("def g(cls: Q) -> None:\n    product(cls)\n");
}

/// The refusal message lists the instance argument among the supported
/// operations.
#[test]
fn the_unsupported_message_names_instance_arguments() {
    refused(
        "product([1])\n",
        "I0404",
        "scalar-, `None`-, object- or class-instance-argument method calls",
    );
}

/// An instance is a keyword value too (Part 8 of #1371's keyword calls), in
/// all three call shapes and beside positional arguments -- lark's
/// `UnexpectedToken(token, expected, state=self, ...)`.
#[test]
fn an_instance_keyword_value_is_admitted_in_every_call_shape() {
    admitted("product(k=Q(1))\n");
    admitted("product.attr(1, k=Q(1), j=None)\n");
    admitted("product['k'](Q(1), state=Q(2))\n");
    admitted(
        "class P:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
         def go(self) -> None:\n        print(str(product(1, state=self)))\n",
    );
}

/// A keyword value keeps the positional refusals: an enum member, and a
/// class method's `cls` named in the message.
#[test]
fn a_refused_keyword_value_is_refused() {
    refused(
        "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n\n\nproduct(k=Color.RED)\n",
        "I0404",
        "passing a `Color` argument to a CPython object's call",
    );
    for call in [
        "product(k=cls)",
        "product.m(1, k=cls)",
        "product['k'](k=cls)",
    ] {
        refused(
            &format!(
                "class K:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n    \
                 @classmethod\n    def make(cls) -> None:\n        {call}\n"
            ),
            "I0404",
            "passing a class method's `cls` argument to a CPython object's",
        );
    }
}
