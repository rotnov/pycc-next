//! Tests of keyword-argument binding (`super`).

use crate::{HirExpr, HirItem, HirStmt};

/// The lowered HIR of `source`, which must lower cleanly.
fn lower(source: &str) -> crate::HirModule {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    crate::lower_checked(&module).expect("fixture must lower")
}

fn lower_err(source: &str) -> pycc_diag::Diagnostic {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    crate::lower_checked(&module).expect_err("fixture must be rejected")
}

/// Part 8 of #1371: a keyword call the binder cannot bind is either
/// refused here with the keyword `C0001` (`true`), or -- when its callee
/// could be a CPython object -- deferred to `pycc_types` as a
/// `HirExpr::KeywordCall` (`false`), which reports the same `C0001` for
/// any callee that is not one (`pycc_types`' `foreign::keyword_call`).
/// Either way the keywords are never bound or erased.
fn keyword_call_refused_here(source: &str) -> bool {
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    match crate::lower_checked(&module) {
        Err(diagnostic) => {
            assert_eq!(diagnostic.code, "C0001", "source: {source}");
            assert_eq!(
                diagnostic.message,
                crate::KEYWORD_CALL_UNSUPPORTED,
                "source: {source}"
            );
            true
        }
        Ok(hir) => {
            let rendered = format!("{hir:?}");
            assert!(
                rendered.contains("KeywordCall {"),
                "source: {source}: {rendered}"
            );
            false
        }
    }
}

/// The argument vector of the first top-level `HirExpr::Call` statement.
fn first_call_args(source: &str) -> Vec<HirExpr> {
    lower(source)
        .items
        .iter()
        .find_map(|item| match item {
            HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call { args, .. })) => {
                Some(args.clone())
            }
            _ => None,
        })
        .expect("fixture has no top-level call statement")
}

/// Every lowered function's name and body, in module order.
fn function_bodies(hir: &crate::HirModule) -> Vec<(&str, &Vec<HirStmt>)> {
    hir.items
        .iter()
        .filter_map(|item| match item {
            HirItem::Function { name, body, .. } => Some((name.as_str(), body)),
            HirItem::TopLevelStmt(_) => None,
        })
        .collect()
}

const DEF_AB: &str = "def f(a: int, b: int) -> None:\n    print(a)\n    print(b)\n\n";

#[test]
fn a_keyword_call_binds_to_the_same_arguments_as_its_positional_twin() {
    let keyword = first_call_args(&format!("{DEF_AB}f(b=2, a=1)\n"));
    let positional = first_call_args(&format!("{DEF_AB}f(1, 2)\n"));
    assert_eq!(keyword, positional);
    assert_eq!(
        keyword,
        vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]
    );
}

#[test]
fn a_mixed_positional_and_keyword_call_binds_the_keyword_to_the_later_parameter() {
    assert_eq!(
        first_call_args(&format!("{DEF_AB}f(1, b=2)\n")),
        vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]
    );
}

#[test]
fn a_keyword_call_written_before_its_own_def_still_binds() {
    // The table is collected from the whole module body before any item
    // is lowered, so forward references bind like any other call.
    assert_eq!(
        first_call_args(&format!(
            "def g() -> None:\n    f(b=2, a=1)\n\n{DEF_AB}g()\n"
        )),
        vec![]
    );
    let hir = lower(&format!(
        "def g() -> None:\n    f(b=2, a=1)\n\n{DEF_AB}g()\n"
    ));
    let body = function_bodies(&hir)
        .into_iter()
        .find_map(|(name, body)| (name == "g").then_some(body))
        .expect("the module must hold `g`");
    assert_eq!(
        body[0],
        HirStmt::ExprStmt(HirExpr::Call {
            callee: "f".to_string(),
            args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
        })
    );
}

#[test]
fn a_keyword_call_inside_a_method_body_binds_too() {
    // `ClassBodyInput` forwards the table to every method body. The
    // compiler cannot catch a missing forward there -- an empty table
    // would merely make this call fall back to `C0001` -- so this test
    // is the only guard on that seam.
    let hir = lower(&format!(
        "{DEF_AB}class C:\n    def m(self) -> None:\n        f(b=2, a=1)\n"
    ));
    let call = function_bodies(&hir)
        .into_iter()
        .find_map(|(_, body)| {
            body.iter().find_map(|stmt| match stmt {
                HirStmt::ExprStmt(call @ HirExpr::Call { callee, .. }) if callee == "f" => {
                    Some(call.clone())
                }
                _ => None,
            })
        })
        .expect("the method body must hold the bound call");
    assert_eq!(
        call,
        HirExpr::Call {
            callee: "f".to_string(),
            args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
        }
    );
}

