//! Unit tests for the module-level walk's per-item diagnostic collection
//! and cascade suppression (Part 2 of #864, #867, D-219).
//!
//! Kept here rather than in `crates/pycc_hir/src/tests.rs` (tracked by
//! #663 as oversized). Every test asserts on codes, messages, spans, and
//! order of the *whole* `lower_all` list; the first entry is always also
//! what `lower_checked` reports, which is D-217's byte-stable first
//! diagnostic.

use super::*;
use crate::pycc_parser_test_helper::parse;

/// Byte span of the `occurrence`-th (0-based) occurrence of `needle` in
/// `source`, in the form `Diagnostic::span` carries.
fn span_of(source: &str, needle: &str, occurrence: usize) -> Span {
    let start = source
        .match_indices(needle)
        .nth(occurrence)
        .map(|(offset, _)| offset)
        .unwrap_or_else(|| panic!("`{needle}` occurrence {occurrence} not found in fixture"));
    Span::new(start as u32, (start + needle.len()) as u32)
}

fn lower_all_err(source: &str) -> Vec<Diagnostic> {
    lower_all(&parse(source)).expect_err("fixture must fail to lower")
}

fn assert_c0001(diagnostic: &Diagnostic, message: &str, span: Span) {
    assert_eq!(diagnostic.code, "C0001");
    assert_eq!(diagnostic.message, message);
    assert_eq!(diagnostic.span, Some(span));
}

const CLASS_BODY_GAP: &str = "a class body statement must be a method definition (`def ...`) or \
                              a class-level attribute assignment (`X = 1`, `X: int = 1`) -- no \
                              other statement kind is supported yet";
/// The re-vehicled class-body rejection these tests ride on (#910).
///
/// `x = 1` in a class body used to be `C0001`; #910 accepts it as an
/// inferred class attribute, so these tests need a statement kind that is
/// still rejected. A single-line `async def` is one, and its span is the
/// whole 31-byte statement -- two distinct method names keep `span_of`'s
/// occurrence-0 lookup unambiguous in the two dual-diagnostic tests.
const ASYNC_METHOD_M: &str = "async def m(self) -> None: pass";
const ASYNC_METHOD_N: &str = "async def n(self) -> None: pass";
const ASYNC_METHOD_GAP: &str = "an async method is not supported yet";
const IMPORT_OS_GAP: &str = "import of module `os` is not supported yet";

#[test]
fn issue_864_reproduction_reports_both_class_body_gaps_in_source_order() {
    let source = "class A:\n    async def m(self) -> None: pass\nclass B:\n    async def n(self) -> None: pass\ndef f(a: int) -> int:\n    \
                  return a + \"s\"\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 2);
    assert_c0001(
        &diagnostics[0],
        ASYNC_METHOD_GAP,
        span_of(source, ASYNC_METHOD_M, 0),
    );
    assert_c0001(
        &diagnostics[1],
        ASYNC_METHOD_GAP,
        span_of(source, ASYNC_METHOD_N, 0),
    );
    // The type error on line 6 is the type checker's, which does not run
    // after an HIR failure (D-219 decision A).
    let first = lower_checked(&parse(source)).unwrap_err();
    assert_eq!(first, diagnostics[0]);
}

#[test]
fn three_unsupported_imports_report_three_diagnostics_and_nothing_else() {
    let source = "import os\nimport sys\nimport re\ndef f() -> int:\n    return 1\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 3);
    assert_c0001(
        &diagnostics[0],
        IMPORT_OS_GAP,
        span_of(source, "import os", 0),
    );
    assert_c0001(
        &diagnostics[1],
        "import of module `sys` is not supported yet",
        span_of(source, "import sys", 0),
    );
    assert_c0001(
        &diagnostics[2],
        "import of module `re` is not supported yet",
        span_of(source, "import re", 0),
    );
}

#[test]
fn an_import_gap_and_a_later_unrelated_gap_are_both_reported() {
    let source = "import os\ndef h(*args: int) -> int:\n    return 0\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 2);
    assert_c0001(
        &diagnostics[0],
        IMPORT_OS_GAP,
        span_of(source, "import os", 0),
    );
    assert_c0001(
        &diagnostics[1],
        "`*args` is not supported yet",
        span_of(source, "(*args: int)", 0),
    );
}

#[test]
fn every_cascade_of_a_skipped_class_is_silent_and_transitive() {
    let source = "class A:\n    async def m(self) -> None: pass\n\
                  class B(A):\n    def __init__(self) -> None:\n        self.v = 1\n\
                  def g(a: A) -> int:\n    return 1\n\
                  x: A = A()\n\
                  type Alias = A\n\
                  Y: TypeAlias = A\n\
                  class D:\n    def __init__(self) -> None:\n        self.v = 1\n    def m(self) -> A:\n        return A()\n\
                  class C(B):\n    def __init__(self) -> None:\n        self.v = 1\n\
                  class E(Alias):\n    def __init__(self) -> None:\n        self.v = 1\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_c0001(
        &diagnostics[0],
        ASYNC_METHOD_GAP,
        span_of(source, ASYNC_METHOD_M, 0),
    );
}

