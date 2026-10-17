//! A store never writes past the end of the stack slot or global it
//! targets.
//!
//! LLVM's verifier accepts `store { i8, i8 } %v, ptr %slot` into an
//! `alloca i8`: an opaque pointer carries no pointee type. Such a store
//! writes beyond the slot. A `--release` build usually deletes it (the
//! stored value is dead or folded), so the defect shows only in a debug
//! build, and there only when the clobbered neighbour -- another slot, a
//! saved register, the return address -- happens to matter for the frame
//! layout at hand. This check makes every such store a deterministic
//! internal error in both profiles, before any pass runs.

use inkwell::llvm_sys::core as llc;
use inkwell::llvm_sys::target as llt;
use inkwell::module::Module;
use inkwell::targets::TargetData;
use inkwell::values::{AsValueRef, InstructionOpcode};

/// Panics naming the first store in `module` that writes more bytes than
/// the alloca or global variable it stores to holds.
pub(crate) fn check(module: &Module<'_>, target_data: &TargetData) {
    for function in module.get_functions() {
        for block in function.get_basic_block_iter() {
            for instruction in block.get_instructions() {
                if instruction.get_opcode() != InstructionOpcode::Store {
                    continue;
                }
                // SAFETY: a `store` has the stored value and the address as
                // its operands 0 and 1; every reference below is a live
                // value or type of `module`, and `target_data` outlives the
                // calls.
                unsafe {
                    let store = instruction.as_value_ref();
                    let value = llc::LLVMGetOperand(store, 0);
                    let address = llc::LLVMGetOperand(store, 1);
                    let Some(slot_ty) = slot_type(address) else {
                        continue;
                    };
                    let td = target_data.as_mut_ptr();
                    let written = llt::LLVMStoreSizeOfType(td, llc::LLVMTypeOf(value));
                    let held = llt::LLVMABISizeOfType(td, slot_ty);
                    assert!(
                        written <= held,
                        "pycc_codegen: internal error: `{}` stores {written} bytes into a \
                         {held}-byte slot: `{}`",
                        function.get_name().to_string_lossy(),
                        crate::compile::llvm_string_to_owned(
                            inkwell::values::AnyValue::print_to_string(&instruction)
                        ),
                    );
                }
            }
        }
    }
}

/// The type a single-element alloca or a global variable holds, or `None`
/// for any other address (an interior pointer, an array alloca, a pointer
/// loaded at run time) -- those are bounded by the code that computes them.
///
/// # Safety
/// `address` must be a live LLVM value.
unsafe fn slot_type(
    address: inkwell::llvm_sys::prelude::LLVMValueRef,
) -> Option<inkwell::llvm_sys::prelude::LLVMTypeRef> {
    // SAFETY (caller): `address` is live.
    unsafe {
        if !llc::LLVMIsAAllocaInst(address).is_null() {
            let count = llc::LLVMGetOperand(address, 0);
            let single = !llc::LLVMIsAConstantInt(count).is_null()
                && llc::LLVMConstIntGetZExtValue(count) == 1;
            return single.then(|| llc::LLVMGetAllocatedType(address));
        }
        if !llc::LLVMIsAGlobalVariable(address).is_null() {
            return Some(llc::LLVMGlobalGetValueType(address));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::check;
    use inkwell::builder::Builder;
    use inkwell::context::Context;
    use inkwell::module::Module;
    use inkwell::targets::TargetData;
    use inkwell::values::StructValue;

    const LAYOUT: &str = "e-m:e-i64:64-i128:128-f80:128-n8:16:32:64-S128";

    /// Builds `void f()` whose body `body` emits, then runs the check.
    fn run(body: impl for<'c> Fn(&'c Context, &Module<'c>, &Builder<'c>)) {
        let context = Context::create();
        let module = context.create_module("store_width");
        let function = module.add_function("f", context.void_type().fn_type(&[], false), None);
        let builder = context.create_builder();
        builder.position_at_end(context.append_basic_block(function, "entry"));
        body(&context, &module, &builder);
        builder.build_return(None).unwrap();
        check(&module, &TargetData::create(LAYOUT));
    }

    /// The two-byte `{ i8, i8 }` `None` literal placeholder.
    fn pair(context: &Context) -> StructValue<'_> {
        context
            .struct_type(&[context.i8_type().into(); 2], false)
            .const_zero()
    }

    #[test]
    fn fitting_stores_and_unbounded_addresses_pass() {
        run(|context, module, builder| {
            let slot = builder.build_alloca(context.i8_type(), "slot").unwrap();
            builder
                .build_store(slot, context.i8_type().const_zero())
                .unwrap();
            let wide = builder.build_alloca(context.i64_type(), "wide").unwrap();
            builder.build_store(wide, pair(context)).unwrap();
            // An array alloca is bounded by the code that sized it.
            let count = context.i64_type().const_int(4, false);
            let array = builder
                .build_array_alloca(context.i8_type(), count, "array")
                .unwrap();
            builder.build_store(array, pair(context)).unwrap();
            let global = module.add_global(context.i64_type(), None, "g");
            global.set_initializer(&context.i64_type().const_zero());
            builder
                .build_store(global.as_pointer_value(), context.i64_type().const_zero())
                .unwrap();
        });
    }

    #[test]
    #[should_panic(expected = "`f` stores 2 bytes into a 1-byte slot")]
    fn a_store_past_an_alloca_is_an_internal_error() {
        run(|context, _module, builder| {
            let slot = builder.build_alloca(context.i8_type(), "slot").unwrap();
            builder.build_store(slot, pair(context)).unwrap();
        });
    }

    #[test]
    #[should_panic(expected = "`f` stores 8 bytes into a 1-byte slot")]
    fn a_store_past_a_global_is_an_internal_error() {
        run(|context, module, builder| {
            let global = module.add_global(context.i8_type(), None, "g");
            global.set_initializer(&context.i8_type().const_zero());
            builder
                .build_store(global.as_pointer_value(), context.i64_type().const_zero())
                .unwrap();
        });
    }
}