#[test]
fn a_keyword_naming_a_parameter_after_the_slash_marker_binds() {
    assert_eq!(
        first_call_args(
            "def f(a: int, /, b: int) -> None:\n    print(a)\n    print(b)\n\nf(1, b=2)\n"
        ),
        vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]
    );
}

#[test]
fn an_unexpected_keyword_name_is_a_type_error_at_the_keyword_span() {
    let source = format!("{DEF_AB}f(1, 2, c=3)\n");
    let diagnostic = lower_err(&source);
    assert_eq!(diagnostic.code, "T0021");
    assert_eq!(
        diagnostic.message,
        "`f` got an unexpected keyword argument `c`"
    );
    let start = u32::try_from(source.find("c=3").expect("fixture holds `c=3`"))
        .expect("fixture is short");
    assert_eq!(
        diagnostic.span,
        Some(pycc_diag::Span::new(start, start + 3))
    );
    assert_eq!(diagnostic.help.as_deref(), Some("`f` accepts `a`, `b`"));
}

#[test]
fn a_stray_keyword_on_a_zero_parameter_callee_reports_no_bindable_names() {
    let diagnostic = lower_err("def f() -> None:\n    return\n\nf(extra=1)\n");
    assert_eq!(diagnostic.code, "T0021");
    assert_eq!(
        diagnostic.message,
        "`f` got an unexpected keyword argument `extra`"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("`f` accepts no keyword arguments")
    );
}

#[test]
fn a_positional_only_parameter_passed_as_a_keyword_is_a_type_error() {
    let diagnostic = lower_err(
        "def f(a: int, /, b: int) -> None:\n    print(a)\n    print(b)\n\nf(a=1, b=2)\n",
    );
    assert_eq!(diagnostic.code, "T0021");
    assert_eq!(
        diagnostic.message,
        "`f` got positional-only parameter `a` passed as a keyword argument"
    );
    assert_eq!(diagnostic.help.as_deref(), Some("pass `a` positionally"));
}

#[test]
fn a_parameter_supplied_twice_is_a_type_error() {
    let diagnostic = lower_err(&format!("{DEF_AB}f(1, a=2, b=3)\n"));
    assert_eq!(diagnostic.code, "T0021");
    assert_eq!(
        diagnostic.message,
        "`f` got multiple values for argument `a`"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("`a` is already supplied positionally at this call")
    );
}

#[test]
fn a_parameter_left_unsupplied_is_a_type_error_at_the_call_span() {
    let diagnostic = lower_err(&format!("{DEF_AB}f(a=1)\n"));
    assert_eq!(diagnostic.code, "T0021");
    assert_eq!(
        diagnostic.message,
        "`f` is missing required argument(s): `b`"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("pass exactly 2 argument(s)")
    );
}

#[test]
fn more_positional_arguments_than_parameters_is_a_type_error() {
    let diagnostic = lower_err("def f(a: int) -> None:\n    print(a)\n\nf(1, 2, a=3)\n");
    assert_eq!(diagnostic.code, "T0021");
    assert_eq!(diagnostic.message, "`f` expects 1 argument(s), got 3");
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("pass exactly 1 argument(s)")
    );
}

#[test]
fn a_keyword_call_to_a_name_bound_twice_keeps_the_capability_rejection() {
    // Runtime dispatch of a redefined name is source-order sensitive,
    // while the table is static: neither `def`'s parameter order may
    // bind the keywords, so the name is left out of the table entirely.
    // Since Part 8 of #1371 the bare-name call is deferred to
    // `pycc_types` (`keyword_call_refused_here`), which refuses it
    // because the name is not a CPython object.
    for (source, refused_here) in [
        (
            "def f(a: int, b: int) -> None:\n    print(a - b)\n\nf(a=10, b=1)\n\n\
             def f(b: int, a: int) -> None:\n    print(a - b)\n\nf(a=10, b=1)\n",
            false,
        ),
        (
            "f = 3\ndef f(a: int, b: int) -> None:\n    print(a - b)\n\nf(a=10, b=1)\n",
            false,
        ),
        (
            "from math import sqrt\ndef sqrt(a: float, b: float) -> float:\n    return a - b\n\nprint(sqrt(b=1.0, a=10.0))\n",
            false,
        ),
    ] {
        assert_eq!(
            keyword_call_refused_here(source),
            refused_here,
            "source: {source}"
        );
    }
}