#[test]
fn a_subscripted_annotation_on_a_skipped_class_is_a_silent_cascade_too() {
    // D-219 for the subscripted spelling (#931): `x: Foo[int]` after a
    // failed `class Foo:` still produces the exact unknown-name `C0001` that
    // `cascade_name` parses back, so the module reports exactly one
    // diagnostic, the class's own. The #931 reject fires only for a base
    // that *resolves*, which an undefined name never does.
    let source = "class Foo:\n    async def m(self) -> None: pass\n\
                  x: Foo[int] = Foo()\n\
                  def g(a: Foo[int]) -> Foo[int]:\n    return a\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_c0001(
        &diagnostics[0],
        ASYNC_METHOD_GAP,
        span_of(source, ASYNC_METHOD_M, 0),
    );
}

#[test]
fn non_cascade_shapes_after_a_skipped_import_stay_reported() {
    // The rejected `import os` poisons `os`, so the bare annotation `p: os`
    // is suppressed as a cascade. This supersedes "correction 7", which read
    // the question as "would `p: os` still be an error once `import os` is
    // supported?" (yes) rather than the question poisoning actually asks:
    // "is the message we print caused by the earlier failure?". `unknown
    // annotation name `os`` is produced *only* because nothing bound `os`,
    // and it is misleading once the import gap is already reported at 1:1;
    // a future `import os` would fail `p: os` through a different path with
    // a correctly worded message. What survives is the boundary of the
    // mechanism: the attribute annotation fails on its own shape rather than
    // on a name lookup, and the value-position `os.getpid()` is not an HIR
    // name lookup at all -- neither is silenced by the poisoned name.
    let source = "import os\n\
                  def h(p: os) -> int:\n    return 1\n\
                  def h2(p: os.PathLike) -> int:\n    return 1\n\
                  def h3() -> int:\n    return os.getpid()\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert_c0001(
        &diagnostics[0],
        IMPORT_OS_GAP,
        span_of(source, "import os", 0),
    );
    assert_eq!(diagnostics[1].code, "C0001");
    assert!(
        diagnostics[1]
            .message
            .starts_with("only a bare name type annotation is supported so far"),
        "{}",
        diagnostics[1].message
    );
    assert_eq!(diagnostics[1].span, Some(span_of(source, "os.PathLike", 0)));
}

#[test]
fn a_skipped_def_does_not_poison_its_name() {
    let source = "def A(*args: int) -> int:\n    return 0\ndef g(a: A) -> int:\n    return 1\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert_c0001(
        &diagnostics[0],
        "`*args` is not supported yet",
        span_of(source, "(*args: int)", 0),
    );
    assert_c0001(
        &diagnostics[1],
        &unknown_annotation_name_message("A"),
        span_of(source, "A", 1),
    );
}

#[test]
fn a_failed_from_import_of_a_marker_base_does_not_silence_the_class_using_it() {
    // Round-1 false-suppression class: `Enum` is resolved by spelling before
    // any table lookup, so `class Color(Enum)` lowers and only the import's
    // own `C0002` is reported.
    // (#892 registered `enum.auto`, so this uses `IntEnum` -- still
    // unregistered -- as the symbol that fails to import.)
    let source = "from enum import Enum, IntEnum\nclass Color(Enum):\n    RED = 1\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].code, "C0002");
    assert_eq!(
        diagnostics[0].message,
        "module `enum` has no importable symbol named `IntEnum`"
    );
    assert_eq!(
        diagnostics[0].span,
        Some(span_of(source, "from enum import Enum, IntEnum", 0))
    );
}

#[test]
fn a_failed_from_import_of_final_does_not_silence_a_final_annotation() {
    let source = "from typing import Final, Optional\ny: Final[int] = 1\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].code, "C0002");
    assert_eq!(
        diagnostics[0].message,
        "module `typing` has no importable symbol named `Optional`"
    );
}

#[test]
fn a_folded_type_checking_block_is_not_a_cascade() {
    let source = "from typing import TYPE_CHECKING\nimport os\nif TYPE_CHECKING:\n    q: os = 1\n\
                  def f() -> int:\n    return 1\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_c0001(
        &diagnostics[0],
        IMPORT_OS_GAP,
        span_of(source, "import os", 0),
    );
}

#[test]
fn a_rejected_alias_colliding_with_a_valid_class_does_not_poison_the_class() {
    let source = "class A:\n    def __init__(self) -> None:\n        self.v = 1\ntype A = int\n\
                  def f(a: A) -> int:\n    return 1\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_c0001(
        &diagnostics[0],
        "type alias `A` collides with a class of the same name already defined in this module",
        span_of(source, "type A = int", 0),
    );
}

#[test]
fn a_successful_class_rebinding_un_poisons_the_name() {
    let source = "class A:\n    async def m(self) -> None: pass\n\
                  class A:\n    def __init__(self) -> None:\n        self.v = 1\n    def m(self) -> A:\n        return self\n\
                  def f(a: A) -> int:\n    return 1\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_c0001(
        &diagnostics[0],
        ASYNC_METHOD_GAP,
        span_of(source, ASYNC_METHOD_M, 0),
    );
}

#[test]
fn a_twice_rejected_then_accepted_class_leaves_no_duplicate_poison_behind() {
    let source = "class A:\n    async def m(self) -> None: pass\n\
                  class A:\n    async def n(self) -> None: pass\n\
                  class A:\n    def __init__(self) -> None:\n        self.v = 1\n\
                  def f(a: A) -> int:\n    return 1\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert_c0001(
        &diagnostics[0],
        ASYNC_METHOD_GAP,
        span_of(source, ASYNC_METHOD_M, 0),
    );
    assert_c0001(
        &diagnostics[1],
        ASYNC_METHOD_GAP,
        span_of(source, ASYNC_METHOD_N, 0),
    );
}

