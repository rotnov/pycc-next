//! Type-checking tests for the synthetic builtin exception classes
//! (Part 1 of #541, extending D-173).
//!
//! HIR lowering now seeds a real `HirClassDef` for each of the seven
//! builtin exception names, which puts them into `Environment::classes`
//! for the first time. Everything here pins the consequences of that:
//! that the pre-existing `except`/`raise` surface is unchanged, that the
//! new surface a real class table unlocks (subclassing, `issubclass`,
//! annotations) behaves, and that a synthetic class is still not a value.
//!
//! In its own submodule rather than appended to the crate's `tests.rs`,
//! per AGENTS.md's decomposability rule (#695).

use super::*;
use crate::{HirClassDef, HirModule, check, check_and_resolve};

fn parse_lower(source: &str) -> HirModule {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    pycc_hir::lower_checked(&module).expect("test fixture must lower")
}

fn check_source(source: &str) -> Result<(), Diagnostic> {
    check(&parse_lower(source))
}

fn resolve_source(source: &str) -> Result<HirModule, Diagnostic> {
    check_and_resolve(&parse_lower(source))
}

/// A module that references a builtin exception name and shadows none, so
/// HIR lowering seeds the synthetic class table into it. Lowering only seeds
/// a *referencing* module (Part 1 of #541's `frontend-perf-gate` fix), so a
/// test that needs a seeded `Environment` has to start from a source like
/// this rather than from a bare `print(1)`.
const SEEDED_SOURCE: &str = "raise ValueError(\"boom\")\n";

fn environment_for(source: &str) -> Environment {
    let hir = parse_lower(source);
    let mut env = Environment::new();
    crate::class::bind_classes(&mut env, &hir);
    env
}

// -- the shadowing predicate (the highest-risk change) ---------------

#[test]
fn seeded_builtin_exception_classes_do_not_read_as_shadowed() {
    // Before #541 these seven names were absent from `Environment::classes`
    // and `is_unshadowed_builtin_exception` read that absence as "not
    // shadowed". Now they are present, so the predicate has to distinguish
    // a synthetic entry from a user one; if it ever stops doing so, every
    // `except`/`raise` in the language starts being rejected.
    let env = environment_for(SEEDED_SOURCE);
    for name in pycc_hir::BUILTIN_EXCEPTION_CLASSES {
        assert!(env.is_synthetic_class(name), "`{name}` must be synthetic");
        assert!(
            is_unshadowed_builtin_exception(&env, &[], name),
            "`{name}` must not read as shadowed"
        );
    }
}

#[test]
fn an_unseeded_module_still_does_not_read_the_flat_seven_as_shadowed() {
    // The property that makes gating the seeding safe: `is_user_defined_class`
    // is `classes.contains_key(name) && !is_synthetic_class(name)`, so an
    // *absent* name is not user-defined and therefore not shadowed -- exactly
    // the pre-#541 reading. A module that never names one of the original
    // flat seven is seeded with none of them, and must still accept
    // `except`/`raise` for those seven, because `resolve_exception_tag`
    // resolves them by name independent of the class table.
    let env = environment_for("print(1)\n");
    for name in pycc_hir::BUILTIN_EXCEPTION_CLASSES {
        assert!(env.lookup_class(name).is_none(), "`{name}` must be absent");
        assert!(!env.is_synthetic_class(name));
        let unshadowed = is_unshadowed_builtin_exception(&env, &[], name);
        if pycc_hir::is_flat_builtin_exception_class(name) {
            assert!(
                unshadowed,
                "`{name}` must not read as shadowed while absent"
            );
        } else {
            // Part 2 of #543 (#739), work item 5: a name past the flat seven
            // has no name-based fallback, so absence from the class table
            // must *not* be read as "unshadowed" -- unlike the flat seven,
            // it has to actually be present to be treated as raisable or
            // catchable. This is what turns the module-wide shadow gate's
            // withheld seeding into a clean `T0021` (see
            // `check_raise_operand`/`check_try_stmt`) instead of a
            // `handler_type_tags` panic when the name is genuinely
            // referenced but seeding was withheld elsewhere in the module.
            assert!(
                !unshadowed,
                "`{name}` (past the flat seven) must not read as unshadowed while absent from the class table"
            );
        }
    }
}

#[test]
fn a_user_class_of_the_same_name_still_reads_as_shadowed() {
    let env = environment_for("class ValueError:\n    def __init__(self) -> None:\n        pass\n");
    assert!(!env.is_synthetic_class("ValueError"));
    assert!(!is_unshadowed_builtin_exception(&env, &[], "ValueError"));
}