#[test]
fn a_kwargs_unpacking_keeps_the_unchanged_capability_rejection() {
    let diagnostic = lower_err(&format!("{DEF_AB}d = {{}}\nf(**d)\n"));
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(
        diagnostic.message,
        "keyword call arguments are not supported yet"
    );
}

#[test]
fn a_def_using_a_parameter_kind_the_table_excludes_is_never_bindable() {
    // Each of these `def`s is rejected by `func::lower_params` on its own
    // terms; the point here is that `signature_of` returns `None` for it
    // rather than recording a signature pycc cannot represent.
    for (source, message) in [
        (
            "def f(*args) -> None:\n    return\n",
            "`*args` is not supported yet",
        ),
        (
            "def f(*, a: int) -> None:\n    print(a)\n",
            "keyword-only parameters are not supported yet",
        ),
        (
            "def f(**kwargs) -> None:\n    return\n",
            "`**kwargs` is not supported yet",
        ),
        // A default value used to be the fourth tuple here. Part 2 of
        // #884 (#1189) admits an admitted-literal default into the
        // table, so that shape now binds -- see
        // `a_default_is_filled_at_a_short_call` below. A default the
        // recognizer does *not* admit is still excluded, which the
        // syntactically-unadmitted tests below cover at the `def`.
    ] {
        let diagnostic = lower_err(&format!("{source}\nf(a=1)\n"));
        assert_eq!(diagnostic.code, "C0001", "source: {source}");
        assert_eq!(diagnostic.message, message, "source: {source}");
    }
}

#[test]
fn every_call_shape_outside_the_bindable_one_is_refused_or_deferred() {
    // `false`: deferred to `pycc_types` since Part 8 of #1371, because
    // the callee could be a CPython object (`keyword_call_refused_here`).
    for (source, refused_here) in [
        // An undefined callee is not in the table.
        ("undefined_callee(extra=1)\n", false),
        // A method call.
        (
            "class C:\n    def m(self, a: int) -> None:\n        print(a)\n\nc = C()\nc.m(a=1)\n",
            false,
        ),
        // A `super()` method call.
        (
            "class B:\n    def m(self, a: int) -> None:\n        print(a)\n\nclass C(B):\n    def m(self, a: int) -> None:\n        super().m(a=1)\n",
            true,
        ),
        // A bare `super()` call.
        (
            "class C:\n    def m(self) -> None:\n        super(x=1)\n",
            true,
        ),
        // A class instantiation.
        (
            "class C:\n    def __init__(self, a: int) -> None:\n        self.a = a\n\nc = C(a=1)\n",
            false,
        ),
        // A builtin.
        ("print(sep=1)\n", false),
        // A container method.
        ("xs = [1]\nxs.append(value=2)\n", true),
        // A stdlib intrinsic through an aliased receiver.
        ("import math as m\n\nx = m.sqrt(x=1.0)\n", true),
        // A generic class instantiation through a subscript callee.
        (
            "class C[T]:\n    def __init__(self, a: int) -> None:\n        self.a = a\n\nc = C[int](a=1)\n",
            true,
        ),
        // `**` unpacking into a callee that may be an object is deferred.
        ("d = {}\nundefined_callee(**d)\n", false),
    ] {
        assert_eq!(
            keyword_call_refused_here(source),
            refused_here,
            "source: {source}"
        );
    }
}

#[test]
fn a_keyword_range_argument_stays_rejected_in_both_positions() {
    for source in [
        "for i in range(stop=3):\n    print(i)\n",
        "xs = [i for i in range(stop=3)]\n",
    ] {
        let diagnostic = lower_err(source);
        assert_eq!(diagnostic.code, "C0001", "source: {source}");
    }
}

// --- Part 2 of #884 (#1189): defaults filled at a short call ---------

const DEF_A_B2: &str = "def f(a: int, b: int = 2) -> None:\n    print(a)\n    print(b)\n\n";

/// Asserts that omitting a defaulted argument produces *exactly* the
/// argument vector writing the same literal explicitly produces.
///
/// This is the design's load-bearing invariant stated as a test: a
/// filled default is byte-identical to its explicit twin, so it can
/// never be checked more or less strictly than the same value at the
/// call site.
fn assert_default_matches_explicit(annotation: &str, literal: &str, expected: HirExpr) {
    let def = format!("def f(a: {annotation} = {literal}) -> None:\n    return\n\n");
    let filled = first_call_args(&format!("{def}f()\n"));
    let explicit = first_call_args(&format!("{def}f({literal})\n"));
    assert_eq!(
        filled, explicit,
        "annotation `{annotation}`, default `{literal}`"
    );
    assert_eq!(
        filled,
        vec![expected],
        "annotation `{annotation}`, default `{literal}`"
    );
}

