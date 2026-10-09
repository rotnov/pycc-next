//! #1050: emission of the scalar-only `ext` export thunks, and the
//! function-pointer dispatch guard both thunk emission and ordinary call
//! emission go through.
//!
//! A generated `--ext` wrapper is C, and C can only call a pycc function
//! whose signature it can spell. For a scalar-only signature it can: the
//! wrapper casts `fnptr_<name>` to a function pointer and calls through it,
//! and `boundary_carrier` in `src/ext_build.rs` pins each C width to what
//! `ty_to_basic_type` gives the callee. An aggregate has no such spelling.
//! pycc's own convention for returning one is not the platform C struct ABI
//! -- measured on aarch64-apple-darwin, a pycc function returning
//! `tuple[int, int, int, int, int]` hands the five words back in `x0`-`x4`
//! where clang's C ABI would pass a hidden `sret` pointer -- so a C
//! declaration of the compiled function would disagree with it silently, in
//! the one direction no compiler on either side can diagnose.
//!
//! So the aggregate never crosses into C at all. Codegen emits one extra
//! function per aggregate-carrying export, `pycc_ext_thunk_<name>`, whose
//! own signature is scalars and pointers only: a tuple parameter arrives as
//! its elements in place, and a tuple return leaves through one trailing
//! out-pointer per element with the thunk itself returning `void`. Inside,
//! on the LLVM side of the seam, the thunk reassembles the aggregate with
//! `insertvalue`, dispatches through `fnptr_<name>` exactly as a Python-level
//! call would, and writes the result back out with `extractvalue`. The
//! convention itself -- the symbol name, the flattening, the out-pointer
//! order -- is stated once in [`crate::ext`], which `src/ext_build.rs` reads
//! to render the matching C.
//!
//! A carve-out from `lib.rs` under AGENTS.md's "Keep source files
//! decomposable" rule, following `call_result.rs`; the tracker for the rest
//! of that file is #545.

use super::*;
use pycc_mir::Ty;
use std::collections::HashSet;

/// Loads `user_function`'s `fnptr_<name>` global and leaves the builder
/// positioned in a block reached only when that pointer is non-null,
/// returning the loaded pointer for an immediately following
/// `build_indirect_call`.
///
/// Issue #22's dispatch discipline: the slot is null until the `def`
/// executes at module level, so a call that reaches it first is Python's
/// `NameError` and not a jump through null. The null path calls
/// `pycc_rt_name_error`, which does not return, and is terminated with
/// `unreachable`.
///
/// Shared by `emit_call`'s ordinary user-function call and by
/// [`emit_export_thunks`] below rather than replicated: a thunk that skipped
/// the guard would turn `--ext`'s own module-import order into a null jump,
/// and a second copy of the sequence is exactly the kind of drift the guard
/// exists to prevent.
pub(super) fn emit_fnptr_dispatch_guard<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    user_function: &UserFunction<'ctx>,
) -> PointerValue<'ctx> {
    let fn_ptr_global = user_function
        .fn_ptr_global
        .as_ref()
        .expect("non-monomorphized user function has a fn_ptr_global");
    let fn_ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    let fn_ptr = builder
        .build_load(fn_ptr_type, fn_ptr_global.as_pointer_value(), "load_fnptr")
        .expect("build_load should not fail for a global function-pointer slot")
        .into_pointer_value();
    let null_ptr = fn_ptr_type.const_null();
    let is_null = builder
        .build_int_compare(IntPredicate::EQ, fn_ptr, null_ptr, "fnptr_is_null")
        .expect("build_int_compare should not fail for a null check");
    let current_fn = builder
        .get_insert_block()
        .expect("builder is always positioned in a block during call emission")
        .get_parent()
        .expect("every block has a parent function");
    let not_null_block = context.append_basic_block(current_fn, "fnptr_not_null");
    let is_null_block = context.append_basic_block(current_fn, "fnptr_is_null");
    builder
        .build_conditional_branch(is_null, is_null_block, not_null_block)
        .expect("build_conditional_branch should not fail for a null-check dispatch");
    // Null path: call pycc_rt_name_error with the function name as a C
    // string, then unreachable (name_error never returns). The name
    // global was created once per function name in the declaration pass
    // and is reused at every call site.
    builder.position_at_end(is_null_block);
    let name_global = user_function
        .name_global
        .as_ref()
        .expect("non-monomorphized user function has a name_global");
    let name_ptr = name_global.as_pointer_value();
    builder
        .build_call(rt.name_error, &[name_ptr.into()], "name_error")
        .expect("build_call should not fail for a well-formed runtime error call");
    builder
        .build_unreachable()
        .expect("build_unreachable terminates the null-pointer path");
    builder.position_at_end(not_null_block);
    fn_ptr
}

