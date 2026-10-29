//! The generated per-class type objects that publish an exported
//! `@staticmethod`, `@classmethod` or instance method as
//! `mod.Class.method`, and -- for a constructible class -- the `tp_init`
//! that makes `mod.Class(...)` build one.
//!
//! Extracted from `src/ext_build.rs` by #1143 under `AGENTS.md`'s
//! decomposability rule, together with `export_name.rs`. #1145 added the
//! constructor half.

use super::export_name::ExtReceiver;
use super::getset::{ExtClassGetsets, getset_c};
use super::instance_copy::{CarrierCopy, carrier_copy};
use super::richcompare::{self, ExtSlotDunders};
use super::{
    ExtCtor, ExtPublishedClass, arg_slot_locals, buffer_releases, c_param_list, defaults,
    fnptr_null_guard, keywords, source_level_name, unpack_args,
};
use pycc_hir::HirModule;

/// The exact C declaration of the generated method-class registration entry
/// point, called from `pycc_ext_exec_module` for the same reason and in the
/// same way as [`USER_EXCEPTION_REGISTER_DECL`]: the `.inc` is included
/// first, so it needs no forward declaration, and both the shim test and
/// the generated-text test assert this one constant so a spelling drift
/// fails an ordinary `cargo test`.
///
/// Emitted unconditionally -- with an empty body when the program exports
/// no method -- so every artifact links.
///
/// [`USER_EXCEPTION_REGISTER_DECL`]: super::USER_EXCEPTION_REGISTER_DECL
pub(crate) const METHOD_TYPE_REGISTER_DECL: &str =
    "static int pycc_ext_register_method_types(PyObject *module)";

/// The exact C declaration of the generated compiled-class `isinstance`
/// (Part 7 of #1371), which the module unit's record (`module_ctx.c`)
/// hands the shim's `pycc_ext_obj_isinstance_compiled`.
pub(crate) const COMPILED_CLASS_ISINSTANCE_DECL: &str =
    "static int pycc_ext_compiled_class_isinstance(PyObject *o, const char *name)";

/// The exact C declaration of the generated published-class lookup the
/// shim's `pycc_ext_publish` (to test that a name is a published class) and
/// `pycc_ext_snapshot_classes` (to copy each class's type object into the
/// exec's snapshot) call (#1199). The `.inc` is included above both callers,
/// so neither needs a forward declaration; the shim test and the
/// generated-text test assert this one constant.
pub(crate) const PUBLISH_CLASS_SLOT_DECL: &str =
    "static PyObject **pycc_ext_publish_class_slot(const char *name)";

/// The generated `NULL`-terminated array of every published class name, in
/// publication order, which the shim's post-body safety net walks (#1199).
pub(crate) const PUBLISH_CLASS_NAMES: &str = "pycc_ext_publish_class_names";

/// The shim's own answer for a class name no published type descends from
/// (`src/ext/module/`): it consults the object's `__class__` as
/// CPython's `isinstance` does before answering False, so a raising
/// `__class__` raises here too. Defined in the shim above the point the
/// companion is included at, so it needs no forward declaration; the shim
/// test asserts the definition against this one spelling.
pub(crate) const UNPUBLISHED_CLASS_ISINSTANCE: &str = "pycc_ext_unpublished_class_isinstance";

