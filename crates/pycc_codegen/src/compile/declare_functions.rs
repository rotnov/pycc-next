//! The first pass of `compile_to_object_with_observer`: declare every
//! user-defined function under its mangled name, with its real signature,
//! before any body is emitted.

use super::*;

/// Declares every `MirItem::Function` of `mir` (see the comments below for
/// the mangling, redefinition and function-pointer-slot rules), returning
/// the per-name call information, every definition in source order and the
/// number of definitions of each name.
#[allow(clippy::type_complexity, clippy::too_many_lines)]
pub(super) fn declare_user_functions<'a, 'ctx>(
    context: &'ctx Context,
    module: &inkwell::module::Module<'ctx>,
    mir: &'a MirModule,
    slice_widened: &std::collections::BTreeSet<String>,
) -> (
    HashMap<&'a str, UserFunction<'ctx>>,
    Vec<(&'a str, FunctionValue<'ctx>)>,
    HashMap<&'a str, usize>,
) {
    let mut user_functions: HashMap<&'a str, UserFunction<'ctx>> = HashMap::new();
    // Maps function name to the number of definitions seen so far, for
    // unique mangled-name generation on redefinition.
    let mut def_counts: HashMap<&'a str, usize> = HashMap::new();
    // List of (function name, LLVM function value) in source order, for
    // the top-level binding pass.
    let mut function_defs_in_order: Vec<(&'a str, FunctionValue<'ctx>)> = Vec::new();
    // Part 2 of #1175 (#1179), review round 2: the buffer-sub-range
    // out-slot fact is resolved **per name, over every definition of it**,
    // not per `def`. Two `def f`s share one `fnptr_f` slot and one
    // `UserFunction::fn_type`, so giving them two arities makes the
    // indirect call ill-typed for whichever definition the slot does not
    // describe -- which is the `verify()` abort a redefinition used to hit
    // when only the slicing definition was widened. See
    // `crate::ext::buffer_slice_out_names` for why the union is the safe
    // direction and for the driver-side half that unions with it.
    for item in &mir.items {
        if let MirItem::Function {
            name,
            params,
            return_ty,
            body,
        } = item
        {
            let mut param_types: Vec<inkwell::types::BasicMetadataTypeEnum> = params
                .iter()
                .map(|(_, ty)| ty_to_basic_type(context, ty.clone()).into())
                .collect();
            // Part 2 of #1175 (#1179): a function whose body returns a
            // sub-range of a buffer parameter hands the sub-range's
            // `has_slice`/`start`/`stop` back through three trailing
            // out-pointers, because the bounds are produced inside this
            // frame while the returned value stays the parameter's own view
            // pointer -- which is what keeps the generated wrapper's
            // `result == &a{index}` identity test exact.
            //
            // The predicate is the body fact alone, deliberately not gated
            // on `options.ext` or on `ext_thunk_required`'s own
            // exportability test: this declaration and `emit_stmt`'s own
            // `MirStmt::ReturnBufferSlice` arm must never disagree about
            // the arity, and `pycc_types` already refuses the shape
            // everywhere a wrapper cannot exist.
            if slice_widened.contains(name.as_str()) {
                let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
                for _ in crate::ext::ext_thunk_out_tys(return_ty, true) {
                    param_types.push(ptr_type.into());
                }
            }
            // A multi-value twin (`pycc_mir::multi_return`) stores its
            // results after the first through one trailing out-pointer
            // each (`MirStmt::SetOut`).
            for _ in pycc_mir::multi_return::out_tys(body) {
                param_types.push(context.ptr_type(inkwell::AddressSpace::default()).into());
            }
            let fn_type = match return_ty {
                pycc_mir::Ty::None => context.void_type().fn_type(&param_types, false),
                other => ty_to_basic_type(context, other.clone()).fn_type(&param_types, false),
            };
            let count = def_counts.entry(name.as_str()).or_insert(0);
            let mangled = if *count == 0 {
                format!("pyfn_{name}")
            } else {
                format!("pyfn_{name}__redef_{count}")
            };
            *count += 1;
            let f = module.add_function(&mangled, fn_type, None);
            if leaf_inline::is_tiny_leaf(body) {
                leaf_inline::mark(context, f);
            }
            function_defs_in_order.push((name.as_str(), f));
            // Only insert into user_functions on the first definition;
            // subsequent definitions update fn_ptr_global at their source
            // position but share the same slot and type info. The param_tys
            // and fn_type come from the first definition (the type checker
            // resolves one signature per name).
            user_functions.entry(name.as_str()).or_insert_with(|| {
                // Monomorphized generic specializations (`0gen_...` names)
                // are compiler-generated, not user-defined: they have no
                // top-level `def` whose execution order matters, so they
                // dispatch directly through `direct_value` instead of
                // through the indirect function-pointer slot.
                let is_monomorphized = name.starts_with("0gen_");
                if is_monomorphized {
                    UserFunction {
                        param_tys: params.iter().map(|(_, ty)| ty.clone()).collect(),
                        fn_ptr_global: None,
                        fn_type,
                        name_global: None,
                        direct_value: Some(f),
                        single_def: None,
                        bound_before_code: false,
                    }
                } else {
                    let fn_ptr_type = context.ptr_type(inkwell::AddressSpace::default());
                    // A method's `name` is dotted (`Grid.scale.static`), and
                    // this global is the one codegen emits that the generated
                    // C *declares* -- `extern void *fnptr_<name>;` in
                    // `pycc::ext_build`'s `wrapper_for`. A dotted C identifier
                    // is not accepted by any compiler, so the symbol carries
                    // the mangled spelling. inkwell 0.9 exposes no alias API
                    // and the workspace has no `llvm-sys` dependency, so this
                    // is a rename and not a C-safe alias beside the dotted
                    // one; nothing outside this workspace consumes the
                    // spelling. `mangle_ext_name` is the identity for a
                    // dot-free name, so every module-level function's global
                    // is byte-identical to what it was.
                    //
                    // This path is not gated on `CompileOptions::ext`, so a
                    // native build renames a method's global too, where no C
                    // ever declares it. That is deliberate: one spelling per
                    // symbol is cheaper to reason about than a mode-dependent
                    // one, and nothing outside this workspace reads it.
                    let fn_ptr_global = module.add_global(
                        fn_ptr_type,
                        None,
                        &format!("fnptr_{}", mangle_ext_name(name)),
                    );
                    fn_ptr_global.set_initializer(&fn_ptr_type.const_null());
                    // Deliberately *not* mangled, on both counts. The symbol
                    // is internal-linkage and never declared from C, so it
                    // needs no C-legal spelling; and its contents are the
                    // name `pycc_rt_raise_name_error` reports, so the array size
                    // and initializer stay on the source dotted name --
                    // mangling them would make a method's `NameError` read
                    // `0m4_Grid5_scale6_static`.
                    let name_global = module.add_global(
                        context.i8_type().array_type(name.len() as u32 + 1),
                        None,
                        &format!("fnname_{name}"),
                    );
                    name_global.set_linkage(Linkage::Internal);
                    name_global.set_constant(true);
                    name_global.set_initializer(&context.const_string(name.as_bytes(), true));
                    UserFunction {
                        param_tys: params.iter().map(|(_, ty)| ty.clone()).collect(),
                        fn_ptr_global: Some(fn_ptr_global),
                        fn_type,
                        name_global: Some(name_global),
                        direct_value: None,
                        single_def: None,
                        bound_before_code: false,
                    }
                }
            });
        }
    }
    (user_functions, function_defs_in_order, def_counts)
}

