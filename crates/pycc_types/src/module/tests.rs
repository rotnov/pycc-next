//! Unit tests for the module-level check driver (Part 3 of #864, #868,
//! D-220): the per-function collectors, the solver-first merge, and the
//! first-diagnostic pins that keep `check`/`check_and_resolve` byte-identical
//! to their pre-#868 selection (D-217 rule 2).
//!
//! The pins assert *literal* `(code, message)` pairs captured from the
//! `a65d1a16` binary (the tree before this part), not `check(..) ==
//! check_all(..)[0]` -- decision B *defines* `check` as `check_all`'s first
//! element, so that identity could never fail and would pin nothing.

use super::*;

/// Lowers a source snippet exactly as the driver does before the type
/// checker runs; every input here lowers, so the type pass is the only pass
/// under test.
fn lower(source: &str) -> HirModule {
    let module = pycc_parser::parse(source).expect("test source must parse");
    pycc_hir::lower_checked(&module).expect("test source must lower")
}

fn codes(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics.iter().map(|d| d.code).collect()
}

fn keyed_codes(diagnostics: &KeyedDiagnostics) -> Vec<(DiagnosticKey, &str)> {
    diagnostics.iter().map(|(key, d)| (*key, d.code)).collect()
}

fn item_name(hir: &HirModule, index: usize) -> &str {
    match &hir.items[index] {
        HirItem::Function { name, .. } => name,
        HirItem::TopLevelStmt(_) => "<top-level>",
    }
}

const T0022_RETURN: &str = "return type mismatch: expected `int`, found `str`";
const T0043_INT_ATTR: &str = "cannot read an attribute on `int`: it is not a class instance";
const T0025_TOP_LEVEL: &str =
    "cannot assign `str` to `x: int`, initializer does not match the declared annotation";
const T0021_USUB: &str = "unary operator USub is not defined for `str`";
const T0021_ADD: &str = "operator Add is not defined for `int` and `str`";

/// The `tests/diagnostics/t0022_types_per_function.py` fixture, verbatim.
const FIXTURE: &str = "def f() -> int:\n    return \"a\"\n\n\ndef g(y: int) -> int:\n    return y.nope\n\n\ndef h(x: int) -> int:\n    return -\"s\"\n\n\ndef main() -> int:\n    return f() + g(1) + h(2)\n";

/// Two fully annotated functions with checker-only errors (`T0043`).
const TWO_CHECKER_ONLY: &str =
    "def f(y: int) -> int:\n    return y.nope\n\n\ndef g(y: int) -> int:\n    return y.nope\n";

/// A failing top-level statement before a broken function.
const PASS_2_FAILURE: &str = "x: int = \"s\"\n\n\ndef g(y: int) -> int:\n    return y.nope\n";

/// Two failing bodies in a module whose unannotated `_h` disables the
/// concrete fast path; `main` resolves `_h`.
const TWO_SOLVER_WITH_HELPER: &str = "def _h(a):\n    return a\n\n\ndef f() -> int:\n    return \"a\"\n\n\ndef g() -> int:\n    return \"b\"\n\n\ndef main() -> int:\n    return _h(1)\n";

/// Two top-level calls of an unannotated helper with conflicting argument
/// types: the one construct that fails in the solver's top-level walk.
const TOP_LEVEL_SOLVER_FAILURE: &str = "def _h(a):\n    return a\n\n\n_h(1)\n_h(\"s\")\n";
const T0021_TOP_LEVEL_SOLVER: &str =
    "argument 1 of private helper `_h`: conflicting inferred types `int` and `str`";

const REDEFINITION: &str =
    "def f(x: int) -> int:\n    return x\n\n\ndef f(x: str) -> str:\n    return x\n";
const T0021_REDEFINITION: &str = "cannot redefine function `f` with a different signature (previous: (int) -> int, current: (str) -> str)";

fn minimal_class_def(
    name: &str,
    bases: &[&str],
    mro: &[&str],
    attrs: &[(&str, Ty)],
) -> HirClassDef {
    HirClassDef {
        class_attrs: Vec::new(),
        exception_type_tag: None,
        name: name.to_string(),
        bases: bases.iter().map(|s| s.to_string()).collect(),
        mro: mro.iter().map(|s| s.to_string()).collect(),
        attrs: attrs
            .iter()
            .map(|(attr_name, ty)| (attr_name.to_string(), ty.clone()))
            .collect(),
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
    }
}