/// One type object per exporting class, plus the registration entry point
/// `pycc_ext_exec_module` calls.
///
/// **`PyType_FromSpec`, not `PyType_FromModuleAndSpec`.** The shim's
/// `m_size` is `0` and its file-scope statics are licensed by its refusal of
/// subinterpreters and of free-threaded hosts, so nothing here needs
/// `PyType_GetModule`.
///
/// **The class list and each table's rows are resolved upstream.**
/// `publications` is [`collect_class_publications`]' output: one entry per
/// class that resolves at least one exported member through its MRO or is
/// constructible, each carrying that MRO-resolved method set with a derived
/// override already shadowing its base's definition. A constructible class
/// that resolves nothing -- one with a perfectly carriable `__init__` but no
/// public method anywhere in its MRO, lark's `ParseConf` (#1450) -- gets a
/// type object whose method table holds only the shared `__copy__` row
/// (#1455) before the sentinel. This function
/// renders that decision and never re-derives it, so the MRO walk exists
/// once (`AGENTS.md`'s canonical-statement rule).
///
/// [`collect_class_publications`]: super::collect_class_publications
///
/// Every published type is a **carrier type** (#1435): its spec carries
/// `basicsize = sizeof(PyccExtInstance)` and the shim's shared
/// `pycc_ext_instance_dealloc`, and the registration enters it in the
/// shim's carrier-type cache, so an instance of the class passed to a call
/// on a CPython object crosses as an instance of this type.
///
/// A class in `ctors` is also **constructible**: its spec adds the slots
/// `Py_tp_new` (`PyType_GenericNew` directly) and the generated per-class
/// `pycc_ext_tp_init_<Class>` below, and it drops
/// `Py_TPFLAGS_DISALLOW_INSTANTIATION`. Every other class keeps the flag
/// that makes `mod.Class()` raise `TypeError: cannot create
/// '<mod>.<Class>' instances`.
///
/// Both shapes keep `Py_TPFLAGS_IMMUTABLETYPE` and neither adds
/// `Py_TPFLAGS_BASETYPE`, so the type's attributes cannot be replaced and
/// `class S(mod.Class): pass` raises `TypeError: type '<mod>.<Class>' is not
/// an acceptable base type`. Subclassing is out of scope rather than
/// forbidden on principle: a subclass would inherit `tp_init` and get an
/// instance whose slot layout is its *base's*.
///
/// `spec.name` is `PYCC_EXT_MODULE_NAME_STR ".<Class>"` as adjacent string
/// literals, which is what gives the type the right `__module__` and
/// `__qualname__`.
///
/// The registration function is emitted unconditionally, with an empty body
/// when nothing exports a method, so every artifact links -- the same
/// discipline [`exception_classes_c`] follows.
///
/// Each created type is also kept, as a strong reference, in the file
/// static `pycc_ext_type_object_<Class>` that
/// [`compiled_class_isinstance_c`] reads (Part 7 of #1371). A re-exec of
/// the module replaces it, so a type from before a reload is no longer
/// consulted -- as a reloaded Python module's new class is not the old one.
///
/// A class in `getsets` gets its field descriptors as `Py_tp_getset`
/// (#1442 for a constructible class, every published class since #1448),
/// and a class in `slots` (#1427) also gets its comparison and hash slots
/// (`richcompare::slot_rows`). A class in either list that is not published
/// gets a hidden carrier type carrying both, which the same registration
/// creates and enters in the carrier-type cache without adding it to the
/// module (`richcompare::hidden_carrier_types_c`).
///
/// [`exception_classes_c`]: super::exception_classes_c
pub(crate) fn method_types_c(
    publications: &[ExtPublishedClass],
    ctors: &[ExtCtor],
    getsets: &[ExtClassGetsets],
    slots: &[ExtSlotDunders],
) -> String {
    let mut out = String::new();
    for published in publications {
        out.push_str(&format!(
            "static PyObject *pycc_ext_type_object_{};\n",
            published.class
        ));
    }
    let mut emitted_getters: Vec<String> = Vec::new();
    for published in publications {
        let class = &published.class;
        let ctor = ctors.iter().find(|ctor| ctor.class == *class);
        out.push_str(&format!(
            "static PyMethodDef pycc_ext_type_methods_{class}[] = {{\n"
        ));
        for export in &published.methods {
            let method = export
                .method
                .as_deref()
                .expect("an export with a class carries a method name");
            // `METH_STATIC` delivers `self == NULL`; `METH_CLASS` delivers
            // the type object, which the wrapper discards; plain
            // `METH_FASTCALL` delivers the instance, which an instance
            // method's wrapper unwraps (#1145). Without `METH_KEYWORDS`
            // CPython still raises the keyword `TypeError` before the
            // wrapper is entered -- D-244 rule 7's closed boundary; a
            // keyword-enabled export (#1461) adds it, the same predicate
            // that gives its wrapper the `kwnames` parameter.
            let flags = keywords::method_flags(
                match export.receiver {
                    ExtReceiver::None => "METH_FASTCALL | METH_STATIC",
                    ExtReceiver::NullCls => "METH_FASTCALL | METH_CLASS",
                    ExtReceiver::SelfInstance => "METH_FASTCALL",
                },
                export,
            );
            out.push_str(&format!(
                "    {{\"{method}\", (PyCFunction)(void (*)(void))pycc_ext_wrap_{symbol}, \
                 {flags}, NULL}},\n",
                symbol = pycc_codegen::mangle_ext_name(&export.name)
            ));
        }
        // Every published type is a carrier type (#1435), so every one gets
        // the shared `__copy__` (#1455); the shim's run-time table decides
        // whether this class copies or refuses. No export can already be
        // named `__copy__`: the export-name rule refuses private names.
        debug_assert!(
            published
                .methods
                .iter()
                .all(|export| export.method.as_deref() != Some("__copy__")),
            "an export named `__copy__` would collide with the shared row"
        );
        out.push_str(
            "    {\"__copy__\", (PyCFunction)(void (*)(void))pycc_ext_instance_copy, \
             METH_NOARGS, NULL},\n",
        );
        // v0.9: the template's `__init_subclass__`, which lets only the
        // published class made from it through (`pycc_ext_publish`).
        out.push_str(
            "    {\"__init_subclass__\", \
             (PyCFunction)(void (*)(void))pycc_ext_template_init_subclass, \
             METH_VARARGS | METH_KEYWORDS | METH_CLASS, NULL},\n",
        );
        out.push_str("    {NULL, NULL, 0, NULL},\n};\n\n");
        if let Some(ctor) = ctor {
            out.push_str(&tp_new_c(ctor, &published.defaults));
            out.push_str(&tp_init_c(ctor, &published.defaults));
        }
        // #1448: after `tp_init` and before the slots, the order a
        // constructible class's text has had since #1427.
        let table = getsets.iter().find(|table| table.class == *class);
        if let Some(table) = table {
            out.push_str(&getset_c(table, &mut emitted_getters));
        }
        let class_slots = slots.iter().find(|entry| entry.class == *class);
        if let Some(entry) = class_slots {
            out.push_str(&richcompare::slot_functions_c(entry, &mut emitted_getters));
        }
        out.push_str(&format!(
            "static PyType_Slot pycc_ext_type_slots_{class}[] = {{\n    \
             {{Py_tp_methods, pycc_ext_type_methods_{class}}},\n"
        ));
        if ctor.is_some() {
            // `PyType_GenericNew` zeroes the whole object, so for the class
            // itself `inst` starts NULL and `mod.Class.__new__(mod.Class)`
            // -- which never runs `tp_init` -- yields a carrier the
            // wrappers' NULL guard refuses rather than one holding
            // indeterminate storage. A Python subclass's object also gets
            // an instance with every slot unassigned (`tp_new_c`).
            out.push_str(&format!(
                "    {{Py_tp_new, pycc_ext_tp_new_{class}}},\n    \
                 {{Py_tp_init, pycc_ext_tp_init_{class}}},\n"
            ));
        }
        // Every published type is a carrier type (#1435): an instance of
        // the class that crosses into CPython as a call argument is boxed
        // in it by `pycc_ext_obj_pack_instance`, constructible or not, so
        // each needs room for the carrier and the deallocator that unlinks
        // it.
        out.push_str("    {Py_tp_dealloc, pycc_ext_instance_dealloc},\n");
        // #1442: installed only when the class has a descriptor, so a class
        // with none keeps its slot array byte for byte; constructible or not
        // since #1448.
        if table.is_some() {
            out.push_str(&format!(
                "    {{Py_tp_getset, pycc_ext_type_getset_{class}}},\n"
            ));
        }
        // #1427: likewise only when the class resolves a comparison or
        // `__hash__`.
        if let Some(entry) = class_slots {
            out.push_str(&richcompare::slot_rows(entry));
        }
        out.push_str("    {0, NULL},\n};\n\n");
        // A non-constructible class keeps the `DISALLOW_INSTANTIATION` flag
        // Part 1 emitted; a constructible one must not refuse its own
        // constructor.
        // v0.9: a template the published class derives from (and, for a
        // class without bases of its own, the published class itself), so
        // `Py_TPFLAGS_BASETYPE`; its `__init_subclass__` refuses any other
        // subclass.
        let flags = match ctor {
            Some(_) => "Py_TPFLAGS_DEFAULT | Py_TPFLAGS_IMMUTABLETYPE | Py_TPFLAGS_BASETYPE",
            None => {
                "Py_TPFLAGS_DEFAULT | Py_TPFLAGS_DISALLOW_INSTANTIATION | \
                 Py_TPFLAGS_IMMUTABLETYPE | Py_TPFLAGS_BASETYPE"
            }
        };
        out.push_str(&format!(
            "static PyType_Spec pycc_ext_type_spec_{class} = {{\n    \
             {spec_name},\n    sizeof(PyccExtInstance),\n    0,\n    \
             {flags},\n    pycc_ext_type_slots_{class},\n}};\n\n",
            spec_name = super::type_spec_name(class)
        ));
    }
    let (hidden, hidden_registration) =
        richcompare::hidden_carrier_types_c(publications, getsets, slots, &mut emitted_getters);
    out.push_str(&hidden);
    if publications.iter().any(|published| !published.constants.is_empty()) {
        out.push_str(SET_CONSTANT_C);
    }
    out.push_str(&compiled_class_isinstance_c(publications));
    out.push_str(&publish_class_c(publications));
    out.push_str(&format!("{METHOD_TYPE_REGISTER_DECL}\n{{\n"));
    if publications.is_empty() && hidden_registration.is_empty() {
        out.push_str("    (void)module;\n    return 0;\n}\n");
        return out;
    }
    out.push_str("    PyObject *type;\n    PyObject *base;\n");
    // v0.9: base templates first, each template deriving from its first
    // published compiled base's (or from the root carrier type), so
    // templates combined by multiple inheritance share their layout.
    for published in template_order(publications) {
        let class = &published.class;
        let base = published.mro.iter().skip(1).find_map(|ancestor| {
            publications
                .iter()
                .find(|other| other.class == *ancestor)
                .map(|other| format!("pycc_ext_type_object_{}", other.class))
        });
        out.push_str(&match base {
            Some(base) => format!("    base = {base};\n"),
            None => "    base = pycc_ext_template_base();\n".to_string(),
        });
        out.push_str("    if (base == NULL) {\n        return -1;\n    }\n");
        // The carrier-type cache (#1435) takes its own reference, so on a
        // failing arm the local one is released, which keeps a failed
        // registration from leaking the type. On success the local
        // reference moves into the class's file static (Part 7 of #1371),
        // releasing the one a previous exec stored there. The type is not
        // added to the module here (#1199): `pycc_ext_snapshot_classes`
        // copies it into this exec's snapshot before the body, and
        // `pycc_ext_publish` binds it from that snapshot once its class
        // statement has run.
        if let Some(entry) = slots.iter().find(|entry| entry.class == *class) {
            let array = format!("pycc_ext_type_slots_{class}");
            out.push_str(&richcompare::unhashable_fixup(entry, &array));
        }
        out.push_str(&format!(
            "    type = PyType_FromSpecWithBases(&pycc_ext_type_spec_{class}, base);\n    \
             if (type == NULL) {{\n        return -1;\n    }}\n{constants}    \
             if (pycc_ext_carrier_register(\"{class}\", type) < 0) {{\n        \
             Py_DECREF(type);\n        return -1;\n    }}\n    \
             Py_XDECREF(pycc_ext_type_object_{class});\n    \
             pycc_ext_type_object_{class} = type;\n",
            constants = class_constants_c(&published.constants)
        ));
    }
    out.push_str(&hidden_registration);
    out.push_str("    return 0;\n}\n");
    out
}

