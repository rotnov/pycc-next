//! Inline reference counting of CPython objects (v0.8).
//!
//! Generated code counts an `object` value through the C shim's
//! `pycc_ext_obj_retain`/`pycc_ext_obj_release` (`Py_XINCREF`/
//! `Py_XDECREF`). Under the shim's `Py_LIMITED_API 0x030D0000` those are
//! calls into libpython, so every count was two calls. A `--release` build
//! defines both in the generated module instead, as internal functions
//! holding CPython 3.13's own inline forms for a 64-bit GIL build,
//! the same ones the shim's `src/ext/module/bridge.c` uses (its comment says
//! why the stable ABI fixes them):
//!
//! * retain: `NULL` is skipped; the low 32 bits of `ob_refcnt` (offset 0)
//!   are incremented unless that wraps them to zero (an immortal object);
//! * release: `NULL` and an object whose low 32 bits are negative (immortal)
//!   are skipped; `ob_refcnt` is decremented and `_Py_Dealloc` (a stable-ABI
//!   function) called at zero.
//!
//! The retain is always inlined (four instructions); the release, whose
//! branches and `_Py_Dealloc` call are most of an inlined copy, stays one
//! call into the module's own copy: inlining it everywhere doubled LLVM's
//! time on mistune's modules for 0.7% fewer instructions at run time.
//!
//! The shim keeps its own external definitions for its C callers; the
//! module's are internal, so the two never clash at link time. Only a
//! 64-bit little-endian target gets them (every Tier-1 target); any other
//! keeps the calls.

use inkwell::module::{Linkage, Module};
use inkwell::targets::TargetData;
use inkwell::values::FunctionValue;
use inkwell::{AddressSpace, IntPredicate};

/// Defines the module's `pycc_ext_obj_retain`/`pycc_ext_obj_release`
/// declarations inline (module doc). Returns how many it defined.
pub(crate) fn inline_object_rc(module: &Module<'_>, target: &TargetData) -> usize {
    if target.get_pointer_byte_size(None) != 8
        || target.get_byte_ordering() != inkwell::targets::ByteOrdering::LittleEndian
    {
        return 0;
    }
    let mut defined = 0;
    if let Some(retain) = declared(module, crate::ext::EXT_OBJ_RETAIN_SYMBOL) {
        define_retain(module, retain);
        defined += 1;
    }
    if let Some(release) = declared(module, crate::ext::EXT_OBJ_RELEASE_SYMBOL) {
        define_release(module, release);
        defined += 1;
    }
    defined
}

/// `name` when the module declares it without a body.
fn declared<'ctx>(module: &Module<'ctx>, name: &str) -> Option<FunctionValue<'ctx>> {
    module
        .get_function(name)
        .filter(|function| function.count_basic_blocks() == 0 && function.count_params() == 1)
}

/// Makes `function` an internal definition, always inlined or never.
fn internal(module: &Module<'_>, function: FunctionValue<'_>, inline: bool) {
    let context = module.get_context();
    function.set_linkage(Linkage::Internal);
    let kind = inkwell::attributes::Attribute::get_named_enum_kind_id(if inline {
        "alwaysinline"
    } else {
        "noinline"
    });
    function.add_attribute(
        inkwell::attributes::AttributeLoc::Function,
        context.create_enum_attribute(kind, 0),
    );
}

fn define_retain(module: &Module<'_>, function: FunctionValue<'_>) {
    internal(module, function, true);
    let context = module.get_context();
    let builder = context.create_builder();
    let i32_type = context.i32_type();
    let object = function.get_nth_param(0).expect("one parameter").into_pointer_value();
    let entry = context.append_basic_block(function, "entry");
    let count = context.append_basic_block(function, "count");
    let store = context.append_basic_block(function, "store");
    let done = context.append_basic_block(function, "done");
    builder.position_at_end(entry);
    let null = builder.build_is_null(object, "null").expect("is_null");
    builder.build_conditional_branch(null, done, count).expect("branch");
    builder.position_at_end(count);
    let low = builder
        .build_load(i32_type, object, "low")
        .expect("load")
        .into_int_value();
    let next = builder
        .build_int_add(low, i32_type.const_int(1, false), "next")
        .expect("add");
    let immortal = builder
        .build_int_compare(IntPredicate::EQ, next, i32_type.const_zero(), "immortal")
        .expect("compare");
    builder.build_conditional_branch(immortal, done, store).expect("branch");
    builder.position_at_end(store);
    builder.build_store(object, next).expect("store");
    builder.build_unconditional_branch(done).expect("branch");
    builder.position_at_end(done);
    builder.build_return(None).expect("return");
}

