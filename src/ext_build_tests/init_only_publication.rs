//! #1450: a constructible class is published even when its MRO resolves no
//! exported method -- lark's `__init__`-only `ParseConf` -- and a class the
//! host could neither construct nor call anything on still is not.

use super::exports::constructible_module;
use super::*;

/// [`constructible_module`] with its one public method removed, so the
/// class's only member is a carriable `__init__`.
fn init_only_module(class: &str) -> HirModule {
    let mut hir = constructible_module(class);
    let area = format!("{class}.area");
    hir.items
        .retain(|item| !matches!(item, HirItem::Function { name, .. } if *name == area));
    hir.class_defs[0]
        .1
        .methods
        .retain(|(name, _)| name != "area");
    hir
}

fn publications_and_ctors(hir: &HirModule) -> (Vec<ExtPublishedClass>, Vec<ExtCtor>) {
    let exports = collect_exports(hir).expect("a carriable program");
    assert!(
        exports.is_empty(),
        "the fixture exports nothing: {exports:?}"
    );
    let publications = collect_class_publications(hir, &exports);
    let ctors = collect_constructors(hir, &publications);
    (publications, ctors)
}

/// `hir`'s carrier descriptor tables (#1448).
fn getsets_of(hir: &HirModule) -> Vec<ExtClassGetsets> {
    collect_carrier_getsets(hir, &collect_carrier_classes(hir))
}

#[test]
fn an_init_only_class_is_published_with_only_the_copy_row_and_a_constructor() {
    let hir = init_only_module("ParseConf");
    let (publications, ctors) = publications_and_ctors(&hir);
    assert_eq!(
        publications,
        vec![ExtPublishedClass {
            class: "ParseConf".to_string(),
            methods: Vec::new(),
            mro: vec!["ParseConf".to_string()],
        }]
    );
    assert_eq!(
        ctors
            .iter()
            .map(|ctor| (ctor.class.as_str(), ctor.name.as_str(), ctor.params.clone()))
            .collect::<Vec<_>>(),
        vec![("ParseConf", "ParseConf.__init__", vec![Ty::Int, Ty::Int])]
    );

    // The rendered type: a method table holding only the shared `__copy__`
    // (#1455), a real `tp_init`,
    // the slot descriptors, no `DISALLOW_INSTANTIATION`, and (#1199) a
    // publication-table entry under the class's own name, which binds the
    // module attribute once the class statement has run.
    let c = method_types_c(&publications, &ctors, &getsets_of(&hir), &[]);
    for needle in [
        "static PyMethodDef pycc_ext_type_methods_ParseConf[] = {\n    \
         {\"__copy__\", (PyCFunction)(void (*)(void))pycc_ext_instance_copy, METH_NOARGS, NULL},\n    \
         {NULL, NULL, 0, NULL},\n};",
        "{Py_tp_init, pycc_ext_tp_init_ParseConf}",
        "{Py_tp_getset, pycc_ext_type_getset_ParseConf}",
        "    if (strcmp(name, \"ParseConf\") == 0) {\n        \
         return &pycc_ext_type_object_ParseConf;\n    }\n",
        "pycc_ext_carrier_register(\"ParseConf\", type)",
    ] {
        assert!(c.contains(needle), "missing {needle:?} in:\n{c}");
    }
    assert!(!c.contains("DISALLOW_INSTANTIATION"), "{c}");
    // And the generated `isinstance` now has a type object to test against.
    assert!(
        compiled_class_isinstance_c(&publications).contains("pycc_ext_type_object_ParseConf"),
        "the published class is missing from the isinstance table"
    );
}