/// The published-class tables per-definition publication reads (#1199):
/// [`PUBLISH_CLASS_NAMES`] and [`PUBLISH_CLASS_SLOT_DECL`], which answers
/// the address of a published class's `pycc_ext_type_object_<Class>` static
/// for its name and `NULL` for any other name.
///
/// Emitted unconditionally -- an empty array and a function answering
/// `NULL` when nothing is published -- so every artifact links. The type
/// statics are filled by `pycc_ext_register_method_types` at registration
/// and copied into the exec's own snapshot by `pycc_ext_snapshot_classes`
/// before the body runs; the module attribute is bound from that snapshot,
/// never from the statics a nested exec may replace, when codegen's
/// publication call for the class's name runs (`pycc_codegen`'s
/// `ext_publish`), or by the safety net after the body for a class whose
/// call never ran.
fn publish_class_c(publications: &[ExtPublishedClass]) -> String {
    let mut out = format!("static const char *const {PUBLISH_CLASS_NAMES}[] = {{\n");
    for published in publications {
        out.push_str(&format!("    \"{}\",\n", published.class));
    }
    out.push_str(&format!(
        "    NULL,\n}};\n\n{PUBLISH_CLASS_SLOT_DECL}\n{{\n"
    ));
    for published in publications {
        let class = &published.class;
        out.push_str(&format!(
            "    if (strcmp(name, \"{class}\") == 0) {{\n        \
             return &pycc_ext_type_object_{class};\n    }}\n"
        ));
    }
    out.push_str("    (void)name;\n    return NULL;\n}\n\n");
    // v0.9: each published class's statement arguments, from which the
    // module makes the class when the statement runs.
    out.push_str(&format!("{PUBLISH_CLASS_BASES_DECL}\n{{\n"));
    for published in publications {
        if let Some(bases) = &published.bases {
            out.push_str(&format!(
                "    if (strcmp(name, \"{}\") == 0) {{\n        return {};\n    }}\n",
                published.class,
                keywords::c_name(bases)
            ));
        }
    }
    out.push_str("    (void)name;\n    return NULL;\n}\n\n");
    out
}

