//! User exception class registration.

use super::*;

/// The generated companion for a program lowered from real source, so the
/// tags and the class order are the ones `pycc_hir::program::finalize`
/// really assigns rather than ones a hand-built `HirClassDef` asserts into
/// existence (`module()` here builds an empty `class_defs`).
fn inc_from_source(source: &str) -> String {
    let dir = pycc_scratch::ScratchDir::new("ext_exception_classes").expect("scratch");
    let src = dir.join("m.py");
    std::fs::write(&src, source).expect("write source");
    let module = crate::frontend::resolve_frontend(&src, Some("m"))
        .unwrap_or_else(|_| panic!("the fixture must type-check"));
    generate_exports_inc(
        "m",
        &[],
        &collect_user_exception_classes(&module),
        &[],
        &[],
        &[],
    )
}

#[test]
fn a_program_with_no_user_exception_class_still_emits_both_class_functions() {
    // Both are emitted unconditionally, with empty bodies: the shim calls
    // them by name whatever the program declares, so an artifact that
    // omitted them would fail to link. A program whose only exception
    // classes are the seeded builtins is this same case -- they carry
    // `None` or a fixed tag below `FIRST_USER_EXCEPTION_TYPE_TAG` and are
    // never table entries.
    let inc = inc_no_classes("m", &[]);
    assert!(
        inc.contains(&format!(
            "{USER_EXCEPTION_LOOKUP_DECL}\n{{\n    (void)tag;\n    return NULL;\n}}\n"
        )),
        "{inc}"
    );
    assert!(
        inc.contains(&format!(
            "{USER_EXCEPTION_REGISTER_DECL}\n{{\n    (void)module;\n    return 0;\n}}\n"
        )),
        "{inc}"
    );
    assert!(!inc.contains("pycc_ext_user_exception_classes["), "{inc}");
    let builtins_only = inc_from_source("try:\n    x = 1\nexcept ValueError:\n    x = 2\n");
    assert!(
        !builtins_only.contains("pycc_ext_user_exception_classes["),
        "{builtins_only}"
    );
}

#[test]
fn a_class_subclassing_exception_is_registered_under_the_qualified_module_name() {
    let inc = inc_from_source("class MyError(Exception):\n    pass\n\nx = 1\n");
    assert!(
        inc.contains("static PyObject *pycc_ext_user_exception_classes[1];"),
        "{inc}"
    );
    assert!(
        inc.contains(
            "        pycc_ext_user_exception_classes[0] = PyErr_NewException(\n            \
             PYCC_EXT_MODULE_NAME_STR \".MyError\", PyExc_Exception, NULL);\n"
        ),
        "{inc}"
    );
    // The cache owns the strong reference and `AddObjectRef` takes its own,
    // so the slot is assigned before the registration is checked.
    assert!(
        inc.contains(
            "    if (PyModule_AddObjectRef(module, \"MyError\", \
             pycc_ext_user_exception_classes[0]) < 0) {\n        return -1;\n    }\n"
        ),
        "{inc}"
    );
    // A non-NULL slot is reused rather than re-minted, so a re-import that
    // re-runs `Py_mod_exec` keeps class identity.
    assert!(
        inc.contains("    if (pycc_ext_user_exception_classes[0] == NULL) {\n"),
        "{inc}"
    );
    // Every tag the table does not hold -- tag 0, the 23..=24 pair C3 keeps
    // as `Exception`, and any sparse gap -- leaves the `switch` through an
    // explicit `return NULL`, never off the end of a non-`void` function.
    assert!(
        inc.contains("    default:\n        return NULL;\n    }\n}\n"),
        "{inc}"
    );
}

#[test]
fn a_class_subclassing_a_builtin_carries_that_builtin_as_its_base() {
    // The C1 case: without the base, a host-side `except ValueError:` does
    // not match what `except ValueError:` catches natively.
    let inc = inc_from_source("class MyValueError(ValueError):\n    pass\n\nx = 1\n");
    assert!(
        inc.contains("PYCC_EXT_MODULE_NAME_STR \".MyValueError\", PyExc_ValueError, NULL);"),
        "{inc}"
    );
}