/// A cross-MRO attribute redeclaration (`T0052`, D-210): hand-built because
/// the `self.x: int = ...` spelling does not lower yet (`C0001`).
fn attribute_redeclaration_module() -> HirModule {
    let base = minimal_class_def("Base", &[], &["Base"], &[("v", Ty::Int)]);
    let derived = minimal_class_def(
        "Derived",
        &["Base"],
        &["Derived", "Base"],
        &[("v", Ty::Bool)],
    );
    HirModule {
        seeded_builtin_exception_classes: false,
        items: Vec::new(),
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("Base".to_string(), base), ("Derived".to_string(), derived)],
    }
}
const T0052_REDECLARATION: &str = "attribute `v` is declared as `bool` in class `Derived` and as `int` in class `Base`, both in the method resolution order of class `Derived`";

/// A module that passes every check phase and fails only in `monomorphize`
/// (the input of `tests/generic_method_instantiation.rs`'s
/// `check_and_resolve_rejects_generic_class_instantiate_for_non_generic_class`):
/// a `GenericClassInstantiate` of a non-generic class `D`.
fn monomorphize_only_failure_module() -> HirModule {
    let self_ty = Ty::Instance(Box::new("D".to_string()));
    let d_init = HirItem::Function {
        name: "D.__init__".to_string(),
        params: vec![("self".to_string(), self_ty), ("x".to_string(), Ty::Int)],
        return_ty: Ty::None,
        body: vec![HirStmt::AttrSet {
            base: HirExpr::Name("self".to_string()),
            attr: "x".to_string(),
            value: HirExpr::Name("x".to_string()),
        }],
    };
    let mut d_class_def = minimal_class_def("D", &[], &["D"], &[("x", Ty::Int)]);
    d_class_def.methods = vec![("__init__".to_string(), "D.__init__".to_string())];
    // A generic function keeps `monomorphize` from returning early.
    let identity = HirItem::Function {
        name: "identity".to_string(),
        params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
        return_ty: Ty::Param(Box::new("T".to_string())),
        body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
    };
    HirModule {
        seeded_builtin_exception_classes: false,
        items: vec![
            d_init,
            identity,
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::GenericClassInstantiate {
                class: "D".to_string(),
                type_arg: Ty::Int,
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ],
        type_aliases: Vec::new(),
        imports: Vec::new(),
        class_defs: vec![("D".to_string(), d_class_def)],
    }
}

// -- first-diagnostic pins (D-217 rule 2) -----------------------------

/// Asserts that both `*_all` entry points report `expected` first for
/// `hir`, and that neither `Err` is empty.
fn assert_first_pinned(hir: &HirModule, expected: (&str, &str)) {
    for (label, diagnostics) in [
        ("check_all", check_all(hir).unwrap_err()),
        (
            "check_and_resolve_all",
            check_and_resolve_all(hir).map(|_| ()).unwrap_err(),
        ),
    ] {
        assert!(!diagnostics.is_empty(), "{label}: Err must never be empty");
        assert_eq!(
            (diagnostics[0].code, diagnostics[0].message.as_str()),
            expected,
            "{label}: first diagnostic drifted from the pre-#868 selection"
        );
    }
}

#[test]
fn first_diagnostic_of_a_redefinition_mismatch_is_pinned() {
    assert_first_pinned(&lower(REDEFINITION), ("T0021", T0021_REDEFINITION));
}

#[test]
fn first_diagnostic_of_a_cross_mro_attribute_redeclaration_is_pinned() {
    let hir = attribute_redeclaration_module();
    assert_first_pinned(&hir, ("T0052", T0052_REDECLARATION));
    // C1: a pre-check failure is the whole report.
    assert_eq!(check_all(&hir).unwrap_err().len(), 1);
}

#[test]
fn first_diagnostic_of_a_pass_2_failure_is_pinned() {
    assert_first_pinned(&lower(PASS_2_FAILURE), ("T0025", T0025_TOP_LEVEL));
}

#[test]
fn first_diagnostic_of_two_failing_concrete_functions_is_pinned() {
    assert_first_pinned(&lower(TWO_CHECKER_ONLY), ("T0043", T0043_INT_ATTR));
}

#[test]
fn first_diagnostic_of_two_failing_solver_bodies_is_pinned() {
    assert_first_pinned(&lower(TWO_SOLVER_WITH_HELPER), ("T0022", T0022_RETURN));
}

#[test]
fn first_diagnostic_of_a_top_level_solver_failure_is_pinned() {
    assert_first_pinned(
        &lower(TOP_LEVEL_SOLVER_FAILURE),
        ("T0021", T0021_TOP_LEVEL_SOLVER),
    );
}

#[test]
fn first_diagnostic_of_the_fixture_is_the_solver_return_type_conflict() {
    assert_first_pinned(&lower(FIXTURE), ("T0022", T0022_RETURN));
}

// -- C6: post-check phases stay one-element ----------------------------

#[test]
fn a_monomorphize_only_failure_is_a_one_element_err_after_a_passing_check() {
    let hir = monomorphize_only_failure_module();
    assert_eq!(check_all(&hir), Ok(()));
    let diagnostics = check_and_resolve_all(&hir).map(|_| ()).unwrap_err();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "T0042");
    assert!(
        diagnostics[0].message.contains("class `D` is not generic"),
        "{}",
        diagnostics[0].message
    );
    assert_eq!(check_and_resolve(&hir).unwrap_err().code, "T0042");
}

