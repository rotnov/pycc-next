//! `ty_to_basic_type`, scalar coercion rejections and slot-binding collection.

use super::*;

#[test]
fn ty_to_basic_type_gives_a_memoryview_a_pointer_representation() {
    // Part 1 of #1027. The one source shape that reaches this arm is a
    // `memoryview` parameter of a `pycc build --ext` export, which needs
    // CPython development headers and so lives in an `#[ignore]`d hosted
    // test that contributes no coverage. The property is header-free
    // though, and it is the one the whole call convention rests on: what
    // crosses the boundary is a pointer to the wrapper's own
    // `PyccExtBufferView`, not CPython's `Py_buffer`, so the emitted IR
    // depends on no CPython struct layout and the representation is the
    // same plain pointer `Ty::Str`/`Ty::Object` already get. Since Part 2
    // of #1175 (#1179) `ext_thunk_required` no longer answers `false` for
    // every such export -- a body that returns `b[start:stop]` carries
    // three out-pointers and so does need a thunk -- but that turns on the
    // *body*, never on this representation.
    let context = Context::create();
    assert!(ty_to_basic_type(&context, Ty::MemoryView).is_pointer_type());
}

#[test]
fn ty_to_basic_type_gives_an_opaque_object_a_pointer_representation() {
    // Part 1 of #1026. No *source* reaches this arm today: an `object`
    // is only ever a module binding, `declare_module_globals` decides a
    // module global's storage with its own match rather than through
    // this function, and the two positions that would route an `object`
    // here -- a function parameter and a return -- are refused before
    // codegen (`I0404` inside the module, `C0003` at the `ext` export
    // boundary). The arm exists anyway because this function's catch-all
    // is a `panic!`, so the first construct that does give an `object` a
    // signature position would turn a supported program into an internal
    // compiler error rather than a diagnostic; this test is what keeps
    // the arm honest until then. `PyObject *` is opaque to pycc, so the
    // only property worth pinning is that it is the same plain pointer
    // `Ty::Str`/`Ty::List` already get.
    let context = Context::create();
    assert!(ty_to_basic_type(&context, Ty::Object).is_pointer_type());
}

#[test]
fn ty_to_basic_type_gives_tuple_a_struct_representation_positionally() {
    // Supersedes `ty_to_basic_type_panics_clearly_for_tuple`, which
    // asserted that `Ty::Tuple` had no LLVM representation at all --
    // accurate through PR-11a, but PR-11b Task 5 gives it a real arm,
    // so a test demanding a panic would now be asserting the absence of
    // this task's own feature.
    //
    // D-115: unlike every other container (all pointer-represented),
    // a tuple's LLVM type is a real `struct` built field-by-field from
    // each element's own `ty_to_basic_type`. Traces the actual field
    // types rather than just "is a struct", so this pins that each
    // position maps to that element type's own scalar representation
    // exactly (`i64`/`i8`/`f64`, mirroring `Ty::Int`/`Ty::Bool`/
    // `Ty::Float`'s own arms) -- a positional mix-up or a widened
    // `bool` field would still be "a struct" but would fail here.
    let context = Context::create();
    let basic_type = ty_to_basic_type(
        &context,
        Ty::Tuple(Box::new(vec![Ty::Int, Ty::Bool, Ty::Float])),
    );
    let struct_ty = basic_type.into_struct_type();
    assert_eq!(struct_ty.count_fields(), 3);
    assert_eq!(
        struct_ty.get_field_type_at_index(0),
        Some(context.i64_type().into()),
        "a tuple's int field keeps Ty::Int's own i64 representation"
    );
    assert_eq!(
        struct_ty.get_field_type_at_index(1),
        Some(context.i8_type().into()),
        "a tuple's bool field keeps Ty::Bool's own i8 representation (D-061, not i1)"
    );
    assert_eq!(
        struct_ty.get_field_type_at_index(2),
        Some(context.f64_type().into()),
        "a tuple's float field keeps Ty::Float's own f64 representation"
    );
    assert!(
        !struct_ty.is_packed(),
        "tuple fields stay naturally aligned -- ty_to_basic_type passes packed=false"
    );
}

