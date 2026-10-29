//! Call, f-string and class instantiation lowering.

use super::*;
use pycc_mir::Ty;

pub(crate) fn emit_call_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::Call { callee, args, ty } = expr else {
        unreachable!()
    };
    // `print(...)` in a value position (`print(a) if c else
    // print(b)`): print, then yield its `None` result.
    if callee == pycc_mir::dispatch::INSTANCE_PY_OVERRIDE {
        return emit_instance_py_override(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            args,
        );
    }
    if callee == pycc_mir::dispatch::INSTANCE_CLASS_IS {
        return emit_instance_class_is(context, builder, module, rt, user_functions, locals, args);
    }
    if callee == "print" {
        string_render::emit_print_call(context, builder, module, rt, user_functions, locals, args);
        return crate::call_result::none_scalar(context);
    }
    // `len` is the second hand-recognized builtin (D-105 point 3),
    // dispatched here for the same reason `print` is: it has no
    // `user_functions` entry, so it must be claimed before the
    // lookup below turns it into an "undefined function" panic.
    // Mirrors `pycc_types`' own `callee == "len"` arm and
    // `pycc_mir`'s, which already type it `Ty::Int` -- so this
    // returns a `Scalar::Int` directly rather than falling through
    // to the declared-return-type dispatch at the end of this arm.
    if callee == "len" {
        let [list_arg] = args.as_slice() else {
            panic!(
                "pycc_codegen: internal error: `len` takes exactly 1 argument, got {} \
                 -- pycc_types::check (T0033) should have rejected this before codegen",
                args.len()
            )
        };
        let scalar = emit_expr(
            context,
            builder,
            module,
            rt,
            user_functions,
            locals,
            list_arg,
        );
        // D-123 relaxed `len()` to also accept a `dict[K, V]`
        // argument alongside `list[T]` (PR-11 Task 5), and PR-11
        // Task 9 relaxed it once more to also accept `set[T]`,
        // dispatched on the argument's own static type -- mirrors
        // `pycc_types`'/`pycc_mir`'s identical relaxation at their
        // own hand-recognized `len` dispatch points.
        let raw_len = match list_arg.ty() {
            Ty::Dict(_) => {
                let dict_ptr = expect_dict_pointer(scalar, "`len`'s argument");
                build_dict_len(builder, rt, dict_ptr)
            }
            Ty::Set(_) | Ty::FrozenSet(_) => {
                let set_ptr = expect_set_pointer(scalar, "`len`'s argument");
                build_int_set_len(builder, rt, set_ptr)
            }
            _ => {
                let list_ptr = expect_list_pointer(scalar, "`len`'s argument");
                list::build_list_len(context, builder, list_ptr)
            }
        };
        release_scalar_if_temporary(context, builder, rt, list_arg, &scalar);
        // The raw count becomes a user-visible
        // `Ty::Int` expression value here (`print(len(x))`,
        // `n = len(x)`), so it is re-tagged -- unlike
        // `MirStmt::ForList`'s own use of the same runtime call,
        // which keeps it raw as a private loop bound.
        return Scalar::Int(raw_i64_to_tagged_int(context, builder, raw_len));
    }
    if callee == "round" && !user_functions.contains_key(callee.as_str()) {
        return emit_round(context, builder, module, rt, user_functions, locals, args);
    }
    // `abs`/`divmod`/`int` over native numbers (`int_builtins`); an
    // `int(o)` of a CPython object keeps its own arm below.
    if let Some(scalar) = int_builtins::emit_call(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        callee,
        args,
    ) {
        return scalar;
    }
    if callee == "float" && !user_functions.contains_key(callee.as_str()) {
        // Hand-recognized builtin -- but, unlike `len` above, only when
        // no `user_functions` entry claims the name first (mirrors
        // `pycc_types`/`pycc_mir`'s own identical guard; see
        // `pycc_types::infer_expr_in`'s comment for why `float`, unlike
        // `len`/`print`, needs one). Reuses `to_float`, which already dispatches
        // `Scalar::Int`/`Scalar::Bool`/`Scalar::Float` correctly (the same
        // helper arithmetic-promotion already calls) and already panics
        // correctly for `Scalar::Str`/`List`/`Dict`/`Set`/`Tuple` -- no new conversion
        // logic, only a new caller. `pycc_types` already rejects a
        // non-numeric or mis-arity call with T0021 before codegen ever runs;
        // this arity check is a defensive backstop against malformed MIR,
        // mirroring `len`'s own internal-error convention immediately above.
        let [arg] = args.as_slice() else {
            panic!(
                "pycc_codegen: internal error: `float` takes exactly 1 argument, got {} \
                 -- pycc_types::check (T0021) should have rejected this before codegen",
                args.len()
            )
        };
        let scalar = emit_expr(context, builder, module, rt, user_functions, locals, arg);
        // Part 4 of #1026 (PR 4a of #1083): a CPython object cannot go
        // through `to_float`, which knows only pycc's own scalars and
        // panics on `Scalar::Object`. It takes the shim helper instead,
        // which runs CPython's own `PyNumber_Float` protocol -- an
        // explicit conversion the author asked for by naming the
        // destination type, not the implicit thunk-seam crossing D-244
        // rule 7 closes (see `foreign_len::emit_to_float`).
        if matches!(scalar, Scalar::Object(_)) {
            let held = object_release::hold(context, module, rt, arg, &scalar);
            let result = foreign_len::emit_to_float(context, builder, module, rt, scalar);
            held.release(builder, rt);
            return result;
        }
        if let Scalar::Num(word) = scalar {
            let value = crate::num::emit_num_to_float(context, builder, rt, word, false);
            heap_rc::release_scalar_if_temporary(context, builder, rt, arg, &scalar);
            return Scalar::Float(value);
        }
        return Scalar::Float(to_float(context, builder, rt, scalar));
    }
    // #1331: `hash(x)`, guarded like `float` above; `pycc_types`
    // admits exactly one argument. A class instance never arrives
    // here: `pycc_mir` lowers it to `MirExpr::InstanceHash` (#1335).
    if let ("hash", [arg]) = (callee.as_str(), args.as_slice())
        && !user_functions.contains_key(callee.as_str())
    {
        return hash::emit_hash(context, builder, module, rt, user_functions, locals, arg);
    }
    if callee == "bool" && !user_functions.contains_key(callee.as_str()) {
        // Part 4 of #1026 (PR 4a of #1083). `pycc_types` admits the
        // `bool` builtin for a `Ty::Object` argument *only* -- every
        // other argument type keeps its C0001 refusal -- and only when
        // no user `def bool` claims the name, which is the same guard
        // `float` above carries and for the same reason. So this arm
        // sees exactly one shape, and both backstops below are against
        // malformed MIR rather than against legitimate source.
        //
        // No new shim symbol: `bool(o)` is the truth test PR 3a already
        // emits for an `if`/`while` condition, widened from `i1` to the
        // `i8` a `Scalar::Bool` carries -- exactly what `truthy`'s own
        // `Scalar::Object` arm does.
        let [arg] = args.as_slice() else {
            panic!(
                "pycc_codegen: internal error: `bool` takes exactly 1 argument, got {} \
                 -- pycc_types::check (T0021) should have rejected this before codegen",
                args.len()
            )
        };
        let scalar = emit_expr(context, builder, module, rt, user_functions, locals, arg);
        let Scalar::Object(ptr) = scalar else {
            panic!(
                "pycc_codegen: internal error: `bool` takes a CPython object argument \
                 -- pycc_types::check (C0001) should have rejected this before codegen"
            )
        };
        let held = object_release::hold(context, module, rt, arg, &scalar);
        let bit = foreign_len::emit_truthy(context, builder, module, rt, ptr);
        held.release(builder, rt);
        return Scalar::Bool(
            builder
                .build_int_z_extend(bit, context.i8_type(), "bool_from_object")
                .expect("build_int_z_extend should not fail widening i1 to i8"),
        );
    }
    // Part 4 of #1026 (PR 4b of #1083): `int(o)` and `str(o)`. Both
    // are the `bool` arm above in every structural respect -- admitted
    // by `pycc_types` for a `Ty::Object` argument *only*, under the
    // same user-defined-function guard, so each sees exactly one shape
    // and both backstops below are against malformed MIR -- and they
    // differ only in which shim helper they reach and what it answers.
    // One arm dispatching on the emitter rather than two near-identical
    // arms, because at this layer the emission genuinely is the same
    // code; `pycc_types` is where the two names diverge, and it already
    // has.
    let object_conversion: Option<ObjectConversionEmitter> = match callee.as_str() {
        "int" => Some(foreign_len::emit_to_int),
        "str" => Some(foreign_len::emit_to_str),
        "repr" => Some(foreign_len::emit_repr),
        _ => None,
    };
    if let Some(emit) = object_conversion
        && !user_functions.contains_key(callee.as_str())
    {
        let [arg] = args.as_slice() else {
            panic!(
                "pycc_codegen: internal error: `{callee}` takes exactly 1 argument, got {} \
                 -- pycc_types::check (T0021) should have rejected this before codegen",
                args.len()
            )
        };
        let scalar = emit_expr(context, builder, module, rt, user_functions, locals, arg);
        if callee == "str"
            && let Some(text) =
                crate::optional_ptr::to_str(context, builder, module, rt, &arg.ty(), &scalar)
        {
            release_scalar_if_temporary(context, builder, rt, arg, &scalar);
            return Scalar::Str(text);
        }
        // `str(xs)` of a list (`pycc_types` admits it beside `str(o)`).
        if callee == "str" && matches!(scalar, Scalar::List(_)) {
            let text = to_str(builder, rt, scalar);
            release_scalar_if_temporary(context, builder, rt, arg, &scalar);
            return Scalar::Str(text);
        }
        if !matches!(scalar, Scalar::Object(_)) {
            panic!(
                "pycc_codegen: internal error: `{callee}` takes a CPython object argument \
                 -- pycc_types::check (C0001) should have rejected this before codegen"
            )
        }
        let held = object_release::hold(context, module, rt, arg, &scalar);
        let result = emit(context, builder, module, rt, scalar);
        held.release(builder, rt);
        return result;
    }
    // Unlike `emit_stmt`'s void-call arm below, there is no
    // `Result` here to propagate a clean, user-facing error
    // through -- `emit_expr` returns a `Scalar` unconditionally, so
    // an undefined callee can only be this crate's own internal
    // error. Real `pycc_types` already rejects any call to an
    // undefined function (T0021) long before codegen runs, so this
    // is a defensive backstop, not a rejection of legitimate
    // source (see `calling_an_undefined_function_as_a_nested_
    // expression_is_an_internal_error` below).
    let user_function = user_functions.get(callee.as_str()).unwrap_or_else(|| {
        panic!(
            "pycc_codegen: internal error: call to undefined function `{callee}` \
             should have been rejected by pycc_types before reaching codegen"
        )
    });
    let _ = user_function; // validated above; build_call_to looks up by name
    let call_site = build_call_to(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        callee,
        args,
    );
    crate::call_result::call_result_scalar(context, call_site, ty)
}

