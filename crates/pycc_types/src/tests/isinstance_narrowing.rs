//! #1476 (Part 3 of #1387): an `isinstance(o, C)` guard narrows an `object`
//! name back to a native type for the reads it dominates
//! (`crate::narrow`), on both walkers.
//!
//! A fully annotated module is decided by the check phase alone; a module
//! holding one unannotated private helper also runs the constraint solver,
//! whose per-function verdict wins (D-220), so each shape is asserted
//! under both.

use super::*;

fn diagnostics(source: &str) -> Vec<pycc_diag::Diagnostic> {
    let parsed = pycc_parser::parse(source).expect("test fixture must parse");
    let mut resolved = pycc_hir::ResolvedImports::default();
    resolved.set_ext_module(true);
    let hir = pycc_hir::lower_module(&parsed, &resolved, None)
        .map(|lowered| lowered.hir)
        .unwrap_or_else(|diagnostics| panic!("{source}\nmust lower: {diagnostics:#?}"));
    check_all(&hir).err().unwrap_or_default()
}

/// One unannotated private helper and a call that resolves it: present, the
/// module runs the constraint solver as well as the check phase.
const SOLVER: &str = "def _id(x):\n    return x\n\n\ndef use() -> int:\n    return _id(1)\n\n\n";

fn checks(source: &str) {
    for prefix in ["", SOLVER] {
        let source = format!("{prefix}{source}");
        let diagnostics = diagnostics(&source);
        assert!(diagnostics.is_empty(), "{source}\n{diagnostics:#?}");
    }
}

fn refused(source: &str, code: &str, message: &str) {
    for prefix in ["", SOLVER] {
        let source = format!("{prefix}{source}");
        let diagnostics = diagnostics(&source);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code == code && d.message.contains(message)),
            "{source}\nexpected {code} {message:?}, got {diagnostics:#?}"
        );
    }
}

/// The probe `o + 1` is refused because `o` is still the object: a narrowed
/// `o` would make it native, and a refusal for any other reason would not
/// name `object`.
fn still_object(source: &str) {
    refused(
        source,
        "T0021",
        "operator Add is not defined for `object` and `int`",
    );
}

#[test]
fn each_scalar_guard_admits_native_operations_on_the_narrowed_name() {
    checks(
        "def f(o: object) -> int:\n    if isinstance(o, int):\n        return o + 1\n    return 0\n",
    );
    checks(
        "def f(o: object) -> float:\n    if isinstance(o, float):\n        return o * 2.0\n    return 0.0\n",
    );
    checks(
        "def f(o: object) -> bool:\n    if isinstance(o, bool):\n        return not o\n    return False\n",
    );
    checks(
        "def f(o: object) -> str:\n    if isinstance(o, str):\n        return o + \"!\"\n    return \"\"\n",
    );
}

#[test]
fn a_compiled_class_guard_admits_its_fields_and_methods() {
    checks(
        "class C:\n    def __init__(self, v: int) -> None:\n        self.v = v\n\n    \
         def twice(self) -> int:\n        return self.v * 2\n\n\n\
         def f(o: object) -> int:\n    if isinstance(o, C):\n        return o.v + o.twice()\n    return 0\n",
    );
}

#[test]
fn outside_the_guard_the_name_is_still_an_object() {
    refused(
        "def f(o: object) -> int:\n    if isinstance(o, int):\n        pass\n    return o + 1\n",
        "T0021",
        "operator Add is not defined for `object` and `int`",
    );
    // The `else` branch of a positive guard is not narrowed.
    still_object(
        "def f(o: object) -> int:\n    if isinstance(o, int):\n        return 0\n    else:\n        return o + 1\n",
    );
}

#[test]
fn a_negated_guard_narrows_the_else_branch_and_the_continuation() {
    checks(
        "def f(o: object) -> int:\n    if not isinstance(o, int):\n        return 0\n    else:\n        return o + 1\n",
    );
    checks(
        "def f(o: object) -> int:\n    if not isinstance(o, int):\n        return 0\n    return o + 1\n",
    );
    // A body that does not terminate narrows nothing after it.
    still_object(
        "def f(o: object) -> int:\n    if not isinstance(o, int):\n        pass\n    return o + 1\n",
    );
}

