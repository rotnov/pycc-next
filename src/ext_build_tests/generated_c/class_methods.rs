//! Exported classes: method tables, constructors and `tp_init`.

use super::*;

/// An exported `@classmethod`. `params` is the receiver-free tail: `cls`
/// never crosses the boundary.
fn class_export(class: &str, method: &str, params: Vec<Ty>, return_ty: Ty) -> ExtExport {
    ExtExport {
        defaults: Vec::new(),
        name: format!("{class}.{method}.classmethod"),
        class: Some(class.to_string()),
        method: Some(method.to_string()),
        returns_buffer_slice: false,
        receiver: ExtReceiver::NullCls,
        param_writable: vec![false; params.len()],
        params,
        return_ty,
        keyword_names: None,
    }
}

#[test]
fn a_program_with_no_exported_method_still_defines_the_registration_function() {
    // Unconditional, so `src/ext/module/` can call it without a
    // preprocessor guard: an artifact whose program exports no method still
    // links.
    let inc = inc_no_classes("m", &[]);
    assert!(inc.contains(METHOD_TYPE_REGISTER_DECL), "{inc}");
    assert!(
        inc.contains("    (void)module;\n    return 0;\n}\n"),
        "{inc}"
    );
    assert!(!inc.contains("PyType_FromSpec"), "{inc}");
    // Part 7 of #1371: the compiled-class `isinstance` is defined too, and
    // with nothing published every class name falls through to the shim's
    // unpublished-class answer.
    assert!(
        inc.contains(&format!(
            "{COMPILED_CLASS_ISINSTANCE_DECL}\n{{\n    (void)name;\n    \
             return {UNPUBLISHED_CLASS_ISINSTANCE}(o);\n}}\n"
        )),
        "{inc}"
    );
    assert!(!inc.contains("pycc_ext_type_object_"), "{inc}");
}

#[test]
fn an_exported_method_gets_a_method_table_a_slot_table_and_a_non_instantiable_spec() {
    let inc = inc_no_classes(
        "m",
        &[
            static_export("Grid", "scale", vec![Ty::Int], Ty::Int),
            class_export("Grid", "make", vec![Ty::Int], Ty::Int),
        ],
    );
    assert!(
        inc.contains(
            "static PyMethodDef pycc_ext_type_methods_Grid[] = {\n    \
             {\"scale\", (PyCFunction)(void (*)(void))pycc_ext_wrap_0m4_Grid5_scale6_static, \
             METH_FASTCALL | METH_STATIC, NULL},\n    \
             {\"make\", (PyCFunction)(void (*)(void))pycc_ext_wrap_0m4_Grid4_make11_classmethod, \
             METH_FASTCALL | METH_CLASS, NULL},\n    \
             {\"__copy__\", (PyCFunction)(void (*)(void))pycc_ext_instance_copy, \
             METH_NOARGS, NULL},\n    {NULL, NULL, 0, NULL},\n};\n"
        ),
        "{inc}"
    );
    // Every published type is a carrier type (#1435), so even one with no
    // constructor carries the shared deallocator.
    assert!(
        inc.contains(
            "static PyType_Slot pycc_ext_type_slots_Grid[] = {\n    \
             {Py_tp_methods, pycc_ext_type_methods_Grid},\n    \
             {Py_tp_dealloc, pycc_ext_instance_dealloc},\n    {0, NULL},\n};\n"
        ),
        "{inc}"
    );
    // `basicsize` is the carrier's, because an instance crossing as a call
    // argument is boxed in this type (#1435); `Py_TPFLAGS_DISALLOW_INSTANTIATION`
    // plus `Py_TPFLAGS_IMMUTABLETYPE` are what make `mod.Grid()` and
    // `mod.Grid.scale = ...` both `TypeError`.
    assert!(
        inc.contains(
            "static PyType_Spec pycc_ext_type_spec_Grid = {\n    \
             PYCC_EXT_MODULE_NAME_STR \".Grid\",\n    sizeof(PyccExtInstance),\n    0,\n    \
             Py_TPFLAGS_DEFAULT | Py_TPFLAGS_DISALLOW_INSTANTIATION | \
             Py_TPFLAGS_IMMUTABLETYPE,\n    pycc_ext_type_slots_Grid,\n};\n"
        ),
        "{inc}"
    );
    // The type enters the shim's carrier-type cache, and a failure releases
    // the local reference. It is not added to the module here (#1199):
    // `pycc_ext_publish` binds it from its static once its class statement
    // has run.
    assert!(
        inc.contains(
            "    type = PyType_FromSpec(&pycc_ext_type_spec_Grid);\n    \
             if (type == NULL) {\n        return -1;\n    }\n    \
             if (pycc_ext_carrier_register(\"Grid\", type) < 0) {\n        \
             Py_DECREF(type);\n        return -1;\n    }\n    \
             Py_XDECREF(pycc_ext_type_object_Grid);\n    pycc_ext_type_object_Grid = type;\n"
        ),
        "{inc}"
    );
}