/// The generated lookup of a published class's statement arguments
/// (`ExtPublishedClass::bases`), which the shim's `pycc_ext_publish` calls:
/// `NULL` for a class published with no bases of its own.
pub(crate) const PUBLISH_CLASS_BASES_DECL: &str =
    "static const char *pycc_ext_publish_class_bases(const char *name)";

/// `publications` with every class after the published classes its MRO
/// names, so a template's base template exists when it is created.
fn template_order(publications: &[ExtPublishedClass]) -> Vec<&ExtPublishedClass> {
    let mut ordered: Vec<&ExtPublishedClass> = Vec::new();
    while ordered.len() < publications.len() {
        let before = ordered.len();
        for published in publications {
            if ordered.iter().any(|done| done.class == published.class) {
                continue;
            }
            let ready = published.mro.iter().skip(1).all(|ancestor| {
                !publications.iter().any(|other| other.class == *ancestor)
                    || ordered.iter().any(|done| done.class == *ancestor)
            });
            if ready {
                ordered.push(published);
            }
        }
        if ordered.len() == before {
            // A cycle cannot come from a linear MRO; keep the rest in order.
            for published in publications {
                if !ordered.iter().any(|done| done.class == published.class) {
                    ordered.push(published);
                }
            }
        }
    }
    ordered
}

