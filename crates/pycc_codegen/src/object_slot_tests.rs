//! IR tests for `object_slot.rs` (Part 1 of #1499): a module-global
//! `object` slot owns its reference.
//!
//! Every test compiles hand-built MIR as an `ext` object -- LLVM's verifier
//! runs before any assertion here is believed -- and reads the text of the
//! module-exec entry, a user function, or the whole module. A rebind is the
//! `Py_XSETREF` sequence `store_owned` emits: load the old value and the
//! owned bit, store the new value, raise the flag and the bit, and release
//! the old value on a `global_release_old` branch taken only when the
//! loaded bit was set and the shim's only-live-activation gate is open.

use super::*;
use crate::{CompileOptions, EXT_MODULE_EXEC_SYMBOL, compile_to_object_with_observer};
use inkwell::values::AnyValue;
use pycc_mir::{FromImport, MirItem, MirModule, MirStmt, Ty};

const RETAIN: &str = "call void @pycc_ext_obj_retain(";
const RELEASE: &str = "call void @pycc_ext_obj_release(";

fn import_copy() -> MirItem {
    MirItem::ForeignImport {
        local_name: "copy".to_string(),
        module_path: "copy".to_string(),
        from: None,
    }
}

fn name(name: &str) -> MirExpr {
    MirExpr::Name {
        name: name.to_string(),
        ty: Ty::Object,
    }
}

/// `copy.<attr>`: a produced new reference.
fn attr(attr: &str) -> MirExpr {
    MirExpr::ObjAttrGet {
        base: Box::new(name("copy")),
        attr: attr.to_string(),
        ty: Ty::Object,
    }
}

fn bind(target: &str, value: MirExpr) -> MirStmt {
    MirStmt::Assign {
        target: target.to_string(),
        value,
    }
}

fn top(stmts: Vec<MirStmt>) -> Vec<MirItem> {
    stmts.into_iter().map(MirItem::TopLevelStmt).collect()
}

/// The whole module's LLVM text and that of each named function, after
/// compiling `items` as an `ext` object.
fn compile(label: &str, items: Vec<MirItem>, names: &[&str]) -> (String, Vec<String>) {
    let dir = pycc_scratch::ScratchDir::new(label).expect("failed to create scratch dir");
    let mut whole = String::new();
    let mut irs = vec![String::new(); names.len()];
    let mut observer = |module: &inkwell::module::Module<'_>, _: Option<&'static str>| {
        whole = crate::llvm_string_to_owned(module.print_to_string());
        for (slot, name) in irs.iter_mut().zip(names) {
            if let Some(function) = module.get_function(name) {
                *slot = crate::llvm_string_to_owned(function.print_to_string());
            }
        }
    };
    compile_to_object_with_observer(
        &MirModule {
            items,
            ..Default::default()
        },
        &dir.join(format!("{label}.o")),
        &CompileOptions {
            ext: true,
            ..CompileOptions::default()
        },
        Some(&mut observer),
    )
    .expect("ext codegen should succeed");
    for (ir, name) in irs.iter().zip(names) {
        assert!(!ir.is_empty(), "no {name} was emitted");
    }
    (whole, irs)
}

/// The module-exec entry's IR after `import copy` and `stmts`.
fn entry_ir(label: &str, stmts: Vec<MirStmt>) -> String {
    let mut items = vec![import_copy()];
    items.extend(top(stmts));
    compile(label, items, &[EXT_MODULE_EXEC_SYMBOL]).1.remove(0)
}

/// The labelled blocks whose label is `prefix` plus only digits, each up to
/// the blank line that ends it.
fn blocks<'a>(ir: &'a str, prefix: &str) -> Vec<&'a str> {
    ir.split("\n\n")
        .filter(|block| {
            let block = block.trim_start_matches('\n');
            block
                .split_once(':')
                .and_then(|(label, _)| label.strip_prefix(prefix))
                .is_some_and(|suffix| suffix.chars().all(|c| c.is_ascii_digit()))
        })
        .collect()
}

/// The SSA name `%name` an instruction line starting with `%name =` and
/// containing `needle` defines.
fn defined_by<'a>(ir: &'a str, needle: &str) -> Vec<&'a str> {
    ir.lines()
        .filter(|line| line.contains(needle))
        .filter_map(|line| line.trim().split_once(" = ").map(|(name, _)| name))
        .collect()
}