/// Emits one `pycc_ext_thunk_<name>` for every export whose signature
/// carries a `tuple`, in `mir.items` order.
///
/// Called only for an `ext` build. Source order rather than
/// `user_functions`' own (a `HashMap`, so unordered) keeps the emitted
/// module byte-identical across runs; a name already emitted is skipped
/// because a redefinition shares one `fnptr_` slot and one signature
/// (`T0021` refuses a redefinition that changes the signature), so a second
/// thunk would be a duplicate symbol for the same call.
pub(super) fn emit_export_thunks<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    mir: &MirModule,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
) {
    let mut emitted: HashSet<&str> = HashSet::new();
    // The codegen-side half of the driver's own per-export fact; see
    // `crate::ext::buffer_slice_out_names` for why it is computed here from
    // MIR rather than threaded in (`ExtExport` never reaches this crate),
    // why it is resolved per *name* rather than per definition, and for the
    // parity test that pins the two answers together. Computed once, outside
    // the loop, precisely so two definitions of one name cannot get two
    // answers -- the defect that made a redefinition forward four arguments
    // through a one-argument `fn_type`.
    let slice_widened = crate::ext::buffer_slice_out_names(&mir.items);
    for item in &mir.items {
        let MirItem::Function {
            name,
            params,
            return_ty,
            ..
        } = item
        else {
            continue;
        };
        let param_tys: Vec<Ty> = params.iter().map(|(_, ty)| ty.clone()).collect();
        let returns_buffer_slice = slice_widened.contains(name.as_str());
        if !ext_thunk_required(name, &param_tys, return_ty, returns_buffer_slice) {
            continue;
        }
        if !emitted.insert(name.as_str()) {
            continue;
        }
        emit_one_thunk(
            context,
            builder,
            module,
            rt,
            name,
            &param_tys,
            return_ty,
            returns_buffer_slice,
            &user_functions[name.as_str()],
        );
    }
}

