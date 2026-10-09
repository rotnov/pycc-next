//! Part 1 of #1447 (#1449): an instance of a regular class compiled in the
//! same module crossing the `--ext` boundary as an argument and as a
//! result -- the module-aware admission, the generated wrapper text, and the
//! shim helpers' placement around the generated include.

use super::exports::constructible_module;
use super::generated_c::inc_no_classes;
use super::*;

fn conf() -> Ty {
    Ty::Instance(Box::new("Conf".to_string()))
}

fn echo_export(receiver: ExtReceiver) -> ExtExport {
    ExtExport {
        defaults: Vec::new(),
        name: "echo".to_string(),
        class: None,
        method: None,
        returns_buffer_slice: false,
        receiver,
        params: vec![Ty::Int, conf()],
        param_writable: vec![false; 2],
        return_ty: conf(),
        keyword_names: None,
    }
}

#[test]
fn an_instance_argument_and_result_are_one_void_pointer_slot_each() {
    let inc = inc_no_classes("m", &[echo_export(ExtReceiver::None)]);
    // The local is the compiled instance pointer, unpacked by the class the
    // shim checks the run-time MRO against; a refusal of argument 2 owes
    // nothing for argument 1, an `int`.
    assert!(inc.contains("    void *a1;\n"), "{inc}");
    assert!(
        inc.contains(
            "    if (pycc_ext_unpack_instance(args[1], \"echo\", 1, \"Conf\", &a1) != 0) {\n        \
             return NULL;\n    }\n"
        ),
        "{inc}"
    );
    // The cast spells the instance as `void *` in both positions, which is
    // what `ty_to_basic_type` gives `Ty::Instance`, and the result goes
    // through the instance packer rather than a scalar one.
    assert!(
        inc.contains("((void * (*)(long long, void *))fnptr_echo)(a0, a1)"),
        "{inc}"
    );
    assert!(
        inc.contains("    return pycc_ext_activation_exit(pycc_ext_pack_instance(result));\n}\n\n"),
        "{inc}"
    );
    assert!(!inc.contains("pycc_ext_pack_object(result)"), "{inc}");
}

#[test]
fn the_instance_helpers_are_declared_above_the_include_and_defined_below_it() {
    let shim = shim_c();
    let include = shim
        .find("#include \"pycc_ext_exports.inc\"")
        .expect("the generated include");
    // The generated wrappers call both, so both are declared before the
    // companion; both definitions need what it defines -- the generated
    // `pycc_ext_carrier_class_isinstance` and the module name macro.
    for (declaration, definition) in [
        (
            "static int pycc_ext_unpack_instance(PyObject *obj, const char *fn_name, \
             Py_ssize_t index,\n                                    const char *class_name, \
             void **out);\n",
            "static int pycc_ext_unpack_instance(PyObject *obj, const char *fn_name, \
             Py_ssize_t index,\n                                    const char *class_name, \
             void **out)\n{",
        ),
        (
            "static PyObject *pycc_ext_pack_instance(void *result);\n",
            "static PyObject *pycc_ext_pack_instance(void *result)\n{",
        ),
    ] {
        let declared = shim.find(declaration).expect(declaration);
        let defined = shim.find(definition).expect(definition);
        assert!(declared < include && include < defined, "{declaration}");
    }
    // Ingress admits only this module's carriers whose instance's run-time
    // class has the declared class on its MRO, and refuses an
    // uninitialized carrier with its own message.
    for needle in [
        "int carrier = PyType_GetSlot(Py_TYPE(obj), Py_tp_dealloc) == (void \
         *)pycc_ext_instance_dealloc;\n",
        "            if (pycc_ext_carrier_class_isinstance(cls, len, class_name)) {\n                \
         *out = inst;\n                return 0;\n",
        "is uninitialized (its __init__ never \"\n                     \"ran)\"",
        "\"%s() argument %zd must be %s.%s, not %U\"",
        "    return pycc_ext_obj_pack_instance(result);\n",
    ] {
        assert!(shim.contains(needle), "{needle}");
    }
}