#[test]
fn ty_to_basic_type_boxes_a_tuple_with_a_non_scalar_element() {
    // A tuple of int/bool/float elements is an SSA struct; any other
    // element (a nested tuple included) makes it a boxed tuple object,
    // held by pointer like a list (`pycc_hir::tuple_is_boxed`).
    let context = Context::create();
    let flat = ty_to_basic_type(&context, Ty::Tuple(Box::new(vec![Ty::Int, Ty::Float])));
    assert_eq!(flat.into_struct_type().count_fields(), 2);
    let nested = ty_to_basic_type(
        &context,
        Ty::Tuple(Box::new(vec![
            Ty::Int,
            Ty::Tuple(Box::new(vec![Ty::Float])),
        ])),
    );
    assert!(nested.is_pointer_type());
}

#[test]
fn ty_to_basic_type_gives_list_dict_and_set_a_pointer_representation_like_str() {
    // Task 5 (D-089) gave `List(_)` a *real* arm here -- the runtime
    // list object is heap-allocated and pointer-referenced exactly
    // like `Str`'s `PyStrObj`, so both must produce the same LLVM
    // representation. PR-11 Task 5 gave `Dict(_)` the identical
    // treatment for the identical reason (`PyDictObj` is heap-
    // allocated and pointer-referenced too), and PR-11 Task 9 gives
    // `Set(_)` the same treatment again (`PyIntSetObj` is heap-
    // allocated and pointer-referenced too) -- unlike `Tuple`, which
    // PR-11b Task 5 gives a by-value *struct* representation instead,
    // precisely because it has no heap object to point at (D-115; see
    // `ty_to_basic_type_gives_tuple_a_struct_representation_
    // positionally` above). Traces through the actual return value
    // (not just
    // "doesn't panic") to prove all four really match, for a
    // `list[int]`/`list[str]` element type, a `dict[str, int]`
    // key/value pair, and a `set[int]` element type --
    // `ty_to_basic_type`'s own `List(_)`/`Dict(_)`/`Set(_)` arms ignore
    // the element/key/value types entirely.
    let context = Context::create();
    let str_repr = ty_to_basic_type(&context, Ty::Str);
    let list_int_repr = ty_to_basic_type(&context, Ty::List(Box::new(Ty::Int)));
    let list_str_repr = ty_to_basic_type(&context, Ty::List(Box::new(Ty::Str)));
    let dict_repr = ty_to_basic_type(&context, Ty::Dict(Box::new((Ty::Str, Ty::Int))));
    let set_repr = ty_to_basic_type(&context, Ty::Set(Box::new(Ty::Int)));
    assert!(str_repr.is_pointer_type());
    assert!(list_int_repr.is_pointer_type());
    assert!(list_str_repr.is_pointer_type());
    assert!(dict_repr.is_pointer_type());
    assert!(set_repr.is_pointer_type());
    assert_eq!(str_repr, list_int_repr);
    assert_eq!(str_repr, list_str_repr);
    assert_eq!(str_repr, dict_repr);
    assert_eq!(str_repr, set_repr);
}