/// The continuation is reached only through the `else`; one that rebinds
/// the name, directly, in an `elif` arm or in a loop inside it, leaves the
/// continuation reading the object, not the guarded class (#1476 review).
#[test]
fn an_else_that_rebinds_the_name_does_not_narrow_the_continuation() {
    still_object(
        "def f(o: object) -> int:\n    if not isinstance(o, int):\n        return 0\n    else:\n        \
         o = 1.5\n    return o + 1\n",
    );
    still_object(
        "def f(o: object, flag: bool) -> int:\n    if not isinstance(o, int):\n        return 0\n    \
         elif flag:\n        o = 2.5\n    return o + 1\n",
    );
    still_object(
        "def f(o: object, n: int) -> int:\n    if not isinstance(o, int):\n        return 0\n    else:\n        \
         while n > 0:\n            o = 1.5\n            n = n - 1\n    return o + 1\n",
    );
    // An `else` that leaves the name alone still narrows it.
    checks(
        "def f(o: object, n: int) -> int:\n    if not isinstance(o, int):\n        return 0\n    else:\n        \
         n = n + 1\n    return o + n\n",
    );
}

/// The same holds for the `Optional` early-exit shape the check phase
/// shares the predicate with.
#[test]
fn an_else_that_rebinds_an_optional_does_not_narrow_the_continuation() {
    let rebound = diagnostics(
        "def f(x: int | None) -> int:\n    if x is None:\n        return 0\n    else:\n        \
         x = None\n    return x + 1\n",
    );
    assert!(rebound.iter().any(|d| d.code == "T0021"), "{rebound:#?}");
    assert!(
        diagnostics(
            "def f(x: int | None) -> int:\n    if x is None:\n        return 0\n    return x + 1\n"
        )
        .is_empty()
    );
}

#[test]
fn rebinding_the_name_ends_the_narrowing() {
    still_object(
        "def f(o: object, p: object) -> int:\n    if isinstance(o, int):\n        o = p\n        return o + 1\n    return 0\n",
    );
}

#[test]
fn a_loop_that_rebinds_the_name_is_not_narrowed_on_entry() {
    still_object(
        "def f(o: object, p: object) -> int:\n    if isinstance(o, int):\n        while o:\n            \
         x = o + 1\n            o = p\n    return 0\n",
    );
}

#[test]
fn a_try_body_is_narrowed_up_to_its_rebinding_and_its_handlers_are_not() {
    // The body is walked in order: the read before the rebinding is native.
    checks(
        "def f(o: object, p: object) -> int:\n    if isinstance(o, int):\n        try:\n            \
         x = o + 1\n            o = p\n            return x\n        except Exception:\n            \
         return 0\n    return -1\n",
    );
    // A `try` that does not rebind the name keeps it narrowed throughout.
    checks(
        "def f(o: object) -> int:\n    if isinstance(o, int):\n        try:\n            \
         return o + 1\n        except Exception:\n            return o + 2\n        \
         finally:\n            print(o + 3)\n    return -1\n",
    );
    // A handler can run after the body's rebinding.
    still_object(
        "def f(o: object, p: object) -> int:\n    if isinstance(o, int):\n        try:\n            \
         o = p\n        except Exception:\n            return o + 1\n    return -1\n",
    );
    // The `finally` can run after any path's rebinding: the body's, a
    // handler's, the `else`'s, or a handler's `as` name -- also when that
    // path narrows the name again before it ends, since a returning guard
    // reaches the `finally` between the two.
    for rebinding in [
        "try:\n            o = p\n        except Exception:\n            pass\n",
        "try:\n            o = p\n            if not isinstance(o, int):\n                return 0\n",
        "try:\n            pass\n        except Exception:\n            o = p\n            \
         if not isinstance(o, int):\n                return 0\n",
        "try:\n            pass\n        except Exception:\n            o = p\n",
        "try:\n            pass\n        except Exception:\n            pass\n        else:\n            o = p\n",
        "try:\n            pass\n        except Exception as o:\n            pass\n",
    ] {
        still_object(&format!(
            "def f(o: object, p: object) -> int:\n    if isinstance(o, int):\n        {rebinding}        \
             finally:\n            print(o + 1)\n    return -1\n"
        ));
    }
    // An `else` continues the body, after its last statement's rebinding.
    still_object(
        "def f(o: object, p: object) -> int:\n    if isinstance(o, int):\n        try:\n            \
         o = p\n        except Exception:\n            return 0\n        else:\n            \
         return o + 1\n    return -1\n",
    );
    // A handler's `as` name is the exception, not the narrowed value.
    refused(
        "def f(o: object) -> int:\n    if isinstance(o, int):\n        try:\n            \
         pass\n        except Exception as o:\n            return o + 1\n    return -1\n",
        "T0021",
        "operator Add is not defined for `Exception` and `int`",
    );
}

