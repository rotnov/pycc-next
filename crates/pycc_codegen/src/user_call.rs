//! Direct calls to compiled user functions.

use super::*;

/// Evaluates every entry in `args` (via `emit_expr`, so each argument is
/// itself an arbitrary expression -- nested calls included, which is
/// exactly what makes recursion with real arguments work) and emits the
/// `call` instruction to the already-resolved `f`. Shared between
/// `emit_expr`'s `Call` arm (a value-producing call used inside a larger
/// expression) and `emit_stmt`'s void-call arm below (a call whose
/// declared return type is `None`, used as a bare statement). A
/// `None`-returning call has no LLVM return value; `emit_expr` separately
/// materializes its canonical unit carrier only when a surrounding
/// expression needs one. This is the one piece both call sites need
/// regardless of whether a value comes back afterward.
///
/// Deliberately does *not* also do the `user_functions` lookup for
/// `callee`: the two call sites disagree on how a missing function should
/// be reported. `emit_stmt`'s arm still has a `Result` to propagate a
/// clean, user-facing error through (matching this crate's pre-Task-5
/// behavior, just generalized from zero-arg-only to any arity); `emit_expr`
/// does not (see its own `Call` arm's comment) -- so each resolves `f`
/// itself, with its own error-handling policy, before calling this helper.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_call_to<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    callee_name: &str,
    args: &[MirExpr],
) -> inkwell::values::CallSiteValue<'ctx> {
    let user_function = &user_functions[callee_name];
    build_call_to_with_leading_args(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        user_function,
        callee_name,
        &[],
        args,
    )
}

/// Generalizes [`build_call_to`] with a `leading_args` prefix -- values
/// already computed by the caller (never marshalled through `emit_expr`,
/// since they have no `MirExpr` of their own) that go first in the emitted
/// `call`'s argument list, before `args`' own marshalled values. Used by
/// `MirExpr::Instantiate`'s codegen (D-154, Part 1 of #375) to pass the
/// freshly allocated instance pointer as `__init__`'s own `self` argument --
/// `build_call_to`'s ordinary path has no `MirExpr` to build that pointer
/// from, since it is a codegen-only value with no HIR/MIR node of its own.
/// `leading_args` is skipped when zipping `args` against
/// `user_function.param_tys`, so `args[0]` still lines up with the *first
/// non-leading* declared parameter type, exactly like `build_call_to`'s own
/// contract for every ordinary call (`leading_args` empty).
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_call_to_with_leading_args<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    user_function: &UserFunction<'ctx>,
    callee_name: &str,
    leading_args: &[inkwell::values::BasicMetadataValueEnum<'ctx>],
    args: &[MirExpr],
) -> inkwell::values::CallSiteValue<'ctx> {
    let callee = resolve_callee(context, builder, rt, user_function);
    let mut arg_values: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = leading_args.to_vec();
    // A leading argument (an instance the caller keeps: `__init__`'s fresh
    // `self`, a `__hash__`/`__eq__` operand) goes to a parameter slot that
    // owns it, so the callee gets a reference of its own.
    for (value, param_ty) in leading_args.iter().zip(&user_function.param_tys) {
        if let (Some(kind), inkwell::values::BasicMetadataValueEnum::PointerValue(ptr)) =
            (heap_rc::heap_kind(rt, param_ty), value)
        {
            heap_rc::emit_incref(context, builder, rt, kind, *ptr);
        }
    }
    // #638 (D-208): a fresh (non-duplicate) `Ty::Int` argument's ownership
    // *transfers* to the callee's parameter slot on the normal path --
    // there is no release call for it anywhere in this function, unlike
    // every other affected site in the inventory. If a later sibling
    // argument's own evaluation raises before `build_call`/
    // `build_indirect_call` below is ever reached, that transfer never
    // happens and the reference is orphaned. `mark` records the stack
    // depth before this loop so the truncate below is recursion-safe: an
    // argument expression can itself contain a nested call whose own
    // marshalling loop pushes and pops its own arguments' words while
    // evaluating this loop's own argument, and this loop's earlier
    // pending entries must stay untouched by that nested truncation.
    let marshalled_args = marshal_args(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        &user_function.param_tys[leading_args.len()..],
        callee_name,
        args,
    );
    arg_values.extend(marshalled_args);
    call_resolved(builder, user_function, callee, &arg_values)
}

