//! Part 1 of #883 (#962): the stdlib receiver shadow check across
//! `import <module> as <alias>`.
//!
//! `pycc_hir` lowers `m.sqrt(x)` after `import math as m` to the canonical
//! `"math.sqrt"` string, so the shadow check here can no longer compare the
//! receiver the user wrote against the bindings in scope -- it has to check
//! every spelling the module's import table binds to `math`. These tests
//! pin that on all three checking paths (the fully annotated concrete
//! path, the un-annotated solver path, and `check_with_signatures_all`
//! through `check_all`), plus the two holes the rewrite closed for the
//! canonical spelling too: a module-level `Maybe` binding and a `def`
//! rebinding.

use super::*;
use pycc_std::StdModule;

fn shadowed(name: &str) -> String {
    format!(
        "`{name}` is a local name here, not the stdlib `math` module -- attribute access on a \
         non-module value is not supported yet"
    )
}

fn check_all_source(source: &str) -> Result<(), Vec<pycc_diag::Diagnostic>> {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    check_all(&hir)
}

fn first_message(source: &str) -> String {
    let diagnostics = check_all_source(source).unwrap_err();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "C0001");
    diagnostics[0].message.clone()
}

// -- the concrete (fully annotated) path -----------------------------------

#[test]
fn aliased_receiver_is_shadowed_by_a_parameter_on_the_concrete_path() {
    // `def f(m: float)` is fully annotated, so `check_all` takes the
    // concrete path whose `Environment` is built by
    // `concrete_function_environment` -- the constructor that never saw
    // the alias table before `check_with_environment_all` started filling
    // it in.
    let message = first_message(
        "import math as m\ndef f(m: float) -> float:\n    return m.sqrt(m)\nprint(f(4.0))\n",
    );
    assert_eq!(message, shadowed("m"));
}

#[test]
fn aliased_constant_is_shadowed_by_a_parameter_on_the_concrete_path() {
    let message = first_message(
        "import math as m\ndef f(m: float) -> float:\n    return m.pi\nprint(f(4.0))\n",
    );
    assert_eq!(message, shadowed("m"));
}

#[test]
fn unshadowed_alias_is_accepted_on_the_concrete_path() {
    check_all_source(
        "import math as m\ndef f(x: float) -> float:\n    return m.sqrt(x) + m.pi\nprint(f(4.0))\n",
    )
    .unwrap();
}

// -- the solver (un-annotated) path ------------------------------------------

#[test]
fn aliased_receiver_is_shadowed_by_a_parameter_on_the_solver_path() {
    // An un-annotated private helper forces `check_all` off the concrete path onto
    // `infer_function_signatures_with_solver`, whose
    // `collect_expr_constraints` has its own copy of the shadow check.
    let message =
        first_message("import math as m\ndef _f(m):\n    return m.sqrt(m)\nprint(_f(4.0))\n");
    assert_eq!(message, shadowed("m"));
}

#[test]
fn aliased_constant_is_shadowed_by_a_parameter_on_the_solver_path() {
    let message = first_message("import math as m\ndef _f(m):\n    return m.pi\nprint(_f(4.0))\n");
    assert_eq!(message, shadowed("m"));
}

#[test]
fn unshadowed_alias_is_accepted_on_the_solver_path() {
    check_all_source("import math as m\ndef _f(x):\n    return m.sqrt(x) + m.pi\nprint(_f(4.0))\n")
        .unwrap();
}

#[test]
fn solver_path_shadow_check_runs_directly_against_the_alias_table() {
    // Pin the solver-side site without `check_all`'s routing in between:
    // the globals environment built by `infer_function_signatures_with_solver`
    // carries the alias table and the per-function literal copies it.
    let module = pycc_parser::parse("import math as m\ndef _f(m):\n    return m.sqrt(m)\n")
        .expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    let local_names = module_function_local_names(&hir);
    let err = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(err.code, "C0001");
    assert_eq!(err.message, shadowed("m"));
}

// -- `check_with_signatures_all` (module level) -------------------------------

#[test]
fn aliased_receiver_is_shadowed_by_a_module_level_rebinding() {
    // Module-level: the `Environment` is `check_with_signatures_all`'s, and
    // `m` is a plain (`Bound`) binding by the time the call is checked.
    let message = first_message("import math as m\nm = 2.0\nprint(m.sqrt(4.0))\n");
    assert_eq!(message, shadowed("m"));
}