#[test]
fn a_default_is_filled_at_a_short_call() {
    assert_eq!(
        first_call_args(&format!("{DEF_A_B2}f(1)\n")),
        first_call_args(&format!("{DEF_A_B2}f(1, 2)\n"))
    );
    assert_eq!(
        first_call_args(&format!("{DEF_A_B2}f(1)\n")),
        vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]
    );
}

#[test]
fn every_call_shape_reaches_the_same_argument_vector() {
    let full = vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(3)];
    for call in ["f(1, 3)", "f(1, b=3)", "f(b=3, a=1)", "f(a=1, b=3)"] {
        assert_eq!(
            first_call_args(&format!("{DEF_A_B2}{call}\n")),
            full,
            "call: {call}"
        );
    }
    let defaulted = vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)];
    for call in ["f(1)", "f(a=1)"] {
        assert_eq!(
            first_call_args(&format!("{DEF_A_B2}{call}\n")),
            defaulted,
            "call: {call}"
        );
    }
}

#[test]
fn each_admitted_literal_default_matches_its_explicit_twin() {
    assert_default_matches_explicit("int", "7", HirExpr::IntLiteral(7));
    assert_default_matches_explicit("float", "1.5", HirExpr::FloatLiteral(1.5));
    assert_default_matches_explicit("bool", "True", HirExpr::BoolLiteral(true));
    assert_default_matches_explicit("bool", "False", HirExpr::BoolLiteral(false));
    assert_default_matches_explicit("str", "\"hi\"", HirExpr::StringLiteral("hi".to_string()));
    assert_default_matches_explicit("int | None", "None", HirExpr::NoneLiteral);
}

#[test]
fn a_folded_sign_on_a_numeric_default_matches_its_explicit_twin() {
    assert_default_matches_explicit("int", "-1", HirExpr::IntLiteral(-1));
    assert_default_matches_explicit("int", "+1", HirExpr::IntLiteral(1));
    assert_default_matches_explicit("float", "-1.5", HirExpr::FloatLiteral(-1.5));
    assert_default_matches_explicit("float", "+1.5", HirExpr::FloatLiteral(1.5));
}

#[test]
fn both_i64_extremes_are_admitted_as_defaults() {
    assert_default_matches_explicit(
        "int",
        "9223372036854775807",
        HirExpr::IntLiteral(i64::MAX),
    );
    // The magnitude alone overflows `i64`; `fold_int_literal_sign`
    // applies the sign before the range check, exactly as the same
    // literal written at the call site is folded.
    assert_default_matches_explicit(
        "int",
        "-9223372036854775808",
        HirExpr::IntLiteral(i64::MIN),
    );
}

#[test]
fn a_def_whose_parameters_all_have_defaults_accepts_a_zero_argument_call() {
    assert_eq!(
        first_call_args("def f(a: int = 1, b: str = \"s\") -> None:\n    return\n\nf()\n"),
        vec![
            HirExpr::IntLiteral(1),
            HirExpr::StringLiteral("s".to_string())
        ]
    );
}

#[test]
fn a_default_on_either_side_of_the_slash_marker_is_filled() {
    assert_eq!(
        first_call_args("def f(a: int = 1, /, b: int = 2) -> None:\n    return\n\nf()\n"),
        vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)]
    );
    assert_eq!(
        first_call_args("def f(a: int = 1, /, b: int = 2) -> None:\n    return\n\nf(9)\n"),
        vec![HirExpr::IntLiteral(9), HirExpr::IntLiteral(2)]
    );
}

/// A parameter after the `/` marker stays bindable by name, so one call
/// can drive the keyword loop and the default-fill loop together: `b` is
/// named, `a` is filled from its default. This is the only shape where
/// both loops act on the same call.
#[test]
fn a_keyword_after_the_slash_marker_binds_while_the_default_before_it_fills() {
    const DEF: &str = "def f(a: int = 1, /, b: int = 2) -> None:\n    return\n\n";
    assert_eq!(
        first_call_args(&format!("{DEF}f(b=3)\n")),
        vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(3)]
    );
    assert_eq!(
        first_call_args(&format!("{DEF}f(9, b=3)\n")),
        vec![HirExpr::IntLiteral(9), HirExpr::IntLiteral(3)]
    );
}