fn define_release(module: &Module<'_>, function: FunctionValue<'_>) {
    internal(module, function, false);
    let context = module.get_context();
    let builder = context.create_builder();
    let i32_type = context.i32_type();
    let i64_type = context.i64_type();
    let ptr_type = context.ptr_type(AddressSpace::default());
    let dealloc = module.get_function("_Py_Dealloc").unwrap_or_else(|| {
        module.add_function(
            "_Py_Dealloc",
            context.void_type().fn_type(&[ptr_type.into()], false),
            Some(Linkage::External),
        )
    });
    let object = function.get_nth_param(0).expect("one parameter").into_pointer_value();
    let entry = context.append_basic_block(function, "entry");
    let check = context.append_basic_block(function, "check");
    let count = context.append_basic_block(function, "count");
    let free = context.append_basic_block(function, "free");
    let done = context.append_basic_block(function, "done");
    builder.position_at_end(entry);
    let null = builder.build_is_null(object, "null").expect("is_null");
    builder.build_conditional_branch(null, done, check).expect("branch");
    builder.position_at_end(check);
    let refcnt = builder
        .build_load(i64_type, object, "refcnt")
        .expect("load")
        .into_int_value();
    let low = builder.build_int_truncate(refcnt, i32_type, "low").expect("trunc");
    let immortal = builder
        .build_int_compare(IntPredicate::SLT, low, i32_type.const_zero(), "immortal")
        .expect("compare");
    builder.build_conditional_branch(immortal, done, count).expect("branch");
    builder.position_at_end(count);
    let next = builder
        .build_int_sub(refcnt, i64_type.const_int(1, false), "next")
        .expect("sub");
    builder.build_store(object, next).expect("store");
    let zero = builder
        .build_int_compare(IntPredicate::EQ, next, i64_type.const_zero(), "zero")
        .expect("compare");
    builder.build_conditional_branch(zero, free, done).expect("branch");
    builder.position_at_end(free);
    builder.build_call(dealloc, &[object.into()], "").expect("call");
    builder.build_unconditional_branch(done).expect("branch");
    builder.position_at_end(done);
    builder.build_return(None).expect("return");
}

#[cfg(test)]
mod tests {
    use super::*;
    use inkwell::context::Context;

    fn target() -> inkwell::targets::TargetMachine {
        crate::target_machine::target_machine_for(Some("x86_64-unknown-linux-gnu"), true)
            .expect("x86_64 target")
    }

    #[test]
    fn defines_both_shim_counts_inline_on_a_64_bit_little_endian_target() {
        let context = Context::create();
        let module = context.create_module("m");
        let ptr = context.ptr_type(AddressSpace::default());
        let fn_type = context.void_type().fn_type(&[ptr.into()], false);
        module.add_function(crate::ext::EXT_OBJ_RETAIN_SYMBOL, fn_type, None);
        module.add_function(crate::ext::EXT_OBJ_RELEASE_SYMBOL, fn_type, None);
        let machine = target();
        assert_eq!(inline_object_rc(&module, &machine.get_target_data()), 2);
        crate::verify_module(&module);
        let ir = crate::llvm_string_to_owned(module.print_to_string());
        assert!(ir.contains("define internal void @pycc_ext_obj_retain(ptr"), "{ir}");
        assert!(ir.contains("define internal void @pycc_ext_obj_release(ptr"), "{ir}");
        assert!(ir.contains("load i32, ptr"), "{ir}");
        assert!(ir.contains("icmp slt i32"), "{ir}");
        assert!(ir.contains("call void @_Py_Dealloc(ptr"), "{ir}");
        assert!(ir.contains("alwaysinline"), "{ir}");
        assert!(ir.contains("noinline"), "{ir}");
    }

    #[test]
    fn leaves_a_module_without_the_declarations_and_other_targets_alone() {
        let context = Context::create();
        let module = context.create_module("m");
        assert_eq!(inline_object_rc(&module, &target().get_target_data()), 0);
        let ptr = context.ptr_type(AddressSpace::default());
        let fn_type = context.void_type().fn_type(&[ptr.into()], false);
        module.add_function(crate::ext::EXT_OBJ_RETAIN_SYMBOL, fn_type, None);
        // Data layouts alone (no backend needed): a 64-bit big-endian one
        // and a 32-bit little-endian one.
        for layout in ["E-m:e-i64:64-n32:64", "e-m:e-p:32:32-i64:64-n32"] {
            let data = TargetData::create(layout);
            assert_eq!(inline_object_rc(&module, &data), 0, "{layout}");
        }
        let ir = crate::llvm_string_to_owned(module.print_to_string());
        assert!(ir.contains("declare void @pycc_ext_obj_retain(ptr"), "{ir}");
    }
}