#[test]
fn an_exported_method_is_never_a_flat_module_level_entry() {
    // The host surface is `mod.Grid.scale(...)`. A flat
    // `mod."Grid.scale"` attribute is not published, in this release or any
    // later one: `pycc_ext_methods[]` carries module-level functions only.
    let inc = inc_no_classes(
        "m",
        &[
            ExtExport {
                defaults: Vec::new(),
                name: "plain".to_string(),
                class: None,
                method: None,
                returns_buffer_slice: false,
                receiver: ExtReceiver::None,
                params: vec![Ty::Int],
                param_writable: vec![false; 1],
                return_ty: Ty::Int,
                keyword_names: None,
            },
            static_export("Grid", "scale", vec![Ty::Int], Ty::Int),
        ],
    );
    let table = inc
        .split("static PyMethodDef pycc_ext_methods[]")
        .nth(1)
        .expect("the module-level table is emitted")
        .split("};")
        .next()
        .expect("the table is terminated");
    assert!(table.contains("\"plain\""), "{table}");
    assert!(!table.contains("Grid"), "{table}");
}

#[test]
fn a_non_ascii_class_name_is_spliced_verbatim_into_the_type_object_identifiers() {
    // `mangle_ext_name` encodes the dot separator; it is not an ASCII fold,
    // and it copies each segment's bytes verbatim. Routing a class name
    // through it would therefore change nothing here, and nothing needs to:
    // a Python identifier is `XID_Start XID_Continue*`, which carries no
    // character that escapes a C identifier or a C string literal, and both
    // clang and GCC accept UTF-8 identifiers. Verified end to end at
    // `5b1fb3a1`: a module whose class is named `Grid\u{e9}` builds with
    // `--ext`, imports, and answers `Grid\u{e9}.scale(21) == 42`.
    let inc = inc_no_classes(
        "m",
        &[static_export("Grid\u{e9}", "scale", vec![Ty::Int], Ty::Int)],
    );
    assert!(
        inc.contains("static PyMethodDef pycc_ext_type_methods_Grid\u{e9}[]"),
        "{inc}"
    );
    assert!(
        inc.contains("static PyType_Spec pycc_ext_type_spec_Grid\u{e9} = {"),
        "{inc}"
    );
    // The host-visible name is the same bytes, in a string literal.
    assert!(
        inc.contains("PYCC_EXT_MODULE_NAME_STR \".Grid\u{e9}\""),
        "{inc}"
    );
}

#[test]
fn a_class_method_wrapper_prepends_the_null_receiver_and_a_static_one_does_not() {
    let inc = inc_no_classes(
        "m",
        &[
            static_export("Grid", "scale", vec![Ty::Int], Ty::Int),
            class_export("Grid", "make", vec![Ty::Int], Ty::Int),
        ],
    );
    // The `@staticmethod` thunk takes exactly the carried parameters.
    assert!(
        inc.contains("extern void *fnptr_0m4_Grid5_scale6_static;"),
        "{inc}"
    );
    assert!(
        inc.contains("extern void *fnptr_0m4_Grid4_make11_classmethod;"),
        "{inc}"
    );
    // The `@classmethod` thunk's native signature still leads with the
    // receiver slot, which every native `Grid.make(...)` call site fills
    // with a null pointer; the wrapper passes `NULL` there and the compiled
    // body never dereferences it.
    let make = thunk_call_line(&inc, "fnptr_0m4_Grid4_make11_classmethod");
    assert!(make.contains("void *,"), "{make}");
    assert!(make.contains("NULL,"), "{make}");
    // The `@staticmethod`'s own cast carries no receiver slot. Anchor on the
    // cast line rather than on the wrapper's first statement: the body opens
    // with `(void)self;`, so a negative assertion cut at the first `;` would
    // hold whatever the cast below it said.
    let scale = thunk_call_line(&inc, "fnptr_0m4_Grid5_scale6_static");
    assert!(!scale.contains("void *,"), "{scale}");
}

