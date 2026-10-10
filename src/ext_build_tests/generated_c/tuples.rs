//! Tuple parameters and returns across the boundary.

use super::*;

#[test]
fn a_tuple_parameter_is_checked_once_then_unpacked_element_by_element() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "total".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Tuple(Box::new(vec![Ty::Int, Ty::Float]))],
            param_writable: vec![false; 1],
            return_ty: Ty::Int,
            keyword_names: None,
        }],
    );
    // One `tuple` argument, so the arity message still says one: the
    // wrapper's argument space is the Python one and never the flattened C
    // one (#1050).
    assert!(
        inc.contains("takes exactly 1 argument (%zd given)"),
        "{inc}"
    );
    // Shape and length first -- that check is what makes every
    // `PyTuple_GetItem` below infallible.
    assert!(
        inc.contains("pycc_ext_unpack_tuple(args[0], \"total\", 0, 2) != 0"),
        "{inc}"
    );
    assert!(
        inc.contains(
            "pycc_ext_unpack_int_at(PyTuple_GetItem(args[0], 0), \"total\", 0, 0, &a0_0) != 0"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "pycc_ext_unpack_float_at(PyTuple_GetItem(args[0], 1), \"total\", 0, 1, &a0_1) != 0"
        ),
        "{inc}"
    );
    // The locals are per element and the call spreads them in place; no
    // aggregate is ever spelled in C.
    assert!(
        inc.contains("    long long a0_0;\n    double a0_1;\n"),
        "{inc}"
    );
    assert!(
        inc.contains("    result = pycc_ext_thunk_total(a0_0, a0_1);\n"),
        "{inc}"
    );
    assert!(
        inc.contains("extern long long pycc_ext_thunk_total(long long, double);\n"),
        "{inc}"
    );
}

#[test]
fn a_tuple_return_arrives_through_out_pointers_and_is_packed_afterwards() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "split".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Int],
            param_writable: vec![false; 1],
            return_ty: Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool])),
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains("extern void pycc_ext_thunk_split(long long, long long *, char *);\n"),
        "{inc}"
    );
    // No `result` local: a tuple return has no single value, and declaring
    // one would be an unused local at best and a type error at worst.
    assert!(!inc.contains("    result;"), "{inc}");
    assert!(!inc.contains("result = pycc_ext_thunk_split"), "{inc}");
    assert!(
        inc.contains("    pycc_ext_thunk_split(a0, &r0, &r1);\n"),
        "{inc}"
    );
    // The pending-exception check stands between the call and any read of
    // the out-pointer locals: a raising call leaves them uninitialized, so
    // packing first would be undefined behaviour rather than a wrong value.
    let call = inc.find("pycc_ext_thunk_split(a0").expect("the call");
    let pending = inc
        .find("pycc_rt_ext_pending_type() >= 0")
        .expect("the check");
    let first_pack = inc.find("e0 = pycc_ext_pack_int").expect("the first pack");
    assert!(call < pending && pending < first_pack, "{inc}");
    // Every element is packed before any failure is acted on: the retain
    // above gives this wrapper a reference the packer then discharges, so
    // bailing at the first failure would leak the rest.
    let second_pack = inc
        .find("e1 = pycc_ext_pack_bool(r1);")
        .expect("the second");
    let null_test = inc.find("if (e0 == NULL || e1 == NULL)").expect("the test");
    assert!(second_pack < null_test, "{inc}");
    assert!(
        inc.contains("        Py_XDECREF(e0);\n        Py_XDECREF(e1);\n"),
        "{inc}"
    );
    // `PyTuple_New` is reached with a fully-owned set, so its own failure
    // path releases unconditionally rather than with `Py_XDECREF`.
    assert!(
        inc.contains(
            "    packed = PyTuple_New(2);\n    if (packed == NULL) {\n        \
             Py_DECREF(e0);\n        Py_DECREF(e1);\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    PyTuple_SetItem(packed, 0, e0);\n    PyTuple_SetItem(packed, 1, e1);\n    \
             return packed;\n"
        ),
        "{inc}"
    );
}

