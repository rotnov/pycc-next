//! v0.12: a published class whose statement arguments the build planned is
//! made at its statement, in one step when it can be (`deferred_classes`),
//! and its plan carries the class body's `__firstlineno__`, `__doc__` and
//! `__static_attributes__` (`class_body_attrs`, `attach_class_bases`).

use super::*;
use crate::ext_build::deferred_classes as deferred;

fn published(class: &str) -> ExtPublishedClass {
    ExtPublishedClass {
        class: class.to_string(),
        methods: Vec::new(),
        mro: vec![class.to_string()],
        constants: Vec::new(),
        bases: None,
        spec: None,
        defaults: Vec::new(),
    }
}

const SOURCE: &str = "class Base:\n    \"\"\"  First.\n\n      Second.\n    \"\"\"\n\n    \
                      def __init__(self) -> None:\n        self.b = 1\n        self.a = 2\n\n\n\
                      @mark\nclass Child(Base):\n    tag = None\n\n    def f(self) -> None:\n        \
                      self.z = 0\n";

fn attached() -> Vec<ExtPublishedClass> {
    let mut publications = vec![published("Base"), published("Child")];
    attach_class_bases(&mut publications, &module(Vec::new()), SOURCE, SOURCE);
    publications
}

#[test]
fn the_plan_carries_the_class_bodys_first_line_docstring_and_static_attributes() {
    let publications = attached();
    assert_eq!(
        publications[0].bases.as_deref(),
        Some(
            "(), {\"__firstlineno__\": 1, \"__doc__\": \"First.\\n\\nSecond.\\n\", \
             \"__static_attributes__\": (\"a\", \"b\"), }"
        )
    );
    assert_eq!(
        publications[0].spec.as_deref(),
        Some("f 1\nd First.\\n\\nSecond.\\n\ns a\ns b\n")
    );
    // The first decorator's line; no docstring; one attribute.
    assert_eq!(
        publications[1].bases.as_deref(),
        Some(
            "(Base), {\"__firstlineno__\": 12, \"tag\": None, \
             \"__static_attributes__\": (\"z\",), }"
        )
    );
    assert_eq!(
        publications[1].spec.as_deref(),
        Some("a Base\nf 12\nn tag\ns z\n")
    );
}

#[test]
fn a_docstring_holding_a_nul_is_only_evaluated_from_the_source() {
    let source = "class C:\n    \"a\\0b\\\\\"\n";
    let mut publications = vec![published("C")];
    attach_class_bases(&mut publications, &module(Vec::new()), source, source);
    assert_eq!(publications[0].spec, None);
    assert_eq!(
        publications[0].bases.as_deref(),
        Some("(), {\"__firstlineno__\": 1, \"__doc__\": \"a\\x00b\\\\\", \"__static_attributes__\": (), }")
    );
}

#[test]
fn a_deferred_class_gets_a_template_maker_and_both_lookups_answer_it() {
    let publications = attached();
    let ordered: Vec<&ExtPublishedClass> = publications.iter().collect();
    let makers = deferred::makers_c(
        &ordered,
        |p| format!("BASE_OF_{}", p.class),
        |p| format!("    CONSTANTS_OF_{};\n", p.class),
    );
    for class in ["Base", "Child"] {
        assert!(makers.contains(&format!(
            "static int pycc_ext_class_constants_{class}(PyObject *type)\n{{\n    \
             CONSTANTS_OF_{class};\n"
        )));
        assert!(makers.contains(&format!("    base = BASE_OF_{class};\n")));
        assert!(makers.contains(&format!(
            "type = PyType_FromSpecWithBases(&pycc_ext_type_spec_{class}, base);"
        )));
        assert!(makers.contains(&format!(
            "    if (pycc_ext_type_object_{class} == NULL) {{\n        \
             pycc_ext_type_object_{class} = Py_NewRef(type);\n    }}\n"
        )));
    }
    let lookups = deferred::lookups_c(&publications);
    assert!(lookups.starts_with(deferred::PUBLISH_CLASS_DEFERRED_DECL));
    assert!(lookups.contains(deferred::CLASS_TEMPLATE_DECL));
    assert!(lookups.contains(
        "    if (strcmp(name, \"Child\") == 0) {\n        \
         *spec = &pycc_ext_type_spec_Child;\n        \
         *template_slot = &pycc_ext_template_object_Child;\n"
    ));
}

#[test]
fn a_class_with_no_plan_keeps_its_template_and_the_lookups_still_link() {
    let mut eager = published("Eager");
    eager.bases = Some("(), {}".to_string());
    assert!(!deferred::is_deferred(&eager));
    assert!(deferred::makers_c(&[&eager], |_| String::new(), |_| String::new()).is_empty());
    let lookups = deferred::lookups_c(&[eager]);
    assert!(!lookups.contains("strcmp"));
    assert!(lookups.contains("    return 0;\n}\n"));
    assert!(lookups.contains("? template_of()\n        : NULL;"));
}