/// Evaluates `args` for parameters of `param_tys`, each value a reference
/// the callee's parameter slot will own: a borrowed argument is retained, a
/// fresh one transferred (and released if a later argument raises).
/// Shared by user-function calls and calls of function values (`closure`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn marshal_args<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    param_tys: &[pycc_mir::Ty],
    callee_name: &str,
    args: &[MirExpr],
) -> Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> {
    let mark = rt.exceptions.pending_int_releases.borrow().len();
    let heap_mark = heap_rc::pending_mark(rt);
    let marshalled_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = args
        .iter()
        .zip(param_tys)
        .enumerate()
        .map(|(index, (a, param_ty))| {
            // Non-strict mode (#3): an object argument to a scalar parameter
            // is converted with a run-time type check (`object_coerce`).
            if object_coerce::coerces_into(a, param_ty) {
                let scalar = object_coerce::emit_coerced(
                    context,
                    builder,
                    module,
                    rt,
                    user_functions,
                    locals,
                    a,
                    param_ty,
                    &format!("{callee_name}() argument {}", index + 1),
                );
                return match scalar {
                    Scalar::Int(v) | Scalar::Bool(v) => v.into(),
                    Scalar::Float(v) => v.into(),
                    Scalar::Str(v) => v.into(),
                    // A `frozenset` or `tuple[str, ...]` copy, fresh: the
                    // parameter slot owns it.
                    Scalar::Set(v) | Scalar::List(v) => v.into(),
                    _ => unreachable!("object_coerce yields a scalar"),
                };
            }
            // Part 2 of #1387: a native argument to an `object` parameter
            // is boxed (`object_box`). The packer borrows the value and the
            // box is a new `PyObject *` reference, so none of the
            // `str`/`int` ownership steps below applies to it.
            if object_box::boxes_into(a, param_ty) {
                return object_box::emit_boxed(
                    context,
                    builder,
                    module,
                    rt,
                    user_functions,
                    locals,
                    a,
                )
                .into();
            }
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, a);
            // The callee's parameter slot owns its argument; until the call
            // takes it, a later argument's exception releases it.
            let scalar = heap_rc::retain_if_heap_duplicate(context, builder, rt, a, scalar);
            heap_rc::hold_owned_value(rt, &a.ty(), &scalar);
            let scalar = retain_if_int_duplicate_and_track_for_exception_edge(
                context, builder, rt, a, scalar,
            );
            // Push *after* the retain-and-track call: `int_temporary_word`
            // (via `push_pending_int_release_if_scalar_temporary`)
            // excludes a duplicate reference by construction, so this is
            // a no-op for that case regardless of ordering -- the
            // retain-and-track call above already pushed a *duplicate*
            // argument's retained word itself, if any. #834 is closed: a
            // borrowed/duplicate argument's extra retain is now protected
            // on this exception edge exactly like an owning argument's own
            // word already was, not merely D-180 residual 3 (which covers
            // a retain that eventually transfers to a real owner, not one
            // abandoned before transfer completes). See
            // https://github.com/rotnov/pycc/issues/834.
            push_pending_int_release_if_scalar_temporary(rt, a, &scalar);
            let scalar = coerce_scalar_to_type(context, builder, scalar, param_ty.clone());
            match scalar {
                Scalar::Generator(v) | Scalar::Closure(v) => v.into(),
                Scalar::Int(v) => v.into(),
                Scalar::Bool(v) => v.into(),
                Scalar::Float(v) => v.into(),
                Scalar::Str(v) => v.into(),
                // Pass-through, identical to `Str`'s arm directly above: a
                // `list[T]` parameter is an opaque pointer at the ABI
                // level exactly like a `str` one (`ty_to_basic_type` gives
                // both the same LLVM type), so argument marshalling needs
                // no list-specific handling at all.
                Scalar::List(v) => v.into(),
                // Pass-through, identical to `List`'s arm directly above: a
                // `dict[K, V]` parameter is an opaque pointer at the ABI
                // level exactly like a `str`/`list[T]` one
                // (`ty_to_basic_type` gives all three the same LLVM type),
                // so argument marshalling needs no dict-specific handling
                // at all.
                Scalar::Dict(v) => v.into(),
                // Pass-through, identical to `List`'s/`Dict`'s arms
                // directly above: a `set[T]` parameter is an opaque pointer
                // at the ABI level exactly like a `str`/`list[T]`/
                // `dict[K, V]` one (`ty_to_basic_type` gives all four the
                // same LLVM type), so argument marshalling needs no
                // set-specific handling at all.
                Scalar::Set(v) => v.into(),
                // Pass-through like the three arms above, but by VALUE
                // rather than by pointer (D-115): a `tuple[...]` argument
                // is an LLVM struct at the ABI level, not an opaque
                // pointer. It still needs no tuple-specific marshalling
                // code, because `BasicMetadataValueEnum` already has a
                // `StructValue` arm -- the calling convention's own
                // by-value aggregate handling is LLVM's job, not this
                // crate's.
                Scalar::Tuple(v) => v.into(),
                // Pass-through, identical to `List`'s/`Dict`'s/`Set`'s arms
                // above: a class-instance parameter (D-154, Part 1 of #375
                // -- `self`, or any other class-typed parameter a future PR
                // might add) is an opaque pointer at the ABI level exactly
                // like a `str`/`list[T]`/`dict[K, V]`/`set[T]` one.
                Scalar::Instance(v) => v.into(),
                // Pass-through by VALUE, identical in kind to `Tuple`'s arm
                // above (D-197, #763, Part 1 of #747): an `Optional[int]`
                // argument is an LLVM `{ i64, i8 }` struct at the ABI level
                // -- `coerce_scalar_to_type` immediately above already
                // built it against `param_ty`, so this arm needs no
                // further conversion, only the same `Into` `BasicMetadataValueEnum`
                // has for any `StructValue`.
                Scalar::Optional(v) => v.into(),
                // Pass-through (Part 1 of #1333): a `Ty::Object` argument
                // reaches an unannotated private helper's solver-inferred
                // `object` parameter or, since Part 1 of #1367, a parameter
                // annotated with a class a foreign import binds -- either
                // way the callee is another pycc function taking the same
                // `ptr` (a public one called from the host gets the object
                // through its `--ext` thunk instead, #1397/#1386). The
                // callee's parameter slot owns the reference: a borrowed
                // argument was retained above.
                Scalar::Object(v) => v.into(),
                // Defensive (Part 2 of #1027): passing `b` to another
                // function is a bare read of the name, which
                // `reject_memoryview_read` refuses with `C0001` for a
                // wrapper-borrowed parameter and
                // `owned_buffer_use_unsupported` for the artifact-owned
                // storage of Part 2a of #1142 (#1165). A `memoryview`
                // parameter exists only on an `--ext` export, which CPython
                // calls, never compiled code.
                Scalar::MemoryView(_) => {
                    panic!(
                        "pycc_codegen: internal error: a memoryview argument is not supported \
                         yet -- pycc_types should have refused this before codegen"
                    )
                }
            }
        })
        .collect();
    // #638 (D-208): every argument above evaluated successfully -- this
    // Rust-level line is reached unconditionally at codegen (emission)
    // time regardless of what the *compiled* program's exception-edge
    // control flow does, since `guard_statement_effects` only emits a
    // runtime branch instruction, never diverts the emitter itself (see
    // `guard_statement_effects`'s own doc comment). Truncate back to
    // `mark` *without* releasing: ownership of every entry pushed above
    // has now transferred to the callee's parameter slots via the
    // `build_call`/`build_indirect_call` below.
    rt.exceptions
        .pending_int_releases
        .borrow_mut()
        .truncate(mark);
    heap_rc::truncate_pending(rt, heap_mark);
    marshalled_args
}