/// The generated `isinstance` against a class compiled in this module
/// (Part 7 of #1371), declared as [`COMPILED_CLASS_ISINSTANCE_DECL`].
///
/// **A carrier is answered before this runs.** Since #1435 an instance of
/// any regular class -- published or not -- can reach the CPython side as a
/// carrier (`pycc_ext_obj_pack_instance`), and the shim's
/// `pycc_ext_obj_isinstance_compiled` answers such an object from its
/// run-time class's MRO ([`carrier_class_isinstance_c`]) without calling
/// this. What reaches this function is therefore an object that carries no
/// compiled instance: a host object, or a published type's object built by
/// `mod.Class.__new__(mod.Class)` without `tp_init`.
///
/// **For those the answer is the published family.** Published types are
/// created with no CPython bases and without `Py_TPFLAGS_BASETYPE`
/// ([`method_types_c`]): `mod.Derived` is not a CPython subclass of
/// `mod.Base`, and nothing can subclass either. So for a class name `C` the
/// function tests the object against the type object of each published
/// class whose pycc MRO contains `C`, in publication order, returning the
/// first non-zero `PyObject_IsInstance` answer -- `1`, or `-1` with the
/// exception set (a raising `__class__`, say). A name with no published
/// descendant -- a private class, one exporting no method that the host
/// cannot construct either, every class of
/// a strict embedded build, which publishes none -- has no type object to test
/// against, and falls through to [`UNPUBLISHED_CLASS_ISINSTANCE`], which
/// answers `0` once it has looked up `__class__` as CPython would.
///
/// Emitted unconditionally, with a body that is only that fall-through when
/// nothing is published, so the shim's caller always links.
pub(crate) fn compiled_class_isinstance_c(publications: &[ExtPublishedClass]) -> String {
    let mut out = format!("{COMPILED_CLASS_ISINSTANCE_DECL}\n{{\n");
    let mut names: Vec<&str> = Vec::new();
    // `object` ends every MRO but is never a compiled class argument
    // (`isinstance(o, object)` is not a `Compiled` test), so it gets no test.
    for name in publications.iter().flat_map(|published| &published.mro) {
        if name != "object" && !names.contains(&name.as_str()) {
            names.push(name);
        }
    }
    if names.is_empty() {
        out.push_str(&format!(
            "    (void)name;\n    return {UNPUBLISHED_CLASS_ISINSTANCE}(o);\n}}\n\n"
        ));
        return out;
    }
    out.push_str("    int found;\n");
    for name in names {
        out.push_str(&format!("    if (strcmp(name, \"{name}\") == 0) {{\n"));
        for published in publications
            .iter()
            .filter(|published| published.mro.iter().any(|ancestor| ancestor == name))
        {
            out.push_str(&format!(
                "        found = PyObject_IsInstance(o, pycc_ext_type_object_{});\n        \
                 if (found != 0) {{\n            return found;\n        }}\n",
                published.class
            ));
        }
        out.push_str("        return 0;\n    }\n");
    }
    out.push_str(&format!(
        "    return {UNPUBLISHED_CLASS_ISINSTANCE}(o);\n}}\n\n"
    ));
    out
}

/// The exact C declaration of the generated carrier `isinstance` (#1435),
/// which the shim's `pycc_ext_obj_isinstance_compiled` calls. That caller is
/// defined below the point the companion is included at, so the shim needs
/// no forward declaration; the shim test asserts the call against this one
/// spelling.
pub(crate) const CARRIER_CLASS_ISINSTANCE_DECL: &str = "static int \
     pycc_ext_carrier_class_isinstance(const unsigned char *cls, size_t len, const char *name)";

/// A class whose instance can cross into CPython as a carrier (#1435), with
/// its MRO, most derived first: one row of [`carrier_class_isinstance_c`]'s
/// table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtCarrierClass {
    /// The class name, as the instances' layout descriptor spells it.
    pub(crate) class: String,
    /// `HirClassDef::mro`.
    pub(crate) mro: Vec<String>,
    /// How the shared `__copy__` copies an instance of the class (#1455):
    /// one row of [`super::instance_copy::carrier_class_copy_kinds_c`].
    pub(crate) copy: CarrierCopy,
}

/// Every class of `module` whose instance `pycc_types` lets cross as a call
/// argument (`foreign.rs`'s `is_carriable_instance`): not an enum, and not
/// an exception class. In declaration order, so the generated text is
/// deterministic.
pub(crate) fn collect_carrier_classes(module: &HirModule) -> Vec<ExtCarrierClass> {
    module
        .class_defs
        .iter()
        .filter(|(class, def)| {
            // Another module's class a package member reads natively has
            // that module's carrier type (`crate::package_build`).
            !class.starts_with(pycc_hir::SIBLING_CLASS_PREFIX)
                && !def.is_enum
                && def.exception_type_tag.is_none()
                && !def
                    .mro
                    .iter()
                    .any(|entry| pycc_hir::is_builtin_exception_class(entry))
        })
        .map(|(class, def)| ExtCarrierClass {
            class: class.clone(),
            mro: def.mro.clone(),
            copy: carrier_copy(module, def),
        })
        .collect()
}

/// The generated `isinstance` of a carrier (#1435), declared as
/// [`CARRIER_CLASS_ISINSTANCE_DECL`]: whether the class named `cls`/`len` --
/// a carrier's run-time class, read from its instance's layout descriptor,
/// which is not NUL-terminated -- has `name` in its MRO.
///
/// A carrier's CPython type says nothing about the pycc hierarchy: a class
/// that gets no published type object gets a base-less type created on demand, and a
/// published type has no CPython bases either. The pycc MRO is the answer
/// CPython gives for the same source, where `type(x)` is the class itself.
/// `object` gets no test, since `isinstance(o, object)` is never a compiled
/// class test, and a name the table does not hold answers `0`.
pub(crate) fn carrier_class_isinstance_c(classes: &[ExtCarrierClass]) -> String {
    let mut out = format!("{CARRIER_CLASS_ISINSTANCE_DECL}\n{{\n");
    if classes.is_empty() {
        out.push_str("    (void)cls;\n    (void)len;\n    (void)name;\n");
    }
    for carrier in classes {
        let class = &carrier.class;
        let tests: Vec<String> = carrier
            .mro
            .iter()
            .filter(|ancestor| *ancestor != "object")
            .map(|ancestor| format!("strcmp(name, \"{ancestor}\") == 0"))
            .collect();
        out.push_str(&format!(
            "    if (len == {len} && memcmp(cls, \"{class}\", {len}) == 0) {{\n        \
             return {tests};\n    }}\n",
            len = class.len(),
            tests = tests.join(" || "),
        ));
    }
    out.push_str("    return 0;\n}\n\n");
    out
}