/// The single line of generated C that casts `symbol` to its native
/// signature and calls it -- the text that decides whether a wrapper
/// prepends a receiver.
fn thunk_call_line<'a>(inc: &'a str, symbol: &str) -> &'a str {
    inc.lines()
        .find(|line| line.contains(&format!(")){symbol})")))
        .unwrap_or_else(|| panic!("no call through `{symbol}` in:\n{inc}"))
}

#[test]
fn a_nullary_class_method_declares_the_receiver_as_its_whole_parameter_list() {
    // `c_param_list` answers `"void"` for an empty carried list, because an
    // empty C parameter list means "unspecified" rather than "no
    // arguments". A `@classmethod` whose receiver-free tail is empty still
    // takes the receiver slot natively, so the declaration must read
    // `(void *)` -- never `(void *, void)`, which does not compile, and
    // never `(void)`, which would disagree with the compiled arity.
    let inc = inc_no_classes("m", &[class_export("Grid", "make", vec![], Ty::Int)]);
    let decl = inc
        .split("extern void *fnptr_0m4_Grid4_make11_classmethod;")
        .nth(1)
        .expect("the classmethod's function-pointer global is emitted");
    assert!(!decl.contains("void *, void"), "{decl}");
    let call = inc
        .split("pycc_ext_wrap_0m4_Grid4_make11_classmethod(")
        .nth(1)
        .expect("the classmethod wrapper is emitted");
    assert!(
        call.contains("(void *))fnptr_0m4_Grid4_make11_classmethod"),
        "{call}"
    );
    assert!(call.contains("(NULL)"), "{call}");
}

#[test]
fn a_method_wrapper_renders_the_source_level_spelling_in_every_host_visible_message() {
    // A message the host reads must never carry a spelling the user did not
    // write: the arity error says `Grid.scale()`, not
    // `Grid.scale.static()` and not `0m4_Grid5_scale6_static()`.
    let inc = inc_no_classes(
        "m",
        &[static_export("Grid", "scale", vec![Ty::Int], Ty::Int)],
    );
    assert!(inc.contains("Grid.scale() takes exactly"), "{inc}");
    assert!(!inc.contains("Grid.scale.static()"), "{inc}");
    assert!(!inc.contains("0m4_Grid5_scale6_static()"), "{inc}");
}

#[test]
fn two_classes_are_emitted_in_first_export_order() {
    // The companion must be byte-identical across runs, so the class order
    // is the export order and never a hash-map walk.
    let inc = inc_no_classes(
        "m",
        &[
            static_export("Zeta", "one", vec![], Ty::Int),
            static_export("Alpha", "two", vec![], Ty::Int),
            static_export("Zeta", "three", vec![], Ty::Int),
        ],
    );
    let zeta = inc.find("pycc_ext_type_methods_Zeta").expect("Zeta");
    let alpha = inc.find("pycc_ext_type_methods_Alpha").expect("Alpha");
    assert!(zeta < alpha, "{inc}");
    let table = inc
        .split("static PyMethodDef pycc_ext_type_methods_Zeta[] = {")
        .nth(1)
        .expect("Zeta's table")
        .split("};")
        .next()
        .expect("terminated");
    assert!(table.contains("\"one\""), "{table}");
    assert!(table.contains("\"three\""), "{table}");
    assert!(!table.contains("\"two\""), "{table}");
}