fn at(ir: &str, needle: &str) -> usize {
    ir.find(needle)
        .unwrap_or_else(|| panic!("`{needle}` not found in:\n{ir}"))
}

/// A rebind of a module global loads the old value, stores the new one and
/// raises the bit, then releases the old value -- and only on the branch
/// that tests the loaded bit. The store comes first, so a finalizer the
/// release runs reads the new value.
#[test]
fn a_rebind_stores_first_and_releases_the_old_value_behind_the_owned_bit() {
    let ir = entry_ir(
        "slot_rebind",
        vec![bind("x", attr("a")), bind("x", attr("b"))],
    );
    let olds = defined_by(&ir, "= load ptr, ptr @pyglobal_x");
    assert_eq!(olds.len(), 2, "{ir}");
    let bits = defined_by(&ir, "= load i8, ptr @pyglobal.owned.x");
    assert_eq!(bits.len(), 2, "{ir}");
    let rebinds = blocks(&ir, "global_release_old");
    // `copy`'s import and the two stores into `x`.
    assert_eq!(rebinds.len(), 3, "{ir}");
    for (old, bit) in olds.iter().zip(&bits) {
        let load = at(&ir, &format!("{old} = load ptr, ptr @pyglobal_x"));
        let store = load + at(&ir[load..], ", ptr @pyglobal_x,");
        let raise = store + at(&ir[store..], "store i8 1, ptr @pyglobal.owned.x");
        let test = raise + at(&ir[raise..], &format!("icmp ne i8 {bit}, 0"));
        let release = test + at(&ir[test..], &format!("{RELEASE}ptr {old})"));
        assert!(load < store && store < raise && raise < test && test < release);
        assert_eq!(
            rebinds
                .iter()
                .filter(|block| block.contains(&format!("{RELEASE}ptr {old})")))
                .count(),
            1,
            "{old} is released on its rebind branch only: {ir}"
        );
    }
    assert_eq!(ir.matches(RELEASE).count(), rebinds.len(), "{ir}");
    assert_eq!(ir.matches(RETAIN).count(), 0, "{ir}");
}

/// The release branch is gated by the shim's activation count as well as
/// the owned bit (#1501): every rebind asks
/// `pycc_ext_obj_rebind_may_release` after its store and branches to
/// `global_release_old` only on the conjunction, so a rebind while another
/// compiled activation of the artifact is live leaks the old value.
#[test]
fn the_release_branch_is_gated_by_the_only_live_activation_predicate() {
    let ir = entry_ir(
        "slot_rebind_gate",
        vec![bind("x", attr("a")), bind("x", attr("b"))],
    );
    let rebinds = blocks(&ir, "global_release_old");
    let gates = defined_by(&ir, "call i32 @pycc_ext_obj_rebind_may_release()");
    assert_eq!(gates.len(), rebinds.len(), "{ir}");
    let owned = defined_by(&ir, "= icmp ne i8 %global_owned");
    assert_eq!(owned.len(), rebinds.len(), "{ir}");
    for (gate, owned) in gates.iter().zip(&owned) {
        let call = at(
            &ir,
            &format!("{gate} = call i32 @pycc_ext_obj_rebind_may_release()"),
        );
        let alone = defined_by(&ir[call..], &format!("= icmp ne i32 {gate}, 0"))[0];
        let both = defined_by(&ir[call..], &format!("= and i1 {owned}, {alone}"))[0];
        let branch = call
            + at(
                &ir[call..],
                &format!("br i1 {both}, label %global_release_old"),
            );
        assert!(call < branch, "{ir}");
    }
    // The last rebind's store into `x` precedes its gate call.
    let last_store = ir.rfind(", ptr @pyglobal_x,").expect("x is stored");
    let last_gate = ir
        .rfind("call i32 @pycc_ext_obj_rebind_may_release()")
        .expect("the gate is called");
    assert!(last_store < last_gate, "{ir}");
}