/// One class's `Py_tp_init`: the boundary `mod.Class(...)` crosses.
///
/// This is `wrapper_for`'s ingress half rewritten for a slot that is not a
/// `METH_FASTCALL` entry point, and every difference is forced by that:
///
/// * **Keywords.** `METH_FASTCALL` without `METH_KEYWORDS` gives a method
///   export D-244 rule 7's closed keyword boundary for free, because
///   CPython refuses a keyword before the wrapper is entered. `tp_init` is
///   handed `kwds` and is the only thing that will ever look at it, so a
///   constructor that is not keyword-enabled writes the same refusal out
///   by hand -- without it `mod.Grid(3, 4, nope=1)` would silently *ignore*
///   the keyword. A keyword-enabled one (#1461) binds `kwds` through
///   `keywords::tp_init_prologue` instead, and its count and items are read
///   from the bound array when a keyword was given.
/// * **Arity.** `nargs` becomes `PyTuple_Size(args)`, with `wrapper_for`'s
///   message shape and spelling. CPython names the same subject for a
///   Python class -- `Grid.__init__() takes ...` -- which is exactly what
///   [`source_level_name`] returns for the mangled `Grid.__init__`.
/// * **Argument access.** `args[i]` becomes `PyTuple_GetItem(args, i)`.
///   Everything past that bridge is [`unpack_args`], shared verbatim with
///   `wrapper_for`: the unpack helper is a function of the declared type
///   alone, so `mod.Grid(True, 4)` and `mod.Grid(3, 4).scale(True)` admit
///   the same object set for the same declared `int`. `docs/RUNTIME.md`
///   claims one admissibility matrix, not two.
/// * **Failure.** Every bail returns `-1`, not `NULL`.
///
/// The parameter list is rendered from the *carried* tail -- `ExtCtor`
/// already dropped `self` -- and the leading `void *` is prepended
/// textually, exactly as `wrapper_for` does for a receiver, so this
/// declaration and codegen's own definition of the same symbol agree.
///
/// **A null guard on the `fnptr_` slot** (#1199), the same
/// [`super::fnptr_null_guard`] `wrapper_for` opens with, placed after the
/// local declarations and before keyword binding, unpacking and
/// `pycc_rt_instance_new`: a class is published only once its own items'
/// slots are stored, so the guard is defence in depth, and it refuses with
/// nothing to release.
///
/// The `pycc_rt_instance_new` allocation is released when the constructor
/// raises, and otherwise owned by the carrier (`pycc_ext_carrier_bind`).
/// The instance is made from `ctor`'s class record (v0.10,
/// `pycc_codegen::record_symbol`), which compiled code defines and which
/// every instance of the class -- compiled code's, the host's -- points
/// at: its descriptor (#1388) and its run-time class.
fn record_declaration(ctor: &ExtCtor) -> String {
    format!(
        "extern const unsigned char {}[];\n",
        pycc_codegen::record_symbol(&ctor.class)
    )
}

/// The C expression allocating an instance of `ctor`'s class.
fn instance_of_record(ctor: &ExtCtor) -> String {
    format!(
        "pycc_ext_instance_of_record({}, {})",
        ctor.slot_names.len(),
        pycc_codegen::record_symbol(&ctor.class)
    )
}

/// v0.9: `Py_tp_new` of a constructible class. `PyType_GenericNew`, and
/// for a Python subclass's object an instance with every slot unassigned:
/// a subclass whose `__init__` does not call the class's still stores its
/// attributes (the slots' setters) and reads its class-level defaults.
fn tp_new_c(ctor: &ExtCtor, defaults: &[String]) -> String {
    let class = &ctor.class;
    // The instance defaults among the slots, stored as `__init__` would
    // store them first.
    let mut stores = String::new();
    for name in defaults.iter().filter(|name| ctor.slot_names.contains(name)) {
        stores.push_str(&format!(
            "        pycc_ext_store_instance_default(self, {}, 0);\n",
            super::c_string_literal(name)
        ));
    }
    format!(
        "{declaration}static PyObject *pycc_ext_tp_new_{class}(PyTypeObject *type, PyObject *args, \
         PyObject *kwds)\n{{\n    \
         PyObject *self = PyType_GenericNew(type, args, kwds);\n    \
         if (self != NULL && !pycc_ext_is_compiled_class((PyObject *)type)) {{\n        \
         pycc_ext_carrier_bind(self, {instance});\n\
         {stores}    }}\n    return self;\n}}\n\n",
        declaration = record_declaration(ctor),
        instance = instance_of_record(ctor)
    )
}

