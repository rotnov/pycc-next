//! Unit tests for the compiler-provided `__name__` binding (W0 of #882,
//! #1156): both seeding gates, every top-level binding form the shadowing
//! gate must recognize, and the seeded item itself.

use super::*;
use crate::module::lower_module;
use crate::pycc_parser_test_helper::parse;
use crate::{HirModule, ResolvedImports};

/// Every unit test here runs the scans with no *driver* import bindings, which
/// is what `lower_module` passes for a module the driver resolved no project
/// import for. That is not a reduced fixture: `scan_imports` adds the module's
/// own module-scope stdlib `import` statements on top of this slice, so the
/// aliased `import typing as t` spelling resolves here exactly as it does under
/// `lower`.
const NO_IMPORTS: &[ImportBinding] = &[];

fn references(source: &str) -> bool {
    references_dunder_name(&parse(source), NO_IMPORTS)
}

fn binds(source: &str) -> bool {
    binds_dunder_name_at_module_scope(&parse(source), NO_IMPORTS)
}

fn mentions(source: &str) -> bool {
    mentions_dunder_name(&parse(source), NO_IMPORTS)
}

fn seed(source: &str, module_name: Option<&str>) -> Option<HirItem> {
    seed_item(&parse(source), module_name, NO_IMPORTS)
}

fn lower(source: &str, module_name: Option<&str>) -> HirModule {
    lower_module(&parse(source), &ResolvedImports::default(), module_name)
        .expect("test fixture must lower")
        .hir
}

/// The `str` literal the module's *first* item assigns to `__name__`, if its
/// first item is such an assignment at all. A seeded module has one by
/// construction; so does a module whose own first statement is
/// `__name__ = "..."`, which is why the shadowing test below checks the value
/// rather than only its presence.
fn first_dunder_name_value(hir: &HirModule) -> Option<&str> {
    match hir.items.first()? {
        HirItem::TopLevelStmt(HirStmt::Assign {
            target,
            value: HirExpr::StringLiteral(literal),
        }) if target == DUNDER_NAME => Some(literal.as_str()),
        _ => None,
    }
}

// -- gate 1: the reference scan -------------------------------------

#[test]
fn a_module_that_never_mentions_the_name_is_not_a_reference() {
    assert!(!references("x: int = 1\nprint(x)\n"));
}

#[test]
fn a_module_level_read_is_a_reference() {
    assert!(references("print(__name__)\n"));
}

#[test]
fn a_read_inside_a_function_body_is_a_reference() {
    assert!(references("def f() -> None:\n    print(__name__)\n"));
}

#[test]
fn a_read_nested_deep_inside_an_expression_is_a_reference() {
    assert!(references(
        "x: list[str] = [s for s in [__name__] if s != \"\"]\n"
    ));
}

#[test]
fn the_scan_stops_descending_once_the_name_is_found() {
    // The first statement matches; the remaining statements (and the
    // remaining children of the matching expression) must not change the
    // answer, which is what the early return in `visit_expr` guarantees.
    assert!(references(
        "print(__name__ + \"x\" + \"y\")\nz: int = 1\nprint(z)\n"
    ));
}

#[test]
fn an_attribute_named_dunder_name_is_not_a_bare_reference() {
    assert!(!references("import m\nprint(m.__name__)\n"));
}

// -- gate 2: the top-level binding scan -----------------------------

#[test]
fn a_module_that_binds_nothing_named_dunder_name_does_not_shadow() {
    assert!(!binds("x: int = 1\nprint(__name__)\n"));
}

#[test]
fn a_plain_assignment_binds() {
    assert!(binds("__name__ = \"custom\"\n"));
}

#[test]
fn a_tuple_unpacking_target_binds() {
    assert!(binds("__name__, other = (\"a\", \"b\")\n"));
}

#[test]
fn a_list_unpacking_target_binds() {
    assert!(binds("[__name__, other] = [\"a\", \"b\"]\n"));
}

#[test]
fn a_starred_unpacking_target_binds() {
    assert!(binds("first, *__name__ = [\"a\", \"b\"]\n"));
}

#[test]
fn an_attribute_assignment_target_does_not_bind() {
    assert!(!binds(
        "import m\nm.__name__ = \"custom\"\nprint(__name__)\n"
    ));
}

#[test]
fn an_annotated_assignment_binds() {
    assert!(binds("__name__: str = \"custom\"\n"));
}

