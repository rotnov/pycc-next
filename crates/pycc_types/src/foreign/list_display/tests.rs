//! Typing of a list display bound to a CPython object slot (Part 2d of
//! #1371): the empty-container pre-pass's rewrite and the checker's
//! element rule, both from source.

use pycc_diag::Span;
use pycc_hir::{ImportBinding, ResolvedImports};

/// Checks `source` lowered as an `ext` module (`ext == true`, where
/// `object` and `Any` are admitted annotations) or a `native` one, with
/// `numpy` bound as a foreign import.
fn check(source: &str, ext: bool) -> Result<(), Vec<pycc_diag::Diagnostic>> {
    let module = pycc_parser::parse(source).expect("test source must parse");
    let mut resolved = ResolvedImports::default();
    resolved.set_ext_module(ext);
    let lowered = pycc_hir::lower_module(&module, &resolved, None).expect("test source must lower");
    let mut hir = pycc_hir::finalize(lowered.hir).expect("test source must finalize");
    hir.imports.push(ImportBinding::Foreign {
        local_name: "numpy".to_string(),
        module_path: "numpy".to_string(),
        from: None,
        site: pycc_hir::ForeignImportSite::Item(0),
        span: Span::new(0, 0),
    });
    crate::check_all(&hir).map(|_| ())
}

fn assert_accepted(source: &str, ext: bool) {
    if let Err(diagnostics) = check(source, ext) {
        panic!("must type-check: {source}\n{diagnostics:?}");
    }
}

fn assert_refused(source: &str, code: &str, phrase: &str) {
    let diagnostics = check(source, true).expect_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?} for {source}");
    assert_eq!(diagnostics[0].code, code, "{diagnostics:?} for {source}");
    assert!(
        diagnostics[0].message.contains(phrase),
        "{diagnostics:?} for {source}"
    );
}

/// The annotation source: every annotation that lowers to an object in an
/// `ext` module admits an empty or a non-empty display of packable
/// elements, mixed types included, and the result is an object -- passed
/// to a CPython callable, returned as one, and measured by `len`.
#[test]
fn a_display_annotated_as_an_object_is_an_object() {
    for source in [
        "def f() -> object:\n    x: object = []\n    return x\n",
        "def f(n: int) -> object:\n    x: object = [n, 'a', 2.5, True, numpy.pi]\n    return x\n",
        "from typing import Any\n\n\ndef f() -> int:\n    x: Any = [1, 2]\n    return len(x)\n",
        "def f() -> object:\n    x: list = [1]\n    return numpy.pi(x)\n",
        // In a nested block: the fast path sees a non-empty annotated
        // display at any depth.
        "def f(n: int) -> object:\n    if n:\n        x: object = [n]\n        return x\n    return numpy.pi\n",
    ] {
        assert_accepted(source, true);
    }
}

/// The binding source: lark `lalr_parser_state.py` lines 94-101, where
/// `s` is an object slice on one branch and `[]` on the other, then
/// handed to a CPython callable. It needs no `object` annotation, so it
/// type-checks in a native module too.
#[test]
fn an_empty_display_bound_to_an_object_name_is_an_object() {
    let lark = "def f(size: int, callbacks: object) -> object:\n    \
        if size:\n        s = numpy.pi[-size:]\n    else:\n        s = []\n    \
        value = callbacks(s)\n    return value\n";
    assert_accepted(lark, true);
    let native = "def f(size: int) -> int:\n    s = numpy.pi[1:]\n    \
        if size:\n        s = []\n    return len(s)\n";
    assert_accepted(native, false);
    // The binding may follow the empty display: the flat environment is
    // order-insensitive, and the checker re-validates in program order.
    assert_accepted(
        "def f() -> int:\n    s = []\n    s = numpy.pi[1:]\n    return len(s)\n",
        false,
    );
}

/// The display's elements are walked by every pass that runs after the
/// rewrite: the private-helper constraint solver (an unannotated helper),
/// generic monomorphization (a generic call inside an element) and
/// protocol monomorphization (a call taking a protocol argument inside an
/// element). A pass that skipped the node would leave the call
/// un-rewritten and fail later.
#[test]
fn every_later_pass_walks_the_display_elements() {
    assert_accepted(
        "def _h():\n    x: object = [1, 'a']\n    return len(x)\n\n\nn = _h()\n",
        true,
    );
    assert_accepted(
        "def ident[T](v: T) -> T:\n    return v\n\n\n\
         def f() -> object:\n    x: object = [ident(1), ident('a')]\n    return x\n",
        true,
    );
    assert_accepted(
        "from typing import Protocol\n\n\n\
         class P(Protocol):\n    def m(self) -> int: ...\n\n\n\
         class A:\n    def m(self) -> int:\n        return 1\n\n\n\
         def use(p: P) -> int:\n    return p.m()\n\n\n\
         def f() -> object:\n    x: object = [use(A())]\n    return x\n",
        true,
    );
}

