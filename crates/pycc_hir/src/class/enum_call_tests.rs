//! Unit tests for `enum_call` (split out so the scanner itself stays
//! under the ~1,000-line decomposition threshold).

use super::enum_class_call_message;
use crate::lower_all;
use pycc_diag::{Diagnostic, Span};

fn lower_err(source: &str) -> Vec<Diagnostic> {
    let module = crate::pycc_parser_test_helper::parse(source);
    lower_all(&module).expect_err("test fixture should be rejected")
}

/// The shapes whose call is suppressed by a frame lower `Ok` here; the
/// CLI then reaches `pycc_types`, which reports the accurate `T0021`
/// (or, for the module-frame cases, the span-less guard).
fn lower_ok(source: &str) {
    let module = crate::pycc_parser_test_helper::parse(source);
    lower_all(&module).expect("the frame must suppress the enum-call scan");
}

/// Byte offset of the `occurrence`-th (0-based) `needle` in `source`.
fn nth_offset(source: &str, needle: &str, occurrence: usize) -> u32 {
    source
        .match_indices(needle)
        .nth(occurrence)
        .map(|(start, _)| start as u32)
        .expect("the call text must occur in the source that many times")
}

fn assert_enum_call_at(diagnostic: &Diagnostic, class_name: &str, call_text: &str, start: u32) {
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(diagnostic.message, enum_class_call_message(class_name));
    assert_eq!(
        diagnostic.span,
        Some(Span::new(start, start + call_text.len() as u32))
    );
}

fn assert_enum_call(diagnostic: &Diagnostic, class_name: &str, call_text: &str, source: &str) {
    assert_enum_call_at(
        diagnostic,
        class_name,
        call_text,
        nth_offset(source, call_text, 0),
    );
}

const COLOR: &str = "class Color(Enum):\n    RED = 1\n    GREEN = 2\n";

// -- the reference shapes (#921) --