/// `y = x` aliases a borrowed global: the store takes its own reference
/// first, so a later rebind of either name cannot free the other's value.
#[test]
fn an_alias_retains_once() {
    let ir = entry_ir(
        "slot_alias",
        vec![bind("x", attr("a")), bind("y", name("x"))],
    );
    assert_eq!(ir.matches(RETAIN).count(), 1, "{ir}");
    let retain = at(&ir, RETAIN);
    let store = retain + at(&ir[retain..], ", ptr @pyglobal_y,");
    assert!(retain < store, "{ir}");
}

/// A produced value is a new reference that moves into the slot as it is.
#[test]
fn a_produced_value_is_not_retained() {
    let ir = entry_ir("slot_produced", vec![bind("x", attr("a"))]);
    assert_eq!(ir.matches(RETAIN).count(), 0, "{ir}");
    assert!(!ir.contains("declare void @pycc_ext_obj_retain"), "{ir}");
}

/// A boxed `None` is CPython's borrowed `Py_None`, so it is retained; a
/// boxed `int` is the packer's new reference, so it is not.
#[test]
fn a_boxed_none_is_retained_and_a_boxed_int_is_not() {
    let ir = entry_ir(
        "slot_boxed",
        vec![
            bind("x", MirExpr::ObjectBox(Box::new(MirExpr::NoneLiteral))),
            bind("y", MirExpr::ObjectBox(Box::new(MirExpr::IntLiteral(7)))),
        ],
    );
    assert_eq!(ir.matches(RETAIN).count(), 1, "{ir}");
    let retain = at(&ir, RETAIN);
    let store_x = at(&ir, ", ptr @pyglobal_x,");
    let store_y = at(&ir, ", ptr @pyglobal_y,");
    assert!(retain < store_x && store_x < store_y, "{ir}");
}

/// A module-level `for` target owns each item: every trip releases the
/// previous one through the same rebind branch.
#[test]
fn a_module_for_target_releases_the_previous_item() {
    let ir = entry_ir(
        "slot_for",
        vec![MirStmt::ForObject {
            var: "x".to_string(),
            iter: attr("a"),
            body: Vec::new(),
        }],
    );
    let olds = defined_by(&ir, "= load ptr, ptr @pyglobal_x");
    assert_eq!(olds.len(), 1, "{ir}");
    let next = at(&ir, "@pycc_ext_obj_iter_next(");
    let load = at(&ir, &format!("{} = load ptr, ptr @pyglobal_x", olds[0]));
    assert!(next < load, "the item is stored after `next()`: {ir}");
    let released = blocks(&ir, "global_release_old")
        .iter()
        .filter(|block| block.contains(&format!("{RELEASE}ptr {})", olds[0])))
        .count();
    assert_eq!(released, 1, "{ir}");
    assert_eq!(ir.matches(RETAIN).count(), 0, "{ir}");
}

/// A foreign import nested in a module-level `if` (#1383) binds through the
/// same owned store as a top-level one: importing a name twice releases
/// the first value.
#[test]
fn a_nested_from_import_owns_its_binding() {
    let from = || {
        Some(FromImport {
            name: "deepcopy".to_string(),
            fromlist: vec!["deepcopy".to_string()],
            index: 0,
            level: 0,
        })
    };
    let nested = MirStmt::ForeignImport {
        bindings: vec![
            ("d".to_string(), "copy".to_string(), from()),
            ("d".to_string(), "copy".to_string(), from()),
        ],
    };
    let ir = entry_ir(
        "slot_nested_import",
        vec![MirStmt::If {
            test: MirExpr::BoolLiteral(true),
            body: vec![nested],
            orelse: Vec::new(),
        }],
    );
    let olds = defined_by(&ir, "= load ptr, ptr @pyglobal_d");
    assert_eq!(olds.len(), 2, "{ir}");
    for old in &olds {
        let released = blocks(&ir, "global_release_old")
            .iter()
            .filter(|block| block.contains(&format!("{RELEASE}ptr {old})")))
            .count();
        assert_eq!(released, 1, "{ir}");
    }
    let imported = at(&ir, "@pycc_ext_obj_import_from(");
    let load = at(&ir, &format!("{} = load ptr, ptr @pyglobal_d", olds[0]));
    assert!(imported < load, "{ir}");
}

