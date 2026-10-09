//! Unit tests for the synthetic builtin exception classes (Part 1 of #541).
//!
//! Kept in their own submodule rather than appended to the crate's existing
//! `tests.rs`, per AGENTS.md's decomposability rule.

use super::*;
use crate::pycc_parser_test_helper::parse;
use crate::{HirModule, lower_checked};

fn lower(source: &str) -> HirModule {
    lower_checked(&parse(source)).expect("test fixture must lower")
}

fn class_named<'m>(hir: &'m HirModule, name: &str) -> Option<&'m HirClassDef> {
    hir.class_defs
        .iter()
        .find(|(class_name, _)| class_name == name)
        .map(|(_, def)| def)
}

// -- the synthetic definitions themselves ---------------------------

#[test]
fn every_builtin_exception_name_gets_exactly_one_definition() {
    let defs = builtin_exception_class_defs();
    assert_eq!(defs.len(), BUILTIN_EXCEPTION_CLASSES.len());
    for name in BUILTIN_EXCEPTION_CLASSES {
        let count = defs.iter().filter(|(n, _)| n == name).count();
        assert_eq!(count, 1, "`{name}` must appear exactly once");
    }
}

#[test]
fn exception_is_the_root_and_owns_the_only_constructor() {
    let defs = builtin_exception_class_defs();
    let (_, root) = defs
        .iter()
        .find(|(name, _)| name == "Exception")
        .expect("`Exception` must be synthesized");
    assert!(root.bases.is_empty());
    assert_eq!(root.mro, vec!["Exception".to_string()]);
    assert_eq!(
        root.methods,
        vec![(
            "__init__".to_string(),
            EXCEPTION_INIT_MANGLED_NAME.to_string()
        )]
    );
}

#[test]
fn every_non_root_builtin_exception_inherits_init_from_its_mro() {
    for (name, def) in builtin_exception_class_defs() {
        if name == "Exception" {
            continue;
        }
        assert!(
            def.methods.is_empty(),
            "`{name}` must inherit `__init__` through its MRO, not redeclare it"
        );
        // Every non-root's `mro` must end at `Exception`, whatever its depth.
        assert_eq!(def.mro.last(), Some(&"Exception".to_string()));
    }
}

/// Part 2 of #543 (#739): every class whose real parent is `Exception`
/// directly (the original flat six plus `OSError`, `ImportError`,
/// `AssertionError` and `AttributeError`) still gets the
/// historical two-entry MRO.
#[test]
fn direct_children_of_exception_get_a_two_entry_mro() {
    let defs = builtin_exception_class_defs();
    for name in [
        "ValueError",
        "TypeError",
        "KeyError",
        "IndexError",
        "ZeroDivisionError",
        "RuntimeError",
        "OSError",
        "ImportError",
        "AssertionError",
        "AttributeError",
    ] {
        let (_, def) = defs
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("`{name}` must be synthesized"));
        assert_eq!(def.bases, vec!["Exception".to_string()]);
        assert_eq!(def.mro, vec![name.to_string(), "Exception".to_string()]);
    }
}

/// #1292: `ModuleNotFoundError` carries CPython's real parentage -- it derives
/// from `ImportError`, not directly from `Exception`. The existing
/// `every_non_root_builtin_exception_inherits_init_from_its_mro` catches a
/// *missing* parent arm (the MRO would not end at `Exception`) but not a
/// class wrongly parented straight to `Exception`; this pin does.
#[test]
fn module_not_found_error_derives_from_import_error() {
    let defs = builtin_exception_class_defs();
    let (_, def) = defs
        .iter()
        .find(|(name, _)| name == "ModuleNotFoundError")
        .expect("`ModuleNotFoundError` must be synthesized");
    assert_eq!(def.bases, vec!["ImportError".to_string()]);
    assert_eq!(
        def.mro,
        vec![
            "ModuleNotFoundError".to_string(),
            "ImportError".to_string(),
            "Exception".to_string(),
        ]
    );
}