#[test]
fn rebinding_a_synthetic_name_with_a_user_definition_clears_the_marking() {
    // `bind_class`/`bind_synthetic_class` are the sole mutators of both
    // tables, and `bind_class` always registers a *user* definition, so a
    // later user definition under the same name must un-mark it -- otherwise a
    // monomorphization pass that re-registers a class could leave a user
    // class marked synthetic and silently uninstantiable.
    let mut env = environment_for(SEEDED_SOURCE);
    assert!(env.is_synthetic_class("ValueError"));
    env.bind_class(
        "ValueError".to_string(),
        HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "ValueError".to_string(),
            bases: Vec::new(),
            mro: vec!["ValueError".to_string()],
            attrs: Vec::new(),
            methods: Vec::new(),
            properties: Vec::new(),
            static_methods: Vec::new(),
            class_methods: Vec::new(),
            type_param: None,
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
    assert!(!env.is_synthetic_class("ValueError"));
    assert!(!is_unshadowed_builtin_exception(&env, &[], "ValueError"));
}

#[test]
fn an_unregistered_name_is_not_synthetic() {
    let env = environment_for("print(1)\n");
    assert!(!env.is_synthetic_class("NotAnException"));
}

// -- the pre-#541 `except`/`raise` surface is unchanged ---------------

#[test]
fn the_existing_raise_and_except_surface_still_checks() {
    check_source("raise ValueError(\"bad\")\n").expect("`raise ValueError(...)` must still check");
    check_source("raise Exception(\"bad\")\n").expect("`raise Exception(...)` must still check");
    check_source("try:\n    print(1)\nexcept ValueError:\n    print(2)\n")
        .expect("`except ValueError:` must still check");
    check_source("try:\n    print(1)\nexcept Exception as e:\n    print(2)\n")
        .expect("`except Exception as e:` must still check");
}

#[test]
fn raise_of_a_bound_builtin_exception_instance_still_checks() {
    check_source("try:\n    print(1)\nexcept ValueError as e:\n    raise e\n")
        .expect("re-raising a bound handler binding must still check");
}

#[test]
fn a_user_class_shadowing_a_builtin_exception_name_still_closes_the_gates() {
    // T0021's gates must stay closed against a user class: shadowing
    // `ValueError` makes `except ValueError:` an unrecognized handler
    // again, exactly as before #541.
    let err = check_source(
        "class ValueError:\n    def __init__(self) -> None:\n        pass\n\n\
         try:\n    print(1)\nexcept ValueError:\n    print(2)\n",
    )
    .expect_err("a shadowed handler class must be rejected");
    assert_eq!(err.code, "T0021");
}

// -- a synthetic builtin exception class is not a value ---------------

#[test]
fn instantiating_a_synthetic_builtin_exception_class_is_rejected() {
    let env = environment_for(SEEDED_SOURCE);
    let err = crate::class::resolve_instantiation(&env, "ValueError", &[], &[Ty::Str])
        .expect_err("a builtin exception class must not be instantiable as a value");
    assert_eq!(err.code, "C0001");
    assert!(
        err.message
            .contains("cannot instantiate builtin exception class"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn every_synthetic_builtin_exception_class_is_rejected_the_same_way() {
    let env = environment_for(SEEDED_SOURCE);
    for name in pycc_hir::BUILTIN_EXCEPTION_CLASSES {
        let err = crate::class::resolve_instantiation(&env, name, &[], &[Ty::Str])
            .expect_err("every builtin exception class must be uninstantiable");
        assert_eq!(err.code, "C0001");
    }
}

#[test]
fn a_user_class_shadowing_a_builtin_exception_name_stays_instantiable() {
    // The rejection is keyed on "synthetic", not on the name, so a user's
    // own `class ValueError:` is an ordinary, instantiable class.
    resolve_source(
        "class ValueError:\n    def __init__(self) -> None:\n        pass\n\n\
         x = ValueError()\nprint(1)\n",
    )
    .expect("a user class named `ValueError` must stay instantiable");
}

#[test]
fn attribute_access_on_a_bare_builtin_exception_class_name_reports_t0044() {
    // A behavior change Part 1 of #541 introduces, pinned here. Before the
    // seeding, `ValueError` was not in the class table and `ValueError.args`
    // reported `T0021 name \`ValueError\` is not defined`. It is now a real
    // class, so the same source reaches the class-attribute path and reports
    // `T0044` instead. The synthetic classes deliberately declare no
    // attribute slots (D-173 propagates a raised exception through global
    // runtime state, not through an allocated instance), so `args` is
    // genuinely absent rather than merely unreachable. See `docs/RUNTIME.md`.
    //
    // This also pins the reference scan's recursion: `ValueError` is spelled
    // only as an `Expr::Attribute`'s value inside a call argument inside a
    // function body. Were the scan not to reach it, the module would be
    // seeded with nothing and this would report `T0021` again.
    let err = check_source("def f() -> None:\n    print(ValueError.args)\n\nf()\n")
        .expect_err("attribute access on a bare builtin exception class must be rejected");
    assert_eq!(err.code, "T0044");
    assert!(
        err.message
            .contains("class `ValueError` has no attribute named `args`"),
        "unexpected message: {}",
        err.message
    );
}

// -- the new surface a real class table unlocks ------------------------

#[test]
fn a_user_subclass_of_a_builtin_exception_class_is_raisable_but_not_a_bound_value() {
    // Part 1 of #541's headline gain -- `MyError` inherits the synthetic
    // `Exception.__init__` through its MRO instead of reaching
    // `resolve_instantiation`'s internal-error panic -- but #714 found
    // that `resolve_instantiation`'s generic instantiation path resolved
    // this to a real, successful call all the way through: codegen has no
    // actual function body for the inherited placeholder, so the produced
    // binary aborted at runtime with a `NameError` naming a symbol the
    // user never wrote. `raise MyError("boom")` stays accepted (MIR
    // constructs the raised value directly, never calling through
    // `Exception.__init__`); binding the same construction to a name is
    // now a `C0001` at compile time instead.
    resolve_source(
        "class MyError(ValueError):\n    pass\n\ndef main() -> None:\n    raise MyError(\"boom\")\n\nmain()\n",
    )
    .expect("raising a user subclass of a builtin exception must stay accepted");

    let err =
        resolve_source("class MyError(ValueError):\n    pass\n\nx = MyError(\"boom\")\nprint(1)\n")
            .expect_err("binding the inherited constructor's result to a name must be rejected");
    assert_eq!(err.code, "C0001");
    assert!(
        err.message.contains("cannot instantiate exception class"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn a_user_subclass_constructor_still_checks_its_arguments() {
    // The inherited constructor's `str` parameter is enforced on the one
    // shape that reaches it post-#714: a fresh `raise` operand.
    let err = resolve_source(
        "class MyError(ValueError):\n    pass\n\ndef main() -> None:\n    raise MyError(1)\n\nmain()\n",
    )
    .expect_err("the inherited constructor's `str` parameter must be enforced");
    assert_eq!(err.code, "T0021");
}

#[test]
fn a_user_class_may_inherit_from_two_builtin_exception_classes() {
    resolve_source("class E(ValueError, TypeError):\n    pass\n\nprint(1)\n")
        .expect("multiple builtin exception bases must linearize and check");
}

#[test]
fn a_builtin_exception_class_is_usable_as_an_annotation() {
    check_source("def handle(e: ValueError) -> int:\n    return 1\n\nprint(1)\n")
        .expect("a builtin exception class must be a valid parameter annotation");
}

#[test]
fn an_annotated_assignment_of_the_wrong_type_is_a_diagnostic_not_a_panic() {
    let err = check_source("x: ValueError = 1\nprint(1)\n")
        .expect_err("an `int` initializer must not satisfy a `ValueError` annotation");
    assert_eq!(err.code, "T0025");
}

#[test]
fn a_return_annotation_of_a_builtin_exception_class_is_a_diagnostic_not_a_panic() {
    let err = check_source("def f() -> ValueError:\n    print(1)\n\nprint(1)\n")
        .expect_err("a function that cannot return a `ValueError` must be rejected");
    assert_eq!(err.code, "T0022");
}

#[test]
fn issubclass_over_the_builtin_exception_hierarchy_is_evaluated() {
    // Only reachable because the hierarchy is now in the class table:
    // before #541 `Exception` had no `HirClassDef` for the MRO walk.
    resolve_source(
        "class MyError(Exception):\n    pass\n\nprint(issubclass(MyError, Exception))\n",
    )
    .expect("`issubclass` over a builtin exception base must resolve");
    resolve_source("print(isinstance(1, ValueError))\n")
        .expect("`isinstance` against a builtin exception class must resolve");
}

// -- attribute access through an `except ... as` binding -------------

/// Before Part 1, `except ValueError as e:` bound `e` to
/// `Ty::Instance("ValueError")` while no `HirClassDef` of that name
/// existed, so any attribute read on `e` aborted the compiler in
/// `class::expect_class` with `internal error: class `ValueError` has no
/// registered HirClassDef`. The seeded definition turns that abort into an
/// ordinary diagnostic. It is `T0044` and not a successful read because
/// the synthetic definitions declare no attribute slots (D-173 propagates
/// a raised exception through global runtime state, not through an
/// allocated instance, so there is no storage a `message`/`args` slot
/// could name).
#[test]
fn reading_an_attribute_off_a_caught_builtin_exception_reports_t0044() {
    let error = check_source(
        "def main() -> None:\n    try:\n        raise ValueError(\"x\")\n    except ValueError as e:\n        print(e.args)\n",
    )
    .expect_err("`args` is not an attribute of the synthetic class");
    assert_eq!(error.code, "T0044");
    assert!(
        error.message.contains("ValueError") && error.message.contains("args"),
        "unexpected message: {}",
        error.message
    );
}

/// The handler binding itself, without an attribute read, is unaffected.
#[test]
fn binding_a_caught_builtin_exception_without_reading_it_still_checks() {
    check_source(
        "def main() -> None:\n    try:\n        raise ValueError(\"x\")\n    except ValueError as e:\n        print(\"caught\")\n",
    )
    .expect("binding a caught builtin exception must keep checking");
}

// -- provenance, not shape, decides synthetic membership (D-188) -----

/// The exact program the D-068 reviewer found regressing: a user
/// `class Exception:` whose lowered `HirClassDef` is byte-for-byte the
/// synthetic one. Under the previous structural-equality detection it was
/// marked synthetic, which made `is_user_defined_class` report the name as
/// *not* user-defined and handed the user's own class to the builtin
/// exception paths, rejecting the call with `C0001`.
#[test]
fn a_user_class_structurally_identical_to_the_synthetic_one_stays_the_users() {
    let source = "class Exception:\n    def __init__(self) -> None:\n        pass\n\ndef main() -> None:\n    x = Exception()\n\nmain()\n";
    let hir = parse_lower(source);
    // The fixture is only meaningful while it really is structurally
    // identical to the synthetic definition -- assert that, so a future
    // change to the synthetic shape cannot make this test pass vacuously.
    let user_def = hir
        .class_defs
        .iter()
        .find(|(name, _)| name == "Exception")
        .map(|(_, def)| def.clone())
        .expect("the user class must be in the class table");
    let synthetic_def = pycc_hir::builtin_exception_class_defs()
        .into_iter()
        .find(|(name, _)| name == "Exception")
        .expect("`Exception` must be synthesized")
        .1;
    assert_eq!(
        user_def, synthetic_def,
        "fixture must stay structurally identical to the synthetic definition"
    );
    assert!(!hir.seeded_builtin_exception_classes);

    let env = environment_for(source);
    assert!(!env.is_synthetic_class("Exception"));
    check(&hir).expect("a user-authored `Exception` must stay instantiable");
}

/// The general property, asserted directly rather than through one shape:
/// a class is marked synthetic **if and only if** this compiler's own HIR
/// lowering seeded it. Provenance travels on
/// `HirModule::seeded_builtin_exception_classes`; no property of a
/// `HirClassDef`'s own shape participates.
#[test]
fn a_class_is_synthetic_exactly_when_lowering_seeded_it() {
    for source in [
        // Seeded: references one of the seven, shadows none.
        SEEDED_SOURCE,
        "class MyError(ValueError):\n    pass\n\nprint(1)\n",
        // Unseeded: never names one of the seven.
        "print(1)\n",
        "class Point:\n    def __init__(self) -> None:\n        pass\n",
        // Unseeded because the module shadows a name -- including with a
        // class structurally identical to a synthetic one.
        "class Exception:\n    def __init__(self) -> None:\n        pass\n",
        "class ValueError:\n    def __init__(self) -> None:\n        pass\n",
        "class TypeError:\n    def __init__(self) -> None:\n        self.n = 1\n",
    ] {
        let hir = parse_lower(source);
        let env = environment_for(source);
        for (name, _) in &hir.class_defs {
            let expected =
                hir.seeded_builtin_exception_classes && pycc_hir::is_builtin_exception_class(name);
            assert_eq!(
                env.is_synthetic_class(name),
                expected,
                "`{name}` synthetic marking must follow lowering provenance in {source:?}"
            );
        }
        // A user-authored class is never marked, whatever its shape.
        if !hir.seeded_builtin_exception_classes {
            for name in pycc_hir::BUILTIN_EXCEPTION_CLASSES {
                assert!(
                    !env.is_synthetic_class(name),
                    "`{name}` must not be synthetic in an unseeded module: {source:?}"
                );
            }
        }
    }
}

/// Monomorphization rebuilds the `HirModule` and re-runs `bind_classes` on
/// the result, so provenance has to survive that rewrite -- otherwise the
/// synthetic classes silently become instantiable after the pass.
#[test]
fn provenance_survives_monomorphization() {
    let resolved = resolve_source(
        "class Box[T]:\n    def __init__(self, value: T) -> None:\n        self.value = value\n\ndef main() -> None:\n    b = Box(1)\n    raise ValueError(\"boom\")\n\nmain()\n",
    )
    .expect("the generic fixture must resolve");
    assert!(resolved.seeded_builtin_exception_classes);
    let mut env = Environment::new();
    crate::class::bind_classes(&mut env, &resolved);
    assert!(env.is_synthetic_class("ValueError"));
    assert!(!env.is_synthetic_class("Box"));
}