#[test]
fn a_tuple_carrying_exported_class_method_goes_through_its_thunk() {
    // A `tuple` in the signature routes the call through
    // `pycc_ext_thunk_<symbol>` (#1050) rather than the `fnptr_` global
    // directly, and the mangled symbol travels into that spelling too.
    let inc = inc_no_classes(
        "m",
        &[class_export(
            "Grid",
            "pair",
            vec![Ty::Int],
            Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])),
        )],
    );
    assert!(
        inc.contains("pycc_ext_thunk_0m4_Grid4_pair11_classmethod"),
        "{inc}"
    );
}

fn grid_ctor(params: Vec<Ty>, slot_names: &[&str]) -> ExtCtor {
    ExtCtor {
        defaults: Vec::new(),
        class: "Grid".to_string(),
        name: "Grid.__init__".to_string(),
        param_writable: vec![false; params.len()],
        params,
        slot_names: slot_names.iter().map(ToString::to_string).collect(),
        slot_kinds: "i".repeat(slot_names.len()),
        keyword_names: None,
    }
}

#[test]
fn an_exported_instance_method_is_a_plain_fastcall_row_that_unwraps_its_receiver() {
    let inc = inc_no_classes("m", &[instance_export("Grid", "area", vec![], Ty::Int)]);
    // Neither `METH_STATIC` nor `METH_CLASS`: a plain `METH_FASTCALL` row is
    // what makes CPython deliver the instance in `self` -- and, through its
    // method-descriptor machinery, type-check that instance before the
    // wrapper is entered.
    assert!(
        inc.contains(
            "    {\"area\", (PyCFunction)(void (*)(void))pycc_ext_wrap_0m4_Grid4_area, \
             METH_FASTCALL, NULL},\n"
        ),
        "{inc}"
    );
    // The NULL guard is reachable, not defensive: `mod.Grid.__new__(mod.Grid)`
    // runs `PyType_GenericNew` and never `tp_init`.
    assert!(
        inc.contains(
            "    void *self_inst = ((PyccExtInstance *)self)->inst;\n    \
             if (self_inst == NULL) {\n        PyErr_SetString(PyExc_TypeError, \
             \"Grid.area() called on an uninitialized instance\");\n        \
             return NULL;\n    }\n"
        ),
        "{inc}"
    );
    // The receiver reaches the compiled call as the real pointer, where a
    // `@classmethod`'s `cls` reaches it as `NULL`.
    assert!(inc.contains("fnptr_0m4_Grid4_area)(self_inst)"), "{inc}");
    // No flat module attribute: the module's own table stays receiver-free.
    assert!(!inc.contains("{\"Grid.area\""), "{inc}");
}