#[test]
fn a_skipped_def_leaves_no_binding_for_a_later_class_to_collide_with() {
    let source = "def A(*args: int) -> int:\n    return 0\n\
                  class A:\n    def __init__(self) -> None:\n        self.v = 1\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_c0001(
        &diagnostics[0],
        "`*args` is not supported yet",
        span_of(source, "(*args: int)", 0),
    );
}

#[test]
fn a_rejected_alias_after_a_valid_class_un_poisons_nothing_and_reports_once() {
    // `type X = A` after a skipped `A` is a silent cascade that poisons `X`;
    // a later `class X:` that lowers un-poisons it, so `def f(x: X)` lowers.
    let source = "class A:\n    async def m(self) -> None: pass\ntype X = A\n\
                  class X:\n    def __init__(self) -> None:\n        self.v = 1\n\
                  def f(x: X) -> int:\n    return 1\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_c0001(
        &diagnostics[0],
        ASYNC_METHOD_GAP,
        span_of(source, ASYNC_METHOD_M, 0),
    );
}

#[test]
fn a_valueless_legacy_alias_declaration_poisons_nothing() {
    // `X: TypeAlias` without a value is lowered as an ordinary annotated
    // assignment (no alias is recorded), so its own rejection must not poison
    // `X`: the later `def f(a: X)` is a genuine gap, not a cascade.
    let source = "X: TypeAlias\n\
                  def f(a: X) -> int:\n    return 1\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert_c0001(
        &diagnostics[0],
        &unknown_annotation_name_message("TypeAlias"),
        span_of(source, "TypeAlias", 0),
    );
    assert_c0001(
        &diagnostics[1],
        &unknown_annotation_name_message("X"),
        span_of(source, "X", 1),
    );
}

#[test]
fn poisonable_name_per_statement_kind() {
    let cases: &[(&str, Option<&str>)] = &[
        ("class C:\n    pass\n", Some("C")),
        ("type X = int\n", Some("X")),
        ("X: TypeAlias = int\n", Some("X")),
        // Valueless legacy spelling binds no alias, so it is not poisonable.
        ("X: TypeAlias\n", None),
        // ruff drops the parentheses, so the target is still a `Name`.
        ("(x): TypeAlias = int\n", Some("x")),
        // Target is not a `Name`.
        ("a.b: TypeAlias = int\n", None),
        ("x[0]: TypeAlias = int\n", None),
        // Annotation is a `Name` other than `TypeAlias`.
        ("X: Final = 1\n", None),
        // Annotation is not a `Name`.
        ("X: list[int] = []\n", None),
        // A plain `import` binds a name only when it lowers: exactly one
        // alias (with or without an `asname`, Part 1 of #883) and a module
        // `pycc_std` resolves.
        ("import math\n", None),
        ("import os\n", Some("os")),
        ("import math as m\n", None),
        ("import numpy as np\n", Some("np")),
        ("import pkg.dep\n", Some("pkg")),
        ("import pkg.dep as d\n", Some("d")),
        ("import math, os\n", Some("math")),
        ("from math import sqrt\n", None),
        ("def f() -> int:\n    return 1\n", None),
        ("x = 1\n", None),
        ("x: int = 1\n", None),
        ("print(1)\n", None),
    ];
    for (source, expected) in cases {
        let module = parse(source);
        assert_eq!(
            poisonable_names(&module.body[0]).first().copied(),
            *expected,
            "{source}"
        );
    }
}

#[test]
fn a_poisoned_container_name_suppresses_a_later_bare_container_annotation() {
    // D-228 (issue #918) review finding: `list` is an ordinary bindable name,
    // so a failed `class list:` poisons it exactly as a failed `class Foo:`
    // poisons `Foo`. The bare-container `C0001` the parameter position then
    // builds has to stay classifiable, or the cascade leaks a second
    // diagnostic the user cannot act on. Asserted against the `Foo` control
    // in the same test so the two can never drift apart.
    for name in ["list", "Foo"] {
        let source = format!(
            "class {name}:\n    x: badtype = 1\n\n\ndef f(x: {name}) -> None:\n    return\n"
        );
        let diagnostics = lower_all_err(&source);
        assert_eq!(diagnostics.len(), 1, "{name}: {diagnostics:#?}");
        assert_eq!(
            diagnostics[0].message,
            unknown_annotation_name_message("badtype")
        );
    }
}

// The cascade classifier's own tests (moved out, then extended for the
// builtin-base builder, in Part 1 of #1283 per AGENTS.md's
// file-decomposition rule) and the builtin-type base tests live in child
// modules.
mod builtin_base;
mod cascade_classifier;

#[test]
fn post_loop_phases_are_skipped_when_anything_was_collected() {
    // The hierarchy is seeded (the module references `ValueError`), one
    // class is rejected: exactly that diagnostic, and no `Exception.__init__`
    // seeding or tag assignment is attempted on the partial table.
    let source = "def f() -> None:\n    raise ValueError(\"x\")\n\
                  class E(ValueError):\n    def __init__(self) -> None:\n        self.v = 1\n\
                  class A:\n    async def m(self) -> None: pass\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_c0001(
        &diagnostics[0],
        ASYNC_METHOD_GAP,
        span_of(source, ASYNC_METHOD_M, 0),
    );
}