/// A dispatcher's run-time class test (`pycc_mir::dispatch`):
/// `pycc_rt_instance_class_is(recv, name, len)` against the class name's
/// bytes, interned once per name.
fn emit_instance_class_is<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    args: &[MirExpr],
) -> Scalar<'ctx> {
    let [receiver, MirExpr::StringLiteral(class)] = args else {
        panic!("pycc_codegen: internal error: a class test takes a receiver and a class name")
    };
    let Scalar::Instance(receiver) =
        emit_expr(context, builder, module, rt, user_functions, locals, receiver)
    else {
        panic!("pycc_codegen: internal error: a class test's receiver is an instance")
    };
    // One load and one compare against the class's record
    // (`crate::class_record`): its layout pointer is the instance's class.
    if let Some(record) = rt.class_records.borrow().get(class.as_str()).copied() {
        let ptr = context.ptr_type(inkwell::AddressSpace::default());
        let layout = builder
            .build_load(ptr, receiver, "class_is_layout")
            .expect("a layout load is well formed")
            .into_pointer_value();
        let same = builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                builder
                    .build_ptr_to_int(layout, context.i64_type(), "class_is_have")
                    .expect("a pointer cast is well formed"),
                builder
                    .build_ptr_to_int(record.descriptor(context), context.i64_type(), "class_is_want")
                    .expect("a pointer cast is well formed"),
                "class_is_same",
            )
            .expect("a pointer comparison is well formed");
        return Scalar::Bool(
            builder
                .build_int_z_extend(same, context.i8_type(), "class_is_i8")
                .expect("a zero extension is well formed"),
        );
    }
    let global_name = format!("pycc_dispatch_class.{class}");
    let len = context.i64_type().const_int(class.len() as u64, false);
    let name = match module.get_global(&global_name) {
        Some(existing) => existing.as_pointer_value(),
        None => {
            crate::exception_value::emit_str_bytes_constant(context, module, class, &global_name).0
        }
    };
    let answer = builder
        .build_call(
            rt.instance_class_is,
            &[receiver.into(), name.into(), len.into()],
            "class_is",
        )
        .expect("build_call should not fail for a well-formed class test")
        .try_as_basic_value()
        .expect_basic("pycc_rt_instance_class_is returns a value")
        .into_int_value();
    let zero = context.i8_type().const_zero();
    let truth = builder
        .build_int_compare(inkwell::IntPredicate::NE, answer, zero, "class_is_bool")
        .expect("an i8 comparison is well formed");
    Scalar::Bool(
        builder
            .build_int_z_extend(truth, context.i8_type(), "class_is_i8")
            .expect("a zero extension is well formed"),
    )
}

