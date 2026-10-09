//! Type-checking tests for raising and catching user-defined exception
//! classes (Part 2 of #541, D-189).
//!
//! In its own submodule rather than appended to the crate's `tests.rs`, per
//! AGENTS.md's decomposability rule.

use super::*;
use crate::check;

fn check_source(source: &str) -> Result<(), Diagnostic> {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    check(&hir)
}

fn expect_error(source: &str) -> Diagnostic {
    check_source(source).expect_err("source must be rejected")
}

const HIERARCHY: &str = "class AppError(Exception):\n    pass\n\n\n\
                         class DatabaseError(AppError):\n    pass\n\n\n";

#[test]
fn raising_a_user_exception_class_is_accepted() {
    check_source(&format!(
        "{HIERARCHY}def main() -> None:\n    raise DatabaseError(\"boom\")\n"
    ))
    .expect("raising a user exception class must be accepted");
}

#[test]
fn catching_a_user_exception_class_is_accepted() {
    check_source(&format!(
        "{HIERARCHY}def main() -> None:\n    try:\n        raise DatabaseError(\"boom\")\n\
         \x20   except AppError:\n        print(\"caught\")\n"
    ))
    .expect("catching a user exception class must be accepted");
}

#[test]
fn raising_an_abstract_exception_subclass_stays_rejected() {
    // #714 review finding: `check_raise_operand`'s `user_exception_class`
    // branch validates a raise operand's constructor arguments directly
    // against the synthetic `Exception.__init__` placeholder's signature,
    // instead of routing through `class::resolve_instantiation` as it did
    // before #714. `resolve_instantiation` also rejects an abstract (#380,
    // PEP 3119) class before ever resolving its constructor -- that guard
    // must survive the #714 shortcut for the one path that still needs it.
    let err = expect_error(
        "from abc import ABC, abstractmethod\n\n\n\
         class AbstractError(Exception, ABC):\n\
         \x20   @abstractmethod\n\
         \x20   def explain(self) -> str: ...\n\n\n\
         def main() -> None:\n\
         \x20   raise AbstractError(\"boom\")\n",
    );
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("cannot instantiate abstract class"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn a_user_exception_class_is_accepted_as_a_raise_cause() {
    check_source(&format!(
        "{HIERARCHY}def main() -> None:\n\
         \x20   raise DatabaseError(\"boom\") from AppError(\"root\")\n"
    ))
    .expect("a user exception class must be accepted as a cause");
}

#[test]
fn raising_a_bound_user_exception_value_stays_rejected() {
    // The memory-safety invariant of Part 2: a value of type
    // `Ty::Instance("AppError")` bound to a name, rather than a fresh
    // `AppError("x")` construction, must stay rejected as a raise operand
    // -- MIR lowers a bound value to `MirExceptionValue::Existing`, which
    // codegen hands to `pycc_rt_exception_raise` as a `*mut PyExceptionObj`
    // while the value is really a `*mut PyInstanceObj`. Accepting it would
    // be memory corruption, not a feature. Part 3 of #541 (#703) makes the
    // two representations one.
    //
    // #714 closed the specific route this test used to reach that bound
    // value (`e = AppError("x")`) at an even earlier point -- that
    // instantiation is now itself a `C0001`, since codegen has no real
    // `__init__` body for a class that only inherits the synthetic
    // placeholder -- so the bound value here comes from a parameter
    // annotation instead, which type-checks a `Ty::Instance("AppError")`
    // binding without ever calling through `resolve_instantiation`.
    let diagnostic = expect_error(&format!(
        "{HIERARCHY}def main(e: AppError) -> None:\n    raise e\n"
    ));
    assert_eq!(diagnostic.code, "T0021");
    assert!(
        diagnostic
            .message
            .contains("can only raise exception instances"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn raising_a_class_outside_the_exception_hierarchy_is_rejected() {
    let diagnostic = expect_error(
        "class Point:\n    def __init__(self, x: int) -> None:\n        self.x = x\n\n\n\
         def main() -> None:\n    raise ValueError(\"a\")\n    raise Point(1)\n",
    );
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn catching_a_class_outside_the_exception_hierarchy_is_rejected() {
    let diagnostic = expect_error(&format!(
        "class Point:\n    def __init__(self, x: int) -> None:\n        self.x = x\n\n\n\
         {HIERARCHY}def main() -> None:\n    try:\n        raise AppError(\"x\")\n\
         \x20   except Point:\n        print(\"caught\")\n"
    ));
    assert_eq!(diagnostic.code, "T0021");
    assert!(
        diagnostic
            .message
            .contains("is not a recognized exception class"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn binding_a_caught_user_exception_with_as_is_a_capability_gap() {
    let diagnostic = expect_error(&format!(
        "{HIERARCHY}def main() -> None:\n    try:\n        raise AppError(\"x\")\n\
         \x20   except AppError as exc:\n        print(\"caught\")\n"
    ));
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("with `as` is not supported yet"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn binding_a_caught_builtin_exception_with_as_still_works() {
    // The `as` rejection above must be scoped to user classes; the builtin
    // form has been accepted since #382 and stays accepted.
    check_source(
        "def main() -> None:\n    try:\n        raise ValueError(\"x\")\n\
         \x20   except ValueError as exc:\n        print(\"caught\")\n",
    )
    .expect("binding a caught builtin exception must still be accepted");
}

const OWN_INIT: &str = "class AppError(Exception):\n\
                        \x20   def __init__(self, code: int) -> None:\n\
                        \x20       self.code = code\n\n\n";

#[test]
fn raising_an_exception_class_with_its_own_constructor_is_a_capability_gap() {
    let diagnostic = expect_error(&format!(
        "{OWN_INIT}def main() -> None:\n    raise AppError(3)\n"
    ));
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("declares or inherits an `__init__` other than"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn catching_an_exception_class_with_its_own_constructor_is_a_capability_gap() {
    let diagnostic = expect_error(&format!(
        "{OWN_INIT}def main() -> None:\n    try:\n        raise ValueError(\"x\")\n\
         \x20   except AppError:\n        print(\"caught\")\n"
    ));
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("declares or inherits an `__init__` other than"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_subclass_inheriting_an_own_constructor_is_also_a_capability_gap() {
    // The gate walks the MRO, so the defect is reported for a subclass that
    // never wrote a constructor of its own but inherits a non-synthetic one.
    let diagnostic = expect_error(&format!(
        "{OWN_INIT}class DatabaseError(AppError):\n    pass\n\n\n\
         def main() -> None:\n    raise DatabaseError(4)\n"
    ));
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic
            .message
            .contains("`DatabaseError` declares or inherits an `__init__` other than"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_bad_constructor_argument_reports_its_own_argument_diagnostic() {
    // Inference runs before the raisability gate so the user sees the real
    // problem rather than a generic "can only raise exception instances".
    let diagnostic = expect_error(&format!(
        "{HIERARCHY}def main() -> None:\n    raise AppError(3)\n"
    ));
    assert_eq!(diagnostic.code, "T0021");
    assert!(
        diagnostic.message.contains("expects `str`"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn a_local_binding_shadowing_an_exception_class_is_not_raisable() {
    let diagnostic = expect_error(&format!(
        "{HIERARCHY}def main() -> None:\n    AppError = 3\n    raise AppError(\"x\")\n"
    ));
    assert_eq!(diagnostic.code, "T0021");
}

#[test]
fn an_ill_typed_raise_constructor_argument_propagates_its_own_error() {
    // #714: the `raise MyError(...)` argument list is inferred directly in
    // `check_raise_operand` (not via `infer_expr_in` -> `resolve_instantiation`
    // like an ordinary call), via
    // `args.iter().map(infer_expr_in).collect::<Result<Vec<_>, _>>()?`. This
    // pins that `?`: an argument that itself fails to type-check (an
    // undefined name, not merely one of the wrong type) must surface that
    // error rather than falling through to `check_call_args`.
    let diagnostic = expect_error(&format!(
        "{HIERARCHY}def main() -> None:\n    raise AppError(undefined_name)\n"
    ));
    assert_eq!(diagnostic.code, "T0021");
    assert!(
        diagnostic.message.contains("undefined_name"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
#[should_panic(expected = "`Exception.__init__` was not registered as an ordinary function")]
fn raising_a_user_exception_class_panics_if_the_synthetic_init_is_somehow_unregistered() {
    // #714's own internal-error precondition, pinned the same way
    // `resolve_instantiation`'s analogous panics are in
    // `crates/pycc_types/src/class/binding.rs`'s test module: `lower_checked`
    // always registers `EXCEPTION_INIT_MANGLED_NAME` as an ordinary function
    // whenever it seeds the builtin exception classes, and
    // `reject_own_constructor` already confirmed the resolved `__init__` is
    // that exact placeholder, so `check_raise_operand`'s own
    // `env.lookup_function(EXCEPTION_INIT_MANGLED_NAME)` can never fail in
    // practice. This test bypasses `lower_checked` and calls
    // `check_raise_operand` directly against a hand-built `Environment` that
    // registers the class but never registers the placeholder function,
    // to reach that otherwise-unreachable branch.
    use pycc_hir::{EXCEPTION_INIT_MANGLED_NAME, HirClassDef};

    let mut env = crate::Environment::new();
    env.bind_class(
        "Ghost".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: Some(0),
            name: "Ghost".to_string(),
            bases: Vec::new(),
            mro: vec!["Ghost".to_string()],
            attrs: Vec::new(),
            methods: vec![(
                "__init__".to_string(),
                EXCEPTION_INIT_MANGLED_NAME.to_string(),
            )],
            type_param: None,
            properties: Vec::new(),
            static_methods: Vec::new(),
            class_methods: Vec::new(),
            is_enum: false,
            implicit_object_init: false,
            method_defaults: Vec::new(),
            enum_members: Vec::new(),
            is_dataclass: false,
            dataclass_fields: Vec::new(),
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        },
    );
    let expr = HirExpr::Call {
        callee: "Ghost".to_string(),
        args: vec![HirExpr::StringLiteral("x".to_string())],
    };
    let _ = super::check_raise_operand(&env, &[], &expr, "can only raise exception instances");
}