#[test]
fn a_class_subclassing_a_user_class_names_that_class_slot_and_follows_it() {
    let inc = inc_from_source(
        "class MyValueError(ValueError):\n    pass\n\n\
         class Deep(MyValueError):\n    pass\n\nx = 1\n",
    );
    assert!(
        inc.contains("static PyObject *pycc_ext_user_exception_classes[2];"),
        "{inc}"
    );
    assert!(
        inc.contains(
            "PYCC_EXT_MODULE_NAME_STR \".Deep\", pycc_ext_user_exception_classes[0], NULL);"
        ),
        "{inc}"
    );
    // The base is created before the subclass that names it. The source is
    // lowered for real here, so this exercises `finalize`'s program-order
    // tag assignment and `resolve_mro`'s base-before-subclass requirement
    // rather than an order a hand-built `HirClassDef` asserted into being.
    let base = inc.find("\".MyValueError\"").expect("the base entry");
    let derived = inc.find("\".Deep\"").expect("the derived entry");
    assert!(base < derived, "{inc}");
}

#[test]
fn a_class_with_two_exception_bases_is_created_from_a_tuple_of_both() {
    // Natively `except ValueError:` catches this class, because matching
    // walks the whole MRO. Taking a single base out of the linearized MRO
    // would pick `MyBase` and drop `ValueError`, so the tuple is what keeps
    // the host side in step.
    let inc = inc_from_source(
        "class MyBase(Exception):\n    pass\n\n\
         class E(MyBase, ValueError):\n    pass\n\nx = 1\n",
    );
    assert!(
        inc.contains(
            "        PyObject *bases = PyTuple_Pack(2, \
             pycc_ext_user_exception_classes[0], PyExc_ValueError);\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains("PYCC_EXT_MODULE_NAME_STR \".E\", bases, NULL);"),
        "{inc}"
    );
    assert!(inc.contains("        Py_DECREF(bases);\n"), "{inc}");
}

#[test]
fn a_group_derived_class_is_excluded_without_shifting_the_class_beside_it() {
    // The sparse-tag case, and the reason entries are keyed by explicit tag
    // rather than indexed by `tag - FIRST_USER_EXCEPTION_TYPE_TAG`: the
    // group class still *consumes* the first user tag while being excluded
    // from the table, so an array sized by entry count and indexed by that
    // offset would send `E` past its bounds, return NULL, and silently
    // flatten `E` to `Exception` with no compile or link error.
    let inc = inc_from_source(
        "class G(ExceptionGroup):\n    pass\n\nclass E(ValueError):\n    pass\n\nx = 1\n",
    );
    let group_tag = FIRST_USER_EXCEPTION_TYPE_TAG;
    let user_tag = FIRST_USER_EXCEPTION_TYPE_TAG + 1;
    assert!(
        inc.contains("static PyObject *pycc_ext_user_exception_classes[1];"),
        "{inc}"
    );
    assert!(
        inc.contains(&format!(
            "    case {user_tag}:\n        return pycc_ext_user_exception_classes[0];\n"
        )),
        "{inc}"
    );
    assert!(!inc.contains(&format!("    case {group_tag}:")), "{inc}");
    assert!(
        inc.contains("PYCC_EXT_MODULE_NAME_STR \".E\", PyExc_ValueError, NULL);"),
        "{inc}"
    );
    assert!(!inc.contains("\".G\""), "{inc}");
}

#[test]
fn a_non_exception_base_is_dropped_from_the_synthesized_class() {
    // A method-only mixin passes the attribute-layout prefix check, so
    // `class E(Mixin, ValueError)` compiles -- but it is not an exception
    // class and has no host-side counterpart, so the synthesized class
    // carries `ValueError` alone and `isinstance(e, Mixin)` diverges from
    // native pycc. `docs/RUNTIME.md` records that residual.
    let inc = inc_from_source(
        "class Mixin:\n    pass\n\nclass E(Mixin, ValueError):\n    pass\n\nx = 1\n",
    );
    assert!(
        inc.contains("PYCC_EXT_MODULE_NAME_STR \".E\", PyExc_ValueError, NULL);"),
        "{inc}"
    );
    assert!(!inc.contains("PyTuple_Pack"), "{inc}");
    assert!(!inc.contains("Mixin"), "{inc}");
}

