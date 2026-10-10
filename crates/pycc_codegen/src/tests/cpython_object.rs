//! CPython `object` scalars in the native lowering.

use super::*;

#[test]
#[should_panic(expected = "expected an int-or-bool operand, got object")]
fn to_numeric_encoded_int_rejects_a_cpython_object_operand() {
    let context = Context::create();
    let builder = context.create_builder();
    to_numeric_encoded_int(&context, &builder, null_object_scalar(&context));
}

#[test]
#[should_panic(expected = "expected a numeric operand, got object")]
fn to_float_rejects_a_cpython_object_operand() {
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    to_float(&context, &builder, &rt, null_object_scalar(&context));
}

#[test]
#[should_panic(expected = "string conversion of a CPython object value must go through the shim")]
fn to_str_rejects_a_cpython_object_operand() {
    // Rendering a foreign object needs a `pycc_ext_obj_*` shim call and its
    // failure edge, which this plain runtime-call helper cannot emit -- so
    // since #1340 both callers (`string_render.rs`) convert an object
    // operand before reaching `to_str`, and this arm is the backstop.
    let context = Context::create();
    let (_module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    to_str(&builder, &rt, null_object_scalar(&context));
}

/// Part 3 of #1026 (PR 3a of #1082): `truthy`'s `Scalar::Object` arm used
/// to be the defensive panic this test pinned. It is now a real capability
/// -- a `pycc_ext_obj_truthy` call plus the module-exec failure edge -- so
/// the assertion moved from a message to the emitted IR.
///
/// The direct `truthy` call is kept rather than compiling real MIR (the
/// module-level shapes are covered in `foreign_len.rs`'s own tests) because
/// it pins the arm to the function that owns it, exactly as
/// `truthiness_of_a_list_value_panics_honestly` above does. It does need a
/// positioned block inside a function spelled [`EXT_MODULE_EXEC_SYMBOL`],
/// because the arm asserts that enclosing function before appending the
/// failure edge's blocks.
#[test]
fn truthiness_of_a_cpython_object_value_calls_the_shim_helper() {
    let context = Context::create();
    let (module, rt) = list_scalar_panic_fixture(&context);
    let builder = context.create_builder();
    let entry = module.add_function(
        EXT_MODULE_EXEC_SYMBOL,
        context.i64_type().fn_type(&[], false),
        None,
    );
    builder.position_at_end(context.append_basic_block(entry, "entry"));
    truthy(
        &context,
        &builder,
        &module,
        &rt,
        null_object_scalar(&context),
    );
    let ir = {
        use inkwell::values::AnyValue;
        crate::llvm_string_to_owned(entry.print_to_string())
    };
    assert!(ir.contains(EXT_OBJ_TRUTHY_SYMBOL), "{ir}");
    assert!(ir.contains("foreign_truthy_failed"), "{ir}");
    assert!(
        ir.contains(&format!("ret i64 {EXT_MODULE_EXEC_FAILED}")),
        "{ir}"
    );
}

#[test]
fn a_private_helper_may_return_a_cpython_object_and_its_result_is_discarded() {
    // The two `Ty::Object` codegen paths that survive PR 2a's narrowing,
    // together in one program: `emit_stmt`'s `MirStmt::Return`
    // pass-through carries a `PyObject *` out of `_module`, and
    // `call_result_scalar`'s `Ty::Object` arm turns the call site's result
    // back into a `Scalar::Object`. Nothing dereferences it -- the
    // `ExprStmt` discards it.
    //
    // The returned value is the foreign module global itself rather than a
    // `numpy.pi` load so the test selects exactly these two arms; an
    // `ObjAttrGet` inside a function body has its own failure-edge tests in
    // `foreign_fail_tests.rs` (#1316). `pycc_types` admits returning a
    // CPython object from a function since Part 1 of #1333, so this is
    // also a shape the front end produces.
    //
    // A public function returning `object` -- spellable since Part 1 of
    // #1367 through a class a foreign import binds -- is exported since
    // #1397 (unchecked against the class, #1386), but its thunk is a
    // separate seam this test does not select.
    compile_ext_items(
        "object_returning_helper",
        with_foreign_numpy(vec![
            MirItem::Function {
                name: "_module".to_string(),
                params: vec![],
                return_ty: Ty::Object,
                body: vec![MirStmt::Return(Some(MirExpr::Name {
                    name: "numpy".to_string(),
                    ty: Ty::Object,
                }))],
            },
            MirItem::TopLevelStmt(MirStmt::ExprStmt(MirExpr::Call {
                callee: "_module".to_string(),
                args: vec![],
                ty: Ty::Object,
            })),
        ]),
    );
}

// -- PR 3c of #1082: the `ForObject` storage-slot type ----------------------

/// `collect_stmt_bindings` produces exactly one storage-slot type per name,
/// so a `for x in <object>:` target must claim an `Object` (pointer) slot
/// even when an earlier statement in the same body already bound the name to
/// a narrower scalar. An `or_insert` here would keep the `Int` and leave the
/// loop storing a `PyObject *` into an `i64` slot -- a miscompilation the
/// `local type drifted` `debug_assert` cannot catch in a release build.
/// `pycc_types` refuses that source shape with `T0023`, so this unit test is
/// the only place the choice is observable.
#[test]
fn a_for_object_target_claims_an_object_slot_over_an_earlier_int_binding() {
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(
        &MirStmt::Assign {
            target: "x".to_string(),
            value: MirExpr::IntLiteral(5),
        },
        &mut bindings,
    );
    assert_eq!(bindings.get("x"), Some(&Ty::Int));
    collect_stmt_bindings(
        &MirStmt::ForObject {
            var: "x".to_string(),
            iter: numpy_pi(),
            body: Vec::new(),
        },
        &mut bindings,
    );
    assert_eq!(bindings.get("x"), Some(&Ty::Object));
}