#[test]
fn aliased_receiver_is_shadowed_by_a_module_level_maybe_binding() {
    // A conditionally bound `m` is `BindingState::Maybe`, for which
    // `Environment::lookup` answers `None`; the check consults
    // `binding_state` so the maybe-bound alias still shadows.
    let message =
        first_message("import math as m\nc = True\nif c:\n    m = 2.0\nprint(m.sqrt(4.0))\n");
    assert_eq!(message, shadowed("m"));
}

#[test]
fn canonical_receiver_is_shadowed_by_a_module_level_maybe_binding() {
    // The same hole closed for the canonical spelling: before #962 the
    // module-level check used `lookup`, which let a maybe-bound `math`
    // through to the libm path.
    let message =
        first_message("import math\nc = True\nif c:\n    math = 2.0\nprint(math.sqrt(4.0))\n");
    assert_eq!(message, shadowed("math"));
}

#[test]
fn aliased_receiver_is_shadowed_by_a_def_above_the_use() {
    let message =
        first_message("import math as m\ndef m() -> float:\n    return 1.0\nprint(m.sqrt(4.0))\n");
    assert_eq!(message, shadowed("m"));
}

#[test]
fn canonical_receiver_is_shadowed_by_a_def_above_the_use() {
    let message =
        first_message("import math\ndef math() -> float:\n    return 1.0\nprint(math.sqrt(4.0))\n");
    assert_eq!(message, shadowed("math"));
}

#[test]
fn aliased_receiver_is_accepted_when_the_shadowing_def_is_below_the_use() {
    // `def_rebound` is filled in source order at module level, so a `def
    // m()` *after* the call has not rebound `m` yet -- valid CPython, and
    // the position sensitivity that `lookup_function` could not give.
    check_all_source("import math as m\nprint(m.sqrt(4.0))\ndef m() -> float:\n    return 1.0\n")
        .unwrap();
}

#[test]
fn unaliased_import_leaves_the_alias_table_empty_and_still_checks_the_canonical_name() {
    // `import math` binds `math` to `math`: no alias row, and the existing
    // canonical shadow check is unchanged.
    let message = first_message("import math\nmath = 2.0\nprint(math.sqrt(4.0))\n");
    assert_eq!(message, shadowed("math"));
}

// -- the validation pass's site directly -------------------------------------

#[test]
fn infer_expr_in_reports_the_bound_alias_spelling() {
    let mut env = Environment::new();
    env.std_module_aliases = vec![("m".to_string(), StdModule::Math)];
    let err = infer_expr_in(
        &env,
        &["m"],
        &HirExpr::Call {
            callee: "math.sqrt".to_string(),
            args: vec![HirExpr::FloatLiteral(2.0)],
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "C0001");
    assert_eq!(err.message, shadowed("m"));

    let err = infer_expr_in(&env, &["m"], &HirExpr::Name("math.pi".to_string())).unwrap_err();
    assert_eq!(err.message, shadowed("m"));
}

#[test]
fn infer_expr_in_prefers_the_canonical_spelling_when_both_are_bound() {
    let mut env = Environment::new();
    env.std_module_aliases = vec![("m".to_string(), StdModule::Math)];
    let err =
        infer_expr_in(&env, &["math", "m"], &HirExpr::Name("math.pi".to_string())).unwrap_err();
    assert_eq!(err.message, shadowed("math"));
}

#[test]
fn infer_expr_in_ignores_a_bound_alias_of_a_different_module() {
    let mut env = Environment::new();
    env.std_module_aliases = vec![("e".to_string(), StdModule::Enum)];
    let ty = infer_expr_in(&env, &["e"], &HirExpr::Name("math.pi".to_string())).unwrap();
    assert_eq!(ty, Ty::Float);
}

#[test]
fn infer_expr_in_accepts_an_unshadowed_alias() {
    let mut env = Environment::new();
    env.std_module_aliases = vec![("m".to_string(), StdModule::Math)];
    let ty = infer_expr_in(
        &env,
        &["x"],
        &HirExpr::Call {
            callee: "math.sqrt".to_string(),
            args: vec![HirExpr::FloatLiteral(2.0)],
        },
    )
    .unwrap();
    assert_eq!(ty, Ty::Float);
}