fn tp_init_c(ctor: &ExtCtor, defaults: &[String]) -> String {
    let class = &ctor.class;
    let arity = ctor.params.len();
    let slots: Vec<_> = super::slot_carriers(&ctor.params, &ctor.param_writable);
    let symbol = pycc_codegen::mangle_ext_name(&ctor.name);
    let source_name = source_level_name(&ctor.name);
    // A constructor never returns a value at all, so it can carry neither a
    // tuple out-slot nor #1179's buffer sub-range out-slots.
    let carried = c_param_list(&slots, &[], false);
    let params = if carried == "void" {
        "void *".to_string()
    } else {
        format!("void *, {carried}")
    };
    // The method part of #1140, exactly as `wrapper_for` applies it: keyed
    // by the class rather than the compiled `__init__` name, because two
    // classes that inherit one `__init__` each get their own `Py_tp_init`.
    let default_prefix = format!("init_{class}");
    let mut out = defaults::default_object_helpers(&default_prefix, &ctor.defaults);
    // #1461: a keyword-enabled constructor binds `kwds` as CPython binds the
    // same call to the uncompiled `__init__`; every other constructor keeps
    // the hand-written refusal below byte for byte.
    let prologue = ctor.keyword_names.as_deref().map(|names| {
        keywords::tp_init_prologue(&default_prefix, source_name, names, &ctor.defaults)
    });
    let (count, item): (String, &dyn Fn(usize) -> String) = match &prologue {
        Some(prologue) => {
            out.push_str(&prologue.before);
            (keywords::tp_init_count(arity), &keywords::tp_init_item)
        }
        None => ("PyTuple_Size(args)".to_string(), &|index| {
            format!("PyTuple_GetItem(args, {index})")
        }),
    };
    out.push_str(&format!("extern void *fnptr_{symbol};\n"));
    out.push_str(&record_declaration(ctor));
    out.push_str(&format!(
        "static int pycc_ext_tp_init_{class}(PyObject *self, PyObject *args, PyObject *kwds)\n\
         {{\n    void *inst;\n"
    ));
    out.push_str(&arg_slot_locals(&slots));
    out.push_str(&fnptr_null_guard(&symbol, source_name, "-1"));
    // v0.9: `def __init__(self, a, *items)` (non-strict): the last carried
    // parameter is the CPython tuple of the positional arguments past the
    // fixed ones, and the call -- keywords included -- is bound as CPython
    // binds it (`pycc_ext_bind_star`).
    let star = ctor
        .vararg
        .as_ref()
        .filter(|_| ctor.defaults.iter().all(Option::is_none) && prologue.is_none());
    match (&prologue, star) {
        (Some(prologue), _) => out.push_str(&prologue.body),
        (None, Some(_)) => {}
        (None, None) => out.push_str(&format!(
            "    if (kwds != NULL && PyDict_Size(kwds) != 0) {{\n        \
             PyErr_SetString(PyExc_TypeError, \"{source_name}() takes no keyword arguments\");\n        \
             return -1;\n    }}\n"
        )),
    }
    if let Some(names) = star {
        let fixed = arity - 1;
        let table: Vec<String> = names
            .iter()
            .map(|name| keywords::c_name(name))
            .chain(std::iter::once("NULL".to_string()))
            .collect();
        out.push_str(&format!(
            "    static const char *const pycc_names[] = {{{table}}};\n    \
             PyObject *pycc_bound[{size}];\n    \
             PyObject *pycc_rest;\n    \
             if (pycc_ext_bind_star(args, kwds, {qualname}, pycc_names, {fixed}, pycc_bound, \
             &pycc_rest) < 0) {{\n        return -1;\n    }}\n",
            table = table.join(", "),
            size = fixed.max(1),
            qualname = keywords::c_name(source_name),
        ));
        out.push_str(&unpack_args(
            &slots,
            source_name,
            &|index| {
                if index == fixed {
                    "pycc_rest".to_string()
                } else {
                    format!("pycc_bound[{index}]")
                }
            },
            "        Py_DECREF(pycc_rest);\n        return -1;\n",
        ));
        out.push_str("    Py_DECREF(pycc_rest);\n");
    } else if ctor.defaults.is_empty() {
        out.push_str(&format!(
            "    if ({count} != {arity}) {{\n        PyErr_Format(PyExc_TypeError, \
             \"{source_name}() takes exactly {arity} argument{plural} (%zd given)\", \
             {count});\n        return -1;\n    }}\n",
            plural = if arity == 1 { "" } else { "s" },
        ));
    } else {
        out.push_str(&defaults::range_arity_check(
            source_name,
            &default_prefix,
            &ctor.defaults,
            &defaults::ArgSource {
                count: &count,
                item,
                fail: "        return -1;\n",
            },
        ));
    }
    if star.is_none() {
        out.push_str(&unpack_args(
            &slots,
            source_name,
            &|index| defaults::arg_expr(&ctor.defaults, index, item(index)),
            "        return -1;\n",
        ));
    }
    out.push_str(&format!("    inst = {};\n", instance_of_record(ctor)));
    let mut call_args = vec!["inst".to_string()];
    for (index, slot) in slots.iter().enumerate() {
        // Deliberately two arms where `wrapper_for` has three.
        // `ctor_descriptor` refuses a `tuple` parameter outright -- no
        // `pycc_ext_thunk_` exists for a constructor, so there would be no
        // callable C entry point for the flattened elements -- which leaves
        // a scalar or (Part 1 of #1447) a same-module instance as the only
        // other carriers a constructor slot can hold, and both pass `a{i}`. A
        // third arm for `BoundaryCarrier::Tuple` would be a line no test
        // could ever execute, which the 100%-changed-lines invariant does
        // not admit (D-242 rule 1).
        call_args.push(match slot {
            super::BoundaryCarrier::Buffer { .. } => format!("&a{index}"),
            _ => format!("a{index}"),
        });
    }
    // `__init__`'s `self` parameter owns a reference of its own; the one
    // `pycc_rt_instance_new` returned goes to the carrier, or is released
    // when the constructor raises.
    out.push_str(&format!(
        "    pycc_rt_instance_incref(inst);\n    \
         ((void (*)({params}))fnptr_{symbol})({call_args});\n",
        call_args = call_args.join(", ")
    ));
    // The same order `wrapper_for` uses, for the same reason: a compiled
    // function that raised left the runtime's thread-local flag set and the
    // carrier must never be published, so the flag is read before `inst` is
    // stored -- and every `Py_buffer` this slot acquired is released on
    // both arms.
    out.push_str(&format!(
        "    if (pycc_rt_ext_pending_type() >= 0) {{\n{}        \
         pycc_rt_instance_decref(inst);\n        \
         pycc_ext_raise_pending();\n        \
         return -1;\n    }}\n{}",
        buffer_releases(&slots, "        "),
        buffer_releases(&slots, "    ")
    ));
    // #1435: storing the instance also links it to `self`, so `self`
    // handed out by one of its methods is this very object.
    out.push_str("    pycc_ext_carrier_bind(self, inst);\n");
    // v0.9: a Python subclass's object reads a default its class body
    // rebinds (`pycc_ext_store_instance_default`).
    let rebound: Vec<&String> =
        defaults.iter().filter(|name| ctor.slot_names.contains(name)).collect();
    if !rebound.is_empty() {
        out.push_str("    if (!pycc_ext_is_compiled_class((PyObject *)Py_TYPE(self))) {\n");
        for name in rebound {
            out.push_str(&format!(
                "        pycc_ext_store_instance_default(self, {}, 1);\n",
                super::c_string_literal(name)
            ));
        }
        out.push_str("    }\n");
    }
    out.push_str("    return 0;\n}\n\n");
    out
}