#[test]
fn a_class_with_only_the_implicit_object_init_is_published_and_takes_no_argument() {
    // `class Empty: pass`: `pycc_hir` records the D-225 implicit
    // zero-argument constructor, which is all the class resolves.
    let mut def = class_def("Empty", None);
    def.implicit_object_init = true;
    def.methods = vec![("__init__".to_string(), "Empty.__init__".to_string())];
    let hir = module_with_classes(
        vec![init_func("Empty", &[], Ty::None)],
        vec![("Empty".to_string(), def)],
    );
    let (publications, ctors) = publications_and_ctors(&hir);
    assert_eq!(
        publications
            .iter()
            .map(|p| (p.class.as_str(), p.methods.len()))
            .collect::<Vec<_>>(),
        vec![("Empty", 0)]
    );
    assert_eq!(
        ctors
            .iter()
            .map(|ctor| (ctor.class.as_str(), ctor.params.len()))
            .collect::<Vec<_>>(),
        vec![("Empty", 0)]
    );
    // No slot, so no descriptor table: the slot array stays as it was.
    let getsets = getsets_of(&hir);
    assert!(getsets.is_empty(), "{:?}", getsets.len());
    let c = method_types_c(&publications, &ctors, &getsets, &[]);
    assert!(c.contains("{Py_tp_init, pycc_ext_tp_init_Empty}"), "{c}");
    assert!(!c.contains("Py_tp_getset"), "{c}");
}

/// Each row keeps the class `__init__`-only and breaks exactly one
/// publication condition. With no method to resolve, the constructor is the
/// only reason to publish, so every row must leave the artifact with no
/// type object at all -- not one the host can neither build nor use.
#[test]
fn an_init_only_class_the_host_cannot_construct_or_name_gets_no_type_object() {
    let mut rows: Vec<(&str, HirModule)> = Vec::new();

    let mut tuple_param = init_only_module("Conf");
    tuple_param.items[0] = init_func(
        "Conf",
        &[("wh", Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])))],
        Ty::None,
    );
    rows.push(("tuple __init__ parameter", tuple_param));

    let mut foreign_instance = init_only_module("Conf");
    foreign_instance.items[0] = init_func(
        "Conf",
        &[("other", Ty::Instance(Box::new("Elsewhere".to_string())))],
        Ty::None,
    );
    rows.push(("uncarriable __init__ parameter", foreign_instance));

    let mut returning = init_only_module("Conf");
    returning.items[0] = init_func("Conf", &[("w", Ty::Int), ("h", Ty::Int)], Ty::Int);
    rows.push(("__init__ return type", returning));

    let mut abstract_class = init_only_module("Conf");
    abstract_class.class_defs[0].1.is_abstract = true;
    rows.push(("is_abstract", abstract_class));

    let mut protocol = init_only_module("Conf");
    protocol.class_defs[0].1.is_protocol = true;
    rows.push(("is_protocol", protocol));

    let mut enum_class = init_only_module("Conf");
    enum_class.class_defs[0].1.is_enum = true;
    rows.push(("is_enum", enum_class));

    let mut tagged = init_only_module("Conf");
    tagged.class_defs[0].1.exception_type_tag = Some(FIRST_USER_EXCEPTION_TYPE_TAG);
    rows.push(("exception_type_tag", tagged));

    let mut builtin_exception = init_only_module("ValueError");
    builtin_exception.class_defs[0].1.exception_type_tag = None;
    rows.push(("is_builtin_exception_class", builtin_exception));

    // Constructible, but not publishable: a private name is never a module
    // attribute (`class_publishable`).
    rows.push(("private name", init_only_module("_Conf")));

    // Constructible and publishable by name, but a monomorphized
    // specialization: its `__init__` has no `fnptr_` global to call.
    rows.push(("0gen_ specialization", init_only_module("0gen_Conf__T_int")));

    let mut no_init = init_only_module("Conf");
    no_init.class_defs[0].1.methods.clear();
    rows.push(("no resolved __init__", no_init));

    for (label, hir) in rows {
        let (publications, ctors) = publications_and_ctors(&hir);
        assert!(
            publications.is_empty(),
            "{label}: published {publications:?}"
        );
        assert!(ctors.is_empty(), "{label}: yielded {ctors:?}");
        assert!(
            !method_types_c(&publications, &ctors, &[], &[]).contains("PyType_Spec"),
            "{label}: rendered a type object"
        );
    }
}