/// Part 2 of #543 (#739): `BrokenPipeError`'s real MRO is 3 ancestor levels
/// deep (`ConnectionError` -> `OSError` -> `Exception`), the deepest in the
/// hierarchy -- this is the regression lock for `builtin_exception_class_defs`'s
/// MRO-walk loop reaching its 2nd and 3rd iterations.
#[test]
fn broken_pipe_error_has_the_full_four_entry_mro() {
    let defs = builtin_exception_class_defs();
    let (_, def) = defs
        .iter()
        .find(|(name, _)| name == "BrokenPipeError")
        .expect("`BrokenPipeError` must be synthesized");
    assert_eq!(def.bases, vec!["ConnectionError".to_string()]);
    assert_eq!(
        def.mro,
        vec![
            "BrokenPipeError".to_string(),
            "ConnectionError".to_string(),
            "OSError".to_string(),
            "Exception".to_string(),
        ]
    );
}

/// `OSError`'s own MRO is the two-entry shape (it derives directly from
/// `Exception`), spot-checked separately from the loop above since it is the
/// root of the new subtree.
#[test]
fn os_error_has_a_two_entry_mro() {
    let defs = builtin_exception_class_defs();
    let (_, def) = defs
        .iter()
        .find(|(name, _)| name == "OSError")
        .expect("`OSError` must be synthesized");
    assert_eq!(def.bases, vec!["Exception".to_string()]);
    assert_eq!(
        def.mro,
        vec!["OSError".to_string(), "Exception".to_string()]
    );
}

#[test]
fn synthetic_definitions_carry_no_shape_beyond_the_hierarchy() {
    for (name, def) in builtin_exception_class_defs() {
        assert!(def.attrs.is_empty(), "`{name}` must have no slots");
        assert!(def.properties.is_empty());
        assert!(def.static_methods.is_empty());
        assert!(def.class_methods.is_empty());
        assert!(def.type_param.is_none());
        assert!(def.enum_members.is_empty());
        assert!(!def.is_dataclass);
        assert!(def.dataclass_fields.is_empty());
        assert!(!def.is_protocol);
        assert!(!def.runtime_checkable);
        assert!(def.protocol_members.is_empty());
        assert!(
            def.abstract_methods.is_empty(),
            "`{name}` must not be abstract -- an abstract class cannot be subclassed \
             without overriding every abstract method"
        );
        assert!(!def.is_abstract);
    }
}

#[test]
fn the_synthetic_constructor_takes_self_and_a_str_message() {
    let HirItem::Function {
        name,
        params,
        return_ty,
        body,
    } = builtin_exception_init_item()
    else {
        panic!("the synthetic constructor must lower to an ordinary function item");
    };
    assert_eq!(name, EXCEPTION_INIT_MANGLED_NAME);
    assert_eq!(
        params,
        vec![
            (
                "self".to_string(),
                Ty::Instance(Box::new("Exception".to_string()))
            ),
            ("message".to_string(), Ty::Str),
        ]
    );
    assert_eq!(return_ty, Ty::None);
    assert_eq!(body, vec![HirStmt::Return(None)]);
}

#[test]
fn lowering_records_provenance_for_the_seeded_names() {
    // The provenance record (D-188) is set exactly when the seven
    // definitions were seeded, and is the *only* thing downstream marks
    // synthetic membership from.
    let seeded = lower("raise ValueError(\"boom\")\n");
    assert!(seeded.seeded_builtin_exception_classes);
    assert_eq!(seeded.class_defs.len(), BUILTIN_EXCEPTION_CLASSES.len());

    // No reference to any of the seven: no seeding, no provenance.
    let unseeded = lower("def main() -> None:\n    pass\n");
    assert!(!unseeded.seeded_builtin_exception_classes);
    assert!(unseeded.class_defs.is_empty());

    // A user class that is byte-for-byte the synthetic `Exception` still
    // carries no provenance, because lowering did not produce it. Both
    // halves are asserted together: the structural-equality half keeps this
    // test honest if the synthetic shape ever changes.
    let user = lower("class Exception:\n    def __init__(self) -> None:\n        pass\n");
    assert!(!user.seeded_builtin_exception_classes);
    let synthetic_exception = builtin_exception_class_defs()
        .into_iter()
        .find(|(name, _)| name == "Exception")
        .expect("`Exception` must be synthesized")
        .1;
    assert_eq!(
        class_named(&user, "Exception"),
        Some(&synthetic_exception),
        "fixture must stay structurally identical to the synthetic definition"
    );
}

// -- the all-or-nothing shadow pre-scan ------------------------------

fn shadows(source: &str) -> bool {
    shadowed_builtin_exception_name(&parse(source)).is_some()
}