/// Sets one class constant on a freshly created type object, consuming
/// `value` (a `NULL` one is a failed constructor whose exception is set).
/// `PyObject_GenericSetAttr` writes the type's own dictionary, which the
/// immutable type's `__setattr__` would refuse, and `PyType_Modified`
/// (emitted once per class) drops the attribute cache.
const SET_CONSTANT_C: &str = "static int pycc_ext_type_set_constant(PyObject *type, const char *name,
                                      PyObject *value)
{
    PyObject *key;
    int status;

    if (value == NULL) {
        return -1;
    }
    key = PyUnicode_FromString(name);
    if (key == NULL) {
        Py_DECREF(value);
        return -1;
    }
    status = PyObject_GenericSetAttr(type, key, value);
    Py_DECREF(key);
    Py_DECREF(value);
    return status;
}

";

/// The C statements setting `constants` on `type` right after it is
/// created, or nothing for a class without one.
fn class_constants_c(constants: &[(String, pycc_hir::ClassAttrValue)]) -> String {
    use pycc_hir::ClassAttrValue;
    if constants.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (name, value) in constants {
        let value = match value {
            ClassAttrValue::Int(i64::MIN) => "PyLong_FromLongLong(-9223372036854775807LL - 1)".to_string(),
            ClassAttrValue::Int(n) => format!("PyLong_FromLongLong({n}LL)"),
            ClassAttrValue::Float(f) if f.is_nan() => "PyFloat_FromDouble(Py_NAN)".to_string(),
            ClassAttrValue::Float(f) if f.is_infinite() => format!(
                "PyFloat_FromDouble({}Py_HUGE_VAL)",
                if *f < 0.0 { "-" } else { "" }
            ),
            ClassAttrValue::Float(f) => format!("PyFloat_FromDouble({f:e})"),
            ClassAttrValue::Bool(b) => format!("PyBool_FromLong({})", i32::from(*b)),
            ClassAttrValue::Str(text) => format!(
                "PyUnicode_FromStringAndSize({}, {})",
                super::c_string_literal(text),
                text.len()
            ),
            ClassAttrValue::ForeignStatic(_) => continue,
        };
        out.push_str(&format!(
            "    if (pycc_ext_type_set_constant(type, {}, {value}) < 0) {{\n        \
             Py_DECREF(type);\n        return -1;\n    }}\n",
            super::c_string_literal(name)
        ));
    }
    out.push_str("    PyType_Modified((PyTypeObject *)type);\n");
    out
}
