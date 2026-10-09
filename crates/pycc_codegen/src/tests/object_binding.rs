//! #1325: binding a CPython object value to a name.
//!
//! `emit_assign`'s `Scalar::Object` arm stores the `PyObject *` into the
//! name's pointer slot with no refcount traffic in either direction. Since
//! Part 1 of #1499 that pass-through is the frame-slot rule only: a
//! module-global object store goes through `object_slot::assign` first,
//! which retains a borrowed value and releases the rebound one
//! (`object_slot_tests.rs` pins that). The fixtures below call
//! `emit_assign` directly with an empty `rt.object_slots` table, so they
//! pin the pass-through every non-module-global slot still takes: the store
//! happens and no release is emitted (#1092's leak-only rule, until Part 2,
//! #1502). The last test compiles the whole module-level shape through real
//! MIR.

use super::*;

/// Declares `x` as a module-level `object` global, emits `assignments`
/// assignments of a null `PyObject *` to it inside a fresh function, and
/// returns that function's IR.
fn assign_object_global_ir(assignments: usize) -> String {
    let context = Context::create();
    let (module, rt) = list_scalar_panic_fixture(&context);
    let bindings = BTreeMap::from([("x".to_string(), Ty::Object)]);
    let globals = declare_module_globals(&context, &module, &bindings);
    let slot = globals.get("x").expect("the object binding gets a slot");
    assert_eq!(slot.ty, Ty::Object);
    assert!(
        slot.initialized.is_some(),
        "an object global gets the initialized flag every module global gets"
    );
    let locals: HashMap<String, StorageSlot> = globals.into_iter().collect();
    let builder = context.create_builder();
    let function = module.add_function(
        "assign_object",
        context.void_type().fn_type(&[], false),
        None,
    );
    builder.position_at_end(context.append_basic_block(function, "entry"));
    for _ in 0..assignments {
        emit_assign(
            &context,
            &builder,
            &rt,
            &locals,
            "x",
            null_object_scalar(&context),
        );
    }
    builder
        .build_return(None)
        .expect("build_return should not fail for a positioned block");
    use inkwell::values::AnyValue;
    crate::llvm_string_to_owned(function.print_to_string())
}

#[test]
fn assigning_a_cpython_object_stores_the_pointer_and_sets_the_flag() {
    let ir = assign_object_global_ir(1);
    assert!(ir.contains("store ptr null, ptr @pyglobal_x"), "{ir}");
    assert!(ir.contains("store i8 1, ptr @pyglobal_init_x"), "{ir}");
    assert!(!ir.contains("call "), "no refcount call of any kind: {ir}");
}

#[test]
fn emit_assign_rebinding_a_cpython_object_emits_no_release() {
    let ir = assign_object_global_ir(2);
    assert_eq!(
        ir.matches("store ptr null, ptr @pyglobal_x").count(),
        2,
        "{ir}"
    );
    assert!(!ir.contains("DecRef"), "a rebinding must not release: {ir}");
    assert!(!ir.contains("call "), "no refcount call of any kind: {ir}");
}

/// The whole module-level shape through real MIR: `x = numpy.pi`, a
/// rebinding `x = numpy`, an alias `y = x`, and a `ForObject` loop over the
/// bare name `x`. `compile_to_object_with_options` runs `module.verify()`,
/// so a slot of the wrong type or a store to an undeclared global fails
/// here.
#[test]
fn a_module_level_object_binding_rebinding_alias_and_loop_compile() {
    let name = |n: &str| MirExpr::Name {
        name: n.to_string(),
        ty: Ty::Object,
    };
    compile_ext_items(
        "object_binding_module",
        with_foreign_numpy(vec![
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: numpy_pi(),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "x".to_string(),
                value: name("numpy"),
            }),
            MirItem::TopLevelStmt(MirStmt::Assign {
                target: "y".to_string(),
                value: name("x"),
            }),
            MirItem::TopLevelStmt(MirStmt::ForObject {
                var: "t".to_string(),
                iter: name("y"),
                body: vec![],
            }),
        ]),
    );
}

/// `collect_stmt_bindings`' `Assign` allow-list admits a `Ty::Object` value,
/// so the name gets a pointer slot; without the entry `emit_assign` would
/// panic on the absent slot.
#[test]
fn an_object_valued_assignment_claims_an_object_slot() {
    let mut bindings = BTreeMap::new();
    collect_stmt_bindings(
        &MirStmt::Assign {
            target: "x".to_string(),
            value: numpy_pi(),
        },
        &mut bindings,
    );
    assert_eq!(bindings.get("x"), Some(&Ty::Object));
}
