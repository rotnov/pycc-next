//! `Ty` naming and size, and capability-error propagation.

use super::*;

#[test]
fn ty_name_returns_the_python_spelling_of_every_scalar_variant() {
    // The four recursive container variants (List/Dict/Set/Tuple) are
    // covered separately by `ty_name_describes_nested_container_types`
    // below, since their expected `.name()` output depends on a nested
    // `Ty` argument rather than being a fixed string per variant.
    assert_eq!(Ty::Int.name(), "int");
    assert_eq!(Ty::Float.name(), "float");
    assert_eq!(Ty::Bool.name(), "bool");
    assert_eq!(Ty::Str.name(), "str");
    assert_eq!(Ty::None.name(), "None");
    assert_eq!(Ty::Infer.name(), "<inferred>");
    assert_eq!(Ty::Param(Box::new("T".to_string())).name(), "T");
    assert_eq!(
        Ty::Instance(Box::new("MyClass".to_string())).name(),
        "MyClass"
    );
    // #380 (PR-20): `Ty::Protocol` name — covered here as a unit test
    // to avoid cargo-llvm-cov issue #276 (instantiation merging).
    assert_eq!(
        Ty::Protocol(Box::new("MyProto".to_string())).name(),
        "MyProto"
    );
    // Part 1 of #1026: `Ty::Object` is the foreign-object type, which no
    // annotation can spell and every operation on which is refused, so no
    // source program renders it -- covering its arm needs a unit test. The
    // spelling still matters: it is what a future diagnostic that reports a
    // foreign value's type would print.
    assert_eq!(Ty::Object.name(), "object");
}

#[test]
fn lowers_a_protocol_typed_parameter_annotation() {
    // #380 (PR-20): exercises `annotation_to_ty`'s `Ty::Protocol`
    // return path (line 1487) — a bare name matching a known protocol
    // class resolves to `Ty::Protocol`, not `Ty::Instance`. Covered
    // here as a unit test to avoid cargo-llvm-cov issue #276.
    let module = pycc_parser_test_helper::parse(
        "from typing import Protocol\n\
             class P(Protocol):\n    def f(self) -> int: ...\n\
             def _marker() -> None:\n    pass\n\
             def g(x: P) -> None:\n    pass\n",
    );
    let hir = lower_checked(&module).unwrap();
    // The function `g` should have a parameter typed as `Ty::Protocol`.
    // A `_marker` function precedes `g` so that the `None` arm of the
    // match is also exercised (protocol methods do not produce HIR
    // items, so without `_marker` the `None` arm would be unreachable).
    let func = hir.items.iter().find_map(|item| match item {
        HirItem::Function { name, params, .. } if name == "g" => Some(params),
        _ => None,
    });
    let params = func.expect("function `g` should exist");
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].0, "x");
    assert_eq!(params[0].1, Ty::Protocol(Box::new("P".to_string())));
}

#[test]
fn ty_list_variant_is_structurally_comparable_and_not_copy() {
    let a = Ty::List(Box::new(Ty::Int));
    let b = Ty::List(Box::new(Ty::Int));
    let c = Ty::List(Box::new(Ty::Str));
    assert_eq!(a, b);
    assert_ne!(a, c);
    // `Ty` is `Clone` but deliberately not `Copy` (a `Box`/`Vec`-holding
    // enum can't implement `Copy`), so producing a second owned value
    // from `a` requires this explicit `.clone()` -- unlike the old flat
    // scalar `Ty`, where an implicit copy would have made `.clone()`
    // merely redundant rather than required. (`.clone()` alone can't
    // prove Copy's absence at compile time -- it also compiles, just
    // redundantly, for a `Copy` type -- so this comment documents the
    // property rather than the assertions below enforcing it.)
    let d = a.clone();
    assert_eq!(a, d);
}