// -- C2: the checker collects one diagnostic per function ---------------

#[test]
fn checker_collects_one_diagnostic_per_failing_function_in_item_order() {
    let hir = lower(TWO_CHECKER_ONLY);
    let local_names = module_function_local_names(&hir);
    let env = concrete_function_environment(&hir).unwrap();
    let collected = check_with_environment_all(&hir, env, &local_names).unwrap_err();
    assert_eq!(
        keyed_codes(&collected),
        vec![
            (DiagnosticKey::Function(0), "T0043"),
            (DiagnosticKey::Function(1), "T0043")
        ]
    );
    for (_, diagnostic) in &collected {
        assert_eq!(diagnostic.message, T0043_INT_ATTR);
    }
}

#[test]
fn checker_collects_one_diagnostic_per_broken_method() {
    let hir = lower(
        "class A:\n    def __init__(self, x: int) -> None:\n        self.x = x\n\n    def m(self) -> int:\n        return self.x.nope\n\n    def n(self) -> int:\n        return self.x.nope\n",
    );
    let local_names = module_function_local_names(&hir);
    let env = concrete_function_environment(&hir).unwrap();
    let collected = check_with_environment_all(&hir, env, &local_names).unwrap_err();
    // Methods are `HirItem::Function`s with mangled names; `A.__init__` is
    // also an item, so the entries are asserted by name, not position.
    let names: Vec<&str> = collected
        .iter()
        .map(|(key, _)| item_name(&hir, key.item_index().unwrap()))
        .collect();
    assert_eq!(names, vec!["A.m", "A.n"]);
    assert_eq!(codes(&drop_keys(collected)), vec!["T0043", "T0043"]);
}

#[test]
fn checker_stops_at_a_failing_top_level_statement_without_running_pass_3() {
    let hir = lower(PASS_2_FAILURE);
    let local_names = module_function_local_names(&hir);
    let env = concrete_function_environment(&hir).unwrap();
    let collected = check_with_environment_all(&hir, env, &local_names).unwrap_err();
    assert_eq!(
        keyed_codes(&collected),
        vec![(DiagnosticKey::TopLevel(0), "T0025")]
    );
    assert_eq!(collected[0].1.message, T0025_TOP_LEVEL);
}

#[test]
fn checker_collects_a_failing_generic_function_body_alongside_an_ordinary_one() {
    // The pass-3 loop has two failure arms (`check_generic_function_in` and
    // `check_function_in`); this module exercises both in one list.
    let hir = lower(
        "def f(y: int) -> int:\n    return y.nope\n\n\ndef ident[T](x: T) -> T:\n    return x + 1\n",
    );
    let local_names = module_function_local_names(&hir);
    let env = concrete_function_environment(&hir).unwrap();
    let collected = check_with_environment_all(&hir, env, &local_names).unwrap_err();
    assert_eq!(
        keyed_codes(&collected),
        vec![
            (DiagnosticKey::Function(0), "T0043"),
            (DiagnosticKey::Function(1), "T0021")
        ]
    );
}