#[test]
fn a_constructible_class_gets_a_tp_init_three_slots_and_a_carrier_sized_spec() {
    let inc = generate_exports_inc(
        "m",
        &[instance_export("Grid", "area", vec![], Ty::Int)],
        &[],
        &flat_publications(&[instance_export("Grid", "area", vec![], Ty::Int)]),
        &[grid_ctor(vec![Ty::Int, Ty::Int], &["w", "h"])],
        &[],
    );
    assert!(
        inc.contains(
            "extern void *fnptr_0m4_Grid8___init__;\n\
             static int pycc_ext_tp_init_Grid(PyObject *self, PyObject *args, PyObject *kwds)\n\
             {\n    void *inst;\n    long long a0;\n    long long a1;\n"
        ),
        "{inc}"
    );
    // D-244 rule 7's keyword boundary for a constructor that is not
    // keyword-enabled (#1461; this fixture's `keyword_names` is `None`),
    // written out by hand because a `tp_init` is not a `METH_FASTCALL`
    // entry point and CPython refuses nothing on its behalf. Without this, a
    // correct positional count plus a keyword would silently ignore the
    // keyword.
    assert!(
        inc.contains(
            "    if (kwds != NULL && PyDict_Size(kwds) != 0) {\n        \
             PyErr_SetString(PyExc_TypeError, \
             \"Grid.__init__() takes no keyword arguments\");\n        return -1;\n    }\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    if (PyTuple_Size(args) != 2) {\n        PyErr_Format(PyExc_TypeError, \
             \"Grid.__init__() takes exactly 2 arguments (%zd given)\", \
             PyTuple_Size(args));\n        return -1;\n    }\n"
        ),
        "{inc}"
    );
    // The same `pycc_ext_unpack_*` helper an ordinary export uses for a
    // declared `int`, bridged through `PyTuple_GetItem` because `tp_init`
    // receives a tuple rather than a `METH_FASTCALL` vector. A
    // `PyArg_ParseTuple` shortcut here would give `mod.Grid(True, 4)` and
    // `mod.Grid(3, 4).area()` two different admissibility matrices for one
    // declared type.
    assert!(
        inc.contains(
            "    if (pycc_ext_unpack_int(PyTuple_GetItem(args, 0), \"Grid.__init__\", 0, &a0) \
             != 0) {\n        return -1;\n    }\n    \
             if (pycc_ext_unpack_int(PyTuple_GetItem(args, 1), \"Grid.__init__\", 1, &a1) \
             != 0) {\n        return -1;\n    }\n"
        ),
        "{inc}"
    );
    // The slot count is `flat_attr_layout`'s, not the argument count, and
    // the compiled constructor is called through its `fnptr_` slot with the
    // instance first -- the same shape `MirExpr::Instantiate` emits.
    assert!(
        inc.contains(
            "    inst = pycc_rt_instance_new(2, \"Grid\\000w\\000h\\000ii\", 11);\n    \
             pycc_rt_instance_incref(inst);\n    \
             ((void (*)(void *, long long, long long))fnptr_0m4_Grid8___init__)\
             (inst, a0, a1);\n"
        ),
        "{inc}"
    );
    // A constructor that raised must not publish its carrier: the pending
    // flag is read before the store, exactly as `wrapper_for` reads it
    // before packing a result.
    assert!(
        inc.contains(
            "    if (pycc_rt_ext_pending_type() >= 0) {\n        \
             pycc_rt_instance_decref(inst);\n        \
             pycc_ext_raise_pending();\n        \
             return -1;\n    }\n    \
             pycc_ext_carrier_bind(self, inst);\n    return 0;\n}\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "static PyType_Slot pycc_ext_type_slots_Grid[] = {\n    \
             {Py_tp_methods, pycc_ext_type_methods_Grid},\n    \
             {Py_tp_new, PyType_GenericNew},\n    \
             {Py_tp_init, pycc_ext_tp_init_Grid},\n    \
             {Py_tp_dealloc, pycc_ext_instance_dealloc},\n    {0, NULL},\n};\n"
        ),
        "{inc}"
    );
    // `DISALLOW_INSTANTIATION` is gone, while `IMMUTABLETYPE` stays and `BASETYPE` is still absent: a host may
    // build an instance but may neither rebind the type's attributes nor
    // subclass it.
    assert!(
        inc.contains(
            "static PyType_Spec pycc_ext_type_spec_Grid = {\n    \
             PYCC_EXT_MODULE_NAME_STR \".Grid\",\n    sizeof(PyccExtInstance),\n    0,\n    \
             Py_TPFLAGS_DEFAULT | Py_TPFLAGS_IMMUTABLETYPE,\n    \
             pycc_ext_type_slots_Grid,\n};\n"
        ),
        "{inc}"
    );
    assert!(!inc.contains("Py_TPFLAGS_BASETYPE"), "{inc}");
}

#[test]
fn a_zero_argument_constructor_declares_a_receiver_only_parameter_list() {
    // `c_param_list` answers `"void"` for an empty carried list, which would
    // declare `void (*)(void, void *)` if it were pasted after the receiver.
    // The receiver-only spelling is `void *` alone.
    let inc = generate_exports_inc(
        "m",
        &[instance_export("Grid", "area", vec![], Ty::Int)],
        &[],
        &flat_publications(&[instance_export("Grid", "area", vec![], Ty::Int)]),
        &[grid_ctor(Vec::new(), &[])],
        &[],
    );
    assert!(
        inc.contains(
            "    if (PyTuple_Size(args) != 0) {\n        PyErr_Format(PyExc_TypeError, \
             \"Grid.__init__() takes exactly 0 arguments (%zd given)\", \
             PyTuple_Size(args));\n        return -1;\n    }\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    inst = pycc_rt_instance_new(0, \"Grid\\000\", 5);\n    \
             pycc_rt_instance_incref(inst);\n    \
             ((void (*)(void *))fnptr_0m4_Grid8___init__)(inst);\n"
        ),
        "{inc}"
    );
}