#[test]
fn an_ordinary_module_shadows_nothing() {
    assert!(!shadows("print(1)\n"));
    assert!(!shadows(
        "class Other:\n    def __init__(self) -> None:\n        pass\n"
    ));
    assert!(!shadows("def helper() -> int:\n    return 1\n"));
    assert!(!shadows("type Alias = int\n"));
    assert!(!shadows("x: int = 1\n"));
    assert!(!shadows("x = 1\n"));
}

#[test]
fn a_class_definition_shadows() {
    assert!(shadows(
        "class ValueError:\n    def __init__(self) -> None:\n        pass\n"
    ));
}

#[test]
fn a_function_definition_shadows() {
    assert!(shadows("def TypeError() -> int:\n    return 1\n"));
}

#[test]
fn a_type_alias_statement_shadows() {
    assert!(shadows("type KeyError = int\n"));
}

#[test]
fn an_annotated_assignment_shadows() {
    assert!(shadows("IndexError: int = 1\n"));
}

#[test]
fn a_plain_assignment_shadows() {
    assert!(shadows("RuntimeError = 1\n"));
}

#[test]
fn an_unpacking_assignment_target_shadows() {
    assert!(shadows("ZeroDivisionError, other = 1, 2\n"));
    assert!(shadows("[Exception, other] = [1, 2]\n"));
    assert!(shadows("*ValueError, other = [1, 2]\n"));
}

#[test]
fn a_non_name_assignment_target_does_not_shadow() {
    // An attribute or subscript target rebinds something other than a bare
    // module-level name, so it cannot shadow a builtin exception name.
    assert!(!shadows("holder.ValueError = 1\n"));
    assert!(!shadows("holder[\"ValueError\"] = 1\n"));
}

// -- seeding during lowering ------------------------------------------

#[test]
fn lowering_seeds_every_builtin_exception_class() {
    // `raise` is the cheapest spelling that makes the module a referencing
    // one; the seeding gate is exercised name-by-name further below.
    let hir = lower("raise ValueError(\"boom\")\n");
    assert_eq!(hir.class_defs.len(), BUILTIN_EXCEPTION_CLASSES.len());
    for name in BUILTIN_EXCEPTION_CLASSES {
        assert!(class_named(&hir, name).is_some(), "`{name}` must be seeded");
    }
}

#[test]
fn a_module_that_references_no_builtin_exception_name_is_seeded_with_none() {
    // The gate that keeps `frontend-perf-gate` green: every entry in
    // `class_defs` costs per-item work in lowering and per-function class
    // binding in `pycc_types`, and a module that never names one of the
    // seven cannot observe the difference.
    let hir = lower("print(1)\n");
    assert!(hir.class_defs.is_empty());
    assert!(
        !hir.items
            .iter()
            .any(|item| matches!(item, HirItem::Function { name, .. } if name
                == EXCEPTION_INIT_MANGLED_NAME))
    );
}

#[test]
fn a_class_bearing_module_that_names_no_builtin_exception_is_seeded_with_none() {
    let hir = lower("class A:\n    def __init__(self) -> None:\n        pass\n");
    assert_eq!(hir.class_defs.len(), 1);
    assert_eq!(hir.class_defs[0].0, "A");
}

#[test]
fn the_module_s_own_classes_come_first_in_source_order() {
    let hir = lower(
        "class A(Exception):\n    pass\n\n\
         class B:\n    def __init__(self) -> None:\n        pass\n",
    );
    assert_eq!(hir.class_defs[0].0, "A");
    assert_eq!(hir.class_defs[1].0, "B");
    assert_eq!(hir.class_defs.len(), 2 + BUILTIN_EXCEPTION_CLASSES.len());
}

// -- the reference gate -----------------------------------------------

fn references(source: &str) -> bool {
    module_references_builtin_exception_name(&parse(source))
}

#[test]
fn every_builtin_exception_name_is_recognized_as_a_reference() {
    for name in BUILTIN_EXCEPTION_CLASSES {
        assert!(references(&format!("x = {name}\n")), "`{name}` must count");
    }
}

#[test]
fn a_module_naming_none_of_the_seven_references_nothing() {
    assert!(!references("print(1)\n"));
    assert!(!references(
        "class A:\n    def __init__(self) -> None:\n        pass\n"
    ));
    // A *similar* name is not one of the seven.
    assert!(!references("x = ValueErrors\n"));
    // A string outside an annotation merely spells one: nothing resolves
    // it as a name.
    assert!(!references("x = \"ValueError\"\n"));
}