/// Marks the functions defined once and never rebound as having a single
/// definition, and those bound before any code can run.
pub(super) fn mark_single_defs<'a, 'ctx>(
    mir: &'a MirModule,
    user_functions: &mut HashMap<&'a str, UserFunction<'ctx>>,
    function_defs_in_order: &[(&'a str, FunctionValue<'ctx>)],
    def_counts: &HashMap<&'a str, usize>,
) {
    // A name defined once and never bound as a variable can only ever hold
    // that one definition (see `UserFunction::single_def`). The ext
    // wrappers only read the slot, so CPython rebinding the module attribute
    // does not reach compiled callers -- the same as for indirect calls.
    let rebound_names = collect_module_bindings(mir);
    for &(name, f) in function_defs_in_order {
        if def_counts[name] == 1
            && !rebound_names.contains_key(name)
            && let Some(uf) = user_functions.get_mut(name)
            && uf.fn_ptr_global.is_some()
        {
            uf.single_def = Some(f);
        }
    }
    // A single definition whose `def` precedes every statement that can run
    // code is bound before any function body can run, so no call reaches
    // its slot while it is still null (see `UserFunction::bound_before_code`).
    for name in nothrow::bound_before_code(mir) {
        if let Some(uf) = user_functions.get_mut(name)
            && uf.single_def.is_some()
        {
            uf.bound_before_code = true;
        }
    }
}