#[test]
fn a_one_argument_constructor_says_argument_in_the_singular() {
    let inc = generate_exports_inc(
        "m",
        &[instance_export("Grid", "area", vec![], Ty::Int)],
        &[],
        &flat_publications(&[instance_export("Grid", "area", vec![], Ty::Int)]),
        &[grid_ctor(vec![Ty::Int], &["w"])],
        &[],
    );
    assert!(
        inc.contains("takes exactly 1 argument (%zd given)"),
        "{inc}"
    );
}

#[test]
fn a_memoryview_constructor_releases_its_buffer_on_every_exit_past_the_acquire() {
    // `memoryview` is admitted at a parameter position, so it is admitted at
    // a constructor's too -- and a `tp_init` owes the same release discipline
    // a wrapper does, on the raising arm as well as the falling-through one.
    let inc = generate_exports_inc(
        "m",
        &[instance_export("Grid", "area", vec![], Ty::Int)],
        &[],
        &flat_publications(&[instance_export("Grid", "area", vec![], Ty::Int)]),
        &[grid_ctor(vec![Ty::MemoryView], &["w"])],
        &[],
    );
    assert!(
        inc.contains("    Py_buffer b0;\n    PyccExtBufferView a0;\n"),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    if (pycc_ext_unpack_memoryview(PyTuple_GetItem(args, 0), \"Grid.__init__\", \
             0, 0, &b0) != 0) {\n        return -1;\n    }\n    a0.ptr = b0.buf;\n    \
             a0.len = (long long)b0.shape[0];\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    if (pycc_rt_ext_pending_type() >= 0) {\n        PyBuffer_Release(&b0);\n        \
             pycc_rt_instance_decref(inst);\n        \
             pycc_ext_raise_pending();\n        \
             return -1;\n    }\n    \
             PyBuffer_Release(&b0);\n"
        ),
        "{inc}"
    );
}

#[test]
fn a_published_class_with_no_constructor_descriptor_is_a_non_instantiable_carrier() {
    // The scope line: publication is decided by `publications` and
    // constructibility by `ctors`, so a class absent from `ctors` gets no
    // `tp_init` and keeps `DISALLOW_INSTANTIATION` -- but since #1435 it is
    // still a carrier type, with the carrier's `basicsize` and the shared
    // deallocator, because its instances can cross as call arguments.
    let exports = [instance_export("Grid", "area", vec![], Ty::Int)];
    let without = generate_exports_inc("m", &exports, &[], &flat_publications(&exports), &[], &[]);
    assert!(!without.contains("pycc_ext_tp_init_Grid"), "{without}");
    assert!(
        without.contains(
            "static PyType_Slot pycc_ext_type_slots_Grid[] = {\n    \
             {Py_tp_methods, pycc_ext_type_methods_Grid},\n    \
             {Py_tp_dealloc, pycc_ext_instance_dealloc},\n    {0, NULL},\n};\n"
        ),
        "{without}"
    );
    assert!(!without.contains("Py_tp_new"), "{without}");
    assert!(
        without.contains(
            "static PyType_Spec pycc_ext_type_spec_Grid = {\n    \
             PYCC_EXT_MODULE_NAME_STR \".Grid\",\n    sizeof(PyccExtInstance),\n    0,\n    \
             Py_TPFLAGS_DEFAULT | Py_TPFLAGS_DISALLOW_INSTANTIATION | \
             Py_TPFLAGS_IMMUTABLETYPE,\n    pycc_ext_type_slots_Grid,\n};\n"
        ),
        "{without}"
    );
}

#[test]
fn a_constructor_descriptor_for_an_unpublished_class_emits_nothing() {
    // The class list is `publications` alone, so a constructor descriptor
    // whose class publishes no method must not conjure a type object -- the
    // two sets are deliberately not the same set.
    let inc = generate_exports_inc("m", &[], &[], &[], &[grid_ctor(vec![Ty::Int], &["w"])], &[]);
    assert!(!inc.contains("pycc_ext_tp_init_Grid"), "{inc}");
    assert!(!inc.contains("PyType_FromSpec"), "{inc}");
}