/// A function's own `x` shadows the module global `x` with a frame slot,
/// which keeps the leak-only, borrowed-pointer rule until Part 2: neither a
/// produced nor a borrowed store retains or releases anything.
#[test]
fn a_function_local_of_a_global_name_keeps_the_frame_rule() {
    let f = MirItem::Function {
        name: "f".to_string(),
        params: Vec::new(),
        return_ty: Ty::None,
        body: vec![bind("x", attr("a")), MirStmt::Return(None)],
    };
    let g = MirItem::Function {
        name: "g".to_string(),
        params: vec![("y".to_string(), Ty::Object)],
        return_ty: Ty::None,
        body: vec![bind("x", name("y")), MirStmt::Return(None)],
    };
    let mut items = vec![import_copy(), f, g];
    items.extend(top(vec![bind("x", attr("a"))]));
    let (_, irs) = compile("slot_shadow", items, &["pyfn_f", "pyfn_g"]);
    for ir in &irs {
        assert_eq!(ir.matches(RETAIN).count(), 0, "{ir}");
        assert_eq!(ir.matches(RELEASE).count(), 0, "{ir}");
        assert!(!ir.contains("@pyglobal_x"), "{ir}");
        assert!(!ir.contains("global_release_old"), "{ir}");
    }
}

/// A compiled-instance attribute store of a borrowed object retains it,
/// because the global it was read from may now release it; a produced
/// value is stored as it is. Neither releases the replaced word (Part 4).
#[test]
fn an_attribute_store_retains_a_borrowed_object_and_releases_nothing() {
    let instance = Ty::Instance(Box::new("C".to_string()));
    let set = |value: MirExpr| MirStmt::AttrSet {
        base: MirExpr::Name {
            name: "o".to_string(),
            ty: instance.clone(),
        },
        slot: 0,
        value,
    };
    let f = MirItem::Function {
        name: "f".to_string(),
        params: vec![
            ("o".to_string(), instance.clone()),
            ("v".to_string(), Ty::Object),
        ],
        return_ty: Ty::None,
        body: vec![set(name("v")), set(attr("a")), MirStmt::Return(None)],
    };
    let (_, irs) = compile("slot_attr_set", vec![import_copy(), f], &["pyfn_f"]);
    let ir = &irs[0];
    assert_eq!(ir.matches(RETAIN).count(), 1, "{ir}");
    assert_eq!(ir.matches(RELEASE).count(), 0, "{ir}");
    let retain = at(ir, RETAIN);
    let first_set = at(ir, "@pycc_rt_instance_set_slot(");
    assert!(retain < first_set, "{ir}");
}

/// A module that stores no borrowed object declares no retain helper.
#[test]
fn a_module_without_a_borrowed_object_store_declares_no_retain() {
    let (whole, _) = compile(
        "slot_no_retain",
        vec![import_copy()],
        &[EXT_MODULE_EXEC_SYMBOL],
    );
    assert!(!whole.contains("@pycc_ext_obj_retain"), "{whole}");
}

/// The exec-start clear is a raw store into each owned bit, in the entry
/// block, before the first statement: it touches neither the value nor its
/// `initialized` flag and releases nothing, so a value an earlier exec
/// stored is left as it was.
#[test]
fn the_exec_start_clear_touches_only_the_owned_bits() {
    let ir = entry_ir("slot_clear", vec![bind("x", attr("a"))]);
    let entry = ir
        .split("\n\n")
        .find(|block| block.contains("entry:"))
        .expect("the module-exec entry has an entry block");
    assert!(
        entry.contains("store i8 0, ptr @pyglobal.owned.copy"),
        "{ir}"
    );
    assert!(entry.contains("store i8 0, ptr @pyglobal.owned.x"), "{ir}");
    assert!(!entry.contains(RELEASE), "{ir}");
    assert!(!entry.contains("ptr @pyglobal_x,"), "{ir}");
    assert!(!entry.contains("@pyglobal_init_x"), "{ir}");
    let clear = at(&ir, "store i8 0, ptr @pyglobal.owned.x");
    let import = at(&ir, "@pycc_ext_obj_import(");
    assert!(clear < import, "{ir}");
}

/// A module with no `object` global declares no owned bit at all, so a
/// native module's IR is unchanged.
#[test]
fn a_module_without_an_object_global_has_no_owned_bit() {
    let (whole, _) = compile(
        "slot_no_object",
        top(vec![bind("n", MirExpr::IntLiteral(1))]),
        &[EXT_MODULE_EXEC_SYMBOL],
    );
    assert!(!whole.contains("pyglobal.owned"), "{whole}");
}