/// Part 1 of #889: annotation lowering resolves a string annotation like
/// its unquoted spelling, so the scan counts one -- top-level (unquoted by
/// `parse_all`) or nested (parsed by the scan itself), at any depth.
#[test]
fn a_builtin_exception_name_inside_a_string_annotation_is_a_reference() {
    for source in [
        "x: \"ValueError\"\n",
        "def f(e: list[\"ValueError\"]) -> None:\n    pass\n",
        "def f(e: list[\"list['ValueError']\"]) -> None:\n    pass\n",
        // a string nested in a type alias value, in both spellings
        "type E = list[\"ValueError\"]\n",
        "E: TypeAlias = list[\"ValueError\"]\n",
    ] {
        assert!(references(source), "must be a reference: {source:?}");
    }
    for source in [
        "def f(e: list[\"Value Error\"]) -> None:\n    pass\n",
        "def f(e: list[\"int\"]) -> None:\n    pass\n",
        "type E = list[\"int\"]\n",
        "E: TypeAlias = list[\"int\"]\n",
        // an ordinary annotated assignment's value is not a type
        "E: str = \"ValueError\"\n",
    ] {
        assert!(!references(source), "must not be a reference: {source:?}");
    }
}

#[test]
fn every_spelling_that_can_reach_the_class_table_counts_as_a_reference() {
    // Each of these is a position from which the frontend consults the class
    // table for one of the seven, so each must seed the module. A spelling
    // missed here fails silently -- as a spurious `C0001` or an
    // internal-compiler-error abort -- which is why the scan is ruff's own
    // exhaustive walker rather than a hand-rolled match.
    for source in [
        // a base class
        "class MyError(ValueError):\n    pass\n",
        // a `raise` operand
        "raise ValueError(\"boom\")\n",
        // an `except` type, at nested depth
        "def f() -> None:\n    try:\n        pass\n    except ValueError:\n        pass\n",
        // an `except ... as` type inside a nested function
        "def outer() -> None:\n    def inner() -> None:\n        try:\n            pass\n        except KeyError as e:\n            print(e)\n",
        // a parameter annotation
        "def f(e: ValueError) -> None:\n    return\n",
        // a return annotation
        "def f() -> TypeError:\n    return\n",
        // an annotated assignment
        "x: IndexError = 1\n",
        // an `isinstance` argument
        "def f(x: int) -> None:\n    print(isinstance(x, RuntimeError))\n",
        // an `issubclass` argument
        "def f() -> None:\n    print(issubclass(ZeroDivisionError, Exception))\n",
        // attribute access on the bare class name
        "def f() -> None:\n    print(ValueError.args)\n",
        // a type alias
        "type Alias = ValueError\n",
        // buried inside a comprehension
        "xs = [ValueError for i in range(3)]\n",
        // buried inside an f-string interpolation
        "s = f\"{Exception}\"\n",
        // buried inside a subscript
        "x = holder[KeyError]\n",
        // a decorator
        "@Exception\ndef f() -> None:\n    return\n",
    ] {
        assert!(references(source), "must be a reference: {source:?}");
    }
}

/// #1369: an `assert` raises `AssertionError` without spelling any
/// builtin exception name, so it must count as a reference on its own --
/// at module level and nested in a function -- or its lowering's
/// `AssertionError(...)` call would find no class.
#[test]
fn an_assert_statement_alone_counts_as_a_reference() {
    assert!(references("assert True\n"));
    assert!(references(
        "def f(x: int) -> None:\n    assert x > 0, \"x\"\n"
    ));
    assert!(!references("def f(x: int) -> None:\n    print(x)\n"));
    assert!(lower("def f(x: int) -> None:\n    assert x > 0\n").seeded_builtin_exception_classes);
}