/// An element with no boxing helper is refused, naming the first one in
/// source order.
#[test]
fn an_unpackable_element_is_refused() {
    for (source, ty) in [
        (
            "def f() -> object:\n    x: object = [[1]]\n    return x\n",
            "list[int]",
        ),
        (
            "def f() -> object:\n    x: object = [1, None]\n    return x\n",
            "None",
        ),
        (
            "class A:\n    pass\n\n\ndef f() -> object:\n    x: object = [A(), [1]]\n    return x\n",
            "list[int]",
        ),
        (
            "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n\n\n\
             def f() -> object:\n    x: object = [1, Color.RED]\n    return x\n",
            "Color",
        ),
        (
            "class E(Exception):\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n\n\
             def f() -> object:\n    x: object = [E(1)]\n    return x\n",
            "E",
        ),
    ] {
        assert_refused(
            source,
            "I0404",
            &format!("a `{ty}` element in a list display bound to a CPython object"),
        );
    }
}

/// #1470: an instance of a regular user class is an element, crossing as
/// its carrier, beside scalars and objects and in a method's own `self`;
/// a class method's `cls` holds no instance and stays refused.
#[test]
fn an_instance_element_is_admitted_and_cls_is_refused() {
    assert_accepted(
        "class A:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n\n\
         class B(A):\n    pass\n\n\n\
         def f() -> object:\n    x: object = [A(1), 2, B(3), numpy.pi]\n    return x\n",
        true,
    );
    assert_accepted(
        "class A:\n    def go(self) -> object:\n        x: object = [self]\n        return x\n",
        true,
    );
    assert_refused(
        "class A:\n    @classmethod\n    def make(cls) -> object:\n        \
         x: object = [1, cls]\n        return x\n",
        "I0404",
        "a class method's `cls` as an element of a list display bound to a CPython object",
    );
}

/// What Part 2d leaves alone keeps its diagnostic: a dict display bound to
/// an object, an empty display with no object evidence, and a non-empty
/// display whose only object evidence is another binding of the name.
#[test]
fn the_shapes_outside_part_2d_keep_their_diagnostics() {
    assert_refused(
        "def f() -> object:\n    x: object = {}\n    return x\n",
        "T0003",
        "an empty dict literal",
    );
    assert_refused(
        "def f() -> object:\n    s = []\n    return s\n",
        "T0003",
        "an empty list literal",
    );
    assert_refused(
        "def f() -> int:\n    s = numpy.pi[1:]\n    s = [1]\n    return len(s)\n",
        "T0023",
        "cannot assign",
    );
}

/// #1445: the object evidence may depend on a name bound inside a `try`
/// suite. Lark `lalr_parser_state.py` binds `action, arg` inside a
/// `try`/`except KeyError` (line 75) and derives the slice bound from it
/// (`size = len(rule.expansion)`, line 91), so `s = value_stack[-size:]`
/// (line 93) only types once the flat binder walks the `try`. Before, `s`
/// was never bound to the object and line 97's `s = []` was `T0003`. The
/// `try`/`except*` spelling takes the same walk.
#[test]
fn an_empty_display_whose_object_evidence_needs_a_try_suite_is_an_object() {
    for handler in ["except KeyError", "except* KeyError"] {
        let source = format!(
            "def f(value_stack: object, states: object) -> object:\n    \
             try:\n        arg = states[0]\n    {handler}:\n        \
             raise ValueError('no rule')\n    \
             size = len(arg)\n    \
             if size:\n        s = value_stack[-size:]\n    else:\n        s = []\n    \
             return s\n"
        );
        assert_accepted(&source, true);
    }
}

/// #1445's native face: the same binder feeds source 2 of the native
/// resolution, so a list bound inside a `try` suite now types an empty
/// display of the same name in a handler, where it was `T0003`.
#[test]
fn a_native_empty_display_typed_from_a_try_bound_list_resolves() {
    assert_accepted(
        "def f(n: int) -> int:\n    try:\n        xs = [n]\n    \
         except ValueError:\n        xs = []\n    return len(xs)\n",
        false,
    );
}