#[test]
fn ty_shrinks_after_boxing_dict_and_tuple_d109() {
    // D-109: before this task, size_of::<Ty>() measured 24 bytes (Vec<Ty>'s
    // ptr+len+cap dominates). This is a real regression guard, not a vibe --
    // it must stay strictly smaller than 24 forever, catching any future
    // change that re-inflates Ty back to its pre-fix size. This test's
    // own numeric assertion documents PR-11's variant set specifically
    // (PR-11 itself added no new Ty variants, only the Dict/Tuple boxing
    // fix) -- every later PR that adds a new dataful variant (D-133's
    // `Ty::Param`, D-154's `Ty::Instance`) is covered instead by the
    // more general `ty_size_stays_within_d109_ceiling` test below, which
    // tracks the ceiling for the *current* variant set rather than
    // pinning it to one historical PR's shape.
    assert_eq!(
        std::mem::size_of::<Ty>(),
        16,
        "size_of::<Ty>() must stay 16 bytes (PR-10 Task 14, D-109) -- if it \
             moves, something accidentally widened Ty's boxing, not the containers \
             themselves",
    );
}

#[test]
fn ty_size_stays_within_d109_ceiling() {
    // Part 1 of #1319 added `Ty::FrozenSet(Box<Ty>)`, the same thin
    // pointer shape as `Ty::Set`; constructing it here keeps the variant
    // in the measured set.
    let frozen = Ty::FrozenSet(Box::new(Ty::Int));
    assert_eq!(frozen.name(), "frozenset[int]");
    // D-133 added `Ty::Param(Box<String>)`; D-154 added
    // `Ty::Instance(Box<String>)`. Both are a single (thin, 8-byte)
    // pointer -- unlike `Box<str>`, which measured 24 bytes here because
    // `str` is unsized (see each variant's own doc comment) -- so
    // neither pushes `size_of::<Ty>()` back past the 16-byte ceiling
    // D-109 established.
    assert!(
        std::mem::size_of::<Ty>() <= 16,
        "size_of::<Ty>() must stay within the D-109 16-byte ceiling; adding \
             Ty::Param(Box<String>) (D-133) and Ty::Instance(Box<String>) (D-154) \
             must not regress it, got {}",
        std::mem::size_of::<Ty>(),
    );
}

#[test]
fn ty_name_describes_nested_container_types() {
    assert_eq!(Ty::Int.name(), "int");
    assert_eq!(Ty::List(Box::new(Ty::Int)).name(), "list[int]");
    assert_eq!(
        Ty::Dict(Box::new((Ty::Str, Ty::Float))).name(),
        "dict[str, float]"
    );
    assert_eq!(Ty::Set(Box::new(Ty::Bool)).name(), "set[bool]");
    assert_eq!(
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Str])).name(),
        "tuple[int, str]"
    );
}

#[test]
fn ty_optional_name_is_inner_pipe_none() {
    // `Ty::Optional`'s own `.name()` (D-197, #763, Part 1 of #747) spells
    // itself the same way PEP 604 source does (`int | None`), not the
    // `typing.Optional[int]` form, matching this compiler's own
    // parse-side entry point.
    assert_eq!(Ty::Optional(Box::new(Ty::Int)).name(), "int | None");
}

#[test]
fn ty_instance_name_is_the_bare_class_name() {
    // Unlike every other dataful variant, `Ty::Instance`'s `.name()`
    // is not wrapped in a `<kind>[...]` shape -- a class instance's
    // type is spelled exactly like the class itself in real Python
    // (`Point`, not `Instance[Point]`).
    assert_eq!(Ty::Instance(Box::new("Point".to_string())).name(), "Point");
}

#[test]
fn unsupported_statement_and_expression_return_spanned_capability_diagnostics() {
    // #435: `pass` is now supported, so use `with` — a valid Python
    // statement that is still unsupported — to exercise the C0001
    // capability error path for statements.
    assert_capability_error(
        "with open(\"x\") as f:\n    pass\n",
        "statement kind not supported yet",
        Span::new(0, 29),
    );
    assert_capability_error(
        "x = lambda: 1\n",
        "expression kind not supported yet",
        Span::new(4, 13),
    );
}

