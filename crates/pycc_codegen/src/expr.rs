//! Expression lowering: `emit_expr` and its per-node dispatcher.

use super::*;

// Large dispatcher arms live in per-topic modules (mechanical extraction:
// each helper re-destructures its own node and runs the arm body verbatim).
mod arith_expr;
mod buffer_expr;
mod call_expr;
mod container_expr;
mod name_read;
mod object_expr;

pub(crate) fn emit_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let value = emit_expr_unchecked(context, builder, module, rt, user_functions, locals, expr);
    // Recursive calls come back through this wrapper. Consequently a node
    // that can set the pending state stops evaluation before its parent can
    // evaluate the next operand, call argument, or other sub-expression.
    // Pure literals, reads, comparisons, and ordinary arithmetic skip the
    // TLS call and branch entirely; D-173's original unconditional guard at
    // every node made tight numeric loops several times slower.
    let nothrow_call = matches!(expr, MirExpr::Call { callee, .. } if rt.nothrow.borrow().contains(callee));
    if expression_can_set_exception(expr) && !nothrow_call {
        guard_statement_effects(context, builder, rt);
    }
    // An operand a later sibling could free takes its own reference
    // (`operand_pin`), after the guard: only a successfully read value.
    heap_rc::retain_if_pinned(context, builder, rt, expr, &value);
    value
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_expr_unchecked<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    // Tasks 3-6 only ever passed this to `emit_expr`'s own recursive calls
    // (clippy's `only_used_in_recursion` lint, part of `-D warnings`,
    // required an underscore-prefixed `_module` name for that shape). Task
    // 7's `MirExpr::StringLiteral` arm below is the first to read it
    // directly (`module.add_global`, to embed a literal's bytes as a
    // constant), so it's no longer only-used-in-recursion -- dropping the
    // underscore.
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    use pycc_mir::Ty;
    match expr {
        MirExpr::IntLiteral(n) => Scalar::Int(emit_int_constant(context, builder, rt, *n)),
        MirExpr::GenNext {
            generator,
            default,
            ty,
        } => crate::generator::emit_next(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            generator,
            default.as_deref(),
            ty,
        ),
        MirExpr::GenToList { generator, ty } => crate::generator::emit_to_list(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            generator,
            ty,
        ),
        MirExpr::MakeClosure {
            function, captures, ..
        } => crate::closure::emit_make_closure(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            function,
            captures,
        ),
        MirExpr::CallClosure { callee, args, ty } => crate::closure::emit_call_closure(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            callee,
            args,
            ty,
        ),
        MirExpr::GenYield { .. }
        | MirExpr::GenYieldFrom { .. }
        | MirExpr::GenSend { .. }
        | MirExpr::GenThrow { .. }
        | MirExpr::GenClose { .. }
        | MirExpr::StopIterationValue { .. } => crate::generator::emit_protocol_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::GenAnyAll { generator, all } => crate::generator::emit_any_all(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            generator,
            *all,
        ),
        // #1346: a receiver evaluated for its effects, then the attribute.
        MirExpr::Sequence { discard, value } => sequence::emit_sequence(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            discard,
            value,
        ),
        // #1254 (D-250): a comprehension in any expression position.
        MirExpr::Comprehension(comp) => emit_comprehension_expr(
            &CompCx {
                context,
                builder,
                module,
                rt,
                user_functions,
                locals,
            },
            comp,
        ),
        MirExpr::RtCall(call) => {
            rt_call::emit_rt_call(context, builder, module, rt, user_functions, locals, call)
        }
        MirExpr::Nd(nd) => crate::ndarray::emit_nd(context, builder, module, rt, user_functions, locals, nd),
        MirExpr::FloatLiteral(f) => Scalar::Float(context.f64_type().const_float(*f)),
        MirExpr::IntBoundary(value) => {
            let scalar = emit_expr(context, builder, module, rt, user_functions, locals, value);
            Scalar::Int(to_encoded_int(context, builder, scalar))
        }
        // Part 2 of #1387: a native value boxed into an `object` binding,
        // rebinding or attribute slot -- see `object_box`.
        MirExpr::ObjectBox(value) => Scalar::Object(object_box::emit_boxed(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            value,
        )),
        // The bare literal `None` (D-197, #763, Part 1 of #747), standing
        // alone rather than already known to be flowing into a
        // predeclared `Optional[inner]` slot -- that context-sensitive
        // wrapping happens at the assignment site
        // (`coerce_scalar_to_type`, driven by the target slot's own
        // declared type), not here, so this arm has no target `inner` type
        // to build a real `{ inner, i8 }` struct against. A minimal `{ i8,
        // i8 }` all-zero placeholder is emitted instead: `pycc_types`
        // accepts a bare `None` expression only as an `Optional[_]`
        // initializer or an `is`/`is not` operand in this PR (never
        // printed, compared for equality, or used arithmetically), and
        // both of those consumers rebuild the correctly-typed struct
        // themselves (`coerce_scalar_to_type` for assignment,
        // `MirExpr::Compare`'s `Is`/`IsNot` codegen below reads the
        // struct's `i8` flag field only, which is at the same offset
        // regardless of the payload's own width) -- see this PR's ADR.
        MirExpr::NoneLiteral => {
            let placeholder_ty = context.struct_type(&[context.i8_type().into(); 2], false);
            Scalar::Optional(placeholder_ty.const_zero())
        }
        MirExpr::NotImplemented => {
            object_expr::emit_not_implemented_expr(context, builder, module, expr)
        }
        MirExpr::OptionalWrap(..) => object_expr::emit_optional_wrap_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        // Issue #769 (Part 2 of #747): the read-side counterpart of
        // `OptionalWrap` immediately above. `pycc_types::check` (via its
        // own `narrow` overlay) has already proven this particular read is
        // reachable only when the value is present, so `value` below
        // always evaluates to a real `Scalar::Optional { payload, present }`
        // struct (never the `MirExpr::NoneLiteral` all-zero placeholder --
        // that placeholder is only ever a `coerce_scalar_to_type` target,
        // never something a narrowing test could recognize as
        // `Optional`). `T0049` (`crates/pycc_hir/src/func.rs`) restricts
        // every `Optional[T]` annotation to `T = int`, so `inner` here is
        // always `Ty::Int` and field 0 is always an already-D-141-encoded
        // int word -- extracting it and returning `Scalar::Int` needs no
        // decode/re-encode step, exactly like `Ty::Optional(_)`'s own
        // `MirExpr::Name` read arm above returns the struct as-is with no
        // transformation of its own fields.
        //
        // Refcount reasoning (bigint case, `n: int | None = <heap bigint>`
        // then `if n is not None: use(n)`): this arm performs a plain
        // *read* of `n`'s slot (via the inner `Name` -> `Ty::Optional`
        // load above), extracting the payload word without incrementing
        // any refcount -- exactly mirroring a bare (non-`Optional`) `int`
        // local's own `MirExpr::Name` read arm, which likewise loads and
        // returns `Scalar::Int` with no retain. Both are borrowed reads:
        // ownership transfer (and therefore the retain that must accompany
        // it) happens only where a value is *stored* into a new owning
        // slot -- `MirStmt::Assign`'s `retain_if_int_duplicate` call, or
        // (for the wrap direction) `OptionalWrap`'s own arm just above.
        // `use(n)` here passing the unwrapped `Scalar::Int` to a function
        // call follows the same already-correct borrowed-argument
        // convention every other bare `int` argument uses; nothing about
        // narrowing changes that convention, so no extra retain belongs in
        // this arm specifically. See `pycc_codegen::bigint_rc`'s
        // `is_owning_producer`/`int_value_is_a_duplicate_reference`
        // classification of this node (mirroring `MirExpr::Name`, not
        // `OptionalWrap`) for the corresponding compile-time classification.
        // #1476 (Part 3 of #1387): an `object` read an `isinstance` guard
        // narrowed -- see `object_unbox`.
        MirExpr::ObjectUnbox(object, inner) => {
            let object = emit_expr(context, builder, module, rt, user_functions, locals, object);
            object_unbox::emit_unboxed(context, builder, module, rt, object, inner)
        }
        MirExpr::OptionalUnwrap(..) => object_expr::emit_optional_unwrap_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::StringLiteral(s) => {
            Scalar::Str(emit_string_literal(context, builder, module, rt, s))
        }
        // D-136: `math.pi` is a compile-time float constant, not a runtime
        // value bound to any local slot -- Task 4's own thin slice emits
        // the literal immediate directly, matching the ADR's explicit "no
        // runtime call at all" design for this one symbol. `std::f64::consts::PI`
        // is Rust's own IEEE-754 double-precision constant for pi, bit-for-bit
        // the same value CPython's `math.pi` uses (both are the nearest
        // representable `f64`/C `double` to the true mathematical constant).
        MirExpr::Name {
            name,
            ty: Ty::Float,
        } if pycc_std::math_constant(name).is_some() => Scalar::Float(
            context
                .f64_type()
                .const_float(pycc_std::math_constant(name).expect("checked")),
        ),
        // PEP 634-636 (#381, PR-21): `None` singleton in a match pattern
        // comparison. `None` is not a bound variable — it is a constant
        // `i8 0` carrier (D-075), emitted inline exactly like `math.pi`
        // above rather than looked up in `locals`.
        MirExpr::Name { name, ty: Ty::None } if name == "None" => {
            Scalar::Bool(context.i8_type().const_zero())
        }
        MirExpr::Name { .. } => {
            name_read::emit_name_expr(context, builder, module, rt, locals, expr)
        }
        MirExpr::BinOp { .. } => {
            arith_expr::emit_bin_op_expr(context, builder, module, rt, user_functions, locals, expr)
        }
        MirExpr::Compare { .. } => arith_expr::emit_compare_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::CompareChain { .. } => arith_expr::emit_compare_chain_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::BoolLiteral(b) => Scalar::Bool(context.i8_type().const_int(u64::from(*b), false)),
        MirExpr::BoolOp { .. } => arith_expr::emit_bool_op_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::IfExp { .. } => {
            arith_expr::emit_if_exp_expr(context, builder, module, rt, user_functions, locals, expr)
        }
        MirExpr::Not(..) => {
            arith_expr::emit_not_expr(context, builder, module, rt, user_functions, locals, expr)
        }
        MirExpr::Call { .. } => {
            call_expr::emit_call_expr(context, builder, module, rt, user_functions, locals, expr)
        }
        MirExpr::FString(..) => call_expr::emit_f_string_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        // `[e1, e2, ...]`: a `pycc_rt_list_new` sized for the elements,
        // then one `pycc_rt_list_append` per element word, in source order
        // (`list::element_word` owns the per-element-type encoding).
        //
        // #1021: a resolved empty `[]`. Structurally `ListLiteral` with no
        // elements -- the same `pycc_rt_list_new()` object, with no
        // `append` calls after it. It is a separate arm only because the
        // variant carries its element type instead of deriving one.
        MirExpr::EmptyList(elem) => {
            Scalar::List(list::build_list_new(context, builder, rt, elem, 0))
        }
        MirExpr::EmptySet(elem) => {
            Scalar::Set(containers::build_set_new(context, builder, rt, elem))
        }
        // #1021: a resolved empty `{}`, the dict counterpart of
        // `EmptyList` immediately above.
        MirExpr::EmptyDict(pair) => Scalar::Dict(containers::build_dict_new(
            context, builder, rt, &pair.0, &pair.1,
        )),
        MirExpr::ListLiteral(..) => container_expr::emit_list_literal_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::Subscript { .. } => container_expr::emit_subscript_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::ListAppend { .. } => container_expr::emit_list_append_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::ListInsert { .. } => container_expr::emit_list_insert_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::DictLiteral(..) => container_expr::emit_dict_literal_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::DictGet { .. } => container_expr::emit_dict_get_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::SetLiteral {
            elements: _,
            ops: Some(_),
        } => container_expr::emit_set_literal_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::SetLiteral {
            elements: _,
            ops: None,
        } => container_expr::emit_set_literal_expr_2(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        // `(e1, e2, ...)` (PR-11b Task 5, D-115). The one container literal
        // in this file that calls into no `pycc_rt` constructor at all:
        // where `ListLiteral`/`DictLiteral`/`SetLiteral` each allocate a
        // heap object and then fill it, this builds a pure SSA aggregate --
        // `get_undef()` for the struct shape, then one
        // `build_insert_value` (LLVM's `insertvalue`) per element in source
        // order, each returning a *new* aggregate value rather than
        // mutating one in place. No alloca, no pointer, and no allocation
        // of the aggregate itself -- but D-107/D-124's leak-only policy
        // does apply to what the fields hold: under D-182 a borrowed `int`
        // element is retained on the way in (see the retain below) and
        // nothing releases it when the tuple dies.
        //
        // Deliberately no `build_untag_checked` per element: a tuple has no
        // runtime storage boundary to validate. Like D-141 containers, a
        // tuple field stores exactly the `Scalar` it was given. An `int`
        // element therefore goes in as its D-141 encoded word (ordinary
        // smallint, bool marker, or bigint pointer), and `MirExpr::Subscript`'s
        // tuple branch returns that word unchanged, so the two sides never
        // disagree and no conversion happens on either.
        // A boxed tuple (`containers.rs`): a tuple object whose words are
        // written in source order, each taking the reference it will own.
        MirExpr::TupleLiteral(elements)
            if pycc_hir::tuple_is_boxed(&elements.iter().map(MirExpr::ty).collect::<Vec<_>>()) =>
        {
            let elem_tys: Vec<pycc_mir::Ty> = elements.iter().map(MirExpr::ty).collect();
            let tuple = containers::build_tuple_new(context, builder, rt, &elem_tys);
            for (index, (element, elem_ty)) in elements.iter().zip(&elem_tys).enumerate() {
                let scalar = emit_expr(
                    context,
                    builder,
                    module,
                    rt,
                    user_functions,
                    locals,
                    element,
                );
                let word = list::element_word(context, builder, rt, elem_ty, element, scalar);
                containers::store_tuple_word(context, builder, tuple, index as u64, word);
            }
            Scalar::List(tuple)
        }
        MirExpr::TupleLiteral(..) => container_expr::emit_tuple_literal_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::Slice { .. } => container_expr::emit_slice_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::ListPop { .. } => container_expr::emit_list_pop_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::DictGetOrDefault { .. } => container_expr::emit_dict_get_or_default_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::SetAdd { .. } => container_expr::emit_set_add_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::Instantiate(..) => call_expr::emit_instantiate_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::AttrGet { .. } => object_expr::emit_attr_get_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        // Part 2 of #1026: the string-keyed runtime sibling of the
        // compile-time-slot `AttrGet` directly above. `foreign_attr::emit`
        // carries the whole contract, including the `NULL` check it emits,
        // the foreign failure edge that check branches to (`foreign_fail.rs`),
        // and which
        // side owns the "CPython raised" transition.
        MirExpr::ObjAttrGet { base, attr, .. } => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            let held = object_release::hold(context, module, rt, base, &base_scalar);
            let result = foreign_attr::emit(context, builder, module, rt, base_scalar, attr);
            held.release(builder, rt);
            result
        }
        MirExpr::ObjMethodCall { .. } => object_expr::emit_obj_method_call_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        // #1313: `callee(args)` on an `object`-typed name (a foreign
        // binding or a `for` loop target), or since Part 2a of #1371 on an
        // object subscript result (`table[k](args)`). CPython's order -- the
        // callee, then each argument left to right -- and no lookup step.
        // `foreign_call::emit_object_call` decides from the callee's MIR
        // shape whether it is a borrow (a name read; the shim helper takes
        // its own reference) or a produced new reference the call consumes.
        MirExpr::ObjCall { callee, args } => foreign_call_emit::emit_direct_call(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            callee,
            args,
        ),
        // Part 8 of #1371: either call above with keyword arguments.
        // `foreign_call_emit` carries the order and the ownership split.
        MirExpr::ObjKeywordCall(call) => foreign_call_emit::emit_keyword_call(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            call,
        ),
        // Part 3 of #1026 (PR 3a of #1082): `len(o)`. `foreign_len` carries
        // the contract -- why the D-141 encode happens inside the shim
        // rather than here, why that leaves exactly one failure edge, and
        // why the out-parameter's `alloca` is hoisted to the entry block.
        MirExpr::ObjLen { base } => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            let held = object_release::hold(context, module, rt, base, &base_scalar);
            let result = foreign_len::emit_len(context, builder, module, rt, base_scalar);
            held.release(builder, rt);
            result
        }
        // Part 11 of #1371: `type(o)`. `foreign_type` carries the contract.
        MirExpr::ObjType { base } => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            let held = object_release::hold(context, module, rt, base, &base_scalar);
            let result = foreign_type::emit_type(context, builder, module, rt, base_scalar);
            held.release(builder, rt);
            result
        }
        MirExpr::ObjSubscript { .. } => object_expr::emit_obj_subscript_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::ObjCompare { .. } => object_expr::emit_obj_compare_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::ObjBinOp { .. } => object_expr::emit_obj_bin_op_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::ObjUnaryOp { op, operand } => foreign_arith::emit_unary(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            *op,
            operand,
        ),
        MirExpr::ObjContains { .. } => object_expr::emit_obj_contains_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::ObjSlice { .. } => object_expr::emit_obj_slice_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::ObjList { .. } | MirExpr::ObjDisplay { .. } => object_expr::emit_obj_list_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::ObjIsInstance { .. } => object_expr::emit_obj_is_instance_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::BufferGet { .. } => buffer_expr::emit_buffer_get_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::BufferLen { .. } => buffer_expr::emit_buffer_len_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::FrozenSetFrom { .. } => container_expr::emit_frozen_set_from_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::BufferAlloc { .. } => buffer_expr::emit_buffer_alloc_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::InstanceHash { operand, via } => hash::emit_instance_hash(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            operand,
            *via,
        ),
        MirExpr::ObjUnpackFloatTuple { .. } => object_expr::emit_obj_unpack_float_tuple_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
        MirExpr::ObjUnpack { value, arity } => {
            let value_scalar =
                emit_expr(context, builder, module, rt, user_functions, locals, value);
            let held = object_release::hold(context, module, rt, value, &value_scalar);
            let result =
                foreign_unpack::emit_unpack(context, builder, module, rt, value_scalar, *arity);
            held.release(builder, rt);
            result
        }
        MirExpr::NullInstance { .. } => {
            let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
            let null_ptr = ptr_type.const_null();
            Scalar::Instance(null_ptr)
        }
        // Part 3A of #541 (#736): `pycc_mir::class::rewrite_exception_to_message`
        // is this node's sole constructor, applied only to an exception-typed
        // expression -- `base` therefore always evaluates to a
        // `Scalar::Instance` pointer to a live `PyExceptionObj`.
        MirExpr::ExceptionMessage(base) => {
            let base_scalar = emit_expr(context, builder, module, rt, user_functions, locals, base);
            emit_exception_message(builder, rt, base_scalar)
        }
        // #1337 (WI-6a): `isinstance` on a caught builtin exception value.
        MirExpr::ExceptionTypeTest { obj, tags } => {
            let obj_scalar = emit_expr(context, builder, module, rt, user_functions, locals, obj);
            exception_isinstance::emit_exception_type_test(context, builder, rt, obj_scalar, tags)
        }
        MirExpr::NamedExpr { .. } => name_read::emit_named_expr_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            expr,
        ),
    }
}
