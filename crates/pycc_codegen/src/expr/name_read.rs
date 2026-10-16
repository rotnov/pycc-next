//! Name reads and walrus (`NamedExpr`) lowering.

use super::*;
use pycc_mir::Ty;

pub(crate) fn emit_name_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::Name { name, ty } = expr else {
        unreachable!()
    };
    let slot = locals
        .get(name)
        .unwrap_or_else(|| panic!("pycc_codegen: internal error: `{name}` has no local slot"));
    debug_assert_eq!(
        &slot.ty, ty,
        "pycc_codegen: internal error: local type drifted"
    );
    if let Some(initialized_ptr) = slot.initialized {
        let initialized = builder
            .build_load(context.i8_type(), initialized_ptr, "global_initialized")
            .expect("build_load should not fail for a declared global flag")
            .into_int_value();
        let is_initialized = builder
            .build_int_compare(
                IntPredicate::NE,
                initialized,
                context.i8_type().const_zero(),
                "global_is_initialized",
            )
            .expect("build_int_compare should not fail for two i8 values");
        let function = builder
            .get_insert_block()
            .expect("name reads are always emitted inside a basic block")
            .get_parent()
            .expect("the current basic block always belongs to a function");
        let ready = context.append_basic_block(function, "global_ready");
        let unbound = context.append_basic_block(function, "global_unbound");
        let branch = builder
            .build_conditional_branch(is_initialized, ready, unbound)
            .expect("build_conditional_branch should not fail for an i1 condition");
        crate::int_fast::weight_branch(context, branch, true);
        builder.position_at_end(unbound);
        // #1316 / Part 1 of #1333: an `object` global read in a
        // function body before its import or assignment ran raises
        // a catchable `NameError`, as CPython does. Any other global read before its binding ran (a function called
        // before the module statement that binds it) raises CPython's
        // `NameError` natively.
        // A function local's flag is an `alloca` (`entry_slots`), a
        // global's a module global: an unbound local is CPython's
        // `UnboundLocalError`, an unbound global its `NameError`.
        let local = inkwell::values::BasicValue::as_instruction_value(&initialized_ptr)
            .is_some_and(|i| i.get_opcode() == inkwell::values::InstructionOpcode::Alloca);
        let raise_symbol = if local {
            "pycc_rt_raise_unbound_local"
        } else {
            "pycc_rt_raise_name_error"
        };
        if local || !foreign_fail::emit_unbound_object_read(context, builder, module, rt, ty, name)
        {
            let ptr = context.ptr_type(inkwell::AddressSpace::default());
            let name_error = module.get_function(raise_symbol).unwrap_or_else(|| {
                module.add_function(
                    raise_symbol,
                    context
                        .void_type()
                        .fn_type(&[ptr.into(), context.i64_type().into()], false),
                    Some(inkwell::module::Linkage::External),
                )
            });
            let bytes = builder
                .build_global_string_ptr(name, &format!("pycc_unbound_name_{name}"))
                .expect("build_global_string_ptr should not fail")
                .as_pointer_value();
            let len = context.i64_type().const_int(name.len() as u64, false);
            builder
                .build_call(name_error, &[bytes.into(), len.into()], "")
                .expect("build_call should not fail for pycc_rt_raise_name_error");
            crate::exception::jump_to_exception_target(context, builder, rt);
        }
        builder.position_at_end(ready);
    }
    // Deviation from the task brief: the brief's version matched
    // `ty` twice -- once to pick `load_ty` (a `BasicTypeEnum`) for
    // `build_load`, once more to wrap the loaded value in the
    // right `Scalar` variant -- with the second match's `Ty::Int`/
    // `Ty::Bool` arms mirroring the first's and a trailing
    // `_ => unreachable!("handled above")`. That second match's
    // catch-all is provably dead code, not merely hard to test:
    // both matches inspect the exact same `ty` value, so any input
    // that isn't `Ty::Int`/`Ty::Bool` already panics in the *first*
    // match (this arm's own "not supported yet" case, still
    // present just below) -- the second match's `_` arm can never
    // be reached by any input, well-formed or not. Merged into one
    // match (matching `ty` once) so there's no redundant
    // impossible-to-cover branch left over.
    match ty {
        Ty::Int => {
            let loaded = builder
                .build_load(context.i64_type(), slot.ptr, "load")
                .expect("build_load should not fail for a slot this function itself allocated");
            Scalar::Int(loaded.into_int_value())
        }
        Ty::Bool => {
            let loaded = builder
                .build_load(context.i8_type(), slot.ptr, "load")
                .expect("build_load should not fail for a slot this function itself allocated");
            Scalar::Bool(loaded.into_int_value())
        }
        Ty::Float => {
            let loaded = builder
                .build_load(context.f64_type(), slot.ptr, "load")
                .expect("build_load should not fail for a slot this function itself allocated");
            Scalar::Float(loaded.into_float_value())
        }
        Ty::Str => {
            let loaded = builder
                .build_load(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    slot.ptr,
                    "load",
                )
                .expect("build_load should not fail for a slot this function itself allocated");
            Scalar::Str(loaded.into_pointer_value())
        }
        Ty::None => {
            let loaded = builder
                .build_load(context.i8_type(), slot.ptr, "load_none")
                .expect("build_load should not fail for a slot this function itself allocated");
            // The static `Ty::None` retains the semantic distinction
            // from a bool even though both use an LLVM `i8` carrier.
            Scalar::Bool(loaded.into_int_value())
        }
        // Same pointer-slot read as `Ty::Str` immediately above --
        // `ty_to_basic_type`'s own `List(_)` arm already allocated
        // this slot as a pointer, so reading it back is identical
        // regardless of the element type.
        //
        // Task 5 (D-089) originally carried the loaded pointer in
        // `Scalar::Str`, safe only for as long as no `MirExpr`
        // could construct a `list[T]` value, and flagged as a
        // Task-11 tripwire for `truthy`/`to_str` specifically.
        // Task 11a (D-107) retired that reuse: the pointer is now
        // `Scalar::List`, so every exhaustive `Scalar` match had to
        // answer for `list[T]` explicitly instead of silently
        // treating it as a `PyStrObj`.
        //
        // `str_value_is_a_duplicate_reference` stays gated on
        // `ty: Ty::Str` (Task 5's own fix for the same reuse). That
        // gate is now redundant with the variant split for this
        // arm, but it is still the correct contract for that
        // function -- and `incref_if_str_duplicate` needs no
        // `List` arm at all, since its `if let Scalar::Str(..)
        // else { scalar }` shape already passes a list through
        // untouched, which is exactly D-107's leak-only policy.
        Ty::List(_) | Ty::VarTuple(_) => {
            let loaded = builder
                .build_load(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    slot.ptr,
                    "load",
                )
                .expect("build_load should not fail for a slot this function itself allocated");
            Scalar::List(loaded.into_pointer_value())
        }
        // Same pointer-slot read as `Ty::List(_)` immediately
        // above (PR-11 Task 5) -- `ty_to_basic_type`'s own
        // `Dict(_)` arm already allocated this slot as a pointer,
        // so reading it back is identical regardless of the
        // key/value types. Every real read of a `dict[str, int]`
        // local -- `len(x)`, `x[k]`, `x[k] = v`'s own dict operand,
        // `for k in x:` -- goes through this arm (directly, or via
        // `emit_dict_name_read`'s synthetic `MirExpr::Name`), so
        // without it every one of those would fall through to the
        // catch-all below and panic on a real, type-checked
        // program instead of reading the value.
        Ty::Dict(_) => {
            let loaded = builder
                .build_load(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    slot.ptr,
                    "load",
                )
                .expect("build_load should not fail for a slot this function itself allocated");
            Scalar::Dict(loaded.into_pointer_value())
        }
        // Same pointer-slot read as `Ty::List(_)`/`Ty::Dict(_)`
        // immediately above (PR-11 Task 9) -- `ty_to_basic_type`'s
        // own `Set(_)` arm already allocated this slot as a
        // pointer, so reading it back is identical regardless of
        // the element type. Every real read of a `set[int]`
        // local -- `len(x)`, `for v in x:` -- goes through this
        // arm (directly, or via `emit_set_name_read`'s synthetic
        // `MirExpr::Name`), so without it every one of those would
        // fall through to the catch-all below and panic on a real,
        // type-checked program instead of reading the value.
        Ty::Set(_) | Ty::FrozenSet(_) => {
            let loaded = builder
                .build_load(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    slot.ptr,
                    "load",
                )
                .expect("build_load should not fail for a slot this function itself allocated");
            Scalar::Set(loaded.into_pointer_value())
        }
        // NOT a pointer read, unlike the four container arms above
        // (PR-11b Task 5, D-115): a tuple slot holds the struct
        // itself, so this loads the whole aggregate back out as an
        // SSA value. The load type must be computed from `ty`
        // rather than hardcoded like every other arm here, since a
        // tuple's LLVM type depends on its own element types.
        //
        // Not optional, and not merely a nicety: `t[0]` -- the only
        // tuple operation D-116 ships besides construction -- lowers
        // to a `MirExpr::Subscript` whose base is exactly this
        // `MirExpr::Name`. Without this arm every literal-index read
        // of a tuple *variable* (as opposed to an inline
        // `(1, 2)[0]`) would fall through to the catch-all below and
        // panic on a real, type-checked program, which is why the
        // module-level and function-local smoke programs for this
        // task both exercise it.
        Ty::Tuple(elements) if pycc_hir::tuple_is_boxed(elements) => {
            let loaded = builder
                .build_load(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    slot.ptr,
                    "load",
                )
                .expect("build_load should not fail for a slot this function itself allocated");
            Scalar::List(loaded.into_pointer_value())
        }
        Ty::Tuple(_) => {
            let loaded = builder
                .build_load(
                    ty_to_basic_type(context, ty.clone()).into_struct_type(),
                    slot.ptr,
                    "load",
                )
                .expect("build_load should not fail for a slot this function itself allocated");
            Scalar::Tuple(loaded.into_struct_value())
        }
        // Same pointer-slot read as `Ty::List(_)`/`Ty::Dict(_)`/
        // `Ty::Set(_)` above (D-154, Part 1 of #375) --
        // `ty_to_basic_type`'s own `Instance(_)` arm already
        // allocated this slot as a pointer, so reading it back is
        // identical regardless of the class. Every real read of a
        // class-instance local -- `p.x`, `p.bump()`, passing `p` as
        // an argument -- goes through this arm, so without it every
        // one of those would fall through to the catch-all below
        // and panic on a real, type-checked program instead of
        // reading the value.
        Ty::Instance(_) => {
            let loaded = builder
                .build_load(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    slot.ptr,
                    "load",
                )
                .expect("build_load should not fail for a slot this function itself allocated");
            Scalar::Instance(loaded.into_pointer_value())
        }
        // NOT a pointer read, identical in kind to `Ty::Tuple(_)`
        // above (D-197, #763, Part 1 of #747): an `Optional[int]`
        // slot holds the `{ int, i8 }` struct itself, so this loads
        // the whole aggregate back out as an SSA value. Every real
        // read of an `Optional[int]` local -- `x is None`,
        // `if x:`, passing `x` as an argument, `return x` -- lowers
        // to exactly this `MirExpr::Name`, so without it every one
        // of those would fall through to the catch-all below and
        // panic on a real, type-checked program instead of reading
        // the value.
        Ty::Optional(_) => {
            let loaded = builder
                .build_load(
                    ty_to_basic_type(context, ty.clone()).into_struct_type(),
                    slot.ptr,
                    "load",
                )
                .expect("build_load should not fail for a slot this function itself allocated");
            Scalar::Optional(loaded.into_struct_value())
        }
        // Part 2 of #1026 (#1081). The same pointer-slot read as
        // `Ty::Instance(_)` above, for the global a foreign
        // `import numpy` binds. The plan for this PR expected the
        // catch-all below to stay the only `Ty::Object` answer here
        // -- `pycc_types` refused binding a CPython object to a
        // name, so no *local* could carry one -- but a foreign
        // import's module global is a storage slot like any other
        // (and since #1325 so is a module-level `x = <object>`),
        // and `MirExpr::ObjAttrGet`'s base -- like
        // `MirExpr::ObjMethodCall`'s, added by PR 2b -- is a plain
        // `MirExpr::Name` read of it. Without this arm the one
        // shape PR 2a actually compiles (`numpy.pi`) panics here
        // before reaching `foreign_attr::emit`.
        //
        // No reference-count traffic accompanies the read: the
        // slot owns its reference, so a read is a borrow
        // (`heap_rc::is_borrowed_read`); a consumer that keeps it
        // retains it, and an operand pinned across a sibling that
        // may rebind it is retained by `emit_expr`.
        Ty::Object => {
            let loaded = builder
                .build_load(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    slot.ptr,
                    "load",
                )
                .expect("build_load should not fail for a slot this function itself allocated");
            Scalar::Object(loaded.into_pointer_value())
        }
        // Part 2 of #1027: the same pointer-slot read again, for a
        // `pycc build --ext` export's `memoryview` parameter. Its
        // slot holds the `PyccExtBufferView *` the generated wrapper
        // passed; `ty_to_basic_type` already maps `Ty::MemoryView`
        // to a plain pointer, so only this load was missing.
        //
        // The one expression that reaches it is a `BufferGet` base,
        // which is the whole of the admitted read surface -- every
        // other read of the name is `reject_memoryview_read`'s
        // `C0001`. No refcount traffic: the buffer is borrowed and
        // wrapper-released, so the read owns nothing.
        Ty::MemoryView => {
            let loaded = builder
                .build_load(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    slot.ptr,
                    "load",
                )
                .expect("build_load should not fail for a slot this function itself allocated");
            Scalar::MemoryView(loaded.into_pointer_value())
        }
        // A generator object: the slot's pointer, borrowed.
        Ty::Generator(_) => {
            let loaded = builder
                .build_load(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    slot.ptr,
                    "load",
                )
                .expect("build_load should not fail for a slot this function itself allocated");
            Scalar::Generator(loaded.into_pointer_value())
        }
        // A function value: the slot's pointer, borrowed.
        Ty::Callable(_) => {
            let loaded = builder
                .build_load(
                    context.ptr_type(inkwell::AddressSpace::default()),
                    slot.ptr,
                    "load",
                )
                .expect("build_load should not fail for a slot this function itself allocated");
            Scalar::Closure(loaded.into_pointer_value())
        }
        other => {
            panic!(
                "pycc_codegen: reading a `{}`-typed local is not supported yet",
                other.name()
            )
        }
    }
}