#[test]
fn the_shims_float_tuple_unpack_is_a_strict_container_with_converting_elements() {
    // PR 4c of #1083's admission rule, read off the shim itself. The two
    // halves answer two different questions and a reviewer must be able to
    // see both: `PyTuple_Check` plus an exact-arity gate for the container
    // (D-115/D-116 leave no shape for a differently sized sequence), and
    // `PyNumber_Float` for the elements (the author wrote `float` in the
    // annotation, so CPython's own conversion is the contract -- PR 4a's
    // rule-7 paragraph).
    let shim = shim_c();
    let body = &shim[shim
        .find("int pycc_ext_obj_unpack_float_tuple(PyObject *o, long long arity, double *out)")
        .expect("the unpack helper")..];
    // Bounded at this helper's own closing brace, the way the `unpack_str`
    // test above bounds its own: the assertions below would otherwise read
    // every later helper's prose too.
    let end = body.find("\n}\n").expect("the helper's end");
    let body = &body[..end];
    let container = body.find("!PyTuple_Check(o)").expect("the container check");
    let arity = body
        .find("if (size != (Py_ssize_t)arity) {")
        .expect("the exact-arity gate");
    let convert = body
        .find("converted = PyNumber_Float(PyTuple_GetItem(o, index));")
        .expect("the element conversion");
    assert!(
        container < arity && arity < convert,
        "the container must be settled before any element is converted:\n{body}"
    );
    // `PyTuple_Check`, never `PyTuple_CheckExact`: a `tuple` subclass is a
    // tuple, and the elements are copied out by value.
    assert!(!body.contains("PyTuple_CheckExact"), "{body}");
    // The elements are converted, never type-checked: a `PyFloat_Check`
    // here would be the closed-seam rule applied to an explicit conversion.
    assert!(!body.contains("PyFloat_Check"), "{body}");
    // Each temporary is released inside the iteration that produced it, so
    // the failing exit holds nothing (#1092 stays where it is).
    let release = body.find("Py_DECREF(converted);").expect("the release");
    let store = body.find("out[index] = value;").expect("the store");
    assert!(
        convert < release && release < store,
        "the temporary must be released before the next iteration:\n{body}"
    );
    // Two distinct refusals, each naming the declared arity: a reviewer
    // reading "strict container" must find exactly the wrong-type and the
    // wrong-length message, and neither may borrow the thunk seam's
    // function-name-and-argument-index phrasing, which has no meaning at an
    // assignment.
    assert!(
        body.contains("\"expected a tuple of %zd floats, got a '%U' object\""),
        "{body}"
    );
    assert!(
        body.contains("\"expected a tuple of %zd floats, got a tuple of length %zd\""),
        "{body}"
    );
    assert!(!body.contains("() argument"), "{body}");
}

#[test]
fn the_shims_float_tuple_unpack_takes_its_arity_as_a_parameter() {
    // The admission rule is *any* fixed arity with every element `float`,
    // so no side of this seam may hard-code the three of
    // `tuple[float, float, float]`. The declaration carries `long long
    // arity`, matching the `i64` argument
    // `crates/pycc_codegen/src/foreign_len.rs` emits, and the helper's name
    // is `pycc_ext_obj_unpack_float_tuple` -- never `pycc_ext_unpack_*`,
    // whose prefix `refusal_completeness.rs` scans and whose suffix set it
    // pins exactly.
    let shim = shim_c();
    assert!(
        shim.contains(
            "int pycc_ext_obj_unpack_float_tuple(PyObject *o, long long arity, double *out)"
        ),
        "{shim}"
    );
    assert!(!shim.contains("pycc_ext_unpack_float_tuple"), "{shim}");
}