/// The byte offset of an instance's carrier word (`pycc_rt::instance`'s
/// `PyInstanceObj::carrier`, after the descriptor pointer and length),
/// whose low bit flags an instance a Python subclass's object carries.
const INSTANCE_CARRIER_OFFSET: u64 = 16;

/// A dispatcher's Python-override test (`pycc_mir::dispatch`): the
/// carrier word's strong flag, read inline, and only when it is set the
/// shim's `pycc_ext_instance_py_override(recv, "member")`.
fn emit_instance_py_override<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    args: &[MirExpr],
) -> Scalar<'ctx> {
    let [receiver, MirExpr::StringLiteral(member)] = args else {
        panic!("pycc_codegen: internal error: an override test takes a receiver and a name")
    };
    let Scalar::Instance(receiver) =
        emit_expr(context, builder, module, rt, user_functions, locals, receiver)
    else {
        panic!("pycc_codegen: internal error: an override test's receiver is an instance")
    };
    let i8_type = context.i8_type();
    let i64_type = context.i64_type();
    let word_ptr = unsafe {
        builder
            .build_gep(
                i8_type,
                receiver,
                &[i64_type.const_int(INSTANCE_CARRIER_OFFSET, false)],
                "carrier_word_ptr",
            )
            .expect("a byte offset into an instance is well formed")
    };
    let word = builder
        .build_load(i64_type, word_ptr, "carrier_word")
        .expect("the carrier word loads")
        .into_int_value();
    let flag = builder
        .build_and(word, i64_type.const_int(1, false), "strong_flag")
        .expect("a mask is well formed");
    let strong = builder
        .build_int_compare(inkwell::IntPredicate::NE, flag, i64_type.const_zero(), "strong")
        .expect("a comparison is well formed");
    let function = builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("an override test is emitted inside a function");
    let entry = builder.get_insert_block().expect("a current block");
    let slow = context.append_basic_block(function, "py_override");
    let join = context.append_basic_block(function, "py_override_done");
    builder
        .build_conditional_branch(strong, slow, join)
        .expect("a branch is well formed");
    builder.position_at_end(slow);
    let global_name = format!("pycc_dispatch_member.{member}");
    let name = match module.get_global(&global_name) {
        Some(existing) => existing.as_pointer_value(),
        None => {
            let bytes = context.const_string(member.as_bytes(), true);
            let global = module.add_global(bytes.get_type(), None, &global_name);
            global.set_initializer(&bytes);
            global.set_constant(true);
            global.set_linkage(inkwell::module::Linkage::Private);
            global.as_pointer_value()
        }
    };
    let helper = module
        .get_function("pycc_ext_instance_py_override")
        .unwrap_or_else(|| {
            let ptr_type = context.ptr_type(inkwell::AddressSpace::default());
            module.add_function(
                "pycc_ext_instance_py_override",
                context
                    .i32_type()
                    .fn_type(&[ptr_type.into(), ptr_type.into()], false),
                None,
            )
        });
    let found = builder
        .build_call(helper, &[receiver.into(), name.into()], "py_override")
        .expect("a call is well formed")
        .try_as_basic_value()
        .expect_basic("pycc_ext_instance_py_override returns an int")
        .into_int_value();
    let found = builder
        .build_int_compare(
            inkwell::IntPredicate::NE,
            found,
            context.i32_type().const_zero(),
            "py_override_bool",
        )
        .expect("a comparison is well formed");
    let slow_end = builder.get_insert_block().expect("a current block");
    builder
        .build_unconditional_branch(join)
        .expect("a branch is well formed");
    builder.position_at_end(join);
    let phi = builder
        .build_phi(context.bool_type(), "py_override_phi")
        .expect("a phi is well formed");
    phi.add_incoming(&[(&context.bool_type().const_zero(), entry), (&found, slow_end)]);
    Scalar::Bool(
        builder
            .build_int_z_extend(phi.as_basic_value().into_int_value(), i8_type, "py_override_i8")
            .expect("a zero extension is well formed"),
    )
}