/// Calls `user_function` with `arg_values`: directly when it has one
/// definition, otherwise through its function-pointer slot (issue #22).
pub(crate) fn dispatch<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    user_function: &UserFunction<'ctx>,
    arg_values: &[inkwell::values::BasicMetadataValueEnum<'ctx>],
) -> inkwell::values::CallSiteValue<'ctx> {
    let callee = resolve_callee(context, builder, rt, user_function);
    call_resolved(builder, user_function, callee, arg_values)
}

/// What a call of a user function goes to once its name has been looked up.
#[derive(Clone, Copy)]
pub(crate) enum Callee<'ctx> {
    /// A function known at compile time.
    Direct(FunctionValue<'ctx>),
    /// The pointer loaded from the function's `fnptr_` slot.
    Indirect(PointerValue<'ctx>),
}

/// Looks `user_function` up as CPython looks up the callee's name: before
/// any argument is evaluated, so a call of a function whose `def` has not
/// run yet raises `NameError` (issue #22) without evaluating an argument.
///
/// Monomorphized generic specializations (`0gen_...` names) have no
/// `fn_ptr_global` -- they are compiler-generated, not user-defined, and go
/// directly through `direct_value`. Any other function goes through its
/// function-pointer slot, whose load, null check and `NameError` path are
/// shared with #1050's `ext` export thunks
/// (`ext_thunk::emit_fnptr_dispatch_guard`, which returns with the builder
/// on the non-null path).
pub(crate) fn resolve_callee<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    user_function: &UserFunction<'ctx>,
) -> Callee<'ctx> {
    if let Some(direct_value) = user_function.direct_value {
        return Callee::Direct(direct_value);
    }
    if let Some(target) = user_function.single_def {
        let caller = builder
            .get_insert_block()
            .and_then(|block| block.get_parent())
            .expect("a call is emitted inside a function");
        if caller != target && !user_function.bound_before_code {
            ext_thunk::emit_fnptr_dispatch_guard(context, builder, rt, user_function);
        }
        return Callee::Direct(target);
    }
    Callee::Indirect(ext_thunk::emit_fnptr_dispatch_guard(
        context,
        builder,
        rt,
        user_function,
    ))
}

/// Emits the call of a [`resolve_callee`] result with `arg_values`.
pub(crate) fn call_resolved<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    user_function: &UserFunction<'ctx>,
    callee: Callee<'ctx>,
    arg_values: &[inkwell::values::BasicMetadataValueEnum<'ctx>],
) -> inkwell::values::CallSiteValue<'ctx> {
    match callee {
        Callee::Direct(function) => builder
            .build_call(function, arg_values, "call_user_fn")
            .expect("build_call should not fail for a well-formed direct call"),
        Callee::Indirect(fn_ptr) => builder
            .build_indirect_call(user_function.fn_type, fn_ptr, arg_values, "call_user_fn")
            .expect("build_indirect_call should not fail for a well-formed indirect call"),
    }
}