/// Emits a single export's thunk.
///
/// The aggregate types here are always taken from `ty_to_basic_type` on the
/// declared `Ty`, never rebuilt field by field: the callee's own `fn_type`
/// was built by exactly that call in the declaration pass, and a second,
/// independently constructed struct type would be a second source of truth
/// for field order and packedness.
#[allow(clippy::too_many_arguments)]
fn emit_one_thunk<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    name: &str,
    param_tys: &[Ty],
    return_ty: &Ty,
    returns_buffer_slice: bool,
    user_function: &UserFunction<'ctx>,
) {
    let out_tys = ext_thunk_out_tys(return_ty, returns_buffer_slice);
    let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
    let mut thunk_params: Vec<inkwell::types::BasicMetadataTypeEnum> =
        ext_thunk_param_tys(param_tys)
            .into_iter()
            .map(|ty| ty_to_basic_type(context, ty).into())
            .collect();
    for _ in &out_tys {
        thunk_params.push(ptr_type.into());
    }
    // A tuple return leaves through the out-pointers, so the thunk itself
    // returns `void` -- the same shape a `-> None` export already has.
    let thunk_type = match return_ty {
        Ty::None | Ty::Tuple(_) => context.void_type().fn_type(&thunk_params, false),
        other => ty_to_basic_type(context, other.clone()).fn_type(&thunk_params, false),
    };
    let thunk = module.add_function(&ext_thunk_symbol(name), thunk_type, None);
    let entry = context.append_basic_block(thunk, "entry");
    builder.position_at_end(entry);
    // Re-assemble each declared parameter from the flattened slots it was
    // spread across, in the same order `ext_thunk_param_tys` spread them.
    let mut next_param: u32 = 0;
    let mut arg_values: Vec<inkwell::values::BasicMetadataValueEnum> = Vec::new();
    for ty in param_tys {
        if let Ty::Tuple(elems) = ty {
            let struct_ty = ty_to_basic_type(context, ty.clone()).into_struct_type();
            let mut aggregate = struct_ty.get_undef();
            for field in 0..elems.len() {
                let element = thunk
                    .get_nth_param(next_param)
                    .expect("the thunk declares one parameter per flattened tuple element");
                next_param += 1;
                aggregate = builder
                    .build_insert_value(aggregate, element, field as u32, "tuple_field")
                    .expect("build_insert_value should not fail for a well-typed tuple field")
                    .into_struct_value();
            }
            arg_values.push(aggregate.into());
        } else {
            arg_values.push(
                thunk
                    .get_nth_param(next_param)
                    .expect("the thunk declares one parameter per scalar")
                    .into(),
            );
            next_param += 1;
        }
    }
    // Part 2 of #1175 (#1179): a buffer sub-range export's out-pointers are
    // **forwarded**, not consumed. The bounds are produced inside the
    // callee's own frame, by the `return` statement that evaluates them, so
    // the callee's LLVM signature carries the three trailing pointers (the
    // declaration pass widens it from the same per-body fact) and the thunk
    // simply hands its own along. That is the opposite direction from a
    // tuple return, whose out-pointers the thunk writes itself after
    // `extractvalue`.
    if returns_buffer_slice {
        for _ in &out_tys {
            arg_values.push(
                thunk
                    .get_nth_param(next_param)
                    .expect("the thunk declares one out-pointer per buffer-slice out-slot")
                    .into(),
            );
            next_param += 1;
        }
    }
    let fn_ptr = emit_fnptr_dispatch_guard(context, builder, rt, user_function);
    let call = builder
        .build_indirect_call(user_function.fn_type, fn_ptr, &arg_values, "call_export")
        .expect("build_indirect_call should not fail for a well-formed indirect call");
    if returns_buffer_slice {
        let value = call
            .try_as_basic_value()
            .expect_basic("a buffer-returning export returns its view pointer");
        builder
            .build_return(Some(&value))
            .expect("build_return should not fail for a buffer-slice thunk");
    } else if !out_tys.is_empty() {
        let aggregate = call
            .try_as_basic_value()
            .expect_basic("a tuple-returning export returns an LLVM struct value")
            .into_struct_value();
        for index in 0..out_tys.len() {
            let element = builder
                .build_extract_value(aggregate, index as u32, "tuple_elem")
                .expect("build_extract_value should not fail for a declared tuple field");
            let out_ptr = thunk
                .get_nth_param(next_param)
                .expect("the thunk declares one out-pointer per returned tuple element")
                .into_pointer_value();
            next_param += 1;
            builder
                .build_store(out_ptr, element)
                .expect("build_store should not fail through a thunk out-pointer");
        }
        builder
            .build_return(None)
            .expect("build_return should not fail for a void thunk");
    } else if matches!(return_ty, Ty::None) {
        builder
            .build_return(None)
            .expect("build_return should not fail for a void thunk");
    } else {
        let value = call
            .try_as_basic_value()
            .expect_basic("a scalar-returning export returns a non-void value");
        builder
            .build_return(Some(&value))
            .expect("build_return should not fail for a scalar thunk");
    }
}

#[cfg(test)]
#[path = "ext_thunk_tests.rs"]
mod ext_thunk_tests;