#[test]
fn check_with_signatures_all_is_ok_for_a_valid_concrete_module() {
    let hir = lower("def main() -> int:\n    return 1\n");
    let local_names = module_function_local_names(&hir);
    let signatures = concrete_function_signatures(&hir).unwrap();
    assert_eq!(
        check_with_signatures_all(&hir, &signatures, &local_names),
        Ok(())
    );
}

// -- C3: the solver collects one diagnostic per failing body -------------

#[test]
fn solver_collects_one_diagnostic_per_failing_body_and_skips_the_post_phases() {
    // `_h` is never called, so the post-phase resolution loop would report
    // `T0021 cannot infer ...` for it; with two bodies collected, the
    // post-phases do not run and only the two body diagnostics are reported.
    let hir = lower(
        "def _h(a):\n    return a\n\n\ndef f() -> int:\n    return \"a\"\n\n\ndef g() -> int:\n    return \"b\"\n",
    );
    let local_names = module_function_local_names(&hir);
    let collected = infer_function_signatures_with_solver_all(&hir, &local_names).unwrap_err();
    assert_eq!(
        keyed_codes(&collected),
        vec![
            (DiagnosticKey::Function(1), "T0022"),
            (DiagnosticKey::Function(2), "T0022")
        ]
    );
    for (_, diagnostic) in &collected {
        assert_eq!(diagnostic.message, T0022_RETURN);
    }
    let first = infer_function_signatures_with_solver(&hir, &local_names).unwrap_err();
    assert_eq!(
        (first.code, first.message.as_str()),
        ("T0022", T0022_RETURN)
    );
    // The same module with only `_h` reports the post-phase diagnostic.
    let hir = lower("def _h(a):\n    return a\n");
    let local_names = module_function_local_names(&hir);
    let collected = infer_function_signatures_with_solver_all(&hir, &local_names).unwrap_err();
    assert_eq!(
        keyed_codes(&collected),
        vec![(DiagnosticKey::Module, "T0021")]
    );
    assert_eq!(
        collected[0].1.message,
        "cannot infer type of parameter `a` in private helper `_h`; add an annotation"
    );
}

#[test]
fn solver_collects_an_implicit_return_failure_in_a_later_body() {
    // `a` fails in its body walk; `f` succeeds and unifies `_g`'s return
    // with `int`; `_g` has no `return`, so its implicit `None` conflicts.
    let hir = lower(
        "def a() -> int:\n    return \"z\"\n\n\ndef f() -> int:\n    return _g()\n\n\ndef _g():\n    x = 1\n",
    );
    let local_names = module_function_local_names(&hir);
    let collected = infer_function_signatures_with_solver_all(&hir, &local_names).unwrap_err();
    assert_eq!(
        keyed_codes(&collected),
        vec![
            (DiagnosticKey::Function(0), "T0022"),
            (DiagnosticKey::Function(2), "T0022")
        ]
    );
    assert_eq!(collected[0].1.message, T0022_RETURN);
    assert_eq!(
        collected[1].1.message,
        "private helper implicit return: conflicting inferred types `int` and `None`"
    );
}

// -- #949: the return conflict's wording follows the return *term* ---------

#[test]
fn an_unannotated_helper_keeps_the_inferred_return_conflict_wording() {
    // `_h` has no return annotation, so its return type really is inferred
    // and the original wording still applies verbatim -- the #949 reword is
    // scoped to declared returns only.
    let hir = lower(
        "def _h(a: int, b: bool):\n    if b:\n        return a\n    return \"s\"\n\n\ndef main() -> int:\n    return _h(1, True)\n",
    );
    let local_names = module_function_local_names(&hir);
    let collected = infer_function_signatures_with_solver_all(&hir, &local_names).unwrap_err();
    assert_eq!(
        keyed_codes(&collected),
        vec![(DiagnosticKey::Function(0), "T0022")]
    );
    assert_eq!(
        collected[0].1.message,
        "private helper return type: conflicting inferred types `int` and `str`"
    );
    assert_eq!(collected[0].1.help, None);
}