#[test]
fn reading_a_list_typed_local_back_out_of_its_alloca_produces_a_list_scalar() {
    // Exercises `emit_expr`'s `Name` arm's `Ty::List(_)` arm directly
    // (added by Task 5, D-089; retargeted from the `Scalar::Str` reuse
    // onto `Scalar::List` by Task 11a, D-107) -- same hand-built-
    // `StorageSlot` convention as `reading_an_unresolved_infer_typed_
    // local_is_an_internal_error` above: hand-building the slot is what
    // lets one `emit_expr` call read a `list[int]` local and the next
    // read a `str` one, with nothing else in the fixture to confuse
    // which variant came from which.
    //
    // D-107's entire point is that a `list[T]` pointer must stop being
    // *indistinguishable* from a `str` pointer at the `Scalar` level,
    // so this reads one `list[int]`-typed and one `str`-typed local
    // through the same `emit_expr` entry point and proves the two now
    // produce different variants.
    //
    // The variant is extracted through a helper exercised with *both*
    // values rather than a single-value `let Scalar::List(ptr) = value
    // else { panic!(..) }`: this arm returns `Scalar::List`
    // unconditionally for a `Ty::List` slot, so a single-value
    // `else`/`_` arm would still be statically unreachable and
    // permanently uncovered under this crate's 100%-region gate
    // (D-014) -- exactly the objection this test's own pre-Task-11a
    // comment raised, which giving `list[T]` its own variant does not
    // by itself remove. Feeding the same helper a `str` read covers
    // its other arm with a real, meaningful assertion instead.
    fn loaded_pointer_type<'ctx>(
        scalar: &Scalar<'ctx>,
    ) -> Option<inkwell::types::PointerType<'ctx>> {
        match scalar {
            Scalar::List(ptr) => Some(ptr.get_type()),
            _ => None,
        }
    }

    let context = Context::create();
    let module = context.create_module("test");
    let builder = context.create_builder();
    let rt = declare_rt_functions(&context, &module);
    let fn_type = context.void_type().fn_type(&[], false);
    let f = module.add_function("f", fn_type, None);
    let block = context.append_basic_block(f, "entry");
    let exception_target = context.append_basic_block(f, "exception");
    builder.position_at_end(block);
    rt.exceptions.targets.borrow_mut().push(exception_target);

    let user_functions: HashMap<&str, UserFunction> = HashMap::new();
    let ptr = builder
        .build_alloca(context.ptr_type(inkwell::AddressSpace::default()), "xs")
        .expect("build_alloca should not fail for a fresh block");
    builder
        .build_store(
            ptr,
            context
                .ptr_type(inkwell::AddressSpace::default())
                .const_null(),
        )
        .expect("build_store should not fail immediately after this function's own alloca");
    let str_ptr = builder
        .build_alloca(context.ptr_type(inkwell::AddressSpace::default()), "s")
        .expect("build_alloca should not fail for a fresh block");
    builder
        .build_store(
            str_ptr,
            context
                .ptr_type(inkwell::AddressSpace::default())
                .const_null(),
        )
        .expect("build_store should not fail immediately after this function's own alloca");
    let locals = HashMap::from([
        (
            "xs".to_string(),
            StorageSlot {
                ptr,
                ty: Ty::List(Box::new(Ty::Int)),
                initialized: None,
            },
        ),
        (
            "s".to_string(),
            StorageSlot {
                ptr: str_ptr,
                ty: Ty::Str,
                initialized: None,
            },
        ),
    ]);

    let list_value = emit_expr(
        &context,
        &builder,
        &module,
        &rt,
        &user_functions,
        &locals,
        &MirExpr::Name {
            name: "xs".to_string(),
            ty: Ty::List(Box::new(Ty::Int)),
        },
    );
    let str_value = emit_expr(
        &context,
        &builder,
        &module,
        &rt,
        &user_functions,
        &locals,
        &MirExpr::Name {
            name: "s".to_string(),
            ty: Ty::Str,
        },
    );
    // The `list[int]` read produces `Scalar::List` carrying the loaded
    // pointer (whose LLVM type must match what `ty_to_basic_type`
    // allocated the slot as); the `str` read, through the very same
    // entry point, does not -- which is the property D-107 exists to
    // establish and the one `Scalar::Str` reuse could never provide.
    assert_eq!(
        loaded_pointer_type(&list_value),
        Some(context.ptr_type(inkwell::AddressSpace::default()))
    );
    assert_eq!(loaded_pointer_type(&str_value), None);
}