// PEP 572 (#774): `target := value`. `name`'s storage slot is
// already predeclared by `collect_expr_bindings` (this node's own
// slot-scanning counterpart to `collect_stmt_bindings`'s
// `MirStmt::Assign` handling), exactly the way `MirStmt::Assign`'s
// own target is predeclared -- so this mirrors that statement's own
// codegen (`emit_stmt`'s `MirStmt::Assign` arm) almost exactly:
// evaluate `value`, retain it if it is a borrowed bigint duplicate
// (T0050 restricts a walrus's value to non-reference-counted scalar
// types, so `incref_if_str_duplicate` never applies here -- there is
// no `str`/container/instance case to retain), then store via the
// same `emit_assign` this statement uses. Unlike a plain assignment
// statement, this node must also yield a value -- routed through a
// synthetic `MirExpr::Name` read of the slot it just stored into
// (mirroring `emit_list_name_read`'s own "round-trip through the
// `Name` arm" precedent just below), so the result gets the exact
// same load/definite-assignment/refcount-borrow treatment an
// ordinary later read of `name` would get -- which is also why
// `bigint_rc::int_value_is_a_duplicate_reference` classifies
// `NamedExpr { ty: Ty::Int, .. }` as borrowed exactly like `Name`:
// the value this arm yields is a second read of the same slot, not
// a second owned reference.
pub(crate) fn emit_named_expr_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::NamedExpr { name, value, ty } = expr else {
        unreachable!()
    };
    let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
    let scalar = retain_if_int_duplicate(context, builder, rt, value, scalar);
    // A counted value (refused by T0050 today, admitted by MIR)
    // follows `MirStmt::Assign`'s slot ownership exactly.
    let owning = !rt.borrowed_locals.borrow().contains(name);
    let scalar = if owning {
        heap_rc::retain_if_heap_duplicate(context, builder, rt, value, scalar)
    } else {
        scalar
    };
    if owning && let Some(slot) = locals.get(name) {
        heap_rc::release_slot(context, builder, rt, &slot.ty, slot.ptr);
    }
    emit_assign(context, builder, rt, locals, name, scalar);
    emit_expr_unchecked(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        &MirExpr::Name {
            name: name.clone(),
            ty: ty.clone(),
        },
    )
}
