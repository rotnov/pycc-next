//! #1254 (Part 1 of #1214, D-250): comprehensions in expression position,
//! plus the solver's binding of a `name = <comp>` statement's target.
//!
//! Every test drives the real parser and HIR lowering through
//! `check_source`, so the checker, the private-helper solver and both
//! monomorphization passes see exactly the tree the compiler builds.

use super::*;

fn message_of(source: &str) -> String {
    check_source(source)
        .expect_err("fixture must be refused")
        .message
}

/// A module-scope unannotated private helper is what makes the solver run.
const PRIVATE_HELPER: &str = "def _twice(v):\n    return v * 2\n";

#[test]
fn a_statement_comprehension_target_is_readable_when_the_solver_runs() {
    for body in [
        "    ys = [v for v in range(k)]\n    return len(ys) + ys[k - 1]\n",
        "    ys = {v for v in range(k)}\n    return len(ys)\n",
        "    d = {\"a\": 1}\n    ys = {s: d[s] for s in d}\n    return ys[\"a\"]\n",
    ] {
        let source =
            format!("{PRIVATE_HELPER}def run(k: int) -> int:\n{body}print(run(4), _twice(3))\n");
        check_source(&source).unwrap_or_else(|d| panic!("{source}: {}", d.message));
    }
}

#[test]
fn a_statement_comprehension_target_rebinds_a_maybe_bound_name() {
    let source = format!(
        "{PRIVATE_HELPER}def run(k: int) -> int:\n    if k > 1:\n        ys = [1]\n    ys = [v for v in range(k)]\n    return len(ys)\nprint(run(4), _twice(3))\n"
    );
    check_source(&source).unwrap();
}

#[test]
fn an_expression_comprehension_infers_a_private_helper_parameter_through_its_element() {
    let source = format!(
        "{PRIVATE_HELPER}def total(xs: list[int]) -> int:\n    t = 0\n    for x in xs:\n        t += x\n    return t\nprint(total([_twice(i) for i in range(3) if i > 0]))\n"
    );
    check_source(&source).unwrap();
}

#[test]
fn an_expression_comprehension_loop_variable_is_not_bound_afterwards() {
    assert_eq!(
        message_of("def f() -> int:\n    n = len([i for i in range(3)])\n    return i\n"),
        "name `i` is not defined"
    );
}

#[test]
fn an_expression_comprehension_applies_the_display_element_gates() {
    for (source, expected) in [
        (
            "print(len([1.5 for i in range(3)]))\n",
            "list codegen only supports `list[int]` in v0.2, got a comprehension producing `list[float]`",
        ),
        (
            "print(len({1.5 for i in range(3)}))\n",
            "set comprehension codegen only supports `set[int]` or a set of a user-class instance (D-122, D-255), got a comprehension producing `set[float]`",
        ),
        (
            "print(len({i: i for i in range(3)}))\n",
            "dict codegen only supports `dict[str, int]` in v0.2, got a comprehension producing `dict[int, int]`",
        ),
    ] {
        assert_eq!(message_of(source), expected, "{source}");
    }
}

#[test]
fn an_expression_comprehension_refuses_a_non_container_iterable() {
    let message = message_of("n = 3\nprint(len([i for i in n]))\n");
    assert_eq!(
        message,
        "`int` cannot be iterated with `for ... in ...` (only list[T]/dict[K, V]/set[T]/frozenset[T] supports this)"
    );
}

#[test]
fn an_expression_comprehension_calling_a_generic_function_is_monomorphized() {
    let hir = check_source(
        "def identity[T](x: T) -> T:\n    return x\nprint(len([identity(i) for i in range(3)]))\n",
    )
    .unwrap();
    assert!(
        hir.items.iter().any(|item| matches!(
            item,
            HirItem::Function { name, .. } if name != "identity" && name.contains("identity")
        )),
        "a specialized `identity` must be emitted"
    );
}

#[test]
fn comprehensions_calling_a_protocol_function_are_monomorphized_in_both_forms() {
    let prelude = "from typing import Protocol\nclass Sized(Protocol):\n    def size(self) -> int: ...\nclass Box:\n    def __init__(self, v: int) -> None:\n        self.v = v\n    def size(self) -> int:\n        return self.v\ndef measure(s: Sized) -> int:\n    return s.size()\n";
    for tail in [
        "print(len([measure(Box(v)) for v in range(3)]))\n",
        "ys = [measure(Box(v)) for v in range(3)]\nprint(len(ys))\n",
        "zs = {measure(Box(v)) for v in range(3)}\nprint(len(zs))\n",
        "d = {\"a\": 1}\nws = {s: measure(Box(d[s])) for s in d}\nprint(len(ws))\n",
        "def run(k: int) -> int:\n    ys = [measure(Box(v)) for v in range(k) if measure(Box(v)) > 0]\n    return len(ys)\nprint(run(3))\n",
    ] {
        let source = format!("{prelude}{tail}");
        check_source(&source).unwrap_or_else(|d| panic!("{tail}: {}", d.message));
    }
}

#[test]
fn a_generic_function_body_checks_calls_inside_an_expression_comprehension() {
    assert_eq!(
        message_of(
            "def identity[T](x: T) -> T:\n    return x\ndef g[T](x: T) -> int:\n    return len([identity(i) for i in range(3)])\n"
        ),
        "generic function `g` calls generic function `identity` -- a generic function cannot call itself or another generic function (recursive generic instantiation is not supported yet)"
    );
    check_source(
        "def g[T](x: T) -> T:\n    n = len([i for i in range(3) if i > 0])\n    return x\nprint(g(4))\n",
    )
    .unwrap();
}

#[test]
fn the_solver_propagates_an_error_from_an_expression_comprehension_s_range() {
    assert_eq!(
        message_of(
            "def _f():\n    k = len([i for i in range(m)])\n    m = 3\n    return k\nprint(_f())\n"
        ),
        "local name `m` is not bound before this use"
    );
}

#[test]
fn an_if_test_holding_an_expression_comprehension_is_walked_for_walrus_targets() {
    check_source(
        "def f() -> int:\n    if len([i for i in range(3)]) > 1:\n        return 1\n    return 0\nprint(f())\n",
    )
    .unwrap();
}