/// #1369: the scan `module::lower_module` uses to refuse an `assert` in a
/// module whose shadow gate withheld the builtin classes finds the first
/// `assert` at any depth, spanned on the statement.
#[test]
fn the_first_assert_statement_is_found_at_any_depth() {
    use crate::exception::first_assert_statement_range;
    assert_eq!(
        first_assert_statement_range(&parse("x = 1\n").body, &[]),
        None
    );
    assert_eq!(
        first_assert_statement_range(&parse("x = 1\nassert x\nassert y\n").body, &[]),
        Some(6..14)
    );
    assert_eq!(
        first_assert_statement_range(
            &parse("def f() -> None:\n    if 1:\n        assert 0\n").body,
            &[]
        ),
        Some(35..43)
    );
}

/// #1369: a class body reaches `lower_stmt` only through its methods, so an
/// `assert` directly in a class body (or in a nested class, which the class
/// lowering refuses too) is not scanned, while one inside a method is.
#[test]
fn the_assert_scan_enters_a_class_only_through_its_methods() {
    use crate::exception::first_assert_statement_range;
    for refused_by_the_class in [
        "class A:\n    assert True\n",
        "class A:\n    class B:\n        def m(self) -> None:\n            assert True\n",
    ] {
        assert_eq!(
            first_assert_statement_range(&parse(refused_by_the_class).body, &[]),
            None,
            "{refused_by_the_class}"
        );
    }
    assert_eq!(
        first_assert_statement_range(
            &parse("class A:\n    x = 1\n\n    def m(self) -> None:\n        assert True\n").body,
            &[]
        ),
        Some(53..64)
    );
}

/// #1369: an `assert` the #790 `TYPE_CHECKING` fold discards is never
/// lowered, so the scan skips it -- in an `if` and in an `elif` arm, the
/// bare and the qualified spelling -- while every live arm of the same
/// chain, the `else` included, is still scanned.
#[test]
fn the_assert_scan_skips_only_the_bodies_the_type_checking_fold_discards() {
    use crate::exception::first_assert_statement_range;
    for dead in [
        "if TYPE_CHECKING:\n    assert 0\n",
        "if typing.TYPE_CHECKING:\n    assert 0\n",
        "if x:\n    pass\nelif TYPE_CHECKING:\n    assert 0\n",
        "def f() -> None:\n    if TYPE_CHECKING:\n        assert 0\n",
    ] {
        assert_eq!(
            first_assert_statement_range(&parse(dead).body, &[]),
            None,
            "{dead}"
        );
    }
    for live in [
        "if TYPE_CHECKING:\n    pass\nelse:\n    assert 0\n",
        "if TYPE_CHECKING:\n    pass\nelif x:\n    assert 0\n",
        "if x:\n    assert 0\n",
    ] {
        assert!(
            first_assert_statement_range(&parse(live).body, &[]).is_some(),
            "{live}"
        );
    }
}

/// #1369: a module that binds a builtin exception name at top level
/// withholds every builtin class, so an `assert` in it is refused once, on
/// the first `assert`, naming the binding responsible.
#[test]
fn an_assert_in_a_module_that_withheld_seeding_is_refused() {
    let source = "class ValueError:\n    pass\n\n\ndef f() -> None:\n    assert 1\n";
    let diagnostic = lower_checked(&parse(source)).expect_err("must be refused");
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains(
            "an `assert` statement needs the builtin `AssertionError`, which is unavailable \
             because this module binds the builtin exception name `ValueError` at top level"
        ),
        "{}",
        diagnostic.message
    );
}

/// #1369 under #867/D-233's per-item collection: the refusal is the
/// diagnostic of the first top-level item containing an `assert`, emitted in
/// source order, so an earlier independently failing item still reports
/// first (D-217 rule 2) and the loop keeps going -- a later `assert` adds no
/// second refusal.
#[test]
fn the_assert_refusal_is_collected_in_source_order_after_an_earlier_failure() {
    let source = "x = 1 @ 2\nclass ValueError:\n    pass\n\n\ndef f() -> None:\n    \
                  assert True\n\n\ndef g() -> None:\n    assert False\n";
    let diagnostics = crate::lower_all(&parse(source)).expect_err("must be refused");
    let messages: Vec<&str> = diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(diagnostics.len(), 2, "{messages:?}");
    assert!(messages[0].contains("binary operator `@`"), "{messages:?}");
    assert!(
        messages[1].contains("an `assert` statement needs the builtin `AssertionError`"),
        "{messages:?}"
    );
    assert_eq!(
        diagnostics[1].span,
        Some(pycc_diag::Span::new(60, 71)),
        "spanned on `f`'s `assert`"
    );
}