pub(crate) fn emit_f_string_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::FString(parts) = expr else {
        unreachable!()
    };
    // Two deviations from the task brief, both here:
    //
    // 1. The brief's own version of this arm's `str_concat` call
    //    ended `.try_as_basic_value().left().expect(msg)` -- same
    //    `.left()`-doesn't-exist-on-this-inkwell-version issue
    //    documented on `to_str` and `BinOp` above, fixed the same
    //    way (`.expect_basic(msg)`).
    // 2. The brief's own `Literal`-part case called `emit_expr`
    //    recursively on a synthetic `MirExpr::StringLiteral(s.
    //    clone())` and then defensively pattern-matched the
    //    `Scalar::Str` back out with a `let ... else {
    //    unreachable!(...) }`. That `unreachable!()` arm is
    //    *provably* dead code (confirmed by `cargo llvm-cov`: an
    //    uncovered region under D-014's 100% gate) -- `MirExpr::
    //    StringLiteral` always evaluates to `Scalar::Str`, nothing
    //    else. Fixed by calling `emit_string_literal` (extracted
    //    from `emit_expr`'s own `StringLiteral` arm) directly,
    //    which returns the `PointerValue` itself with no `Scalar`
    //    round-trip and so no impossible arm to cover -- see that
    //    helper's own doc comment.
    let mut acc: Option<PointerValue<'ctx>> = None;
    // The text so far is released if a later part raises.
    let mark = heap_rc::pending_mark(rt);
    for part in parts {
        heap_rc::truncate_pending(rt, mark);
        if let Some(prev) = acc {
            heap_rc::hold_owned(rt, heap_rc::HeapKind::Str, prev);
        }
        let part_str = match part {
            pycc_mir::MirFStringPart::Literal(s) => {
                emit_string_literal(context, builder, module, rt, s)
            }
            pycc_mir::MirFStringPart::Interpolation(inner) => {
                string_render::emit_fstring_interpolation(
                    context,
                    builder,
                    module,
                    rt,
                    user_functions,
                    locals,
                    inner,
                )
            }
        };
        acc = Some(match acc {
            None => part_str,
            Some(prev) => {
                let joined = builder
                    .build_call(
                        rt.str_concat,
                        &[prev.into(), part_str.into()],
                        "fstring_concat",
                    )
                    .expect("build_call should not fail for a well-formed concatenation")
                    .try_as_basic_value()
                    .expect_basic("pycc_rt_str_concat returns a non-void pointer")
                    .into_pointer_value();
                builder
                    .build_call(rt.str_decref, &[prev.into()], "fstring_decref_prev")
                    .expect("build_call should not fail for a well-formed decref");
                builder
                    .build_call(rt.str_decref, &[part_str.into()], "fstring_decref_part")
                    .expect("build_call should not fail for a well-formed decref");
                joined
            }
        });
    }
    heap_rc::truncate_pending(rt, mark);
    Scalar::Str(acc.unwrap_or_else(|| {
        // An empty `FString(vec![])` never actually reaches this arm --
        // `pycc_hir`'s own f-string lowering always produces at least
        // one `Literal` part (an empty Python f-string `f""` still
        // lowers to `FString(vec![FStringPart::Literal("")])`, never a
        // truly empty `Vec`) -- but guard it explicitly rather than
        // silently returning a dangling/null pointer if that assumption
        // is ever wrong.
        panic!("pycc_codegen: internal error: an f-string with zero parts should not be reachable")
    }))
}

