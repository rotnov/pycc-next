//! Unit tests for `exception_isinstance.rs` (#1337, WI-6a).

use super::{isinstance_tags, tests_by_tag};
use pycc_hir::{HirClassDef, builtin_exception_class_defs};
use std::collections::HashMap;

fn seeded() -> HashMap<String, HirClassDef> {
    builtin_exception_class_defs().into_iter().collect()
}

fn user_class(name: &str, mro: &[&str], tag: Option<u8>) -> HirClassDef {
    let mut def = seeded()["ValueError"].clone();
    def.name = name.to_string();
    def.bases = vec![mro[1].to_string()];
    def.mro = mro.iter().map(|n| n.to_string()).collect();
    def.exception_type_tag = tag;
    def
}

#[test]
fn only_a_seeded_non_group_builtin_is_tested_by_tag() {
    let classes = seeded();
    assert!(tests_by_tag("ValueError", &classes));
    assert!(tests_by_tag("Exception", &classes));
    assert!(tests_by_tag("FileNotFoundError", &classes));
    assert!(!tests_by_tag("ExceptionGroup", &classes));
    assert!(!tests_by_tag("BaseExceptionGroup", &classes));
    assert!(!tests_by_tag("Plain", &classes));
    // A program shadowing a builtin name seeds nothing: its class is plain.
    let mut shadowed = HashMap::new();
    shadowed.insert(
        "ValueError".to_string(),
        user_class("ValueError", &["ValueError", "object"], None),
    );
    assert!(!tests_by_tag("ValueError", &shadowed));
}

#[test]
fn a_target_s_tags_are_its_own_and_every_raisable_subclass() {
    let mut classes = seeded();
    classes.insert(
        "E".to_string(),
        user_class("E", &["E", "ValueError", "Exception"], Some(200)),
    );
    classes.insert(
        "M".to_string(),
        user_class("M", &["M", "Mixin", "KeyError", "Exception"], Some(201)),
    );
    let mut value = isinstance_tags("ValueError", &classes);
    value.sort_unstable();
    assert_eq!(value, vec![1, 200]);
    assert_eq!(isinstance_tags("E", &classes), vec![200]);
    // A plain mixin has no tag of its own; only its raisable subclass counts.
    assert_eq!(isinstance_tags("Mixin", &classes), vec![201]);
    assert!(isinstance_tags("Plain", &classes).is_empty());
}