#[test]
fn a_shadowing_class_that_fails_to_lower_still_takes_the_shadow_gate() {
    // The shadow gate is a whole-module scan decided before the loop: with
    // `ValueError` shadowed, nothing is seeded, so the second (valid)
    // `class ValueError` does not collide with a synthetic entry -- it
    // lowers and un-poisons the name, leaving exactly one diagnostic. Had
    // the gate looked at what actually lowered, the seeded synthetic class
    // would have made the rebinding a second "defined more than once".
    let source = "class ValueError:\n    async def m(self) -> None: pass\n\
                  class ValueError:\n    def __init__(self) -> None:\n        self.v = 1\n\
                  def f(e: ValueError) -> int:\n    return 1\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_c0001(
        &diagnostics[0],
        ASYNC_METHOD_GAP,
        span_of(source, ASYNC_METHOD_M, 0),
    );
}

#[test]
fn lower_checked_is_the_first_element_view_of_lower_all() {
    let fixtures = [
        "import os\nimport sys\n",
        "class A:\n    async def m(self) -> None: pass\nclass B:\n    async def n(self) -> None: pass\n",
        "def f(a: Foo) -> int:\n    return 1\n",
        "def h(*args: int) -> int:\n    return 0\n",
    ];
    for source in fixtures {
        let module = parse(source);
        let all = lower_all(&module).expect_err("fixture must fail to lower");
        assert!(!all.is_empty());
        assert_eq!(lower_checked(&module).unwrap_err(), all[0], "{source}");
    }
}

#[test]
fn a_clean_module_lowers_through_lower_all() {
    let module = parse(
        "class A:\n    def __init__(self) -> None:\n        self.v = 1\ndef f(a: A) -> int:\n    return 1\n",
    );
    let hir = lower_all(&module).expect("module must lower");
    assert_eq!(hir.class_defs.len(), 1);
    assert_eq!(hir.class_defs[0].0, "A");
}

/// Lowers `source` with every project import answered by the same
/// driver-supplied rejection, the way `src/modules.rs` answers an import
/// that names no file (`T0021`) or closes a cycle (`E0108`).
fn lower_with_not_found(source: &str, code: &'static str, message: &str) -> Vec<Diagnostic> {
    let parsed = parse(source);
    let mut resolved = ResolvedImports::default();
    for request in crate::project_import_requests(&parsed) {
        resolved.insert(
            request.span,
            crate::ResolvedImport::NotFound {
                code,
                message: message.to_string(),
            },
        );
    }
    lower_module(&parsed, &resolved, None).expect_err("fixture must fail to lower")
}

#[test]
fn a_rejected_project_import_poisons_the_names_it_would_have_bound() {
    // #898: the driver's own rejection reaches the walk as the import
    // item's error, so D-219's poisoning applies to it exactly as to any
    // other skipped item -- the later annotation is a cascade, not a gap.
    let source = "from .dep import Point\n\
                  def use(p: Point) -> int:\n    return 1\n\
                  x: Point = Point()\n";
    let message = "no module named `.dep` in `.`";
    let diagnostics = lower_with_not_found(source, "T0021", message);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].code, "T0021");
    assert_eq!(diagnostics[0].message, message);
    assert_eq!(
        diagnostics[0].span,
        Some(span_of(source, "from .dep import Point", 0))
    );
}

const CLASS_BODY: &str = "    def __init__(self) -> None:\n        self.v = 1\n";

#[test]
fn a_rejected_plain_import_poisons_its_alias() {
    // `import pkg.dep as d` names a dotted module `pycc_std` cannot resolve
    // (aliasing itself lowers for a stdlib module since Part 1 of #883),
    // and the name it would have bound is `d`, so a later `d` is a cascade
    // while an unrelated name is still reported.
    let source = format!("import pkg.dep as d\nclass Foo(d):\n{CLASS_BODY}");
    let diagnostics = lower_all_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message,
        "import of module `pkg.dep` is not supported yet"
    );

    let source = format!("import pkg.dep as d\nclass Foo(pkg):\n{CLASS_BODY}");
    let diagnostics = lower_all_err(&source);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert!(
        diagnostics[1]
            .message
            .contains("inherits from unknown class `pkg`"),
        "unexpected second diagnostic: {:#?}",
        diagnostics[1]
    );
}

#[test]
fn a_rejected_dotted_import_poisons_only_its_first_segment() {
    // `import pkg.dep` binds `pkg`, not `pkg.dep`, so poisoning the whole
    // dotted name would leave the real cascade unsuppressed.
    let source = format!("import pkg.dep\nclass Foo(pkg):\n{CLASS_BODY}");
    let diagnostics = lower_all_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message,
        "import of module `pkg.dep` is not supported yet"
    );
}

#[test]
fn a_rejected_stdlib_import_poisons_the_name_it_would_have_bound() {
    let source = format!("import os\nclass Foo(os):\n{CLASS_BODY}");
    let diagnostics = lower_all_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message, IMPORT_OS_GAP);
}

#[test]
fn a_multi_name_import_poisons_every_name_it_would_have_bound() {
    let source = format!(
        "import pkg.dep, other\nclass Foo(pkg):\n{CLASS_BODY}class Bar(other):\n{CLASS_BODY}"
    );
    let diagnostics = lower_all_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    // #1280: the statement lowers alias by alias and fails on its first
    // failing alias, yet still poisons every name it would have bound --
    // so `other` produces no cascade either.
    assert_eq!(
        diagnostics[0].message,
        "import of module `pkg.dep` is not supported yet"
    );
}

#[test]
fn a_successful_stdlib_import_poisons_nothing() {
    // `import math` lowers, so it suppresses nothing: a later use of the
    // bound name as a class base is a genuine diagnostic, not a cascade.
    let source = format!("import math\nclass Foo(math):\n{CLASS_BODY}");
    let diagnostics = lower_all_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert!(
        diagnostics[0]
            .message
            .contains("inherits from unknown class `math`"),
        "unexpected diagnostic: {:#?}",
        diagnostics[0]
    );
}

