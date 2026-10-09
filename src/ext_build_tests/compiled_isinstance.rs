//! Generated-text tests for the compiled-class `isinstance` the `--ext`
//! build emits for Part 7 of #1371: the published-family dispatch and the
//! per-class type-object statics it reads.
//!
//! Kept out of `generated_c.rs`, which is past `AGENTS.md`'s ~1,000-line
//! decomposability threshold.

use super::generated_c::{inc_no_classes, static_export};
use super::*;

/// Part 7 of #1371: a class name answers through every published type
/// whose MRO contains it, in publication order -- a base through itself
/// and its published subclass, a subclass through itself only -- each
/// shared ancestor gets one test per name, not one per class, and `object`,
/// never a compiled class argument, gets none.
#[test]
fn the_compiled_class_isinstance_tests_each_published_type_whose_mro_names_the_class() {
    let published = |class: &str, mro: &[&str]| ExtPublishedClass {
        class: class.to_string(),
        methods: vec![static_export(class, "m", vec![], Ty::Int)],
        mro: mro.iter().map(|name| name.to_string()).collect(),
    };
    let c = compiled_class_isinstance_c(&[
        published("Base", &["Base", "object"]),
        published("Derived", &["Derived", "Base", "object"]),
    ]);
    let test = |class: &str| {
        format!(
            "        found = PyObject_IsInstance(o, pycc_ext_type_object_{class});\n        \
             if (found != 0) {{\n            return found;\n        }}\n"
        )
    };
    let expected = format!(
        "{COMPILED_CLASS_ISINSTANCE_DECL}\n{{\n    int found;\n    \
         if (strcmp(name, \"Base\") == 0) {{\n{base}{derived}        return 0;\n    }}\n    \
         if (strcmp(name, \"Derived\") == 0) {{\n{derived}        return 0;\n    }}\n    \
         return {UNPUBLISHED_CLASS_ISINSTANCE}(o);\n}}\n\n",
        base = test("Base"),
        derived = test("Derived"),
    );
    assert_eq!(c, expected);
}

#[test]
fn every_published_class_keeps_its_type_object_in_a_file_static() {
    let inc = inc_no_classes(
        "m",
        &[static_export("Grid", "scale", vec![Ty::Int], Ty::Int)],
    );
    assert!(
        inc.contains("static PyObject *pycc_ext_type_object_Grid;\n"),
        "{inc}"
    );
    // The static is declared before the `isinstance` that reads it.
    let declared = inc
        .find("static PyObject *pycc_ext_type_object_Grid;")
        .unwrap();
    let read = inc.find(COMPILED_CLASS_ISINSTANCE_DECL).unwrap();
    assert!(declared < read, "{inc}");
}

/// #1435: a carrier answers from its run-time class's MRO -- each class a
/// length-checked `memcmp` on the descriptor's name, then one `strcmp` per
/// MRO entry but `object` -- and an unknown class answers 0.
#[test]
fn the_carrier_isinstance_answers_from_each_classs_mro() {
    let carrier = |class: &str, mro: &[&str]| ExtCarrierClass {
        class: class.to_string(),
        mro: mro.iter().map(|name| name.to_string()).collect(),
        copy: CarrierCopy::Kinds(String::new()),
    };
    let c = carrier_class_isinstance_c(&[
        carrier("Q", &["Q", "object"]),
        carrier("Derived", &["Derived", "Q", "object"]),
    ]);
    let expected = format!(
        "{CARRIER_CLASS_ISINSTANCE_DECL}\n{{\n    \
         if (len == 1 && memcmp(cls, \"Q\", 1) == 0) {{\n        \
         return strcmp(name, \"Q\") == 0;\n    }}\n    \
         if (len == 7 && memcmp(cls, \"Derived\", 7) == 0) {{\n        \
         return strcmp(name, \"Derived\") == 0 || strcmp(name, \"Q\") == 0;\n    }}\n    \
         return 0;\n}}\n\n"
    );
    assert_eq!(c, expected);
}

/// With no carriable class the function still links, answering 0.
#[test]
fn the_carrier_isinstance_of_a_classless_module_answers_zero() {
    assert_eq!(
        carrier_class_isinstance_c(&[]),
        format!(
            "{CARRIER_CLASS_ISINSTANCE_DECL}\n{{\n    (void)cls;\n    (void)len;\n    \
             (void)name;\n    return 0;\n}}\n\n"
        )
    );
}

/// The table holds every regular class, published or not, generic ones
/// included, and leaves out enums and exception classes, which never cross
/// (`foreign.rs`'s `is_carriable_instance`).
#[test]
fn every_regular_class_is_a_carrier_class() {
    let source = "from enum import Enum\n\n\nclass Color(Enum):\n    RED = 1\n\n\n\
                  class E(Exception):\n    pass\n\n\nclass V(ValueError):\n    pass\n\n\n\
                  class Q:\n    def __init__(self, n: int) -> None:\n        self.n = n\n\n\n\
                  class R(Q):\n    pass\n\n\nclass G[T]:\n    def __init__(self, x: T) -> None:\n        \
                  self.x = x\n\n\nprint(G[int](1).x)\nprint(R(2).n)\n";
    let module = pycc_parser::parse(source).expect("test fixture must parse");
    let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
    let resolved = pycc_types::check_and_resolve(&hir).expect("test fixture must check");
    let classes: Vec<(String, Vec<String>)> = collect_carrier_classes(&resolved)
        .into_iter()
        .map(|carrier| (carrier.class, carrier.mro))
        .collect();
    let names: Vec<&str> = classes.iter().map(|(class, _)| class.as_str()).collect();
    // The generic's instantiation is a class of its own in HIR and gets a
    // (harmless) row too; only its name is mangled, so it is not pinned.
    assert_eq!(names[..3], ["Q", "R", "G"], "{classes:?}");
    for refused in ["Color", "E", "V"] {
        assert!(!names.contains(&refused), "{classes:?}");
    }
    assert_eq!(classes[1].1[..2], ["R".to_string(), "Q".to_string()]);
}

/// The companion defines the carrier `isinstance` from the table it is
/// handed, and the shim's `pycc_ext_obj_isinstance_compiled` -- defined
/// after the companion include -- calls it for a carrier before the
/// published-family rule.
#[test]
fn the_shim_answers_a_carrier_before_the_published_family() {
    let carriers = [ExtCarrierClass {
        class: "Q".to_string(),
        mro: vec!["Q".to_string()],
        copy: CarrierCopy::Kinds(String::new()),
    }];
    let inc = generate_exports_inc("m", &[], &[], &[], &[], &carriers);
    assert!(
        inc.contains(&carrier_class_isinstance_c(&carriers)),
        "{inc}"
    );
    let shim = shim_c();
    let include = shim.find("#include \"pycc_ext_exports.inc\"").unwrap();
    let definition = shim
        .find("int pycc_ext_obj_isinstance_compiled(PyObject *o, const char *name)\n{")
        .unwrap();
    assert!(include < definition);
    let body = &shim[definition..];
    let carrier = body
        .find("return pycc_ext_carrier_class_isinstance(cls, len, name);")
        .unwrap();
    let family = body
        .find("return pycc_ext_compiled_class_isinstance(o, name);")
        .unwrap();
    assert!(carrier < family);
    assert!(CARRIER_CLASS_ISINSTANCE_DECL.contains(" pycc_ext_carrier_class_isinstance("));
}