#[test]
fn an_instance_is_admitted_only_for_a_regular_class_of_the_module() {
    // Part 1 of #1447 (#1449). `boundary_carrier` maps every `Ty::Instance`
    // to a carrier shape, so the module-aware narrowing is the whole of the
    // admission: a regular class the module defines is carried at both
    // positions, and an enum, an exception class, a seeded builtin
    // exception and a name the module does not define are each a `C0003`
    // naming the class.
    let conf = || Ty::Instance(Box::new("Conf".to_string()));
    let regular = || ("Conf".to_string(), class_def("Conf", None));
    let hir = module_with_classes(
        vec![func("echo", &[("c", conf())], conf())],
        vec![regular()],
    );
    let exports = collect_exports(&hir).expect("a regular class of the module is carried");
    assert_eq!(exports.len(), 1);
    assert_eq!(exports[0].params, vec![conf()]);
    assert_eq!(exports[0].return_ty, conf());

    let mut enum_class = class_def("Conf", None);
    enum_class.is_enum = true;
    let mut builtin_exception = class_def("Conf", None);
    builtin_exception.mro = vec!["Conf".to_string(), "ValueError".to_string()];
    let refused: Vec<(&str, Vec<(String, pycc_hir::HirClassDef)>)> = vec![
        ("enum", vec![("Conf".to_string(), enum_class)]),
        (
            "exception",
            vec![("Conf".to_string(), class_def("Conf", Some(7)))],
        ),
        (
            "builtin exception base",
            vec![("Conf".to_string(), builtin_exception)],
        ),
        ("not in the module", Vec::new()),
    ];
    for (label, class_defs) in refused {
        let at_param = module_with_classes(
            vec![func("take", &[("c", conf())], Ty::Int)],
            class_defs.clone(),
        );
        let gaps = collect_exports(&at_param).expect_err(label);
        assert_eq!(gaps[0].code, EXT_CAPABILITY_CODE, "{label}");
        let message = &gaps[0].message;
        assert!(
            message.contains("parameter `c: Conf`"),
            "{label}: {message}"
        );

        let at_return = module_with_classes(vec![func("give", &[], conf())], class_defs);
        let gaps = collect_exports(&at_return).expect_err(label);
        let message = &gaps[0].message;
        assert!(
            message.contains("return type `-> Conf`"),
            "{label}: {message}"
        );
    }
}

#[test]
fn a_tuple_or_optional_of_instances_stays_a_capability_gap() {
    // Part 1 of #1447 (#1449): an instance has no `_at` element shim, so
    // `tuple[Conf]` is refused at both positions even though `Conf` alone
    // is carried, and so is `Optional[Conf]`, which has no carrier at all.
    let tuple = || Ty::Tuple(Box::new(vec![conf()]));
    let optional = || Ty::Optional(Box::new(conf()));
    for (label, wrapped) in [("tuple", tuple()), ("Optional", optional())] {
        for (item, position) in [
            (
                func("take", &[("t", wrapped.clone())], Ty::Int),
                "parameter `t: ",
            ),
            (func("give", &[], wrapped.clone()), "return type `-> "),
        ] {
            let hir = module_with_classes(
                vec![item],
                vec![("Conf".to_string(), class_def("Conf", None))],
            );
            let gaps = collect_exports(&hir).expect_err(label);
            assert_eq!(gaps.len(), 1, "{label}");
            assert_eq!(gaps[0].code, EXT_CAPABILITY_CODE, "{label}");
            let message = &gaps[0].message;
            assert!(message.contains(position), "{label}: {message}");
        }
    }
}

#[test]
fn an_init_taking_a_same_module_instance_makes_the_class_constructible() {
    // Part 1 of #1447 (#1449): the shape lark's `ParserState.__init__(self,
    // parse_conf: ParseConf, ...)` has. The class becomes constructible, so
    // its instance methods are published, and the generated `tp_init`
    // unpacks the argument through the instance helper.
    let mut hir = constructible_module("Grid");
    hir.items[0] = init_func(
        "Grid",
        &[
            ("conf", Ty::Instance(Box::new("Conf".to_string()))),
            ("w", Ty::Int),
        ],
        Ty::None,
    );
    hir.class_defs
        .push(("Conf".to_string(), class_def("Conf", None)));
    let exports = collect_exports(&hir).expect("a carriable signature");
    assert!(
        exports.iter().any(|export| export.name == "Grid.area"),
        "{exports:?}"
    );
    let publications = collect_class_publications(&hir, &exports);
    let ctors = collect_constructors(&hir, &publications);
    assert_eq!(ctors.len(), 1);
    assert_eq!(
        ctors[0].params,
        vec![Ty::Instance(Box::new("Conf".to_string())), Ty::Int]
    );
    let inc = generate_exports_inc("m", &exports, &[], &publications, &ctors, &[]);
    assert!(
        inc.contains(
            "if (pycc_ext_unpack_instance(PyTuple_GetItem(args, 0), \"Grid.__init__\", 0, \
             \"Conf\", &a0) != 0) {"
        ),
        "{inc}"
    );
    assert!(inc.contains("(void *, void *, long long))fnptr_"), "{inc}");
    assert!(inc.contains("(inst, a0, a1)"), "{inc}");
}
