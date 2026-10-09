//! Part 2 of #541 (D-189): assignment of runtime exception type tags to
//! user-declared exception classes during HIR lowering.
//!
//! Kept in its own file rather than appended to `crates/pycc_hir/src/tests.rs`
//! (already several thousand lines) per this repository's
//! "keep source files decomposable" rule.

use crate::pycc_parser_test_helper;
use crate::{FIRST_USER_EXCEPTION_TYPE_TAG, MAX_USER_EXCEPTION_CLASSES, lower_checked};

fn tags_of(source: &str) -> Vec<(String, Option<u8>)> {
    let module = pycc_parser_test_helper::parse(source);
    lower_checked(&module)
        .expect("source must lower cleanly")
        .class_defs
        .iter()
        .map(|(name, def)| (name.clone(), def.exception_type_tag))
        .collect()
}

#[test]
fn user_exception_classes_are_tagged_in_source_order_from_seven() {
    let tags = tags_of(
        "class AppError(Exception):\n    pass\n\n\n\
         class DatabaseError(AppError):\n    pass\n\n\n\
         class ConfigError(AppError):\n    pass\n\n\n\
         def main() -> None:\n    raise DatabaseError(\"x\")\n",
    );
    assert_eq!(
        tags[..3].to_vec(),
        vec![
            ("AppError".to_string(), Some(30)),
            ("DatabaseError".to_string(), Some(31)),
            ("ConfigError".to_string(), Some(32)),
        ]
    );
}

#[test]
fn the_first_user_tag_sits_immediately_above_the_builtin_range() {
    assert_eq!(FIRST_USER_EXCEPTION_TYPE_TAG, 30);
    let tags = tags_of(
        "class AppError(Exception):\n    pass\n\n\n\
         def main() -> None:\n    raise AppError(\"x\")\n",
    );
    assert_eq!(tags[0].1, Some(FIRST_USER_EXCEPTION_TYPE_TAG));
}

#[test]
fn the_original_flat_seven_are_never_tagged_by_this_pass() {
    // D-188 property 3: `None` here means "not raisable by *this per-module
    // tag-assignment pass*", never "synthetic". The original flat seven are
    // the most raisable classes there are and still carry `None`, because
    // their tags are fixed constants resolved by name rather than assigned
    // per module. Part 2 of #543 (#739) widened `BUILTIN_EXCEPTION_CLASSES`
    // with the 16-member `OSError` family, which *does* carry a tag of its
    // own -- fixed at HIR-construction time in `builtin_exception_class_defs`,
    // not by this per-module pass either, but nonzero -- so this test checks
    // only the flat seven, not every `is_builtin_exception_class` name.
    let tags = tags_of(
        "class AppError(Exception):\n    pass\n\n\n\
         def main() -> None:\n    raise AppError(\"x\")\n",
    );
    for (name, tag) in &tags {
        if crate::is_flat_builtin_exception_class(name) {
            assert_eq!(
                *tag, None,
                "flat builtin `{name}` must carry no assigned tag"
            );
        }
    }
}

#[test]
fn the_oserror_family_carries_its_fixed_tag_alongside_a_user_class() {
    // The 16-member `OSError` family's tags are fixed at HIR-construction
    // time, not by the per-module user-class pass, but they must still show
    // up correctly in a module's `class_defs` next to a user-declared class.
    let tags = tags_of(
        "class AppError(Exception):\n    pass\n\n\n\
         def main() -> None:\n    raise AppError(\"x\")\n",
    );
    let os_error = tags
        .iter()
        .find(|(name, _)| name == "OSError")
        .expect("`OSError` must be seeded");
    assert_eq!(os_error.1, Some(7));
}

#[test]
fn a_class_outside_the_exception_hierarchy_is_not_tagged() {
    let tags = tags_of(
        "class Point:\n    def __init__(self, x: int) -> None:\n        self.x = x\n\n\n\
         class AppError(Exception):\n    pass\n\n\n\
         def main() -> None:\n    raise AppError(\"x\")\n",
    );
    assert_eq!(tags[0], ("Point".to_string(), None));
    assert_eq!(tags[1].1, Some(30));
}

#[test]
fn a_module_that_never_names_a_builtin_exception_tags_nothing() {
    // Without seeding there is no builtin hierarchy for an MRO to reach, so
    // the tag pass must not run at all.
    let tags = tags_of(
        "class Point:\n    def __init__(self, x: int) -> None:\n        self.x = x\n\n\n\
         def main() -> None:\n    p = Point(1)\n    print(p.x)\n",
    );
    assert_eq!(tags, vec![("Point".to_string(), None)]);
}

fn many_exception_classes(count: usize) -> String {
    let mut source = String::new();
    for index in 0..count {
        source.push_str(&format!("class E{index}(Exception):\n    pass\n\n\n"));
    }
    source.push_str("def main() -> None:\n    raise E0(\"x\")\n");
    source
}

#[test]
fn the_maximum_number_of_user_exception_classes_still_lowers() {
    // 255 - 30: the top tag is reserved for a bridged foreign
    // non-`Exception` (#1316).
    assert_eq!(MAX_USER_EXCEPTION_CLASSES, 225);
    let module =
        pycc_parser_test_helper::parse(&many_exception_classes(MAX_USER_EXCEPTION_CLASSES));
    let lowered = lower_checked(&module).expect("225 exception classes must lower");
    let last = lowered
        .class_defs
        .iter()
        .find(|(name, _)| name == "E224")
        .expect("the last class must be present");
    assert_eq!(last.1.exception_type_tag, Some(u8::MAX - 1));
}

#[test]
fn one_exception_class_past_the_maximum_is_rejected() {
    let module =
        pycc_parser_test_helper::parse(&many_exception_classes(MAX_USER_EXCEPTION_CLASSES + 1));
    let diagnostic = lower_checked(&module).expect_err("226 exception classes must be rejected");
    assert_eq!(diagnostic.code, "C0001");
    assert!(
        diagnostic.message.contains("at most 225"),
        "unexpected message: {}",
        diagnostic.message
    );
}

#[test]
fn no_user_exception_class_is_ever_given_the_foreign_base_tag() {
    // #1316: tag 255 belongs to a bridged foreign non-`Exception`, which
    // `except Exception` must not match; a user class carrying it would
    // silently escape every `except Exception:` handler.
    assert_eq!(crate::FOREIGN_BASE_EXCEPTION_TYPE_TAG, u8::MAX);
    let module =
        pycc_parser_test_helper::parse(&many_exception_classes(MAX_USER_EXCEPTION_CLASSES));
    let lowered = lower_checked(&module).expect("the maximum must lower");
    assert!(
        lowered
            .class_defs
            .iter()
            .all(|(_, def)| def.exception_type_tag != Some(crate::FOREIGN_BASE_EXCEPTION_TYPE_TAG))
    );
}