#[test]
fn a_rejected_project_import_poisons_the_alias_not_the_source_name() {
    // The name a `from ... import x as y` statement binds locally is `y`,
    // so `y` is the cascade to suppress and `x` stays a genuine unknown
    // name. The sibling test above only covers `asname == name`, where the
    // two spellings coincide and cannot discriminate the two.
    let message = "no module named `.dep` in `.`";

    let aliased = "from .dep import helper as h\n\
                   class Foo(h):\n    def __init__(self) -> None:\n        self.v = 1\n";
    let diagnostics = lower_with_not_found(aliased, "T0021", message);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].code, "T0021");

    let source_name = "from .dep import helper as h\n\
                       class Bar(helper):\n    def __init__(self) -> None:\n        self.v = 1\n";
    let diagnostics = lower_with_not_found(source_name, "T0021", message);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].code, "T0021");
    assert!(
        diagnostics[1]
            .message
            .contains("inherits from unknown class `helper`"),
        "unexpected second diagnostic: {:#?}",
        diagnostics[1]
    );
}

#[test]
fn an_import_cycle_poisons_transitively() {
    let source = "from dep import Base\n\
                  class Sub(Base):\n    def __init__(self) -> None:\n        self.v = 1\n\
                  def use(p: Sub) -> int:\n    return 1\n";
    let message = "import cycle: `a.py` -> `b.py` -> `a.py`";
    let diagnostics = lower_with_not_found(source, "E0108", message);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].code, "E0108");
    assert_eq!(diagnostics[0].message, message);
}

#[test]
fn a_rejected_stdlib_from_import_poisons_the_names_it_would_have_bound() {
    // A stdlib `from` import fails exactly as a project one can, so it
    // poisons the same way: the statement below binds nothing, and the
    // later `bogus` is a cascade of the `C0002` above it.
    let source = format!("from math import bogus\nclass Foo(bogus):\n{CLASS_BODY}");
    let diagnostics = lower_all_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].code, "C0002");

    let annotated = "from math import bogus\ndef f(p: bogus) -> int:\n    return 1\n";
    let diagnostics = lower_all_err(annotated);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].code, "C0002");
}

#[test]
fn an_aliased_stdlib_from_import_poisons_its_asname_only() {
    // `from math import sqrt as s` binds `s`, so a later `s` is the
    // cascade and a later `sqrt` is a genuine unknown name.
    let source = format!("from math import sqrt as s\nclass Foo(s):\n{CLASS_BODY}");
    let diagnostics = lower_all_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message,
        "`from ... import x as y` aliasing is not supported yet"
    );

    let source = format!("from math import sqrt as s\nclass Bar(sqrt):\n{CLASS_BODY}");
    let diagnostics = lower_all_err(&source);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert!(
        diagnostics[1]
            .message
            .contains("inherits from unknown class `sqrt`"),
        "unexpected second diagnostic: {:#?}",
        diagnostics[1]
    );
}

#[test]
fn a_wildcard_stdlib_from_import_poisons_the_modules_whole_export_list() {
    // `from math import *` would have bound every name `math` exports, so
    // each of them is a cascade of the rejection -- while a name the module
    // does not export is still reported.
    let source = format!("from math import *\nclass Foo(sqrt):\n{CLASS_BODY}");
    let diagnostics = lower_all_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message,
        "`from ... import *` (wildcard import) is not supported yet"
    );

    let source = format!("from math import *\nclass Bar(nope):\n{CLASS_BODY}");
    let diagnostics = lower_all_err(&source);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert!(
        diagnostics[1]
            .message
            .contains("inherits from unknown class `nope`"),
        "unexpected second diagnostic: {:#?}",
        diagnostics[1]
    );
}