#[test]
fn capability_errors_propagate_through_every_supported_container() {
    // `(1, 2)` was this table's "genuinely unhandled at every level"
    // poison fixture -- a list literal used to fill this role (see
    // `a_tuple_literal_expression_lowers_successfully`'s own comment)
    // until Task 7 (D-105) added list-literal lowering, and a tuple
    // literal took over in turn until this task (PR-11b Task 2, D-116)
    // added tuple-literal lowering too. `lambda: 1` (parenthesized
    // throughout, purely to dodge grammar ambiguity with the
    // surrounding syntax -- not because any position here actually
    // requires it) takes over now, since `lower_expr` still has no
    // `Expr::Lambda` arm.
    let cases = [
        // #435: `pass` is now supported (filtered as a no-op in
        // `lower_body`), so use `with open("x") as f: pass` — a valid
        // Python statement that is still unsupported — to exercise the
        // C0001 capability error path in statement positions.
        (
            "function body",
            "def _f():\n    with open(\"x\") as f:\n        pass\n",
        ),
        ("if test", "if (lambda: 1):\n    print(1)\n"),
        (
            "if else body",
            "if True:\n    print(1)\nelse:\n    with open(\"x\") as f:\n        pass\n",
        ),
        ("while test", "while (lambda: 1):\n    print(1)\n"),
        (
            "while body",
            "while True:\n    with open(\"x\") as f:\n        pass\n",
        ),
        (
            "one-argument range stop",
            "for i in range((lambda: 1)):\n    print(i)\n",
        ),
        (
            "two-argument range start",
            "for i in range((lambda: 1), 1):\n    print(i)\n",
        ),
        (
            "two-argument range stop",
            "for i in range(0, (lambda: 1)):\n    print(i)\n",
        ),
        (
            "three-argument range start",
            "for i in range((lambda: 1), 1, 1):\n    print(i)\n",
        ),
        (
            "three-argument range stop",
            "for i in range(0, (lambda: 1), 1):\n    print(i)\n",
        ),
        (
            "three-argument range step",
            "for i in range(0, 1, (lambda: 1)):\n    print(i)\n",
        ),
        (
            "for body",
            "for i in range(1):\n    with open(\"x\") as f:\n        pass\n",
        ),
        ("return value", "def _f():\n    return (lambda: 1)\n"),
        (
            "elif test",
            "if True:\n    print(1)\nelif (lambda: 1):\n    print(2)\n",
        ),
        (
            "elif body",
            "if True:\n    print(1)\nelif True:\n    with open(\"x\") as f:\n        pass\n",
        ),
        (
            "nested else body",
            "if True:\n    print(1)\nelif True:\n    print(2)\nelse:\n    with open(\"x\") as f:\n        pass\n",
        ),
        ("binary left operand", "x = (lambda: 1) + 1\n"),
        ("binary right operand", "x = 1 + (lambda: 1)\n"),
        ("f-string interpolation", "x = f\"{(lambda: 1)}\"\n"),
        ("comparison left operand", "x = (lambda: 1) == 1\n"),
        ("comparison right operand", "x = 1 == (lambda: 1)\n"),
    ];

    for (container, source) in cases {
        let module = pycc_parser_test_helper::parse(source);
        let diagnostic = lower_checked(&module).unwrap_err();
        assert_eq!(diagnostic.code, "C0001", "wrong diagnostic for {container}");
        assert!(
            diagnostic
                .message
                .contains("expression kind not supported yet")
                || diagnostic
                    .message
                    .contains("statement kind not supported yet"),
            "wrong message for {container}: {}",
            diagnostic.message
        );
        assert!(diagnostic.span.is_some(), "missing span for {container}");
    }
}
