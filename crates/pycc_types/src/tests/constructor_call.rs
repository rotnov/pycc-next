//! #1342: an unannotated private helper whose return is a constructor call
//! `C(...)` on a non-generic user class infers `C`; the check phase stays the
//! authority on the call's validity, and every callee that is not a
//! non-generic user class keeps its earlier diagnostic.

use super::*;

fn return_of(source: &str, function: &str) -> Ty {
    let hir = check_source(source).unwrap_or_else(|err| {
        panic!(
            "{source}\nexpected to type-check, got {}: {}",
            err.code, err.message
        )
    });
    hir.items
        .iter()
        .find_map(|item| match item {
            HirItem::Function {
                name, return_ty, ..
            } if name == function => Some(return_ty.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no item `{function}`"))
}

fn refused(source: &str, code: &str, message: &str) {
    let err = check_source(source).expect_err(source);
    assert_eq!(err.code, code, "{source}: {}", err.message);
    assert!(
        err.message.contains(message),
        "{source}\nexpected a message containing {message:?}, got {:?}",
        err.message
    );
}

fn instance(name: &str) -> Ty {
    Ty::Instance(Box::new(name.to_string()))
}

const CLASS: &str = "class R:\n    def __init__(self, v: int) -> None:\n        self.v = v\n\n    \
    def get(self) -> int:\n        return self.v\n";

#[test]
fn a_returned_constructor_call_infers_the_class() {
    // The issue's own program.
    let source = format!("{CLASS}\n\ndef _mk():\n    return R(1)\n\nprint(_mk().v)\n");
    assert_eq!(return_of(&source, "_mk"), instance("R"));
}

#[test]
fn a_local_bound_from_a_constructor_call_infers_the_class() {
    let source = format!("{CLASS}\n\ndef _mk():\n    r = R(1)\n    return r\n\nprint(_mk().v)\n");
    assert_eq!(return_of(&source, "_mk"), instance("R"));
}

#[test]
fn a_method_called_on_a_constructed_instance_infers_its_return() {
    // Composes with #1420's method-call term.
    let source = format!("{CLASS}\n\ndef _g():\n    return R(5).get()\n\nprint(_g())\n");
    assert_eq!(return_of(&source, "_g"), Ty::Int);
}

#[test]
fn a_subclass_with_an_inherited_init_infers_the_subclass() {
    let source = format!(
        "{CLASS}\n\nclass D(R):\n    pass\n\ndef _mk():\n    return D(4)\n\nprint(_mk().v)\n"
    );
    assert_eq!(return_of(&source, "_mk"), instance("D"));
}

#[test]
fn a_constructed_instance_passed_to_a_helper_types_its_parameter() {
    // The call site's `R(2)` unifies into `_g`'s parameter term, and the
    // parameter flows to the return; a module-level binding from a
    // constructor call compiles alongside it.
    let source =
        format!("{CLASS}\n\ndef _g(r):\n    return r\n\nx = R(1)\nprint(_g(R(2)).v, x.v)\n");
    assert_eq!(return_of(&source, "_g"), instance("R"));
}

#[test]
fn a_subclass_instance_returned_as_its_base_stays_refused() {
    // Neither phase admits returning a subclass instance as its base
    // (pre-existing); the solver now types `D(2)` and so reports its own
    // declared-return wording, as it already did for `def f(d: D) -> R:
    // return d`.
    let source = format!(
        "{CLASS}\n\nclass D(R):\n    pass\n\ndef make() -> R:\n    return D(2)\n\nprint(make().v)\n"
    );
    refused(&source, "T0022", "expected `R`, found `D`");
}

#[test]
fn a_method_returning_a_constructor_call_infers_the_class() {
    let source = format!("{CLASS}\n    def __copy__(self):\n        return R(self.v)\n");
    assert_eq!(return_of(&source, "R.__copy__"), instance("R"));
}

#[test]
fn a_constructor_argument_of_the_wrong_type_is_refused_by_the_check_phase() {
    let source = format!("{CLASS}\n\ndef _mk():\n    return R(\"x\")\n\nprint(_mk().v)\n");
    refused(
        &source,
        "T0021",
        "argument 1 of `R` expects `int`, got `str`",
    );
}

#[test]
fn a_constructor_call_of_the_wrong_arity_is_refused_by_the_check_phase() {
    let source = format!("{CLASS}\n\ndef _mk():\n    return R(1, 2)\n\nprint(_mk().v)\n");
    refused(&source, "T0021", "`R` expects 1 argument(s), got 2");
}

#[test]
fn an_abstract_class_keeps_the_check_phases_precise_refusal() {
    // The solver answers the instance; the check phase's `C0001` is what
    // the program is refused with, not a "cannot infer return type".
    let source = "from abc import ABC, abstractmethod\n\nclass A(ABC):\n    @abstractmethod\n    \
        def f(self) -> int: ...\n\ndef _mk():\n    return A()\n\nprint(_mk().f())\n";
    refused(source, "C0001", "cannot instantiate abstract class `A`");
}

#[test]
fn a_protocol_class_answers_nothing_and_keeps_the_prior_t0021() {
    // An instance answer would make the check phase's method resolution
    // refuse the top-level `_mk().f()` with `T0044` (the protocol's stub `f`
    // is not a callable member), and that pass-2 error masks the pass-3
    // "cannot instantiate protocol class"; answering nothing keeps the base
    // tree's diagnostic.
    let source = "from typing import Protocol\n\nclass P(Protocol):\n    \
        def f(self) -> int: ...\n\ndef _mk():\n    return P()\n\nprint(_mk().f())\n";
    refused(
        source,
        "T0021",
        "cannot infer return type of private helper `_mk`",
    );
}

#[test]
fn a_join_with_another_type_is_a_conflict() {
    let source = format!(
        "{CLASS}\n\ndef _mk(flag: bool):\n    if flag:\n        return R(1)\n    return 3\n\nprint(_mk(True))\n"
    );
    refused(&source, "T0022", "conflicting inferred types `R` and `int`");
}

#[test]
fn a_generic_class_constructor_call_answers_no_term() {
    let source = "class G[T]:\n    def __init__(self, v: T) -> None:\n        self.v = v\n\n\
        def _mk():\n    return G(1)\n\nprint(_mk().v)\n";
    refused(
        source,
        "T0021",
        "cannot infer return type of private helper `_mk`",
    );
}

#[test]
fn a_builtin_exception_constructor_keeps_its_capability_gap() {
    // The seeded `ValueError` class is in the class table, but the
    // `is_known_callable_builtin` arm runs first.
    refused(
        "def _mk():\n    return ValueError(\"x\")\n\n_mk()\n",
        "C0001",
        "call to builtin `ValueError` is valid Python but not implemented yet",
    );
}

#[test]
fn a_class_named_like_a_builtin_keeps_its_capability_gap() {
    // `range` is a known callable builtin, so a user `class range` keeps
    // the pre-existing `C0001` (the constructor term is consulted last).
    refused(
        "class range:\n    def __init__(self) -> None:\n        pass\n\n\
         def _mk():\n    return range()\n\n_mk()\n",
        "C0001",
        "call to builtin `range`",
    );
}