#[test]
fn an_augmented_assignment_binds() {
    assert!(binds("__name__ += \"suffix\"\n"));
}

#[test]
fn a_function_definition_binds() {
    assert!(binds("def __name__() -> None:\n    pass\n"));
}

#[test]
fn an_async_function_definition_binds() {
    assert!(binds("async def __name__() -> None:\n    pass\n"));
}

#[test]
fn a_class_definition_binds() {
    assert!(binds("class __name__:\n    pass\n"));
}

#[test]
fn a_type_alias_binds() {
    assert!(binds("type __name__ = int\n"));
}

#[test]
fn a_for_loop_target_binds() {
    assert!(binds("for __name__ in [\"a\"]:\n    pass\n"));
}

/// A genuine `Stmt::For { is_async: true, .. }`, which is the node the gate's
/// doc comment claims to cover -- ruff carries `async` as a flag on the same
/// variant, so a plain `for` would not exercise the claim at all. A bare
/// top-level `async for` parses fine here; it is rejected later, at lowering,
/// by the D-148 context-validity check, and this gate runs before that.
#[test]
fn an_async_for_loop_target_binds() {
    assert!(binds("async for __name__ in [\"a\"]:\n    pass\n"));
}

#[test]
fn a_with_as_target_binds() {
    assert!(binds("with open(\"f\") as __name__:\n    pass\n"));
}

#[test]
fn a_with_item_without_an_as_target_does_not_bind() {
    assert!(!binds("with open(\"f\"):\n    print(__name__)\n"));
}

#[test]
fn an_import_alias_binds() {
    assert!(binds("import math as __name__\n"));
}

#[test]
fn an_unaliased_import_of_a_dotted_module_binds_only_its_root() {
    assert!(!binds("import os.path\nprint(__name__)\n"));
    assert!(binds("import __name__.sub\n"));
}

#[test]
fn a_from_import_alias_binds() {
    assert!(binds("from math import sqrt as __name__\n"));
}

#[test]
fn an_unaliased_from_import_of_the_name_binds() {
    assert!(binds("from m import __name__\n"));
}

#[test]
fn a_binding_inside_a_function_body_is_not_a_top_level_binding() {
    assert!(!binds(
        "def f() -> None:\n    __name__ = \"local\"\n    print(__name__)\n"
    ));
}

#[test]
fn a_binding_inside_a_class_body_is_not_a_top_level_binding() {
    assert!(!binds(
        "class C:\n    __name__: str = \"in-class\"\n\nprint(__name__)\n"
    ));
}

#[test]
fn an_unrelated_top_level_statement_does_not_bind() {
    assert!(!binds("if True:\n    pass\n\nprint(__name__)\n"));
}

// -- the seed itself -------------------------------------------------

#[test]
fn no_module_name_means_no_seed() {
    assert_eq!(seed("print(__name__)\n", None), None);
}

#[test]
fn no_reference_means_no_seed() {
    assert_eq!(seed("x: int = 1\n", Some("__main__")), None);
}

#[test]
fn a_top_level_binding_withholds_the_seed() {
    assert_eq!(
        seed("__name__ = \"custom\"\nprint(__name__)\n", Some("__main__")),
        None
    );
}

#[test]
fn a_referencing_unshadowed_module_is_seeded_with_the_supplied_name() {
    assert_eq!(
        seed("print(__name__)\n", Some("ext_mod")),
        Some(HirItem::TopLevelStmt(HirStmt::Assign {
            target: DUNDER_NAME.to_string(),
            value: HirExpr::StringLiteral("ext_mod".to_string()),
        }))
    );
}

// -- lowering wires the seed in as the module's first item ----------

#[test]
fn lowering_puts_the_seed_ahead_of_every_user_item() {
    let hir = lower("x: int = 1\nprint(__name__)\n", Some("__main__"));
    assert_eq!(first_dunder_name_value(&hir), Some("__main__"));
    assert_eq!(hir.items.len(), 3);
}

#[test]
fn lowering_without_a_module_name_seeds_nothing() {
    let hir = lower("x: int = 1\n", None);
    assert_eq!(first_dunder_name_value(&hir), None);
}