#[test]
fn a_declared_return_names_the_annotation_even_when_the_conflict_is_raised_reversed() {
    // `f`'s declared `int` unifies against `_h`'s *inference variable*, which
    // `f`'s own call already resolved to `str`. That path raises the conflict
    // with the actual type first and the declared type second (the reverse of
    // the plain `(Ok, Ok)` order), so before #949 this rendered as
    // "conflicting inferred types `str` and `int`" -- both misdescribed and
    // backwards.
    let hir = lower(
        "def _h(a):\n    return a\n\n\ndef f() -> int:\n    return _h(\"s\")\n\n\ndef main() -> int:\n    return f()\n",
    );
    let local_names = module_function_local_names(&hir);
    let collected = infer_function_signatures_with_solver_all(&hir, &local_names).unwrap_err();
    assert_eq!(
        keyed_codes(&collected),
        vec![(DiagnosticKey::Function(1), "T0022")]
    );
    assert_eq!(collected[0].1.message, T0022_RETURN);
    assert_eq!(collected[0].1.help.as_deref(), Some("return a `int` value"));
}

#[test]
fn solver_reports_a_top_level_failure_alone() {
    let hir = lower(TOP_LEVEL_SOLVER_FAILURE);
    let local_names = module_function_local_names(&hir);
    let collected = infer_function_signatures_with_solver_all(&hir, &local_names).unwrap_err();
    assert_eq!(
        keyed_codes(&collected),
        vec![(DiagnosticKey::TopLevel(2), "T0021")]
    );
    assert_eq!(collected[0].1.message, T0021_TOP_LEVEL_SOLVER);
}

#[test]
fn a_none_keyed_solver_list_always_has_length_one() {
    // The `KeyedDiagnostics` invariant `merge_solver_first` relies on, over
    // every module-level solver failure shape reachable from source: the
    // top-level walk, the binop post-phase, and the resolution loop.
    for source in [
        TOP_LEVEL_SOLVER_FAILURE,
        "def f(a: int) -> int:\n    x = a + 1\n    y = x + \"s\"\n    return y\n\n\ndef g(y: int) -> int:\n    return y.nope\n",
        "def _h(a):\n    return a\n",
    ] {
        let hir = lower(source);
        let local_names = module_function_local_names(&hir);
        let collected = infer_function_signatures_with_solver_all(&hir, &local_names).unwrap_err();
        assert_eq!(collected.len(), 1, "{source}");
        assert!(
            !matches!(collected[0].0, DiagnosticKey::Function(_)),
            "{source}"
        );
    }
}

// -- C4: the per-function solver-first merge -----------------------------

#[test]
fn merge_reports_solver_entries_then_checker_only_entries() {
    let hir = lower(FIXTURE);
    let diagnostics = check_all(&hir).unwrap_err();
    assert_eq!(codes(&diagnostics), vec!["T0022", "T0021", "T0043"]);
    assert_eq!(diagnostics[0].message, T0022_RETURN);
    assert_eq!(diagnostics[1].message, T0021_USUB);
    assert_eq!(diagnostics[2].message, T0043_INT_ATTR);
    // `check_and_resolve_all` merges identically.
    let resolved = check_and_resolve_all(&hir).map(|_| ()).unwrap_err();
    assert_eq!(codes(&resolved), vec!["T0022", "T0021", "T0043"]);
}

#[test]
fn merge_lets_the_checker_supply_a_function_whose_solver_diagnostic_is_post_phase() {
    // `h`'s `x + "s"` is a `propagate_binop_constraints` diagnostic: raised
    // after every body was walked, so it is skipped once `f`'s body failed,
    // and the checker's `Some(h)` entry is what reports `h`.
    let hir = lower(
        "def f() -> int:\n    return \"a\"\n\n\ndef g(y: int) -> int:\n    return y.nope\n\n\ndef h(x: int) -> int:\n    return x + \"s\"\n\n\ndef main() -> int:\n    return f() + g(1) + h(2)\n",
    );
    let diagnostics = check_all(&hir).unwrap_err();
    assert_eq!(codes(&diagnostics), vec!["T0022", "T0043", "T0021"]);
    assert_eq!(diagnostics[2].message, T0021_ADD);
}