#[test]
fn a_module_level_zero_argument_call_is_rejected_at_the_call() {
    let source = format!("{COLOR}c = Color()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1);
    assert_enum_call(&diagnostics[0], "Color", "Color()", &source);
}

#[test]
fn a_value_lookup_call_inside_a_function_body_is_rejected() {
    let source = format!("{COLOR}def f() -> None:\n    c = Color(1)\n    print(c.value)\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1);
    assert_enum_call(&diagnostics[0], "Color", "Color(1)", &source);
}

#[test]
fn calls_before_and_after_the_class_are_both_found_in_loop_order() {
    // The `def` precedes the class in source, so only the syntactic
    // pre-collection can know `Color` when the `def` is scanned.
    let source = format!("def f() -> None:\n    Color(1)\n{COLOR}Color(2)\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 2);
    assert_enum_call(&diagnostics[0], "Color", "Color(1)", &source);
    assert_enum_call(&diagnostics[1], "Color", "Color(2)", &source);
}

#[test]
fn a_nested_call_is_found() {
    let source = format!("{COLOR}print(Color(1))\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1);
    assert_enum_call(&diagnostics[0], "Color", "Color(1)", &source);
}

#[test]
fn a_non_enum_class_call_and_a_member_access_are_untouched() {
    let source = format!(
        "class P:\n    def __init__(self, x: int) -> None:\n        self.x = x\n{COLOR}p = P(1)\nc = Color.RED\nprint(c.value)\n"
    );
    lower_ok(&source);
}

#[test]
fn a_str_enum_class_call_is_rejected_the_same_way() {
    let source = "class S(StrEnum):\n    A = \"a\"\ns = S(\"a\")\n";
    let diagnostics = lower_err(source);
    assert_eq!(diagnostics.len(), 1);
    assert_enum_call(&diagnostics[0], "S", "S(\"a\")", source);
}

#[test]
fn a_docstring_only_enum_class_call_is_rejected() {
    // #744 accepts a member-less enum; `enum_members` is empty, so only
    // the provenance marker (`is_enum`/the syntactic set) can catch it.
    let source = "class E(Enum):\n    \"doc\"\ne = E()\n";
    let diagnostics = lower_err(source);
    assert_eq!(diagnostics.len(), 1);
    assert_enum_call(&diagnostics[0], "E", "E()", source);
}

#[test]
fn a_call_to_a_poisoned_enum_class_is_a_suppressed_cascade() {
    // `class E(Enum): pass` fails to lower and poisons `E` (D-219); the
    // later `E()` is a consequence of that skip, not a second gap.
    let source = "class E(Enum):\n    pass\ne = E()\n";
    let diagnostics = lower_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "C0001");
    assert_ne!(diagnostics[0].message, enum_class_call_message("E"));
}

#[test]
fn a_cascade_silenced_item_still_reports_a_call_to_another_enum_class() {
    // `class E(Enum): pass` fails and poisons `E`; `def f(e: E)` then fails
    // only because its annotation names the poisoned class, so its own
    // diagnostic is silenced as a D-219 cascade. The scan still runs over
    // that item (D-233 decision 6), so the `Color()` inside it is reported.
    let source = format!("{COLOR}class E(Enum):\n    pass\ndef f(e: E) -> None:\n    Color()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    assert_ne!(diagnostics[0].message, enum_class_call_message("E"));
    assert_enum_call(&diagnostics[1], "Color", "Color()", &source);
}

#[test]
fn a_call_to_a_redefined_enum_class_is_suppressed_with_the_duplicate() {
    // D-219 poisons the name of *any* failing `class` statement, so the
    // duplicate `class Color:` poisons `Color` and the later `Color()`
    // is filtered as a cascade even though the first definition is a
    // real enum. Pinned as a documented limit (see the function doc),
    // not as the preferred outcome: the module still fails to compile.
    let source = format!("{COLOR}class Color:\n    pass\nc = Color()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    let message = &diagnostics[0].message;
    assert!(message.contains("defined more than once"), "{message}");
}

#[test]
fn a_keyword_call_is_deferred_with_no_lowering_diagnostic() {
    // Since Part 8 of #1371 the keyword call lowers to a
    // `HirExpr::KeywordCall` and `pycc_types` reports its one `C0001`, so
    // the skipped scan leaves lowering with no diagnostic at all.
    lower_ok(&format!("{COLOR}c = Color(value=1)\n"));
}

#[test]
fn a_class_whose_base_is_not_a_bare_name_is_not_an_enum_class() {
    // `class Color(enum.Enum):` takes the pre-collection's non-`Name`
    // arm and is rejected by `lower_class`'s own base-shape `C0001`;
    // the later `Color()` is then a poisoned-name cascade.
    let source = "class Color(enum.Enum):\n    RED = 1\nc = Color()\n";
    let diagnostics = lower_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    let message = &diagnostics[0].message;
    assert!(
        message.contains("a base class must be a bare name"),
        "{message}"
    );
}

// -- scope-local bindings keep their `T0021` (#944): the item lowers --

#[test]
fn a_parameter_that_shadows_the_enum_suppresses_the_scan() {
    lower_ok(&format!("{COLOR}def f(Color: int) -> None:\n    Color()\n"));
}

#[test]
fn a_local_assignment_that_shadows_the_enum_suppresses_the_scan() {
    lower_ok(&format!(
        "{COLOR}def f() -> None:\n    Color = 1\n    Color()\n"
    ));
}

#[test]
fn an_except_handler_name_suppresses_a_call_after_it() {
    lower_ok(&format!(
        "{COLOR}def f() -> None:\n    try:\n        pass\n    except ValueError as Color:\n        Color()\n"
    ));
}

#[test]
fn a_module_level_except_handler_without_a_name_binds_nothing() {
    // The module frame is built lazily (only for a module with an enum
    // class), so this shape is what walks a name-less handler there.
    let source = format!("{COLOR}try:\n    pass\nexcept ValueError:\n    Color()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1);
    assert_enum_call(&diagnostics[0], "Color", "Color()", &source);
}

#[test]
fn an_except_handler_name_suppresses_a_call_before_it() {
    // The frame is the whole scope, not position-aware (limit (i)).
    lower_ok(&format!(
        "{COLOR}def f() -> None:\n    Color()\n    try:\n        pass\n    except ValueError as Color:\n        pass\n"
    ));
}

#[test]
fn a_match_capture_suppresses_the_scan() {
    lower_ok(&format!(
        "{COLOR}def f(x: int) -> None:\n    match x:\n        case Color:\n            Color()\n"
    ));
}

#[test]
fn a_match_star_capture_suppresses_the_scan() {
    lower_ok(&format!(
        "{COLOR}def f(xs: list[int]) -> None:\n    match xs:\n        case [*Color]:\n            Color()\n"
    ));
}

#[test]
fn a_match_mapping_rest_capture_suppresses_the_scan() {
    lower_ok(&format!(
        "{COLOR}def f(d: dict[str, int]) -> None:\n    match d:\n        case {{**Color}}:\n            Color()\n"
    ));
}

#[test]
fn a_module_level_assignment_that_shadows_the_enum_suppresses_the_scan() {
    // Side by side with `a_module_level_zero_argument_call_is_rejected_at_the_call`:
    // `class Color(Enum)` itself is not a module-frame binding, a plain
    // `Color = 1` is.
    lower_ok(&format!("{COLOR}Color = 1\nColor()\n"));
}

#[test]
fn a_module_level_for_target_that_shadows_the_enum_suppresses_the_scan() {
    lower_ok(&format!("{COLOR}for Color in range(3):\n    Color()\n"));
}

#[test]
fn a_module_level_call_before_a_later_shadowing_assignment_is_suppressed() {
    // Limit (i): the module frame is not position-aware, so the later
    // `Color = 1` hides the earlier `Color()`; `pycc_types`' span-less
    // guard still rejects it.
    lower_ok(&format!("{COLOR}Color()\nColor = 1\n"));
}

#[test]
fn a_comprehension_target_suppresses_a_sibling_call_in_the_same_def() {
    // Limit (i): the comprehension target is recorded in the enclosing
    // `def`'s frame, so a sibling `Color(1)` outside the comprehension
    // is suppressed too; `pycc_types` still rejects it.
    lower_ok(&format!(
        "{COLOR}def f() -> None:\n    xs = [Color for Color in range(3)]\n    Color(1)\n"
    ));
}

// -- `TYPE_CHECKING` guards (#790 fold, PR #971 review): dead bodies --

const TC: &str = "from typing import TYPE_CHECKING\n";

#[test]
fn a_call_under_a_type_checking_guard_is_dead_code_and_not_scanned() {
    lower_ok(&format!("{TC}{COLOR}if TYPE_CHECKING:\n    Color(1)\n"));
}

#[test]
fn a_call_under_an_elif_type_checking_guard_is_dead_code_and_not_scanned() {
    lower_ok(&format!(
        "{TC}{COLOR}x = 1\nif x == 2:\n    pass\nelif TYPE_CHECKING:\n    Color(1)\n"
    ));
}

#[test]
fn a_qualified_type_checking_guard_inside_a_def_is_folded_the_same_way() {
    lower_ok(&format!(
        "import typing\n{COLOR}def f() -> None:\n    if typing.TYPE_CHECKING:\n        Color(1)\n"
    ));
}

#[test]
fn the_live_clauses_around_a_type_checking_guard_are_still_scanned() {
    // The leading `if` is live (not a guard), the `elif` is folded, the
    // `else` is live: exactly the two live calls, in source order.
    let source = format!(
        "{TC}{COLOR}x = 1\nif x == 2:\n    Color()\nelif TYPE_CHECKING:\n    Color(1)\nelse:\n    Color(2)\n"
    );
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    assert_enum_call(&diagnostics[0], "Color", "Color()", &source);
    assert_enum_call(&diagnostics[1], "Color", "Color(2)", &source);
}

#[test]
fn the_else_of_a_leading_type_checking_guard_is_live() {
    let source = format!("{TC}{COLOR}if TYPE_CHECKING:\n    pass\nelse:\n    Color()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_enum_call(&diagnostics[0], "Color", "Color()", &source);
}

#[test]
fn a_module_level_binding_under_a_type_checking_guard_does_not_shadow() {
    // The guarded `Color = 1` never runs, so the runtime `Color` is the
    // enum and the call is reported -- the frame skips the dead body.
    let source = format!("{TC}{COLOR}if TYPE_CHECKING:\n    Color = 1\nColor()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_enum_call(&diagnostics[0], "Color", "Color()", &source);
}

#[test]
fn a_def_level_binding_under_a_type_checking_guard_does_not_shadow() {
    let source = format!(
        "{TC}{COLOR}def f() -> None:\n    if TYPE_CHECKING:\n        Color = 1\n    Color()\n"
    );
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_enum_call(&diagnostics[0], "Color", "Color()", &source);
}

#[test]
fn an_aliased_module_level_guard_binding_is_the_documented_frame_residual() {
    // Limit (iv): the module frame is computed before the loop lowers
    // `import typing as t`, so `t.TYPE_CHECKING` is not recognized there
    // and the dead `Color = 1` still suppresses the call (over-suppression;
    // `pycc_types`' guard rejects it). Inside a `def` the scan runs with
    // the import known, so the same alias folds -- see the next test.
    lower_ok(&format!(
        "import typing as t\n{COLOR}if t.TYPE_CHECKING:\n    Color = 1\nColor()\n"
    ));
}

#[test]
fn an_aliased_guard_inside_a_def_is_folded_once_the_import_is_lowered() {
    let source = format!(
        "import typing as t\n{COLOR}def f() -> None:\n    if t.TYPE_CHECKING:\n        Color = 1\n    Color()\n"
    );
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_enum_call(&diagnostics[0], "Color", "Color()", &source);
}

// -- frame boundaries and item ordering (#944): exact counts --

#[test]
fn a_lambda_parameter_shadow_keeps_exactly_the_lambda_diagnostic() {
    let source = format!("{COLOR}g = lambda Color: Color()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    let message = &diagnostics[0].message;
    assert!(message.contains("a `lambda`"), "{message}");
}

#[test]
fn a_parameter_less_lambda_body_call_is_reported_after_the_lambda_diagnostic() {
    let source = format!("{COLOR}g = lambda: Color()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    assert!(diagnostics[0].message.contains("a `lambda`"));
    assert_enum_call(&diagnostics[1], "Color", "Color()", &source);
}

#[test]
fn a_walrus_inside_a_lambda_does_not_bind_the_module_frame() {
    // The binder does not descend into a lambda: the walrus binds in the
    // lambda's own scope, so the module-level `Color()` is still found
    // (after the lambda's own `C0001`, the item's first diagnostic).
    let source = format!("{COLOR}g = lambda: (Color := 1)\nColor()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    assert!(diagnostics[0].message.contains("a `lambda`"));
    assert_enum_call(&diagnostics[1], "Color", "Color()", &source);
}

#[test]
fn a_walrus_in_a_parameter_default_binds_the_module_frame() {
    // Default values are evaluated at definition time in the enclosing
    // scope: `Color` is rebound to an `int` at module level, so the later
    // `Color()` is left to the type checker. The `def` itself is the one
    // diagnostic: a walrus is outside the default subset Part 2 of #884
    // (#1189) admits.
    let source = format!("{COLOR}def f(x: int = (Color := 1)) -> None:\n    pass\nColor()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "C0001", "{diagnostics:?}");
    assert!(
        diagnostics[0]
            .message
            .starts_with("only a literal `int`, `float`, `bool`, `str`, or `None` default"),
        "{diagnostics:?}"
    );
}

#[test]
fn a_walrus_in_a_default_of_an_unannotated_def_binds_the_module_frame() {
    // Same as above without a return annotation: the binder walks the
    // parameters alone and still rebinds `Color` in the module frame.
    let source = format!("{COLOR}def f(x: int = (Color := 1)):\n    pass\nColor()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "C0001", "{diagnostics:?}");
    assert!(
        diagnostics[0]
            .message
            .starts_with("only a literal `int`, `float`, `bool`, `str`, or `None` default"),
        "{diagnostics:?}"
    );
}

#[test]
fn a_walrus_in_a_decorator_binds_the_module_frame() {
    let source = format!("{COLOR}@(Color := 1)\ndef f() -> None:\n    pass\nColor()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        !diagnostics[0].message.contains("enum class"),
        "{diagnostics:?}"
    );
}

#[test]
fn a_walrus_in_a_return_annotation_binds_the_module_frame() {
    let source = format!("{COLOR}def f() -> (Color := 1):\n    pass\nColor()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        !diagnostics[0].message.contains("enum class"),
        "{diagnostics:?}"
    );
}

#[test]
fn a_walrus_in_a_class_base_binds_the_module_frame() {
    let source = format!("{COLOR}class P((Color := 1)):\n    pass\nColor()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        !diagnostics[0].message.contains("enum class"),
        "{diagnostics:?}"
    );
}

#[test]
fn a_walrus_in_a_type_parameter_bound_binds_the_module_frame() {
    // A type-parameter bound is not lowered, so the definition itself
    // passes; the rebound `Color()` is left to the type checker.
    lower_ok(&format!(
        "{COLOR}def f[T: (Color := 1)]() -> None:\n    pass\nColor()\n"
    ));
}

#[test]
fn a_walrus_in_a_class_decorator_binds_the_module_frame() {
    let source = format!("{COLOR}@(Color := 1)\nclass P:\n    pass\nColor()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        !diagnostics[0].message.contains("enum class"),
        "{diagnostics:?}"
    );
}

#[test]
fn a_walrus_in_a_class_type_parameter_binds_the_module_frame() {
    // A type-parameter bound is not lowered, so the definition itself
    // passes; the rebound `Color()` is left to the type checker.
    lower_ok(&format!(
        "{COLOR}class P[T: (Color := 1)]:\n    pass\nColor()\n"
    ));
}

#[test]
fn a_walrus_in_a_function_body_does_not_bind_the_module_frame() {
    // The body is the `def`'s own scope: the module-level `Color()` is
    // still an attributable enum call.
    let source = format!("{COLOR}def f() -> None:\n    (Color := 1)\nColor()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_enum_call(&diagnostics[0], "Color", "Color()", &source);
}

#[test]
fn a_value_less_module_annotation_does_not_bind_the_module_frame() {
    // `Color: int` at module level declares metadata and leaves the class
    // bound, so the call is still an attributable enum call.
    let source = format!("{COLOR}Color: int\nColor()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_enum_call(&diagnostics[0], "Color", "Color()", &source);
}

#[test]
fn a_module_annotation_with_a_value_binds_the_module_frame() {
    lower_ok(&format!("{COLOR}Color: int = 3\nColor()\n"));
}

#[test]
fn a_value_less_annotation_inside_a_function_still_makes_the_name_local() {
    lower_ok(&format!(
        "{COLOR}def f() -> None:\n    Color: int\n    Color()\n"
    ));
}

#[test]
fn a_walrus_in_a_lambda_body_binds_the_lambda_frame() {
    // `Color` is the lambda-local `int` for the sibling call: only the
    // lambda's own diagnostic remains.
    let source = format!("{COLOR}g = lambda: ((Color := 1), Color())\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].message.contains("a `lambda`"));
}

#[test]
fn a_walrus_in_a_nested_lambda_does_not_bind_the_outer_lambda_frame() {
    let source = format!("{COLOR}g = lambda: (lambda: (Color := 1), Color())\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    assert!(diagnostics[0].message.contains("a `lambda`"));
    assert_enum_call(&diagnostics[1], "Color", "Color()", &source);
}

#[test]
fn a_shadow_in_one_function_does_not_leak_into_a_sibling() {
    let source = format!(
        "{COLOR}def f() -> None:\n    Color = 1\n    Color()\ndef g() -> None:\n    Color()\n"
    );
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_enum_call_at(
        &diagnostics[0],
        "Color",
        "Color()",
        nth_offset(&source, "Color()", 1),
    );
}

#[test]
fn a_shadow_inside_a_function_does_not_bind_the_module_frame() {
    let source = format!("{COLOR}def g() -> None:\n    Color = 1\nColor()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_enum_call(&diagnostics[0], "Color", "Color()", &source);
}

#[test]
fn a_raised_enum_call_is_rejected_at_the_call() {
    let source = format!("{COLOR}raise Color()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_enum_call(&diagnostics[0], "Color", "Color()", &source);
}

#[test]
fn a_failed_item_carries_its_own_diagnostic_before_the_scan_s() {
    let source = format!("{COLOR}def f() -> None:\n    with open(\"x\") as y:\n        Color()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "C0001");
    let message = &diagnostics[0].message;
    assert!(message.contains("with"), "{message}");
    assert_enum_call(&diagnostics[1], "Color", "Color()", &source);
}

#[test]
fn a_call_scanned_before_its_class_fails_is_reported_first() {
    // Limit (iv): the `def` is scanned before the class item poisons
    // `Color`, so the true enum-call report precedes the class's own.
    let source = "def f() -> None:\n    Color(1)\nclass Color(Enum):\n    pass\n";
    let diagnostics = lower_err(source);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    assert_enum_call(&diagnostics[0], "Color", "Color(1)", source);
    assert_eq!(diagnostics[1].code, "C0001");
    assert_ne!(diagnostics[1].message, enum_class_call_message("Color"));
}

#[test]
fn a_starred_argument_call_reports_the_starred_diagnostic_then_the_call() {
    let source = format!("{COLOR}def f(xs: list[int]) -> None:\n    Color(*xs)\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    let starred = nth_offset(&source, "*xs", 0);
    assert_eq!(diagnostics[0].code, "C0001");
    assert_eq!(diagnostics[0].span, Some(Span::new(starred, starred + 3)));
    assert_enum_call(&diagnostics[1], "Color", "Color(*xs)", &source);
}

#[test]
fn a_class_body_call_reports_the_attribute_diagnostic_then_the_call() {
    let source = format!("{COLOR}class K:\n    X = Color()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    let message = &diagnostics[0].message;
    assert!(message.contains("class attribute `X`"), "{message}");
    assert_enum_call(&diagnostics[1], "Color", "Color()", &source);
}

#[test]
fn a_class_body_shadow_is_the_documented_false_kind_residual() {
    // Limit (i): a class body gets no frame, so `Color = 1` there does
    // not suppress the scan. Pinned so the residual cannot drift.
    let source = format!("{COLOR}class K:\n    Color = 1\n    X = Color()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    let message = &diagnostics[0].message;
    assert!(message.contains("class attribute `X`"), "{message}");
    assert_enum_call(&diagnostics[1], "Color", "Color()", &source);
}

#[test]
fn a_method_body_call_is_rejected_at_the_call() {
    let source = format!("{COLOR}class K:\n    def m(self) -> None:\n        Color()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_enum_call(&diagnostics[0], "Color", "Color()", &source);
}

#[test]
fn a_class_body_binding_does_not_reach_a_method_body() {
    let source =
        format!("{COLOR}class K:\n    Color = 1\n    def m(self) -> None:\n        Color()\n");
    let diagnostics = lower_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_enum_call(&diagnostics[0], "Color", "Color()", &source);
}

/// Limit (vii): a module that binds one name through two module-level
/// `def`/`class`/`import`/`type` statements (one of them the enum
/// class) is reported by the collision diagnostic alone -- the scan
/// never claims a call to that name, in either definition order
/// (PR #971 review: `def Color()` / `Color()` / `class Color(Enum)`,
/// and then an ordinary `class Color` in the same position, used to
/// report a false-kind enum-call `C0001` first).
fn assert_only_the_collision_is_reported(source: &str) {
    let diagnostics = lower_err(source);
    let messages: Vec<&str> = diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(
        messages
            .iter()
            .filter(|m| m.contains("collides with") || m.contains("defined more than once"))
            .count(),
        1,
        "exactly one collision diagnostic expected, got {messages:?}"
    );
    assert!(
        messages
            .iter()
            .all(|m| !m.contains("cannot call enum class")),
        "the scan must not claim a rebound name, got {messages:?}"
    );
}

#[test]
fn a_def_bound_name_is_never_scanned_when_the_def_comes_first() {
    assert_only_the_collision_is_reported(
        "from enum import Enum\n\ndef Color() -> int:\n    return 1\n\nColor()\n\nclass Color(Enum):\n    RED = 1\n",
    );
}

#[test]
fn a_def_bound_name_is_never_scanned_when_the_class_comes_first() {
    assert_only_the_collision_is_reported(
        "from enum import Enum\n\nclass Color(Enum):\n    RED = 1\n\ndef Color() -> int:\n    return 1\n\nColor()\n",
    );
}

#[test]
fn a_def_body_call_to_a_def_bound_enum_name_is_not_scanned_either() {
    assert_only_the_collision_is_reported(
        "from enum import Enum\n\ndef use() -> None:\n    Color()\n\ndef Color() -> int:\n    return 1\n\nclass Color(Enum):\n    RED = 1\n",
    );
}

#[test]
fn an_ordinary_class_bound_name_is_never_scanned_when_the_class_comes_first() {
    assert_only_the_collision_is_reported(
        "from enum import Enum\n\nclass Color:\n    pass\n\nColor()\n\nclass Color(Enum):\n    RED = 1\n",
    );
}

#[test]
fn a_type_alias_bound_name_is_never_scanned_when_the_alias_comes_first() {
    assert_only_the_collision_is_reported(
        "from enum import Enum\n\ntype Color = int\n\nColor()\n\nclass Color(Enum):\n    RED = 1\n",
    );
}

#[test]
fn module_rebound_names_lists_names_bound_by_two_or_more_statements() {
    let module = crate::pycc_parser_test_helper::parse(concat!(
        "import a.b\n",
        "import c as a\n",
        "from m import x, y as z\n",
        "from n import *\n",
        "def z() -> None:\n    def inner() -> None:\n        pass\n",
        "class K:\n    def method(self) -> None:\n        pass\n",
        "type K = int\n",
        "async def once() -> None:\n    pass\n",
        "class Once(Enum):\n    pass\n",
        "def x() -> None:\n    pass\n",
        "def x() -> None:\n    pass\n",
    ));
    assert_eq!(
        super::module_rebound_names(&module.body),
        vec!["a", "z", "K", "x"]
    );
}

#[test]
fn module_rebound_names_keeps_an_identical_repeated_import() {
    let module = crate::pycc_parser_test_helper::parse(concat!(
        "from colors import Color\n",
        "from colors import Color\n",
        "from .colors import Shade\n",
        "from .colors import Shade\n",
        "import a.b\n",
        "import a.b\n",
        "from other import Color\n",
        "import pkg.tone as Shade\n",
        "def a() -> None:\n    pass\n",
    ));
    assert_eq!(
        super::module_rebound_names(&module.body),
        vec!["Color", "Shade", "a"]
    );
}

#[test]
fn module_rebound_names_ignores_a_future_import() {
    // PR #971 review: `lower_future_import` binds nothing, so a feature
    // name an enum class shares is no rebinding; a relative
    // `from .__future__ import x` is an ordinary import and still counts.
    let module = crate::pycc_parser_test_helper::parse(concat!(
        "from __future__ import annotations\n",
        "from enum import Enum\n",
        "class annotations(Enum):\n    A = 1\n",
        "from .__future__ import Shade\n",
        "class Shade(Enum):\n    A = 1\n",
    ));
    assert_eq!(super::module_rebound_names(&module.body), vec!["Shade"]);
}

#[test]
fn an_enum_named_after_a_future_feature_is_still_reported_at_the_call() {
    let source = "from __future__ import annotations\nfrom enum import Enum\n\nclass annotations(Enum):\n    A = 1\n\ndef f() -> None:\n    annotations()\n";
    let diagnostics = lower_err(source);
    assert_eq!(diagnostics.len(), 1);
    assert_enum_call(&diagnostics[0], "annotations", "annotations()", source);
}