#[test]
fn a_bare_shadowing_class_is_not_seeded_on_the_shadow_gate_alone() {
    // The shadow gate alone decides this source: the reference scan never
    // sees it. `ReferenceScan` counts an `Expr::Name` (or an `assert`,
    // #1369, which this source does not contain), and this source
    // contains no `Expr::Name` at all -- a `ClassDef`'s own name is a bare
    // `Identifier` on the statement node, and the `-> None` annotation
    // parses as `Expr::NoneLiteral`, not as a name.
    let source = "class ValueError:\n    def __init__(self) -> None:\n        pass\n";
    assert!(!references(source));
    assert!(shadows(source));
    let hir = lower(source);
    assert!(!hir.seeded_builtin_exception_classes);
    assert_eq!(hir.class_defs.len(), 1);
}

#[test]
fn a_module_passing_the_reference_gate_is_still_blocked_by_the_shadow_gate() {
    // Both gates are exercised here: the annotation's `Expr::Name` makes
    // this a genuine reference, and the class's own name makes it a shadow.
    // Shadowing wins, so nothing is seeded -- and the annotation resolves
    // against the user's own class.
    let source = "class ValueError:\n    def __init__(self) -> None:\n        pass\n\ndef f(e: ValueError) -> None:\n    pass\n";
    assert!(references(source));
    assert!(shadows(source));
    let hir = lower(source);
    assert!(!hir.seeded_builtin_exception_classes);
}

#[test]
fn a_module_that_shadows_one_name_is_seeded_with_none_of_them() {
    let hir = lower("class ValueError:\n    def __init__(self) -> None:\n        pass\n");
    assert_eq!(hir.class_defs.len(), 1);
    assert_eq!(hir.class_defs[0].0, "ValueError");
    assert!(class_named(&hir, "Exception").is_none());
    assert!(
        !hir.items
            .iter()
            .any(|item| matches!(item, HirItem::Function { name, .. } if name
                == EXCEPTION_INIT_MANGLED_NAME))
    );
}

#[test]
fn a_user_class_can_inherit_from_a_builtin_exception_class() {
    let hir = lower("class MyError(ValueError):\n    pass\n");
    let def = class_named(&hir, "MyError").expect("`MyError` must be lowered");
    assert_eq!(def.bases, vec!["ValueError".to_string()]);
    assert_eq!(
        def.mro,
        vec![
            "MyError".to_string(),
            "ValueError".to_string(),
            "Exception".to_string()
        ]
    );
}

#[test]
fn the_synthetic_constructor_is_emitted_only_when_a_user_class_inherits_it() {
    let without = lower("try:\n    raise ValueError(\"x\")\nexcept ValueError:\n    print(1)\n");
    assert!(
        !without
            .items
            .iter()
            .any(|item| matches!(item, HirItem::Function { name, .. } if name
                == EXCEPTION_INIT_MANGLED_NAME)),
        "a module with no builtin-exception subclass needs no constructor body"
    );

    let with = lower("class MyError(ValueError):\n    pass\n");
    assert_eq!(
        with.items
            .iter()
            .filter(|item| matches!(item, HirItem::Function { name, .. } if name
                == EXCEPTION_INIT_MANGLED_NAME))
            .count(),
        1,
        "a module with a builtin-exception subclass gets exactly one constructor body"
    );
}

#[test]
fn a_user_class_may_inherit_from_two_builtin_exception_classes() {
    let hir = lower("class E(ValueError, TypeError):\n    pass\n");
    let def = class_named(&hir, "E").expect("`E` must be lowered");
    assert_eq!(
        def.mro,
        vec![
            "E".to_string(),
            "ValueError".to_string(),
            "TypeError".to_string(),
            "Exception".to_string()
        ]
    );
}

// -- the seeded definitions take part in every existing name check ---

#[test]
fn a_class_after_a_shadowing_alias_is_still_rejected() {
    // The pre-scan sees the `type Exception = int` binding and seeds
    // nothing, so this stays the ordinary alias-vs-class collision it was
    // before #541 rather than a collision against a synthetic definition.
    let err = lower_checked(&parse(
        "type Exception = int\n\nclass Exception:\n    def __init__(self) -> None:\n        pass\n",
    ))
    .expect_err("a class colliding with an alias must be rejected");
    assert_eq!(err.code, "C0001");
}