// `ClassName(args)` (D-154, Part 1 of #375): allocate a fresh
// instance whose slots are all unassigned (`pycc_rt_instance_new`,
// given the class's own already-resolved slot count and, since
// #1388, its interned layout descriptor), then call the
// mangled `__init__` with that pointer as `self`, followed by
// `args` -- see `MirExpr::Instantiate`'s own doc comment for why
// this needs `build_call_to_with_leading_args` rather than the
// ordinary `build_call_to` every ordinary/method call above uses.
pub(crate) fn emit_instantiate_expr<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    expr: &MirExpr,
) -> Scalar<'ctx> {
    let MirExpr::Instantiate(inst) = expr else {
        unreachable!()
    };
    let pycc_mir::InstantiateExpr {
        ctor,
        class_name,
        slot_names,
        args,
        ..
    } = inst.as_ref();
    let count = context.i64_type().const_int(slot_names.len() as u64, false);
    // The class's record (`crate::class_record`): its descriptor's address
    // is the instance's run-time class.
    let record = rt
        .class_records
        .borrow()
        .get(class_name.as_str())
        .copied()
        .filter(|record| record.slots() == slot_names.len());
    let (layout, layout_len) = match record {
        Some(record) => (record.descriptor(context), record.len(context)),
        None => attr_slot::instance_layout_constant(
            context,
            module,
            class_name,
            slot_names,
            match &inst.ty {
                Ty::Instance(class) => rt.rc_class_kinds.borrow().get(class.as_str()).cloned(),
                _ => None,
            }
            .filter(|kinds| kinds.len() == slot_names.len())
            .as_deref(),
        ),
    };
    let instance_ptr = builder
        .build_call(
            rt.instance_new,
            &[count.into(), layout.into(), layout_len.into()],
            "instance_new",
        )
        .expect("build_call should not fail for a well-formed instance allocation")
        .try_as_basic_value()
        .expect_basic("pycc_rt_instance_new returns a non-void pointer")
        .into_pointer_value();
    let ctor_function = user_functions.get(ctor.as_str()).unwrap_or_else(|| {
        panic!(
            "pycc_codegen: internal error: constructor `{ctor}` should have been \
             registered as an ordinary user function -- pycc_hir::class::lower_class \
             mangles every method, including `__init__`, into HirModule::items"
        )
    });
    // The new instance is released if an argument or `__init__`
    // raises: the guard runs while it is still held.
    let mark = heap_rc::pending_mark(rt);
    let counted = heap_rc::heap_kind(rt, &inst.ty).is_some();
    if counted {
        heap_rc::hold_owned(rt, heap_rc::HeapKind::Instance, instance_ptr);
    }
    build_call_to_with_leading_args(
        context,
        builder,
        module,
        rt,
        user_functions,
        locals,
        ctor_function,
        ctor,
        &[instance_ptr.into()],
        args,
    );
    if counted {
        guard_statement_effects(context, builder, rt);
    }
    heap_rc::truncate_pending(rt, mark);
    Scalar::Instance(instance_ptr)
}
