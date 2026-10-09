//! The D-219 cascade classifier's own tests: every cascade-shaped `C0001`
//! message builder round-trips through `cascade_name`, and every other
//! diagnostic shape is rejected by each parser step.
//!
//! The two tests were moved here from `module/tests.rs` for Part 1 of #1283
//! (#1318) per AGENTS.md's file-decomposition rule, then extended for the
//! builtin-base builder: the round-trip test was renamed for its fourth
//! builder and gained the builtin-base lines, and the rejection test gained
//! a builtin-base case whose suffix does not match.
//! The `lower_module` suppression behaviour those messages drive is tested
//! in the parent module and in `builtin_base`.

use super::*;

#[test]
fn cascade_name_round_trips_all_four_message_builders() {
    let annotation = unsupported(unknown_annotation_name_message("Foo"), 0..3);
    assert_eq!(cascade_name(&annotation), Some("Foo"));
    let base = unsupported(unknown_base_message("Derived", "Base"), 0..3);
    assert_eq!(cascade_name(&base), Some("Base"));
    // The real producers, end to end.
    let diagnostics = lower_all_err("def f(a: Foo) -> int:\n    return 1\n");
    assert_eq!(cascade_name(&diagnostics[0]), Some("Foo"));
    let diagnostics = lower_all_err("class D(Base):\n    def m(self) -> int:\n        return 1\n");
    assert_eq!(cascade_name(&diagnostics[0]), Some("Base"));
    // The bare-container builder (D-228) is cascade-shaped too: a module
    // whose `class list:` failed poisons the name `list`, and a later
    // `x: list` must be suppressed exactly as `x: Foo` is after a failed
    // `class Foo:`. Pinned against the real producer, not just the builder,
    // so a rewording cannot silently make it unclassifiable again.
    let bare = unsupported(bare_container_annotation_message("list", "list[int]"), 0..4);
    assert_eq!(cascade_name(&bare), Some("list"));
    let diagnostics = lower_all_err("def f(a: list) -> int:\n    return 1\n");
    assert_eq!(diagnostics[0].code, "C0001");
    assert_eq!(cascade_name(&diagnostics[0]), Some("list"));
    // Part 1 of #1283: the builtin-type base message names its base, so a
    // failed user `class frozenset:` still silences `class F(frozenset)`.
    // Pinned against the real producer as well as the builder.
    let builtin = unsupported(builtin_base_message("fzset", "frozenset"), 0..5);
    assert_eq!(cascade_name(&builtin), Some("frozenset"));
    let diagnostics = lower_all_err("class fzset(frozenset):\n    pass\n");
    assert_eq!(diagnostics[0].code, "C0001");
    assert_eq!(cascade_name(&diagnostics[0]), Some("frozenset"));
}

#[test]
fn cascade_name_rejects_every_other_diagnostic_shape() {
    // Each parser step fails on its own input so every region is exercised.
    let cases: &[(&'static str, &str)] = &[
        // Annotation prefix ok, suffix fails; base prefix fails.
        ("C0001", "type annotation `x` foo"),
        // Annotation prefix fails; base prefix ok, infix split fails (the
        // real generic-base message from `validate_bases`).
        (
            "C0001",
            "class `B` cannot inherit from generic class `A` -- generic classes as bases are not \
             supported yet",
        ),
        // Base prefix and infix ok, suffix fails.
        (
            "C0001",
            "class `B` inherits from unknown class `A` -- trailing junk",
        ),
        // Neither prefix.
        ("C0001", CLASS_BODY_GAP),
        // D-228 (issue #918): bare-container prefix ok, infix split fails --
        // the third parser's own negative branch.
        ("C0001", "a bare `list` annotation, reworded"),
        // Part 1 of #1283: builtin-base prefix and infix ok, suffix fails --
        // the fourth parser's own negative branch. (Its prefix-ok,
        // infix-fails branch is the generic-base case above, and its
        // prefix-fails branch is `CLASS_BODY_GAP`.)
        (
            "C0001",
            "class `B` inherits from builtin type `list` -- trailing junk",
        ),
        // Not a `C0001` at all, even with a cascade-shaped message.
        ("T0044", "type annotation `x` is not supported yet"),
        (
            "C0002",
            "module `enum` has no importable symbol named `auto`",
        ),
    ];
    for (code, message) in cases {
        let diagnostic = Diagnostic::error(code, message.to_string(), Span::new(0, 1));
        assert_eq!(cascade_name(&diagnostic), None, "{code}: {message}");
    }
}