#[test]
fn a_short_call_to_a_name_bound_twice_is_left_unfilled() {
    // Neither `def`'s default is spliced: the call reaches `pycc_types`
    // exactly as written, and its arity check rejects it there.
    assert_eq!(
        first_call_args(
            "def f(a: int = 1) -> None:\n    return\n\nf()\n\ndef f(a: int = 2) -> None:\n    return\n\nf()\n"
        ),
        vec![]
    );
}

#[test]
fn an_unannotated_private_def_still_fills_its_default() {
    // No type is inferred from a default: `a` keeps `Ty::Infer`, and the
    // fill is the same literal an explicit call would carry.
    assert_eq!(
        first_call_args("def _f(a = 1):\n    return\n\n_f()\n"),
        vec![HirExpr::IntLiteral(1)]
    );
}

#[test]
fn a_def_with_no_default_leaves_a_short_call_untouched() {
    // `needs_default_fill` answers `false` for a signature without a
    // single default, so a zero-keyword short call reaches `pycc_types`
    // with exactly the argument vector the user wrote, and the arity
    // error stays that crate's to report.
    assert_eq!(
        first_call_args(&format!("{DEF_AB}f(1)\n")),
        vec![HirExpr::IntLiteral(1)]
    );
}

#[test]
fn a_short_call_to_a_callee_outside_the_table_is_left_alone() {
    // `needs_default_fill` answers `false` for an unknown callee, so the
    // zero-keyword path must not touch the argument vector.
    assert_eq!(
        first_call_args("undefined_callee(1)\n"),
        vec![HirExpr::IntLiteral(1)]
    );
}

#[test]
fn a_default_the_recognizer_rejects_keeps_the_def_out_of_the_table() {
    // `signature_of` is infallible and purely syntactic: a `def` whose
    // default it cannot represent is simply absent from the table, so a
    // keyword call against it is never bound against a signature pycc
    // cannot materialize. Since Part 8 of #1371 the call is deferred to
    // `pycc_types` (which refuses it, `f` not being a CPython object),
    // so the `def`'s own `C0001` is the only diagnostic lowering reports.
    let source = "def f(a: int = len(\"x\")) -> None:\n    return\n\nf(a=1)\n";
    let module = crate::pycc_parser_test_helper::parse(source);
    let table = super::SignatureTable::collect(&module.body, |_| false);
    assert!(!table.binds("f"));
    let diagnostics = crate::lower_all(&module).expect_err("fixture must be rejected");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_ne!(
        diagnostics[0].message,
        crate::KEYWORD_CALL_UNSUPPORTED,
        "{diagnostics:?}"
    );
}

#[test]
fn a_default_that_only_the_type_rule_rejects_is_still_in_the_table() {
    // The syntactic recognizer admits `1` at a `float` parameter; the
    // *type* rule at the `def` rejects it. The `def`'s own diagnostic is
    // what the user sees -- the call site adds nothing.
    let diagnostic = lower_err("def f(a: float = 1) -> None:\n    return\n\nf()\n");
    assert_eq!(diagnostic.code, "T0021");
    assert_eq!(
        diagnostic.message,
        "default value of parameter `a` of `f` expects `float`, got `int`"
    );
}

#[test]
fn a_defaulted_def_leaves_an_over_long_call_untouched() {
    // `needs_default_fill` gates on `positional_len < names.len()`, so a
    // call with too many arguments is never rewritten and `pycc_types`
    // still sees the vector the user wrote.
    assert_eq!(
        first_call_args(&format!("{DEF_A_B2}f(1, 2, 3)\n")),
        vec![
            HirExpr::IntLiteral(1),
            HirExpr::IntLiteral(2),
            HirExpr::IntLiteral(3)
        ]
    );
}

#[test]
fn a_required_parameter_before_a_defaulted_one_is_still_required() {
    let diagnostic = lower_err(&format!("{DEF_A_B2}f(b=3)\n"));
    assert_eq!(diagnostic.code, "T0021");
    assert_eq!(
        diagnostic.message,
        "`f` is missing required argument(s): `a`"
    );
}

/// The zero-keyword default-fill route in `expr.rs` can still fail: a
/// call short of the callee's arity reaches the binder, the defaulted
/// parameters are filled, and a *required* parameter that is still
/// unsupplied is reported from there. This pins the error-propagation
/// arm of that route, which the keyword route above does not exercise.
#[test]
fn a_zero_keyword_call_missing_a_required_argument_is_reported_from_the_binder() {
    let diagnostic = lower_err(&format!("{DEF_A_B2}f()\n"));
    assert_eq!(diagnostic.code, "T0021");
    assert_eq!(
        diagnostic.message,
        "`f` is missing required argument(s): `a`"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("pass exactly 2 argument(s)")
    );
}