#[test]
fn a_tuple_return_retains_each_int_element_before_packing_it() {
    // #1050 regression: a returned tuple's fields arrive *borrowed*. Codegen
    // retains at a `return` only for a `Scalar::Int`, and the export thunk
    // `extractvalue`s each field straight into its out-pointer, so a stored
    // tuple (`saved = (2**62,)`; `return saved`) hands the wrapper a word the
    // module global still owns. `pycc_ext_pack_int` releases that word on its
    // `OverflowError` path, so without this retain the second call to the
    // export faults inside the host interpreter.
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "split".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![],
            param_writable: vec![],
            return_ty: Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool, Ty::Float])),
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains(
            "    pycc_rt_bigint_retain(r0);\n    e0 = pycc_ext_pack_int(\"split\", r0);\n"
        ),
        "{inc}"
    );
    // Retain and release share one predicate, so the pairing has to come from
    // one list: only the `int` slots take a reference, because only the `int`
    // packer discharges one. A retain at a `bool` or `float` slot would be an
    // unbalanced +1 on every successful call.
    assert!(!inc.contains("pycc_rt_bigint_retain(r1)"), "{inc}");
    assert!(!inc.contains("pycc_rt_bigint_retain(r2)"), "{inc}");
    assert_eq!(inc.matches("pycc_rt_bigint_retain(").count(), 1, "{inc}");
}

#[test]
fn a_one_element_tuple_keeps_its_tuple_shape_in_both_directions() {
    // Arity 1 is the shape a flattening bug erases first: `tuple[int]` and
    // `int` occupy the same single slot at the thunk, and only the
    // `PyTuple_Check` on the way in and the `PyTuple_New(1)` on the way out
    // keep the Python-level types apart.
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "wrap".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Tuple(Box::new(vec![Ty::Int]))],
            param_writable: vec![false; 1],
            return_ty: Ty::Tuple(Box::new(vec![Ty::Int])),
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains("extern void pycc_ext_thunk_wrap(long long, long long *);\n"),
        "{inc}"
    );
    assert!(
        inc.contains("pycc_ext_unpack_tuple(args[0], \"wrap\", 0, 1) != 0"),
        "{inc}"
    );
    assert!(inc.contains("    packed = PyTuple_New(1);\n"), "{inc}");
}

#[test]
fn several_tuple_parameters_keep_one_local_namespace_each() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "dot".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![
                Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])),
                Ty::Tuple(Box::new(vec![Ty::Float, Ty::Bool])),
            ],
            param_writable: vec![false; 2],
            return_ty: Ty::Float,
            keyword_names: None,
        }],
    );
    // `a{argument}_{element}` and never a single running counter: the second
    // tuple's first element is `a1_0`, not `a2`.
    assert!(
        inc.contains(
            "    long long a0_0;\n    long long a0_1;\n    double a1_0;\n    char a1_1;\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains("    result = pycc_ext_thunk_dot(a0_0, a0_1, a1_0, a1_1);\n"),
        "{inc}"
    );
    // Each tuple is checked against its own arity, at its own argument
    // index, and each element reports its own position.
    assert!(
        inc.contains("pycc_ext_unpack_tuple(args[1], \"dot\", 1, 2) != 0"),
        "{inc}"
    );
    assert!(
        inc.contains(
            "pycc_ext_unpack_bool_at(PyTuple_GetItem(args[1], 1), \"dot\", 1, 1, &a1_1) != 0"
        ),
        "{inc}"
    );
    assert!(
        inc.contains("takes exactly 2 arguments (%zd given)"),
        "{inc}"
    );
}

#[test]
fn an_earlier_str_argument_is_released_when_a_later_tuple_is_refused() {
    // The only owning argument type is `str`, and a refused `tuple` after it
    // exits before the call that would have consumed the reference. A tuple
    // owes nothing itself: its elements are copied out by value.
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "tag".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Str, Ty::Tuple(Box::new(vec![Ty::Int]))],
            param_writable: vec![false; 2],
            return_ty: Ty::Str,
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains(
            "    if (pycc_ext_unpack_tuple(args[1], \"tag\", 1, 1) != 0) {\n        \
             pycc_rt_str_decref(a0);\n        return NULL;\n    }\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains(
            "    if (pycc_ext_unpack_int_at(PyTuple_GetItem(args[1], 0), \"tag\", 1, 0, &a1_0) \
             != 0) {\n        pycc_rt_str_decref(a0);\n        return NULL;\n    }\n"
        ),
        "{inc}"
    );
    assert!(
        inc.contains("    result = pycc_ext_thunk_tag(a0, a1_0);\n"),
        "{inc}"
    );
}