#[test]
fn merge_reports_a_function_flagged_by_both_phases_once_with_the_solver_text() {
    // Both phases flag `f` with different text; exactly one line, the
    // solver's.
    let hir = lower("def f() -> int:\n    return \"a\"\n");
    let local_names = module_function_local_names(&hir);
    let env = concrete_function_environment(&hir).unwrap();
    let checker = check_with_environment_all(&hir, env, &local_names).unwrap_err();
    assert_eq!(
        keyed_codes(&checker),
        vec![(DiagnosticKey::Function(0), "T0022")]
    );
    assert_ne!(checker[0].1.message, T0022_RETURN);
    let diagnostics = check_all(&hir).unwrap_err();
    assert_eq!(codes(&diagnostics), vec!["T0022"]);
    assert_eq!(diagnostics[0].message, T0022_RETURN);
}

#[test]
fn merge_appends_a_module_level_checker_entry_the_solver_did_not_flag() {
    let hir = lower("x: int = \"s\"\n\n\ndef f() -> int:\n    return \"a\"\n");
    let diagnostics = check_all(&hir).unwrap_err();
    assert_eq!(codes(&diagnostics), vec!["T0022", "T0025"]);
    assert_eq!(diagnostics[0].message, T0022_RETURN);
    assert_eq!(diagnostics[1].message, T0025_TOP_LEVEL);
}

#[test]
fn merge_drops_the_checker_list_after_a_module_level_solver_failure() {
    // Rule 4's third outcome: the checker fails at pass 2 with its own
    // `(TopLevel(0), T0025)` for the top-level statement, but the solver's
    // top-level walk accepts it and its post-body
    // `propagate_binop_constraints` fails with `(Module, T0021)` -- that one
    // solver line is reported and the checker's list, top-level entry
    // included, is dropped.
    let hir = lower("x: int = \"s\"\n\n\ndef f(a: int) -> int:\n    y = a + \"s\"\n    return y\n");
    let diagnostics = check_all(&hir).unwrap_err();
    assert_eq!(codes(&diagnostics), vec!["T0021"]);
}

#[test]
fn merge_reports_a_module_level_solver_failure_alone() {
    // The solver's `T0021 operator Add` for `f` is a post-phase diagnostic
    // (`None`-keyed); the checker also flags `f` and `g`, but neither is
    // appended: one line, as before this part.
    let hir = lower(
        "def f(a: int) -> int:\n    x = a + 1\n    y = x + \"s\"\n    return y\n\n\ndef g(y: int) -> int:\n    return y.nope\n",
    );
    let local_names = module_function_local_names(&hir);
    let env = concrete_function_environment(&hir).unwrap();
    let checker = check_with_environment_all(&hir, env, &local_names).unwrap_err();
    assert_eq!(
        keyed_codes(&checker),
        vec![
            (DiagnosticKey::Function(0), "T0021"),
            (DiagnosticKey::Function(1), "T0043")
        ]
    );
    let diagnostics = check_all(&hir).unwrap_err();
    assert_eq!(codes(&diagnostics), vec!["T0021"]);
    assert_eq!(diagnostics[0].message, T0021_ADD);
}

#[test]
fn merge_reports_only_the_solver_list_when_there_is_no_concrete_pass() {
    let hir = lower(TWO_SOLVER_WITH_HELPER);
    let diagnostics = check_all(&hir).unwrap_err();
    assert_eq!(codes(&diagnostics), vec!["T0022", "T0022"]);
}

#[test]
fn merge_solver_first_arms_are_keyed_not_content_based() {
    let d = |code: &'static str| Diagnostic::error(code, code.to_string(), Span::new(0, 0));
    // Key present in `solver`: dropped from `concrete`; key absent: kept, in
    // `concrete`'s order, after every solver entry.
    let solver = vec![
        (DiagnosticKey::Function(2), d("S2")),
        (DiagnosticKey::Function(0), d("S0")),
    ];
    let concrete = vec![
        (DiagnosticKey::Function(0), d("C0")),
        (DiagnosticKey::Module, d("CN")),
        (DiagnosticKey::Function(1), d("C1")),
    ];
    let merged = merge_solver_first(solver.clone(), Some(concrete.clone()));
    assert_eq!(codes(&drop_keys(merged)), vec!["S2", "S0", "CN", "C1"]);
    // No concrete pass: the solver list alone.
    assert_eq!(
        codes(&drop_keys(merge_solver_first(solver, None))),
        vec!["S2", "S0"]
    );
    // A `Module`-keyed solver failure is reported alone, whatever `concrete`
    // holds -- even an identical-content entry is never deduplicated by
    // text (C7), only by key.
    let module_level_failure = vec![(DiagnosticKey::Module, d("SN"))];
    assert_eq!(
        codes(&drop_keys(merge_solver_first(
            module_level_failure,
            Some(concrete)
        ))),
        vec!["SN"]
    );
}