#[test]
fn a_successful_stdlib_from_import_poisons_nothing() {
    // The import lowers, so `sqrt` is bound -- it is simply not usable as a
    // base class, and that diagnostic is genuine rather than a cascade.
    let source = format!("from math import sqrt\nclass Foo(sqrt):\n{CLASS_BODY}");
    let diagnostics = lower_all_err(&source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert!(
        diagnostics[0]
            .message
            .contains("inherits from unknown class `sqrt`"),
        "unexpected diagnostic: {:#?}",
        diagnostics[0]
    );
}

/// Every import shape whose lowering outcome is decidable without a
/// project resolver, one row per rejection branch of
/// `import::lower_import_stmt` plus the accepting shapes around them.
/// `#898`'s review loop found this arm-vs-arm mirroring broken three
/// separate times -- once per repair round, each fix scoped to the single
/// site just reported -- so the invariant is asserted over a corpus rather
/// than one shape at a time. A new rejection branch added to
/// `lower_import_stmt` without a matching `poisonable_names` arm fails
/// here as soon as its shape joins this list.
///
/// Every row is driven through `lower_all`, which lowers against
/// `ResolvedImports::default()`, so the corpus cannot reach
/// `lower_import_stmt`'s `ResolvedImport::Foreign` branch (Part 1 of
/// #1026) -- that branch fires only on a span-keyed driver answer, and no
/// row here wants one. That case is a deliberate divergence from the
/// biconditional rather than another row, so it is pinned by the sibling
/// test `a_foreign_import_lowers_and_still_poisons_its_name` below.
const IMPORT_SHAPES: &[&str] = &[
    // `Stmt::Import`: accepted, then one row per rejection branch.
    "import math\n",
    "import math as m\n", // `asname` on a stdlib module lowers (Part 1 of #883)
    "import enum\n",
    "import os\n",              // module `pycc_std` does not resolve
    "import pkg.dep\n",         // dotted, unresolvable
    "import pkg.dep as d\n",    // `asname`, dotted, unresolvable
    "import math, enum\n",      // several aliases, all resolve (#1280)
    "import math as m, enum\n", // several aliases, one `asname` (#1280)
    "import math, os\n",        // several aliases, a later one unresolvable
    "import pkg.dep, other\n",  // several aliases, the first unresolvable
    // `Stmt::ImportFrom`, stdlib arm: accepted, then its rejection branches.
    "from math import sqrt\n",
    "from math import sqrt, pi\n",
    "from math import bogus\n",       // not a registered symbol
    "from math import sqrt, bogus\n", // one of several is not
    "from math import sqrt as s\n",   // `asname`
    "from math import *\n",           // wildcard
    "from os import path\n",          // module `pycc_std` does not resolve
    // The `__future__` directive (D-229): accepted, then its rejection
    // branches. Single statements only: `body[0]` is what is inspected
    // below but the *whole* module is lowered, so a two-statement row (a
    // late future import, say) would break the biconditional -- the
    // position case has its own test.
    "from __future__ import annotations\n",
    "from __future__ import annotations, division\n",
    "from __future__ import annotations as ann\n", // `asname`
    "from __future__ import notafeature\n",        // not a CPython feature
    "from __future__ import barry_as_FLUFL\n",     // CPython-valid, pycc `C0001`
    "from __future__ import *\n",                  // wildcard
];

#[test]
fn a_failing_import_poisons_and_a_lowering_one_does_not() {
    for source in IMPORT_SHAPES {
        let module = parse(source);
        let statement = &module.body[0];
        let poisoned = poisonable_names(statement);
        let lowered = lower_all(&module).is_ok();
        assert_eq!(
            lowered,
            poisoned.is_empty(),
            "`{}` lowers={lowered} but poisons {poisoned:?} -- \
             `poisonable_names` must mirror `lower_import_stmt`'s success \
             condition exactly: a shape that lowers poisons nothing, and \
             every shape that fails poisons what it would have bound",
            source.trim_end()
        );
    }
}

/// The one import shape the corpus above cannot reach: a bare, unaliased,
/// non-stdlib, non-project name that the driver answered
/// `ResolvedImport::Foreign` for. It *lowers* -- to an
/// `ImportBinding::Foreign` -- and `poisonable_names` still names it,
/// because that function sees one statement and never the answer table.
///
/// This pins the divergence `module::poisonable_names`'s docstring
/// records, not a defect: `lower_module`'s loop only `retain`s poisoned
/// names on the `Ok` arm, so a prediction made for a statement that then
/// lowered suppresses nothing. If someone later teaches the `Stmt::Import`
/// arm about this case, this test fails and the docstring has to be
/// revisited with it.
#[test]
fn a_foreign_import_lowers_and_still_poisons_its_name() {
    let source = "import numpy\n";
    let module = parse(source);
    let statement = &module.body[0];
    let mut resolved = ResolvedImports::default();
    resolved.insert(span_of(source, "numpy", 0), crate::ResolvedImport::Foreign);

    let lowered = lower_module(&module, &resolved, None).expect("a foreign import must lower");
    assert_eq!(
        lowered.hir.imports,
        vec![ImportBinding::Foreign {
            local_name: "numpy".to_string(),
            module_path: "numpy".to_string(),
            from: None,
            site: crate::ForeignImportSite::Item(0),
            span: Span::new(0, "import numpy".len() as u32),
        }]
    );
    assert_eq!(poisonable_names(statement), vec!["numpy"]);
}

/// The aliased counterpart (#1291): `import numpy as np` lowers to a
/// foreign binding of `np`, and the mirror still predicts `[np]`.
#[test]
fn an_aliased_foreign_import_lowers_and_still_poisons_its_alias() {
    let source = "import numpy as np\n";
    let module = parse(source);
    let statement = &module.body[0];
    let mut resolved = ResolvedImports::default();
    resolved.insert(
        span_of(source, "numpy as np", 0),
        crate::ResolvedImport::Foreign,
    );

    let lowered = lower_module(&module, &resolved, None).expect("a foreign import must lower");
    assert_eq!(
        lowered.hir.imports,
        vec![ImportBinding::Foreign {
            local_name: "np".to_string(),
            module_path: "numpy".to_string(),
            from: None,
            site: crate::ForeignImportSite::Item(0),
            span: Span::new(0, "import numpy as np".len() as u32),
        }]
    );
    assert_eq!(poisonable_names(statement), vec!["np"]);
}

// ---------------------------------------------------------------------------
// `from __future__ import ...` (#919, D-229): a compiler directive that lowers
// to nothing, with CPython 3.14's `SyntaxError`s reported as `L0001`.
// ---------------------------------------------------------------------------

/// `__future__.all_feature_names` on CPython 3.14 minus `barry_as_FLUFL`.
/// Deliberately a copy of `import::NOOP_FUTURE_FEATURES` rather than an
/// import of it: this is a parity check of the production list against
/// CPython's, so sharing the constant would make the test tautological.
/// A CPython release that changes the feature set updates both lists.
const NOOP_FUTURE_FEATURES: &[&str] = &[
    "nested_scopes",
    "generators",
    "division",
    "absolute_import",
    "with_statement",
    "print_function",
    "unicode_literals",
    "generator_stop",
    "annotations",
];

fn lower_all_ok(source: &str) -> HirModule {
    lower_all(&parse(source))
        .unwrap_or_else(|diagnostics| panic!("must lower: {:?}", diagnostics[0].message))
}

fn assert_l0001(diagnostic: &Diagnostic, message: &str, span: Span) {
    assert_eq!(diagnostic.code, "L0001");
    assert_eq!(diagnostic.message, message);
    assert_eq!(diagnostic.span, Some(span));
}

#[test]
fn each_noop_future_feature_lowers_to_nothing() {
    // One name per statement, so a name accidentally dropped from the
    // accepted set fails by name rather than hiding behind the others.
    for name in NOOP_FUTURE_FEATURES {
        let source = format!("from __future__ import {name}\nx: int = 1\n");
        let hir = lower_all_ok(&source);
        assert!(hir.imports.is_empty(), "`{name}` must bind nothing");
        assert_eq!(hir.items.len(), 1, "`{name}` must contribute no item");
    }
}

#[test]
fn a_multi_name_future_import_lowers_to_nothing() {
    let hir = lower_all_ok("from __future__ import annotations, division\n");
    assert!(hir.imports.is_empty());
    assert!(hir.items.is_empty());
}

#[test]
fn the_issue_919_reproduction_lowers() {
    let hir =
        lower_all_ok("from __future__ import annotations\n\ndef f(x: int) -> int:\n    return x\n");
    assert!(hir.imports.is_empty());
    assert_eq!(hir.items.len(), 1);
}

#[test]
fn barry_as_flufl_is_a_c0001_naming_the_feature() {
    let source = "from __future__ import barry_as_FLUFL\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 1);
    assert_c0001(
        &diagnostics[0],
        "the `barry_as_FLUFL` future feature (`<>` in place of `!=`) is not supported yet",
        span_of(source, source.trim_end(), 0),
    );
}

#[test]
fn an_unknown_future_feature_is_an_l0001_with_cpythons_wording() {
    for (source, message) in [
        (
            "from __future__ import notafeature\n",
            "future feature notafeature is not defined",
        ),
        (
            "from __future__ import *\n",
            "future feature * is not defined",
        ),
        ("from __future__ import braces\n", "not a chance"),
        // Names are checked left to right: the unknown one wins over a
        // no-op one after it.
        (
            "from __future__ import bogus, annotations\n",
            "future feature bogus is not defined",
        ),
        // ... and over a `barry_as_FLUFL` before it -- the name pass runs
        // before the capability pass.
        (
            "from __future__ import barry_as_FLUFL, bogus\n",
            "future feature bogus is not defined",
        ),
    ] {
        let diagnostics = lower_all_err(source);
        assert_eq!(diagnostics.len(), 1, "{source}");
        assert_l0001(
            &diagnostics[0],
            message,
            span_of(source, source.trim_end(), 0),
        );
    }
}

#[test]
fn an_aliased_future_feature_is_the_generic_alias_c0001() {
    // CPython accepts `annotations as ann` and binds a `_Feature` object
    // pycc never models, so this is a capability gap, not a syntax error.
    let source = "from __future__ import annotations as ann\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 1);
    assert_c0001(
        &diagnostics[0],
        "`from ... import x as y` aliasing is not supported yet",
        span_of(source, source.trim_end(), 0),
    );
    // The alias check runs before the `barry_as_FLUFL` one.
    let source = "from __future__ import barry_as_FLUFL as b\n";
    let diagnostics = lower_all_err(source);
    assert_c0001(
        &diagnostics[0],
        "`from ... import x as y` aliasing is not supported yet",
        span_of(source, source.trim_end(), 0),
    );
}

#[test]
fn an_aliased_unknown_future_feature_is_still_the_l0001() {
    // The name pass precedes the alias pass: CPython rejects the name
    // before it ever binds anything.
    let source = "from __future__ import notafeature as n\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 1);
    assert_l0001(
        &diagnostics[0],
        "future feature notafeature is not defined",
        span_of(source, source.trim_end(), 0),
    );
}

#[test]
fn a_future_import_may_follow_a_docstring_or_another_future_import() {
    let hir = lower_all_ok("\"\"\"doc\"\"\"\nfrom __future__ import annotations\nx: int = 1\n");
    assert!(hir.imports.is_empty());
    let hir = lower_all_ok(
        "from __future__ import annotations\nfrom __future__ import division\nx: int = 1\n",
    );
    assert!(hir.imports.is_empty());
    // An implicitly concatenated string is one `StringLiteral` node, so
    // it is a docstring too.
    let hir = lower_all_ok("\"a\" \"b\"\nfrom __future__ import annotations\n");
    assert!(hir.imports.is_empty());
}

#[test]
fn a_future_import_after_any_other_statement_is_a_position_l0001() {
    const MESSAGE: &str = "from __future__ imports must occur at the beginning of the file";
    for source in [
        "import math\nfrom __future__ import annotations\n",
        "x: int = 1\nfrom __future__ import annotations\n",
        // The position check precedes every name check ...
        "x: int = 1\nfrom __future__ import notafeature\n",
        // ... and the alias check.
        "import math\nfrom __future__ import annotations as ann\n",
        // The prologue is contiguous: a docstring *between* two future
        // imports ends it.
        "from __future__ import annotations\n\"\"\"doc\"\"\"\nfrom __future__ import division\n",
        // An f-string at index 0 is not a docstring (a bytes literal is
        // not one either, but pycc rejects the literal itself first, so
        // that shape is pinned on `future_prologue_len` below instead).
        "f\"doc\"\nfrom __future__ import annotations\n",
        // A bare string that is not at index 0 is not a docstring either.
        "x: int = 1\n\"doc\"\nfrom __future__ import annotations\n",
    ] {
        let diagnostics = lower_all_err(source);
        // The *last* future import is the late one.
        let start = source
            .rfind("from __future__")
            .expect("every fixture has a future import");
        let end = start + source[start..].trim_end().len();
        assert_l0001(
            &diagnostics[0],
            MESSAGE,
            Span::new(start as u32, end as u32),
        );
        // The statement contributed nothing, so nothing is poisoned and the
        // late import is the only diagnostic.
        assert_eq!(diagnostics.len(), 1, "{source}");
    }
}

#[test]
fn future_prologue_len_counts_the_docstring_and_the_contiguous_future_run() {
    for (source, expected) in [
        ("", 0),
        ("\"doc\"\n", 1),
        (
            "\"doc\"\nfrom __future__ import annotations\nx: int = 1\n",
            2,
        ),
        ("x: int = 1\nfrom __future__ import annotations\n", 0),
        (
            "from __future__ import annotations\nx: int = 1\nfrom __future__ import division\n",
            1,
        ),
        (
            "from __future__ import annotations\nfrom __future__ import division\n",
            2,
        ),
        // A relative `from .__future__ import x` is not the directive.
        ("from .__future__ import annotations\n", 0),
        ("f\"doc\"\nfrom __future__ import annotations\n", 0),
        ("b\"doc\"\nfrom __future__ import annotations\n", 0),
    ] {
        assert_eq!(
            future_prologue_len(&parse(source).body),
            expected,
            "{source:?}"
        );
    }
}

#[test]
fn a_rejected_future_import_poisons_the_names_it_would_have_bound() {
    // `from __future__ import notafeature` fails, so a later read of
    // `notafeature` is a cascade of that failure and is suppressed: the
    // `L0001` is the only diagnostic.
    let source =
        "from __future__ import notafeature\ndef f(a: notafeature) -> int:\n    return 1\n";
    let diagnostics = lower_all_err(source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "L0001");
    // `poisonable_names` is asname-aware for the failing shapes ...
    let module = parse("from __future__ import notafeature as n\n");
    assert_eq!(poisonable_names(&module.body[0]), vec!["n"]);
    // ... poisons `barry_as_FLUFL` because pycc rejects it ...
    let module = parse("from __future__ import annotations, barry_as_FLUFL\n");
    assert_eq!(
        poisonable_names(&module.body[0]),
        vec!["annotations", "barry_as_FLUFL"]
    );
    // ... poisons the literal `*` for a wildcard (nothing to expand) ...
    let module = parse("from __future__ import *\n");
    assert_eq!(poisonable_names(&module.body[0]), vec!["*"]);
    // ... and poisons nothing for a shape that lowers.
    let module = parse("from __future__ import annotations, division\n");
    assert!(poisonable_names(&module.body[0]).is_empty());
}

#[test]
fn a_bare_import_of_dunder_future_is_unchanged() {
    // Only the `from` form is the directive; `import __future__` still
    // takes the ordinary `Stmt::Import` path.
    let source = "import __future__\n";
    let diagnostics = lower_all_err(source);
    assert_c0001(
        &diagnostics[0],
        "import of module `__future__` is not supported yet",
        span_of(source, source.trim_end(), 0),
    );
}

// ---------------------------------------------------------------------------
// #1213: chained assignment at module top level.

/// A chain expands into one top-level item per piece, and its temporary is
/// never a definition `program::link` can collide on -- whether the chain is
/// a top-level statement or nested inside one (`killed_names` recurses).
#[test]
fn a_chained_assignment_records_its_targets_but_never_its_temporary() {
    let source =
        "def g() -> int:\n    return 1\na = b = g()\nfor i in range(2):\n    c = d = g()\n";
    let module = parse(source);
    let lowered =
        lower_module(&module, &ResolvedImports::default(), None).expect("chains must lower");
    let mut defined: Vec<&str> = lowered
        .definition_spans
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    defined.sort_unstable();
    assert_eq!(defined, ["a", "b", "c", "d", "g", "i"]);
    // `g`, the temporary, `a`, `b`, then the `for` loop.
    assert_eq!(lowered.hir.items.len(), 5);
    let chain = span_of(source, "a = b = g()", 0);
    assert!(
        lowered
            .definition_spans
            .iter()
            .filter(|(name, _)| name == "a" || name == "b")
            .all(|(_, span)| *span == chain)
    );
}

/// A refused piece of a chain records nothing, not even the pieces lowered
/// before it: `lower_top_level_item`'s `Err` contract. `t` is a bare name,
/// so the first piece is the copied `a = t`, lowered before the refused
/// tuple piece. Had it reached `state.items`, the module's class/value
/// collision check would refuse the later `class a` with a second `C0001`;
/// the single diagnostic proves it did not.
#[test]
fn a_chain_with_a_refused_piece_records_nothing() {
    let source = "a = (b, c) = t\nclass a:\n    pass\n";
    let diagnostics = lower_module(&parse(source), &ResolvedImports::default(), None)
        .expect_err("a tuple piece is refused");
    assert_eq!(diagnostics[0].code, "C0001");
    assert_eq!(diagnostics.len(), 1);
}