/// Each `object` global's owned bit is an internal `i8` that starts clear,
/// named with a dot no Python identifier contains.
#[test]
fn an_owned_bit_is_an_internal_zero_initialised_byte() {
    let (whole, _) = compile(
        "slot_bit_decl",
        vec![import_copy()],
        &[EXT_MODULE_EXEC_SYMBOL],
    );
    assert!(
        whole.contains("@pyglobal.owned.copy = internal global i8 0"),
        "{whole}"
    );
}

/// A store outside the module-exec entry is a misclassified frame slot and
/// must fail the build rather than release a borrowed reference.
#[test]
#[should_panic]
fn an_owned_store_outside_the_module_exec_entry_panics() {
    let context = Context::create();
    let module = context.create_module("slot_outside");
    let builder = context.create_builder();
    let function = module.add_function(
        "not_the_entry",
        context.void_type().fn_type(&[], false),
        None,
    );
    builder.position_at_end(context.append_basic_block(function, "entry"));
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let slot = StorageSlot {
        ptr: module.add_global(ptr, None, "value").as_pointer_value(),
        ty: Ty::Object,
        initialized: None,
    };
    let bit = module
        .add_global(context.i8_type(), None, "bit")
        .as_pointer_value();
    store_owned(&context, &builder, &module, &slot, bit, ptr.const_null());
}

/// `store_new_reference` into a slot with no owned bit is the frame rule's
/// plain store, raising the slot's `initialized` flag when it has one.
#[test]
fn a_new_reference_into_a_frame_slot_is_a_plain_store() {
    let context = Context::create();
    let module = context.create_module("slot_frame");
    let builder = context.create_builder();
    let rt = declare_rt_functions(&context, &module);
    let function = module.add_function("frame", context.void_type().fn_type(&[], false), None);
    builder.position_at_end(context.append_basic_block(function, "entry"));
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let flag = module
        .add_global(context.i8_type(), None, "flag")
        .as_pointer_value();
    for initialized in [None, Some(flag)] {
        let slot = StorageSlot {
            ptr: module.add_global(ptr, None, "value").as_pointer_value(),
            ty: Ty::Object,
            initialized,
        };
        assert!(owned_bit(&rt, &slot).is_none());
        store_new_reference(&context, &builder, &module, &rt, &slot, ptr.const_null());
    }
    builder
        .build_return(None)
        .expect("build_return should not fail");
    let ir = crate::llvm_string_to_owned(function.print_to_string());
    assert_eq!(ir.matches("store ptr null").count(), 2, "{ir}");
    assert_eq!(ir.matches("store i8 1, ptr @flag").count(), 1, "{ir}");
    assert!(!ir.contains(RELEASE), "{ir}");
}

/// `assign` leaves every store that is not an `object` value into a
/// module-global `object` slot to `emit_assign`: an unknown target, a
/// non-object value, and an `object` slot without an owned bit.
#[test]
fn assign_declines_everything_but_an_owned_object_slot() {
    let context = Context::create();
    let module = context.create_module("slot_decline");
    let builder = context.create_builder();
    let rt = declare_rt_functions(&context, &module);
    let function = module.add_function("frame", context.void_type().fn_type(&[], false), None);
    builder.position_at_end(context.append_basic_block(function, "entry"));
    let ptr = context.ptr_type(inkwell::AddressSpace::default());
    let mut locals = HashMap::new();
    locals.insert(
        "o".to_string(),
        StorageSlot {
            ptr: module.add_global(ptr, None, "o").as_pointer_value(),
            ty: Ty::Object,
            initialized: None,
        },
    );
    let object = Scalar::Object(ptr.const_null());
    let int = Scalar::Int(context.i64_type().const_zero());
    let value = name("o");
    for (target, scalar) in [("missing", object), ("o", int), ("o", object)] {
        assert!(!assign(
            &context, &builder, &module, &rt, &locals, target, &value, scalar
        ));
    }
    builder
        .build_return(None)
        .expect("build_return should not fail");
    let ir = crate::llvm_string_to_owned(function.print_to_string());
    assert!(!ir.contains("store"), "{ir}");
}