#[test]
fn lowering_a_shadowing_module_seeds_nothing() {
    let hir = lower("__name__ = \"custom\"\nprint(__name__)\n", Some("__main__"));
    // Two items, not three: the user's own assignment and the `print`. The
    // first item carries the user's value, so no seed was prepended.
    assert_eq!(first_dunder_name_value(&hir), Some("custom"));
    assert_eq!(hir.items.len(), 2);
}

// -- module scope, not just the direct children of the module body --------
//
// Each of these was invisible to the earlier statement-target scan, so the
// module was seeded and the user's own binding was then rejected with `T0023`.

#[test]
fn a_binding_nested_in_a_compound_statement_binds() {
    assert!(binds("if flag:\n    __name__ = 7\n"));
    assert!(binds("while flag:\n    __name__ = 7\n"));
    assert!(binds("for i in xs:\n    __name__ = 7\n"));
    assert!(binds("with ctx():\n    __name__ = 7\n"));
    assert!(binds("try:\n    __name__ = 7\nexcept E:\n    pass\n"));
}

#[test]
fn a_binding_nested_two_levels_deep_binds() {
    assert!(binds("if a:\n    if b:\n        __name__ = 7\n"));
}

#[test]
fn a_def_or_class_nested_in_a_compound_statement_binds() {
    assert!(binds(
        "if flag:\n    def __name__() -> int:\n        return 1\n"
    ));
    assert!(binds("if flag:\n    class __name__:\n        pass\n"));
}

#[test]
fn a_walrus_binds_wherever_it_appears_in_module_scope() {
    assert!(binds("print((__name__ := 7))\n"));
    assert!(binds("if (__name__ := 7) > 3:\n    pass\n"));
    assert!(binds("while (__name__ := 7) > 3:\n    break\n"));
}

#[test]
fn a_match_capture_binds_in_every_capturing_pattern() {
    assert!(binds("match x:\n    case __name__:\n        pass\n"));
    assert!(binds("match x:\n    case [*__name__]:\n        pass\n"));
    assert!(binds("match x:\n    case {**__name__}:\n        pass\n"));
    assert!(binds("match x:\n    case 1 | __name__:\n        pass\n"));
    assert!(binds("match x:\n    case [1, __name__]:\n        pass\n"));
    // A later case's pattern is still visited after the name is found, which
    // is the scan's own short-circuit rather than the walk's.
    assert!(binds(
        "match x:\n    case __name__:\n        pass\n    case 1:\n        pass\n"
    ));
}

#[test]
fn an_except_as_name_binds() {
    assert!(binds("try:\n    pass\nexcept E as __name__:\n    pass\n"));
}

#[test]
fn an_import_binds_its_local_name() {
    // The arm the driver's old `definition_spans` gate could not see at all:
    // imports are deliberately absent from that table.
    assert!(binds("import __name__\n"));
    assert!(binds("import other as __name__\n"));
    assert!(binds("from m import y as __name__\n"));
}

#[test]
fn a_binding_inside_a_function_or_class_body_does_not_bind_module_scope() {
    assert!(!binds(
        "def f() -> int:\n    __name__ = 7\n    return __name__\n"
    ));
    assert!(!binds("class C:\n    __name__ = 7\n"));
    assert!(!binds(
        "def f() -> int:\n    if (__name__ := 7) > 3:\n        return 1\n    return 0\n"
    ));
    assert!(!binds(
        "if flag:\n    def f() -> int:\n        __name__ = 7\n        return __name__\n"
    ));
}

#[test]
fn mentions_is_the_union_of_both_per_module_gates() {
    // The dependency test the driver applies. A read alone counts, which is
    // exactly what `binds` does not see, and a binding that is not a read
    // counts too, which is what `references` does not see.
    assert!(mentions("print(__name__)\n"));
    assert!(mentions("import __name__\n"));
    assert!(mentions("if flag:\n    __name__ = 7\n"));
    assert!(!mentions("x = 7\nprint(x)\n"));
}

#[test]
fn mentions_counts_a_read_reached_only_through_a_function_body() {
    // The shape a module-scope binding test cannot see: the dependency's own
    // module scope neither binds nor reads the name, yet its top-level call
    // reaches the read before the entry module's seed runs.
    let source = "def show() -> str:\n    return __name__\n\nprint(show())\n";
    assert!(!binds(source));
    assert!(mentions(source));
}