#[test]
fn after_a_try_the_name_is_narrowed_on_every_fall_through_path() {
    // A handler that rebinds the name and returns never reaches the read.
    checks(
        "def f(o: object, p: object) -> int:\n    if isinstance(o, int):\n        try:\n            \
         pass\n        except Exception:\n            o = p\n            return 0\n        \
         return o + 1\n    return -1\n",
    );
    // One that falls through does.
    still_object(
        "def f(o: object, p: object) -> int:\n    if isinstance(o, int):\n        try:\n            \
         pass\n        except Exception:\n            o = p\n        return o + 1\n    return -1\n",
    );
    // A `finally` that rebinds the name ends the narrowing after it.
    still_object(
        "def f(o: object, p: object) -> int:\n    if isinstance(o, int):\n        try:\n            \
         pass\n        finally:\n            o = p\n        return o + 1\n    return -1\n",
    );
    // A negated guard in a `finally` narrows nothing after the `try`: a guard
    // narrows past itself only when its body returns (a `raise` does not
    // count, `pycc_hir::definitely_terminates`), and a `return` in a
    // `finally` is refused at lowering (`L0001`).
    still_object(
        "def f(o: object) -> int:\n    try:\n        pass\n    finally:\n        \
         if not isinstance(o, int):\n            raise ValueError()\n    return o + 1\n",
    );
}

/// A guard on a class another compiled class derives from keeps the
/// object (static dispatch would run the base's methods on a subclass
/// instance); the leaf narrows.
#[test]
fn a_class_with_a_compiled_subclass_keeps_the_object() {
    let classes = "class B:\n    def __init__(self) -> None:\n        self.x = 1\n\n\n\
                   class D(B):\n    pass\n\n\n";
    still_object(&format!(
        "{classes}def f(o: object) -> int:\n    if isinstance(o, B):\n        \
         return o + 1\n    return 0\n"
    ));
    checks(&format!(
        "{classes}def f(o: object) -> int:\n    if isinstance(o, D):\n        \
         return o.x + 1\n    return 0\n"
    ));
}

/// A module function spelled like the class shadows it: the guard is
/// refused rather than narrowed (CPython raises `TypeError` there).
#[test]
fn a_function_named_like_the_class_refuses_the_guard() {
    refused(
        "def int(x: str) -> str:\n    return x\n\n\n\
         def f(o: object) -> int:\n    if isinstance(o, int):\n        return 1\n    \
         return 0\n",
        "I0404",
        "against the function `int`",
    );
}

#[test]
fn a_shadowed_class_name_or_isinstance_does_not_narrow() {
    still_object(
        "def isinstance(a: object, b: object) -> bool:\n    return True\n\n\n\
         def f(o: object) -> int:\n    if isinstance(o, int):\n        return o + 1\n    return 0\n",
    );
}

#[test]
fn unnarrowable_classes_and_shapes_keep_the_object() {
    // Each probe is `o + 1`, whose refusal names `object` only while `o` is
    // still the object; a narrowed `o` would name its class instead.
    let shapes = [
        // A tuple of classes.
        "def f(o: object) -> int:\n    if isinstance(o, (int, str)):\n        return o + 1\n    return 0\n",
        // A container class.
        "def f(o: object) -> int:\n    if isinstance(o, list):\n        return o + 1\n    return 0\n",
        // A compound test.
        "def f(o: object, b: bool) -> int:\n    if isinstance(o, int) and b:\n        return o + 1\n    return 0\n",
        // An enum.
        "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n\n\n\
         def f(o: object) -> int:\n    if isinstance(o, Color):\n        return o + 1\n    return 0\n",
        // An exception class.
        "class Boom(Exception):\n    pass\n\n\n\
         def f(o: object) -> int:\n    if isinstance(o, Boom):\n        return o + 1\n    return 0\n",
    ];
    for shape in shapes {
        refused(
            shape,
            "T0021",
            "operator Add is not defined for `object` and `int`",
        );
    }
}