// -- the solver-passes path and the `Ok` arms --------------------------

#[test]
fn a_checker_failure_after_a_successful_solver_run_is_reported() {
    let hir = lower("def _h(a):\n    return a\n\n\ndef f() -> int:\n    return _h(1).nope\n");
    let diagnostics = check_all(&hir).unwrap_err();
    assert_eq!(codes(&diagnostics), vec!["T0043"]);
    assert_eq!(diagnostics[0].message, T0043_INT_ATTR);
    let resolved = check_and_resolve_all(&hir).map(|_| ()).unwrap_err();
    assert_eq!(codes(&resolved), vec!["T0043"]);
}

#[test]
fn valid_modules_are_ok_on_both_the_concrete_and_the_solver_path() {
    for source in [
        "def main() -> int:\n    return 1\n",
        "def _h(a):\n    return a\n\n\ndef main() -> int:\n    return _h(1)\n",
    ] {
        let hir = lower(source);
        assert_eq!(check_all(&hir), Ok(()), "{source}");
        assert!(check_and_resolve_all(&hir).is_ok(), "{source}");
    }
}

// -- The public keyed entry points (#898) --------------------------------

#[test]
fn check_all_keyed_reports_a_failing_function_body_by_item_index() {
    let hir =
        lower("def f() -> int:\n    return 1\n\n\ndef g(y: int) -> int:\n    return y.nope\n");
    let collected = check_all_keyed(&hir).expect_err("the second body must fail");
    assert_eq!(
        keyed_codes(&collected),
        vec![(DiagnosticKey::Function(1), "T0043")]
    );
    assert_eq!(collected[0].0.item_index(), Some(1));
    // The unkeyed view is exactly the same list without its keys.
    assert_eq!(codes(&check_all(&hir).unwrap_err()), vec!["T0043"]);
}

#[test]
fn check_all_keyed_reports_a_failing_top_level_statement_by_item_index() {
    let hir = lower("def f() -> int:\n    return 1\n\n\nx: int = \"a\"\n");
    let collected = check_all_keyed(&hir).expect_err("the top-level statement must fail");
    assert_eq!(collected.len(), 1, "{collected:#?}");
    assert_eq!(collected[0].0, DiagnosticKey::TopLevel(1));
    assert_eq!(collected[0].0.item_index(), Some(1));
}

#[test]
fn a_pre_check_failure_has_no_owning_item() {
    // An incompatible redefinition is rejected before any item is checked,
    // so it belongs to the program rather than to one item; the driver
    // renders such a diagnostic against the entry file.
    let hir =
        lower("def f(x: int) -> int:\n    return x\n\n\ndef f(x: str) -> str:\n    return x\n");
    let collected = check_all_keyed(&hir).expect_err("the redefinition must be rejected");
    assert_eq!(collected.len(), 1, "{collected:#?}");
    assert_eq!(collected[0].0, DiagnosticKey::Module);
    assert_eq!(collected[0].0.item_index(), None);
}

#[test]
fn check_and_resolve_all_keyed_carries_the_same_keys_as_its_unkeyed_view() {
    let hir =
        lower("def f() -> int:\n    return 1\n\n\ndef g(y: int) -> int:\n    return y.nope\n");
    let collected = check_and_resolve_all_keyed(&hir).expect_err("the second body must fail");
    assert_eq!(
        keyed_codes(&collected),
        vec![(DiagnosticKey::Function(1), "T0043")]
    );
    assert_eq!(
        codes(&check_and_resolve_all(&hir).unwrap_err()),
        vec!["T0043"]
    );
}

#[test]
fn check_and_resolve_all_keyed_resolves_a_valid_module() {
    let hir = lower("def f(x: int) -> int:\n    return x\n");
    let resolved = check_and_resolve_all_keyed(&hir).expect("a valid module must resolve");
    assert_eq!(resolved.items.len(), hir.items.len());
}