#[test]
#[should_panic(
    expected = "pycc_codegen: internal error: an Optional[_] payload had an unsupported LLVM representation"
)]
fn truthy_of_an_optional_with_a_pointer_payload_panics_as_an_internal_error() {
    // `truthy`'s `Scalar::Optional` arm dispatches on the extracted
    // payload's own LLVM type (`FloatValue` for `Ty::Float`, an `i8`-wide
    // `IntValue` for `Ty::Bool`, any other `IntValue` for `Ty::Int`) --
    // #809 widened the accepted inner types to exactly `{int, float,
    // bool}`, so every real `Scalar::Optional` this function ever sees
    // has a payload matching one of those three shapes. The match's
    // catch-all `other => panic!(...)` arm exists purely as a defensive
    // backstop for a payload shape none of the three real inner types can
    // produce -- e.g. a pointer payload, which is what `Optional[str]`
    // would need if `pycc_types`' T0049 gate (which rejects `Optional[T]`
    // for every `T` outside `{int, float, bool}` before codegen) were
    // ever somehow bypassed. This pins that backstop directly with a
    // hand-built `Scalar::Optional` carrying a pointer-typed payload,
    // mirroring `truthiness_of_a_list_value_panics_honestly` above's use
    // of `truthy` as a direct call rather than compiling real MIR.
    let context = Context::create();
    let (module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    // `truthy`'s `Scalar::Optional` arm calls `build_extract_value`
    // before reaching the defensive match, so -- unlike
    // `truthiness_of_a_tuple_value_panics_honestly` above, whose arm
    // panics immediately -- the builder needs a real positioned block to
    // extract from, not just a bare `Context`/`Builder` pair.
    let fn_type = context.void_type().fn_type(&[], false);
    let f = module.add_function("f", fn_type, None);
    let block = context.append_basic_block(f, "entry");
    builder.position_at_end(block);
    let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
    let struct_ty = context.struct_type(&[ptr_ty.into(), context.i8_type().into()], false);
    let optional_with_pointer_payload = struct_ty.const_named_struct(&[
        ptr_ty.const_null().into(),
        context.i8_type().const_int(1, false).into(),
    ]);
    truthy(
        &context,
        &builder,
        &module,
        &rt,
        Scalar::Optional(optional_with_pointer_payload),
    );
}

#[test]
#[should_panic(expected = "internal error: expected an int-or-bool operand, got list")]
fn to_numeric_encoded_int_rejects_a_list_operand() {
    // Unlike `truthy`/`to_str` above, this one really is defensive:
    // `pycc_types`' `numeric_result_type` has no `as_numeric` mapping
    // for `Ty::List`, so any arithmetic with a list operand is rejected
    // as `T0021` long before codegen. Hence the "internal error"
    // wording (matching this function's own neighbouring `Float`/`Str`
    // arms) rather than the "not supported yet" feature-gap wording
    // `truthy`/`to_str` use. Exercised by calling `to_numeric_encoded_int`
    // directly, since a list operand cannot reach it through any MIR --
    // `emit_expr`'s `BinOp` arm claims a `Ty::List` result with its own
    // container-specific panic first (see
    // `a_list_result_binop_is_not_yet_supported` below), so no
    // arithmetic shape gets this far.
    let context = Context::create();
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    to_numeric_encoded_int(&context, &builder, Scalar::List(ptr));
}

#[test]
#[should_panic(expected = "internal error: expected an int-or-bool operand, got float")]
fn to_encoded_int_rejects_a_non_int_compatible_operand() {
    // `MirExpr::IntBoundary` is only produced for the valid Bool -> Int
    // subtype boundary. Keep its defensive fallback fail-closed if a
    // malformed MIR module ever supplies another scalar kind.
    let context = Context::create();
    let builder = context.create_builder();
    to_encoded_int(
        &context,
        &builder,
        Scalar::Float(context.f64_type().const_float(1.0)),
    );
}

#[test]
#[should_panic(expected = "internal error: expected a numeric operand, got list")]
fn to_float_rejects_a_list_operand() {
    // Same defensive-arm rationale as `to_numeric_encoded_int_rejects_a_list_
    // operand` directly above, for `to_float`'s own match.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    to_float(&context, &builder, &rt, Scalar::List(ptr));
}

#[test]
#[should_panic(expected = "internal error: expected an int-or-bool operand, got dict")]
fn to_numeric_encoded_int_rejects_a_dict_operand() {
    // The `dict[K, V]` counterpart of `to_numeric_encoded_int_rejects_a_list_
    // operand` above -- genuinely defensive for the identical reason:
    // `pycc_types`' `numeric_result_type` has no `as_numeric` mapping
    // for `Ty::Dict` either, so no real MIR reaches this arm.
    let context = Context::create();
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    to_numeric_encoded_int(&context, &builder, Scalar::Dict(ptr));
}

#[test]
#[should_panic(expected = "internal error: expected a numeric operand, got dict")]
fn to_float_rejects_a_dict_operand() {
    // Same defensive-arm rationale as `to_numeric_encoded_int_rejects_a_dict_
    // operand` directly above, for `to_float`'s own match.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let ptr = context
        .ptr_type(inkwell::AddressSpace::default())
        .const_null();
    to_float(&context, &builder, &rt, Scalar::Dict(ptr));
}

#[test]
fn collect_expr_bindings_skips_a_walrus_target_whose_type_is_outside_the_allow_list() {
    // `collect_expr_bindings`'s `matches!` allow-list (#774) mirrors
    // `collect_stmt_bindings`'s own `MirStmt::Assign` arm allow-list, and
    // its doc comment explains that every `ty` reaching it in practice is
    // expected to already satisfy T0050's stricter upstream restriction
    // (`pycc_types::expr::is_walrus_value_ty_supported` only ever lets a
    // walrus value be `Int`/`Float`/`Bool`/`None`, or `Optional` of one of
    // those) -- so the `false` (skip) branch is not reachable through any
    // real, `check_source`-accepted program, only defensively present "if
    // that restriction ever changes." `Ty::Protocol` is one such
    // currently-unreachable-in-practice type this function's allow-list
    // does not include; calling `collect_expr_bindings` directly with a
    // hand-built `MirExpr::NamedExpr` typed `Protocol` (bypassing
    // `pycc_types`/`pycc_hir` validation entirely, the same pattern this
    // workspace already uses for `pycc_types`'s own similarly-defensive
    // `Ty::Optional(Ty::Str)` walrus fixture) reaches this branch directly
    // and confirms the binding is skipped rather than inserted.
    let expr = MirExpr::NamedExpr {
        name: "p".to_string(),
        value: Box::new(MirExpr::Name {
            name: "p_src".to_string(),
            ty: Ty::Protocol(Box::new("Comparable".to_string())),
        }),
        ty: Ty::Protocol(Box::new("Comparable".to_string())),
    };
    let mut bindings = BTreeMap::new();
    collect_expr_bindings(&expr, &mut bindings);
    assert!(
        bindings.is_empty(),
        "a walrus target typed outside the allow-list must not be predeclared: {bindings:?}"
    );
}

// PEP 572 (#774): `pycc_mir::MirExpr::collect_named_expr_bindings` is a
// shared function statically linked into both `pycc_mir`'s own test binary
// and this crate's -- each links its own separate compiled copy, so this
// crate's test binary needs its own direct exercise of every recursive arm
// independent of `pycc_mir`'s own tests for the same method (see that
// crate's `collect_named_expr_bindings_walks_into_*` tests for the
// counterpart coverage in its own binary). These four tests mirror the
// `IntBoundary`/`ListLiteral`/`SetLiteral`/`Slice`/`Instantiate` arms this
// crate's `collect_expr_bindings` recurses through via that shared method.
#[test]
fn collect_expr_bindings_walks_into_an_int_boundary() {
    let expr = MirExpr::IntBoundary(Box::new(MirExpr::NamedExpr {
        name: "z".to_string(),
        value: Box::new(MirExpr::IntLiteral(1)),
        ty: Ty::Int,
    }));
    let mut bindings = BTreeMap::new();
    collect_expr_bindings(&expr, &mut bindings);
    assert_eq!(bindings.get("z"), Some(&Ty::Int));
}

#[test]
fn collect_expr_bindings_walks_into_a_list_and_set_literal_element() {
    let list_expr = MirExpr::ListLiteral(vec![MirExpr::NamedExpr {
        name: "l".to_string(),
        value: Box::new(MirExpr::IntLiteral(1)),
        ty: Ty::Int,
    }]);
    let mut bindings = BTreeMap::new();
    collect_expr_bindings(&list_expr, &mut bindings);
    assert_eq!(bindings.get("l"), Some(&Ty::Int));

    let set_expr = MirExpr::SetLiteral {
        elements: vec![MirExpr::NamedExpr {
            name: "s".to_string(),
            value: Box::new(MirExpr::IntLiteral(1)),
            ty: Ty::Int,
        }],
        ops: None,
    };
    let mut bindings = BTreeMap::new();
    collect_expr_bindings(&set_expr, &mut bindings);
    assert_eq!(bindings.get("s"), Some(&Ty::Int));
}

#[test]
fn collect_expr_bindings_walks_into_a_slice_bound() {
    let expr = MirExpr::Slice {
        base: Box::new(MirExpr::Name {
            name: "lst".to_string(),
            ty: Ty::List(Box::new(Ty::Int)),
        }),
        start: Some(Box::new(MirExpr::NamedExpr {
            name: "b".to_string(),
            value: Box::new(MirExpr::IntLiteral(0)),
            ty: Ty::Int,
        })),
        stop: None,
        step: None,
    };
    let mut bindings = BTreeMap::new();
    collect_expr_bindings(&expr, &mut bindings);
    assert_eq!(bindings.get("b"), Some(&Ty::Int));
}

#[test]
fn collect_expr_bindings_walks_into_an_instantiate_arg() {
    let expr = MirExpr::Instantiate(Box::new(InstantiateExpr {
        ctor: "C.__init__".to_string(),
        class_name: "C".to_string(),
        slot_names: vec!["s0".to_string()],
        args: vec![MirExpr::NamedExpr {
            name: "a".to_string(),
            value: Box::new(MirExpr::IntLiteral(1)),
            ty: Ty::Int,
        }],
        ty: Ty::Instance(Box::new("C".to_string())),
    }));
    let mut bindings = BTreeMap::new();
    collect_expr_bindings(&expr, &mut bindings);
    assert_eq!(bindings.get("a"), Some(&Ty::Int));
}

#[test]
fn collect_stmt_bindings_includes_a_list_typed_assignment_target() {
    // Task 5 (D-089) added `Ty::List(_)` to this allow-list -- Task 11
    // depends on a `list[int]` local's binding already being collected
    // here, so this is a real, deliberate inclusion to verify, not
    // just a louder panic elsewhere.
    let stmt = MirStmt::Assign {
        target: "xs".to_string(),
        value: MirExpr::Name {
            name: "xs".to_string(),
            ty: Ty::List(Box::new(Ty::Int)),
        },
    };
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(&stmt, &mut bindings);
    assert_eq!(
        bindings.get("xs"),
        Some(&Ty::List(Box::new(Ty::Int))),
        "a list[int]-typed assignment target's binding should be collected"
    );
}

#[test]
fn collect_stmt_bindings_includes_a_none_typed_assignment_target() {
    let stmt = MirStmt::Assign {
        target: "result".to_string(),
        value: MirExpr::Name {
            name: "result".to_string(),
            ty: Ty::None,
        },
    };
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(&stmt, &mut bindings);
    assert_eq!(
        bindings.get("result"),
        Some(&Ty::None),
        "a None-typed assignment target needs a real storage slot"
    );
}

#[test]
fn collect_stmt_bindings_includes_a_dict_typed_assignment_target() {
    // PR-11 Task 5 joins `Ty::Dict(_)` to this allow-list, mirroring
    // `Ty::List(_)`'s own inclusion directly above for the identical
    // reason: this task's own codegen (`declare_module_globals`/
    // `storage_slot_at_entry`) depends on a `dict[str, int]` local's
    // binding already being collected here.
    let stmt = MirStmt::Assign {
        target: "x".to_string(),
        value: MirExpr::Name {
            name: "x".to_string(),
            ty: Ty::Dict(Box::new((Ty::Str, Ty::Int))),
        },
    };
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(&stmt, &mut bindings);
    assert_eq!(
        bindings.get("x"),
        Some(&Ty::Dict(Box::new((Ty::Str, Ty::Int)))),
        "a dict[str, int]-typed assignment target's binding should be collected"
    );
}

#[test]
fn collect_stmt_bindings_includes_a_set_typed_assignment_target() {
    // PR-11 Task 9 joins `Ty::Set(_)` to this allow-list, mirroring
    // `Ty::List(_)`/`Ty::Dict(_)`'s own inclusion above for the
    // identical reason: this task's own codegen
    // (`declare_module_globals`/`storage_slot_at_entry`) depends on a
    // `set[int]` local's binding already being collected here.
    let stmt = MirStmt::Assign {
        target: "xs".to_string(),
        value: MirExpr::Name {
            name: "xs".to_string(),
            ty: Ty::Set(Box::new(Ty::Int)),
        },
    };
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(&stmt, &mut bindings);
    assert_eq!(
        bindings.get("xs"),
        Some(&Ty::Set(Box::new(Ty::Int))),
        "a set[int]-typed assignment target's binding should be collected"
    );
}

#[test]
fn collect_stmt_bindings_includes_a_tuple_typed_assignment_target() {
    // Inverts `collect_stmt_bindings_excludes_a_tuple_typed_assignment_
    // target`, which asserted the opposite: `Tuple` was excluded from
    // the allow-list while no codegen existed for it. PR-11b Task 5
    // joins `Ty::Tuple(_)` to that list, mirroring `Ty::List(_)`/
    // `Ty::Dict(_)`/`Ty::Set(_)`'s own inclusion, for the identical
    // reason -- `declare_module_globals`/`storage_slot_at_entry` can
    // only allocate a slot for a binding that was collected here, so
    // without this `x = (1, 2)` would reach codegen with no storage at
    // all.
    let stmt = MirStmt::Assign {
        target: "xs".to_string(),
        value: MirExpr::Name {
            name: "xs".to_string(),
            ty: Ty::Tuple(Box::new(vec![Ty::Int, Ty::Float])),
        },
    };
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(&stmt, &mut bindings);
    assert_eq!(
        bindings.get("xs"),
        Some(&Ty::Tuple(Box::new(vec![Ty::Int, Ty::Float]))),
        "a tuple[int, float]-typed assignment target's binding should be collected"
    );
}

#[test]
fn collect_stmt_bindings_excludes_a_dict_set_target() {
    // `d[k] = v` (PR-11 Task 4) reassigns an existing binding's
    // contents, not a name -- mirrors `pycc_types::collect_local_names`'s
    // own identical `HirStmt::DictSet` test. `dict` itself must not
    // pick up an entry from this statement.
    let stmt = MirStmt::DictSet {
        dict: "x".to_string(),
        key: MirExpr::StringLiteral("a".to_string()),
        value: MirExpr::IntLiteral(1),
    };
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(&stmt, &mut bindings);
    assert_eq!(bindings.get("x"), None);
}

#[test]
fn collect_stmt_bindings_binds_a_for_dict_loop_variable_as_str_and_recurses_into_its_body() {
    // `MirStmt::ForDict` now binds its own loop variable (PR-11 Task 5),
    // mirroring `MirStmt::ForList`'s own `Ty::Int` hardcode -- but
    // `Ty::Str`, since a dict's iterated key type is always exactly
    // `Ty::Str` (see this arm's own doc comment). It must also still
    // recurse into `body`, exactly like `ForList`/`ForRange`/`If`/
    // `While` above, so a nested, ordinary statement's binding is not
    // silently dropped.
    let stmt = MirStmt::ForDict {
        var: "k".to_string(),
        var_ty: pycc_mir::Ty::Str,
        dict: "d".to_string(),
        body: vec![MirStmt::Assign {
            target: "y".to_string(),
            value: MirExpr::IntLiteral(1),
        }],
    };
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(&stmt, &mut bindings);
    assert_eq!(bindings.get("y"), Some(&Ty::Int));
    assert_eq!(bindings.get("k"), Some(&Ty::Str));
}

#[test]
fn collect_stmt_bindings_binds_a_for_set_loop_variable_as_int_and_recurses_into_its_body() {
    // `MirStmt::ForSet` now binds its own loop variable (PR-11 Task 9),
    // mirroring `MirStmt::ForList`'s own `Ty::Int` hardcode exactly --
    // a set's iterated element type is always exactly `Ty::Int` (see
    // this arm's own doc comment), unlike `ForDict`'s `Ty::Str` key.
    // Superseded Task 8's own version of this test (`collect_stmt_
    // bindings_recurses_into_a_for_set_loop_body_without_binding_its_
    // own_var`), which pinned the pre-Task-9 deferred-decision
    // behavior. It must also still recurse into `body`, exactly like
    // `ForList`/`ForDict`/`ForRange`/`If`/`While` above, so a nested,
    // ordinary statement's binding is not silently dropped.
    let stmt = MirStmt::ForSet {
        var_ty: pycc_mir::Ty::Int,
        var: "v".to_string(),
        set: "s".to_string(),
        body: vec![MirStmt::Assign {
            target: "y".to_string(),
            value: MirExpr::IntLiteral(1),
        }],
    };
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(&stmt, &mut bindings);
    assert_eq!(bindings.get("y"), Some(&Ty::Int));
    assert_eq!(bindings.get("v"), Some(&Ty::Int));
}