#[test]
fn a_first_binding_from_the_narrowed_name_is_an_object_slot() {
    // Both branches bind `y`, one from the narrowed name and one from the
    // object: the slot is `object` either way.
    checks(
        "def f(o: object, p: object) -> object:\n    if isinstance(o, str):\n        y = o\n    else:\n        y = p\n    return y\n",
    );
    // An annotated binding takes the narrowed value.
    checks(
        "def f(o: object) -> int:\n    if isinstance(o, int):\n        y: int = o\n        return y + 1\n    return 0\n",
    );
    // A non-bare read is native: `y` is an `int` slot, and a later object
    // rebinding is refused.
    refused(
        "def f(o: object, p: object) -> int:\n    if isinstance(o, int):\n        y = o + 0\n        y = p\n        return y\n    return 0\n",
        "T0023",
        "cannot assign `object` to `y`, previously inferred as `int`",
    );
}

#[test]
fn identity_and_a_nested_guard_see_the_object() {
    checks(
        "def f(o: object, p: object) -> bool:\n    if isinstance(o, str):\n        return o is p\n    return False\n",
    );
    checks(
        "def f(o: object) -> bool:\n    if isinstance(o, str):\n        return o is None or o is not None\n    return False\n",
    );
    checks(
        "def f(o: object) -> int:\n    if isinstance(o, int):\n        if isinstance(o, bool):\n            return 1\n        return o + 2\n    return 0\n",
    );
}

/// A chain cannot carry an object, so an `is` link on a narrowed read is
/// refused as it is without the guard; an ordered link stays native.
#[test]
fn a_chained_identity_link_on_a_narrowed_read_is_refused() {
    for test in ["o is not None is not None", "0 < o is not None"] {
        refused(
            &format!(
                "def f(o: object) -> bool:\n    if isinstance(o, int):\n        return {test}\n    return False\n"
            ),
            "I0404",
            "a chained comparison with a CPython object operand",
        );
    }
    checks(
        "def f(o: object) -> bool:\n    if isinstance(o, int):\n        return 0 < o < 10\n    return False\n",
    );
}

#[test]
fn an_inferred_return_and_an_object_return_both_take_the_narrowed_value() {
    checks(
        "def _inc(o: object):\n    if isinstance(o, int):\n        return o + 1\n    return 0\n\n\n\
         def g(o: object) -> int:\n    return _inc(o)\n",
    );
    checks(
        "def f(o: object) -> object:\n    if isinstance(o, str):\n        return o\n    return o\n",
    );
}

#[test]
fn an_unannotated_parameter_is_not_narrowed() {
    // `x` is a solver variable, not an `object` binding, so the guard does
    // not narrow it: `x + 1` constrains it to `int` as it would unguarded.
    checks(
        "def _g(x):\n    if isinstance(x, int):\n        return x + 1\n    return 0\n\n\n\
         def g() -> int:\n    return _g(2)\n",
    );
    still_object(
        "def _g(x):\n    if isinstance(x, int):\n        return x + 1\n    return 0\n\n\n\
         def g(o: object) -> int:\n    return _g(o)\n",
    );
}

#[test]
fn a_binding_named_like_the_class_or_a_maybe_bound_operand_does_not_narrow() {
    refused(
        "def f(o: object, int: object) -> int:\n    if isinstance(o, int):\n        return o + 1\n    return 0\n",
        "T0021",
        "operator Add is not defined for `object` and `int`",
    );
    // A maybe-bound operand is refused (`T0041`) at the guard's own read, so
    // no narrowing can make the read inside it admissible.
    refused(
        "def f(p: object, b: bool) -> int:\n    if b:\n        o = p\n    if isinstance(o, int):\n        return o + 1\n    return 0\n",
        "T0041",
        "",
    );
}

#[test]
fn a_guarded_branch_that_rebinds_the_name_still_joins() {
    checks(
        "def f(o: object, p: object) -> object:\n    if isinstance(o, int):\n        x = o + 1\n        o = p\n    return o\n",
    );
}

#[test]
fn a_maybe_bound_rebinding_ends_the_narrowing_after_the_if() {
    still_object(
        "def f(o: object, p: object, b: bool) -> int:\n    if not isinstance(o, int):\n        return 0\n    \
         if b:\n        o = p\n    return o + 1\n",
    );
}
