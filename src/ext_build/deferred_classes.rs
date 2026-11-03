//! v0.12: published classes made once, at their statement.
//!
//! Until v0.12 every published class had two type objects: a *template*
//! made at module init (`PyType_FromSpecWithBases`, its layout, methods,
//! slots and field descriptors) and the class itself, made from it when the
//! class statement ran (`pycc_ext_make_public_class_in`). A class whose
//! statement arguments the build planned (`ExtPublishedClass::spec`) now
//! gets no template at init. When its statement runs, the shim makes the
//! class in one step from the template's spec when the statement is the
//! common one -- bases that are only compiled classes or `object`
//! (`pycc_ext_make_public_class_once_in`, `src/ext/module/class_maker.c`) --
//! and otherwise makes the template then, and the class from it as before.
//! A template is also made on first need before the statement runs: an
//! instance crossing into CPython (`pycc_ext_carrier_type`), a template of
//! this module deriving from it, or the post-body safety net.
//!
//! This module renders the per-class half of that: the class's template
//! maker and constants setter, and the lookups the shim calls by name.

use super::ExtPublishedClass;

/// The exact C declaration of the generated lookup `pycc_ext_publish` calls
/// for a class it made no template for: 1 with the class's spec, the
/// static holding its template (NULL until one is made), its template
/// maker and its constants setter; 0 for any other name.
pub(crate) const PUBLISH_CLASS_DEFERRED_DECL: &str =
    "static int pycc_ext_publish_class_deferred(const char *name, PyType_Spec **spec, \
     PyObject ***template_slot, PyObject *(**template_of)(void), int (**constants)(PyObject *))";

/// The exact C declaration of the generated template lookup the module's
/// record carries (`PyccExtModule::class_template`).
pub(crate) const CLASS_TEMPLATE_DECL: &str =
    "static PyObject *pycc_ext_class_template(const char *name)";

/// Whether `published` is made at its statement: its arguments are
/// planned (`ExtPublishedClass::spec`), the one case the shim can make it
/// in one step. A class evaluated from its source text, or published with
/// no bases of its own, keeps the template made at init.
pub(crate) fn is_deferred(published: &ExtPublishedClass) -> bool {
    published.bases.is_some() && published.spec.is_some()
}

/// The template static, maker and constants setter of each deferred class,
/// in `ordered` (template) order, so a maker calls only makers defined
/// above it. `base_of` gives the C expression of a class's template base.
///
/// The template has a static of its own (`pycc_ext_template_object_<Class>`,
/// which every published class has, cleared as each exec starts): the
/// class's type static holds the class once its statement made it, and a
/// template made later -- for a template deriving from it, or an instance
/// crossing before a nested exec's statement -- must not be that class. A
/// template made before the statement is also the type static until the
/// statement replaces it, as every template was before v0.12.
pub(crate) fn makers_c(
    ordered: &[&ExtPublishedClass],
    base_of: impl Fn(&ExtPublishedClass) -> String,
    constants_of: impl Fn(&ExtPublishedClass) -> String,
) -> String {
    let mut out = String::new();
    for published in ordered.iter().filter(|published| is_deferred(published)) {
        let class = &published.class;
        out.push_str(&format!(
            "static int pycc_ext_class_constants_{class}(PyObject *type)\n{{\n{constants}    \
             (void)type;\n    return 0;\n}}\n\n",
            constants = constants_of(published)
        ));
        out.push_str(&format!(
            "static PyObject *pycc_ext_class_template_{class}(void)\n{{\n    \
             PyObject *type;\n    PyObject *base;\n\n    \
             if (pycc_ext_template_object_{class} != NULL) {{\n        \
             return pycc_ext_template_object_{class};\n    }}\n    \
             base = {base};\n    if (base == NULL) {{\n        return NULL;\n    }}\n    \
             type = PyType_FromSpecWithBases(&pycc_ext_type_spec_{class}, base);\n    \
             if (type == NULL) {{\n        return NULL;\n    }}\n    \
             if (pycc_ext_class_constants_{class}(type) < 0\n        \
             || pycc_ext_carrier_register(\"{class}\", type) < 0) {{\n        \
             Py_DECREF(type);\n        return NULL;\n    }}\n    \
             pycc_ext_template_object_{class} = type;\n    \
             if (pycc_ext_type_object_{class} == NULL) {{\n        \
             pycc_ext_type_object_{class} = Py_NewRef(type);\n    }}\n    \
             return type;\n}}\n\n",
            base = base_of(published)
        ));
    }
    out
}

/// [`PUBLISH_CLASS_DEFERRED_DECL`] and [`CLASS_TEMPLATE_DECL`], emitted
/// unconditionally (answering 0 and NULL when no class is deferred) so
/// every artifact links.
pub(crate) fn lookups_c(publications: &[ExtPublishedClass]) -> String {
    let mut out = format!("{PUBLISH_CLASS_DEFERRED_DECL}\n{{\n");
    for published in publications.iter().filter(|published| is_deferred(published)) {
        let class = &published.class;
        out.push_str(&format!(
            "    if (strcmp(name, \"{class}\") == 0) {{\n        \
             *spec = &pycc_ext_type_spec_{class};\n        \
             *template_slot = &pycc_ext_template_object_{class};\n        \
             *template_of = pycc_ext_class_template_{class};\n        \
             *constants = pycc_ext_class_constants_{class};\n        return 1;\n    }}\n"
        ));
    }
    out.push_str(
        "    (void)name;\n    (void)spec;\n    (void)template_slot;\n    (void)template_of;\n    \
         (void)constants;\n    \
         return 0;\n}\n\n",
    );
    out.push_str(&format!(
        "{CLASS_TEMPLATE_DECL}\n{{\n    PyType_Spec *spec;\n    PyObject **template_slot;\n    \
         PyObject *(*template_of)(void);\n    int (*constants)(PyObject *);\n\n    \
         return pycc_ext_publish_class_deferred(name, &spec, &template_slot, &template_of, \
         &constants)\n        \
         ? template_of()\n        : NULL;\n}}\n\n"
    ));
    out
}