#[test]
fn a_type_checking_guarded_binding_is_not_a_module_scope_binding() {
    // `lower_stmt` constant-folds the guarded body away (#790), so the
    // assignment emits no store and cannot collide with the seed.
    let source = "from typing import TYPE_CHECKING\n\nif TYPE_CHECKING:\n    __name__ = 7\n\nprint(__name__)\n";
    assert!(!binds(source));
    assert!(references(source));
    assert!(seed(source, Some("m")).is_some());
}

#[test]
fn a_qualified_type_checking_guard_folds_the_same_way() {
    let source = "import typing\n\nif typing.TYPE_CHECKING:\n    __name__ = 7\n\nprint(__name__)\n";
    assert!(!binds(source));
}

#[test]
fn a_type_checking_guarded_read_is_not_a_mention() {
    // The dependency gate must not count it either: a folded read observes
    // nothing, so it cannot see an uninitialized global.
    let source = "from typing import TYPE_CHECKING\n\nif TYPE_CHECKING:\n    print(__name__)\n";
    assert!(!references(source));
    assert!(!mentions(source));
}

#[test]
fn the_else_arm_of_a_folded_guard_is_still_live() {
    // Only the guarded body is dead; the `else` runs whenever the guard is
    // skipped, which at run time is always.
    let source = "from typing import TYPE_CHECKING\n\nif TYPE_CHECKING:\n    pass\nelse:\n    __name__ = 7\n";
    assert!(binds(source));
    let read = "from typing import TYPE_CHECKING\n\nif TYPE_CHECKING:\n    pass\nelif print(__name__):\n    pass\n";
    assert!(references(read));
}

#[test]
fn a_non_type_checking_guard_binds_normally() {
    assert!(binds("if flag:\n    __name__ = 7\n"));
}

#[test]
fn an_aliased_type_checking_guard_folds_the_same_way() {
    // `scan_imports` reconstructs the `typing as t` binding `std_receiver`
    // needs, so this folds exactly as the bare and qualified spellings do.
    let source = "import typing as t\n\nif t.TYPE_CHECKING:\n    __name__ = 7\n\nprint(__name__)\n";
    assert!(!binds(source));
    assert!(references(source));
    assert!(seed(source, Some("m")).is_some());
}

#[test]
fn an_alias_of_another_stdlib_module_does_not_fold() {
    // `scan_imports` records every stdlib alias, but `is_type_checking_guard`
    // still admits only `typing`, so an unrelated module's attribute is a
    // live guard.
    let source = "import math as t\n\nif t.TYPE_CHECKING:\n    __name__ = 7\n";
    assert!(binds(source));
}

#[test]
fn a_non_stdlib_alias_does_not_fold() {
    let source = "import nowhere as t\n\nif t.TYPE_CHECKING:\n    __name__ = 7\n";
    assert!(binds(source));
}

#[test]
fn an_elif_type_checking_guard_folds_like_a_leading_one() {
    // #790 folds the guard in the `elif` position too
    // (`lower_elif_else_clauses`), so neither gate may count that arm.
    let source = "from typing import TYPE_CHECKING\n\nif flag:\n    pass\nelif TYPE_CHECKING:\n    __name__ = 7\n\nprint(__name__)\n";
    assert!(!binds(source));
    assert!(references(source));
    assert!(seed(source, Some("m")).is_some());
}

#[test]
fn a_read_inside_a_folded_elif_is_not_a_mention() {
    let source = "from typing import TYPE_CHECKING\n\nif flag:\n    pass\nelif TYPE_CHECKING:\n    print(__name__)\n";
    assert!(!references(source));
    assert!(!mentions(source));
}

#[test]
fn the_arms_around_a_folded_elif_stay_live() {
    // Only the guarded arm is dead: the leading `if` above it and the `else`
    // below it both lower normally, so a binding in either still shadows.
    let above = "from typing import TYPE_CHECKING\n\nif flag:\n    __name__ = 7\nelif TYPE_CHECKING:\n    pass\n";
    assert!(binds(above));
    let below = "from typing import TYPE_CHECKING\n\nif flag:\n    pass\nelif TYPE_CHECKING:\n    pass\nelse:\n    __name__ = 7\n";
    assert!(binds(below));
}

#[test]
fn a_live_if_test_is_still_scanned() {
    // The leading test of a non-guard `if` is lowered, so a read there counts.
    assert!(references("if print(__name__):\n    pass\n"));
}