#[test]
fn a_nullary_export_returning_a_tuple_declares_only_its_out_pointers() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "origin".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: Vec::new(),
            param_writable: Vec::new(),
            return_ty: Ty::Tuple(Box::new(vec![Ty::Float, Ty::Float])),
            keyword_names: None,
        }],
    );
    // The combined list is non-empty even though the export takes nothing,
    // so `void` as a parameter list would be wrong here -- the out-pointers
    // are real parameters.
    assert!(
        inc.contains("extern void pycc_ext_thunk_origin(double *, double *);\n"),
        "{inc}"
    );
    assert!(
        inc.contains("    pycc_ext_thunk_origin(&r0, &r1);\n"),
        "{inc}"
    );
    assert!(
        inc.contains("takes exactly 0 arguments (%zd given)"),
        "{inc}"
    );
    // `pack_float` takes no function name: `PyFloat_FromDouble` cannot fail
    // on a value, unlike D-141's bigint egress.
    assert!(inc.contains("    e0 = pycc_ext_pack_float(r0);\n"), "{inc}");
}

#[test]
fn the_thunk_is_declared_as_a_function_and_called_without_a_cast() {
    // The load-bearing structural property of the whole seam. pycc's
    // aggregate calling convention is not the platform C struct ABI --
    // measured on aarch64-apple-darwin, a pycc function returning
    // `tuple[int x 5]` hands the words back in `x0`-`x4` where clang would
    // pass a hidden `sret` pointer -- so the thunk exists to keep every
    // aggregate on the LLVM side. Declaring it the way the scalar path
    // declares `fnptr_<name>` (a `void *` global, cast at the call to a
    // function-pointer type) reproduced a SIGBUS: the cast form type-checks
    // against nothing, and the `--ext` link defers undefined symbols
    // (`-undefined dynamic_lookup` on Mach-O, `-Bsymbolic` on ELF) so a
    // disagreement here links cleanly and dies on the first call.
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "pair".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int]))],
            param_writable: vec![false; 1],
            return_ty: Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])),
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains(
            "extern void pycc_ext_thunk_pair(long long, long long, long long *, \
             long long *);\n"
        ),
        "{inc}"
    );
    assert!(!inc.contains("extern void *pycc_ext_thunk_"), "{inc}");
    assert!(
        !inc.contains("(*)(long long, long long, long long *"),
        "{inc}"
    );
    // The call goes through the thunk alone; `fnptr_pair` is read only by
    // #1199's null guard, never cast to a function type.
    assert!(!inc.contains("fnptr_pair)("), "{inc}");
    assert_eq!(inc.matches("fnptr_pair").count(), 2, "{inc}");
    // A scalar-only export in the same module keeps the legacy spelling
    // byte for byte: #1050 widens the boundary without rewriting what
    // #1036 through #1049 already emit.
    let scalar = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "square".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Int],
            param_writable: vec![false; 1],
            return_ty: Ty::Int,
            keyword_names: None,
        }],
    );
    assert!(scalar.contains("extern void *fnptr_square;"), "{scalar}");
    assert!(
        scalar.contains("result = ((long long (*)(long long))fnptr_square)(a0);"),
        "{scalar}"
    );
    assert!(!scalar.contains("pycc_ext_thunk_"), "{scalar}");
}

#[test]
fn a_tuple_carrying_export_returning_none_assigns_nothing_and_fabricates_none() {
    let inc = inc_no_classes(
        "m",
        &[ExtExport {
            defaults: Vec::new(),
            name: "record".to_string(),
            class: None,
            method: None,
            returns_buffer_slice: false,
            receiver: ExtReceiver::None,
            params: vec![Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool]))],
            param_writable: vec![false; 1],
            return_ty: Ty::None,
            keyword_names: None,
        }],
    );
    assert!(
        inc.contains("extern void pycc_ext_thunk_record(long long, char);\n"),
        "{inc}"
    );
    assert!(
        inc.contains("    pycc_ext_thunk_record(a0_0, a0_1);\n"),
        "{inc}"
    );
    assert!(!inc.contains("result"), "{inc}");
    assert!(inc.contains("    return Py_NewRef(Py_None);\n"), "{inc}");
}
