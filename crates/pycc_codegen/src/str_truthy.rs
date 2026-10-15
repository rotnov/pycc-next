//! String conversion (`to_str`), string literal construction and truthiness.

use super::*;

/// Converts any scalar to a fresh, owned `str` object matching CPython's
/// `str(x)` for that value (Task 8) -- reused unchanged by Task 10's
/// `print`. `str` itself passes through (already a `str`, and per this
/// function's own contract already an owned reference by the time it gets
/// here -- see `emit_expr`'s `FString` arm, the only caller); every other
/// type goes through its own `pycc_rt_*_to_str` conversion.
///
/// Two deviations from the task brief, both here:
///
/// 1. The brief's own version of this function took a `context: &'ctx
///    Context` parameter (matching `to_numeric_encoded_int`/`to_float`'s own
///    shape) -- but no branch below actually needs it: unlike `to_float`'s
///    `Bool` case (which builds a real `context`-dependent conversion
///    instruction), every branch here only ever picks which already-built
///    `RtFns` field and value to call `build_call` with, and `build_call`
///    itself needs no `Context` at all. An unused parameter would fail
///    this crate's `-D warnings` (`unused_variables` applies to
///    parameters, not just local bindings). Dropped rather than
///    underscore-prefixed, since it's not merely unused in *this* body but
///    structurally unneeded by this function's own job -- matching this
///    file's established convention of removing what's provably
///    unnecessary rather than working around it (see e.g. `emit_expr`'s
///    own `module`-parameter history note).
/// 2. The brief's own version of this function's final `build_call` ended
///    `.try_as_basic_value().left().expect(msg)` -- but (per the doc
///    comment on `emit_expr`'s `BinOp` arm) this inkwell version's
///    `try_as_basic_value()` returns its own `ValueKind` enum, not
///    `either::Either`, so `.left()` doesn't exist on it and the brief's
///    code as written doesn't compile. Fixed to `.expect_basic(msg)`, the
///    same fix already applied throughout this file (e.g.
///    `build_float_rt_binop` above).
pub(crate) fn to_str<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    scalar: Scalar<'ctx>,
) -> PointerValue<'ctx> {
    let (rt_fn, arg): (FunctionValue<'ctx>, inkwell::values::BasicMetadataValueEnum) = match scalar
    {
        // `<generator object g at 0x...>`, `<function f at 0x...>`.
        Scalar::Generator(v) => (rt.gens.repr, v.into()),
        Scalar::Closure(v) => (rt.closures.repr, v.into()),
        Scalar::Str(v) => return v,
        Scalar::Int(v) => (rt.int_to_str, v.into()),
        Scalar::Float(v) => (rt.float_to_str, v.into()),
        Scalar::Bool(v) => (rt.bool_to_str, v.into()),
        // `str(xs)`: the runtime renders every element kind.
        Scalar::List(v) => (rt.list_repr, v.into()),
        // `str(d)`: the runtime renders every key and value kind.
        Scalar::Dict(v) => (rt.dict_repr, v.into()),
        // `str(s)`: CPython's element order (`pycc_rt::set`); the object
        // knows whether it is a frozenset.
        Scalar::Set(v) => {
            let context = rt.context;
            return builder
                .build_call(
                    rt.containers.set_repr,
                    &[v.into(), context.i8_type().const_zero().into()],
                    "set_repr",
                )
                .expect("build_call should not fail for a well-formed set repr")
                .try_as_basic_value()
                .expect_basic("pycc_rt_set_repr returns a pointer")
                .into_pointer_value();
        }
        // A struct tuple: boxed for the runtime's `repr`, then released.
        Scalar::Tuple(tuple) => return containers::struct_tuple_repr(builder, rt, tuple),
        // #378 (PR-18): a class instance with a `__repr__` method is
        // converted to a string at the MIR level -- `rewrite_instance_to_repr`
        // in `pycc_mir` rewrites instance-typed f-string interpolations and
        // `print` arguments to `__repr__` calls, so the codegen's `to_str`
        // receives a `str` scalar from the `__repr__` call, never an
        // `Instance` scalar. Since #977 (D-237) `pycc_types` rejects every
        // `print` argument and f-string interpolation that neither that
        // rewrite nor `rewrite_exception_to_message` renders with `C0001`,
        // so no type-checked program reaches this arm; it stays as
        // defence-in-depth for a hand-built MIR (see `tests.rs`) and
        // panics honestly, matching the pre-#378 behavior.
        Scalar::Instance(_) => {
            panic!(
                "pycc_codegen: string conversion of a class instance without `__repr__` is not supported yet"
            )
        }
        // `str()` of an `Optional[int|float|bool]`: `"None"` or the
        // payload's text, by the payload's representation.
        Scalar::Optional(v) => {
            let payload = builder
                .build_extract_value(v, 0, "opt_payload")
                .expect("build_extract_value should not fail on an Optional's field 0");
            let present = builder
                .build_extract_value(v, 1, "opt_present")
                .expect("build_extract_value should not fail on an Optional's field 1");
            let which = match payload {
                inkwell::values::BasicValueEnum::FloatValue(_) => 1,
                inkwell::values::BasicValueEnum::IntValue(i)
                    if i.get_type().get_bit_width() == 8 =>
                {
                    2
                }
                _ => 0,
            };
            return builder
                .build_call(
                    rt.optional_to_str[which],
                    &[payload.into(), present.into()],
                    "opt_str",
                )
                .expect("build_call should not fail for an Optional's str()")
                .try_as_basic_value()
                .expect_basic("pycc_rt_optional_*_to_str returns a pointer")
                .into_pointer_value();
        }
        // Defensive rather than a reachable feature gap, unlike the
        // container arms above: since #1340 both callers convert a CPython
        // object *before* reaching here -- `print` through the shim's
        // `PyObject_Str` helper in its write phase and an f-string
        // interpolation through its `PyObject_Format` helper
        // (`string_render.rs`) -- because rendering a foreign object needs a
        // `pycc_ext_obj_*` shim call and its failure edge, neither of which
        // this plain runtime-call helper can emit.
        Scalar::Object(_) => {
            panic!(
                "pycc_codegen: internal error: string conversion of a CPython object value must \
                 go through the shim -- string_render should have converted it first"
            )
        }
        // Defensive, unlike the `List`/`Dict` arms above and like
        // `Object`'s: `print(b)` and `f"{b}"` are both bare reads of a
        // `memoryview`-bound name, refused with `C0001` before codegen runs
        // by `reject_memoryview_read` for a wrapper-borrowed parameter and
        // by `owned_buffer_use_unsupported` for the artifact-owned storage
        // Part 2a of #1142 (#1165) added. Part 2 of #1027 admits `b[i]`,
        // #1116 `len(b)` and Part 1 of #1142 `b[i] = v`; nothing else.
        Scalar::MemoryView(_) => {
            panic!(
                "pycc_codegen: internal error: string conversion of a memoryview value is not \
                 supported yet -- pycc_types should have refused this before codegen"
            )
        }
    };
    builder
        .build_call(rt_fn, &[arg], "to_str")
        .expect("build_call should not fail for a well-formed conversion")
        .try_as_basic_value()
        .expect_basic("every pycc_rt_*_to_str function returns a non-void pointer")
        .into_pointer_value()
}

/// Builds a `str` object from a compile-time literal's bytes (Task 7),
/// embedding them as a private constant global and calling
/// `pycc_rt_str_from_literal` -- never null-terminated, since a Python
/// `str` can contain an embedded `\0`, so the byte length is always passed
/// explicitly rather than relying on termination.
///
/// Extracted (Task 8) out of `emit_expr`'s own `MirExpr::StringLiteral` arm
/// below so `MirExpr::FString`'s `Literal`-part case can build the exact
/// same `str` object directly, as a `PointerValue`, without going through
/// `emit_expr`'s general `&MirExpr -> Scalar` dispatch just to immediately
/// pattern-match the `Scalar::Str` back out again. That defensive
/// pattern-match (the task brief's own version used a `let Scalar::Str(ptr)
/// = emit_expr(..., &MirExpr::StringLiteral(s.clone())) else { unreachable!
/// (...) }`) is *provably* dead code given this function's own contract --
/// `MirExpr::StringLiteral` always evaluates to `Scalar::Str`, unconditionally,
/// nothing else ever calls this helper with a different result type -- and
/// `cargo llvm-cov`'s region coverage confirmed it empirically: that
/// `unreachable!()` arm's region never executed under any test, an
/// uncovered region under this project's 100%-line-and-region coverage gate
/// (D-014). Removed by sharing this typed-`PointerValue`-returning helper
/// instead, the same "no redundant impossible-to-cover branch" convention
/// this file already applies elsewhere (see `emit_expr`'s `Name` arm's own
/// doc comment).
pub(crate) fn emit_string_literal<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    s: &str,
) -> PointerValue<'ctx> {
    let bytes = s.as_bytes();
    let array_ty = context.i8_type().array_type(bytes.len() as u32);
    let global = module.add_global(array_ty, None, "str_lit");
    global.set_initializer(&context.const_string(bytes, false));
    global.set_constant(true);
    global.set_linkage(Linkage::Private);
    let ptr = global.as_pointer_value();
    let len = context.i64_type().const_int(bytes.len() as u64, false);
    // The site's object, built on its first evaluation
    // (`pycc_rt_str_from_literal_cached`).
    let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
    let slot = module.add_global(ptr_ty, None, "str_lit_cache");
    slot.set_initializer(&ptr_ty.const_null());
    slot.set_linkage(Linkage::Private);
    // Every evaluation after the first reads the cached object: it is
    // immortal (`pycc_rt::str_index::IMMORTAL`), so the reference this
    // expression stands for needs no count of its own.
    let cached = builder
        .build_load(ptr_ty, slot.as_pointer_value(), "str_lit_cached")
        .expect("build_load should not fail for a literal's cache slot")
        .into_pointer_value();
    let built = builder
        .build_is_null(cached, "str_lit_unbuilt")
        .expect("build_is_null should not fail for a pointer");
    let function = builder
        .get_insert_block()
        .and_then(|block| block.get_parent())
        .expect("a string literal is emitted inside a function");
    let entry = builder.get_insert_block().expect("the builder is positioned");
    let build_block = context.append_basic_block(function, "str_lit_build");
    let join = context.append_basic_block(function, "str_lit_join");
    let branch = builder
        .build_conditional_branch(built, build_block, join)
        .expect("build_conditional_branch should not fail for a literal's cache test");
    crate::int_fast::weight_branch(context, branch, false);
    builder.position_at_end(build_block);
    let fresh = builder
        .build_call(
            rt.str_from_literal,
            &[ptr.into(), len.into(), slot.as_pointer_value().into()],
            "str_lit_obj",
        )
        .expect("build_call should not fail for a well-formed string literal construction")
        .try_as_basic_value()
        .expect_basic("pycc_rt_str_from_literal returns a non-void pointer")
        .into_pointer_value();
    let built_end = builder.get_insert_block().expect("the builder is positioned");
    builder
        .build_unconditional_branch(join)
        .expect("build_unconditional_branch should not fail for a literal's join");
    builder.position_at_end(join);
    let phi = builder
        .build_phi(ptr_ty, "str_lit")
        .expect("build_phi should not fail for a pointer");
    phi.add_incoming(&[(&cached, entry), (&fresh, built_end)]);
    phi.as_basic_value().into_pointer_value()
}

/// Turns any supported `Scalar` into an LLVM `i1` for use as a `br`
/// condition -- the shared truthiness check behind `if`/`while` (Task 4),
/// now including `str` (Task 7): `False` only for the empty string,
/// delegated to `pycc_rt_str_truthy` (D-059's representation is opaque to
/// this crate).
/// `module` is threaded in for the `Scalar::Object` arm alone (Part 3 of
/// #1026, PR 3a of #1082): that arm declares and calls the shim's
/// `pycc_ext_obj_truthy`, and inkwell 0.9's `FunctionValue` exposes no
/// `get_parent`, so the LLVM module cannot be recovered from the builder's
/// own insertion point the way the enclosing *function* can.
pub(crate) fn truthy<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    scalar: Scalar<'ctx>,
) -> inkwell::values::IntValue<'ctx> {
    let as_i8 = match scalar {
        // A generator object is always truthy.
        Scalar::Generator(_) | Scalar::Closure(_) => context.i8_type().const_int(1, false),
        Scalar::Bool(v) => v,
        Scalar::Int(v) => builder
            .build_call(rt.int_truthy, &[v.into()], "int_truthy")
            .expect("build_call should not fail for a well-formed truthiness check")
            .try_as_basic_value()
            .expect_basic("pycc_rt_int_truthy returns a non-void i8")
            .into_int_value(),
        // Python's `bool(x)` for a `float` is `False` only for exactly
        // `0.0`/`-0.0` -- `NaN` is truthy -- so this needs the
        // *unordered*-or-not-equal predicate, `UNE`, not `ONE` (same
        // distinction as the `Compare` arm's `NotEq` predicate above).
        Scalar::Float(v) => {
            let zero = context.f64_type().const_float(0.0);
            let cond = builder
                .build_float_compare(FloatPredicate::UNE, v, zero, "float_truthy")
                .expect("build_float_compare should not fail for two f64 operands");
            builder
                .build_int_z_extend(cond, context.i8_type(), "bool_from_float_truthy")
                .expect("build_int_z_extend should not fail widening i1 to i8")
        }
        Scalar::Str(v) => builder
            .build_call(rt.str_truthy, &[v.into()], "str_truthy")
            .expect("build_call should not fail for a well-formed truthiness check")
            .try_as_basic_value()
            .expect_basic("pycc_rt_str_truthy returns a non-void i8")
            .into_int_value(),
        // A list is truthy exactly when it is non-empty.
        Scalar::List(ptr) => list::list_truthy(context, builder, ptr),
        // A dict is truthy exactly when it is non-empty.
        Scalar::Dict(ptr) => {
            let len = builder
                .build_call(rt.dict_len, &[ptr.into()], "dict_len")
                .expect("build_call should not fail for pycc_rt_dict_len")
                .try_as_basic_value()
                .expect_basic("pycc_rt_dict_len returns i64")
                .into_int_value();
            let nonzero = builder
                .build_int_compare(
                    inkwell::IntPredicate::NE,
                    len,
                    context.i64_type().const_zero(),
                    "dict_truthy",
                )
                .expect("build_int_compare should not fail");
            builder
                .build_int_z_extend(nonzero, context.i8_type(), "dict_truthy_i8")
                .expect("build_int_z_extend should not fail")
        }
        // Part 1 of #1319: a `set[int]`/`frozenset[int]` value is truthy
        // exactly when it is non-empty, which its runtime length answers.
        Scalar::Set(ptr) => frozenset::set_truthy(context, builder, rt, ptr),
        // A real, reachable feature gap -- but NOT "identical in kind" to
        // the panicking `List`/`Dict` arms above in one respect (the `Set`
        // arm stopped panicking when Part 1 of #1319 gave it real
        // truthiness): `list[T]`'s own `if xs:`/`while xs:` reachability
        // predates this whole PR-11 effort entirely (established back in
        // PR-10, D-107); `dict`'s own reachability, while more recent (PR-11a's own
        // HIR literal lowering), was already in place before this PR
        // (PR-11b) started -- neither is something this PR's own diff
        // turned from a clean diagnostic into a panic. `tuple[...]`'s
        // reachability here IS exactly that: it is new as of this PR's own
        // Task 2 (`HirExpr::TupleLiteral` lowering, `crates/pycc_hir/src/
        // lib.rs`) -- before that commit, any program containing a tuple
        // literal failed to lower at all and got a clean `C0001`
        // ("expression kind not supported yet") diagnostic instead of ever
        // reaching this function. `pycc_types` places no type restriction
        // on an `if`/`while` condition, so `if t:` for a `tuple[...]` local
        // type-checks today and lands here. D-116 ships only construction
        // and literal-index reads, so v0.2 has no tuple truthiness
        // semantics -- and CPython's own rule (a tuple is falsey only when
        // empty) is not derivable from this representation for free
        // anyway, since D-116 admits no empty tuple in the first place.
        // Panics honestly rather than guessing. See the D-116
        // deferred-capability list in
        // `docs/decisions/D-116-tuple-v0-2-scope-int-bool-float-elements-only.md`
        // and `docs/ROADMAP.md`'s matching follow-up for this
        // new-as-of-PR-11b reachability.
        Scalar::Tuple(_) => {
            panic!("pycc_codegen: truthiness of a tuple[...] value is not supported yet")
        }
        // Unlike every container arm above, a class instance's truthiness
        // (D-154, Part 1 of #375) needs no runtime call and no honest-panic
        // deferral: this PR ships no `__bool__`/`__len__` (both explicitly
        // out of scope), so CPython's own default-object rule applies
        // unconditionally -- `bool(x)` is `True` for any instance with
        // neither, with no data to inspect and no way for it to ever be
        // `False`. A plain constant `1` correctly implements that rule
        // rather than deferring it.
        Scalar::Instance(_) => context.i8_type().const_int(1, false),
        // `Optional[int | float | bool]` (D-197, #763, Part 1 of #747;
        // widened to `float`/`bool` by #809): unlike every
        // container/instance arm above, this one is real rather than a
        // deferred panic -- CPython's `bool(x)` for `x: T | None` is
        // `False` for `None` and otherwise `bool(<the T value>)` (so a
        // present-but-zero/false payload is still falsy), and both halves
        // are directly readable from this representation: field 1 is the
        // present/absent flag, and field 0 (when present) is a payload
        // whose own truthiness this function already knows how to compute
        // for every inner type `T0049` admits -- `Ty::Int`'s D-141-encoded
        // `int` via `pycc_rt_int_truthy` (`Scalar::Int`'s own arm above),
        // `Ty::Float`'s plain `f64` via the *unordered*-not-equal-to-zero
        // compare (`Scalar::Float`'s own arm above, mirrored exactly
        // below), and `Ty::Bool`'s plain `i8` `0`/`1`, which already *is*
        // its own truthiness with no conversion needed (`Scalar::Bool`'s
        // own arm above).
        //
        // This function receives no `Ty` for the inner type -- `Scalar::
        // Optional` carries only the LLVM struct value -- so the inner
        // type is recovered from field 0's own LLVM type instead, which
        // `ty_to_basic_type`'s `Ty::Optional` arm makes unambiguous per
        // inner type: `f64` for `Ty::Float`, `i64` for `Ty::Int`, `i8` for
        // `Ty::Bool`. Every producer of a `Scalar::Optional` reaching this
        // function (`coerce_scalar_to_type`, `OptionalWrap`'s codegen, a
        // `Name` read of an `Optional`-typed local) already builds the
        // struct with the target's own inner-typed payload, so this
        // dispatch is exact, not a heuristic.
        Scalar::Optional(v) => {
            let present = builder
                .build_extract_value(v, 1, "opt_present")
                .expect("build_extract_value should not fail reading field 1 of a 2-field struct")
                .into_int_value();
            let payload = builder
                .build_extract_value(v, 0, "opt_payload")
                .expect("build_extract_value should not fail reading field 0 of a 2-field struct");
            let payload_truthy = match payload {
                inkwell::values::BasicValueEnum::FloatValue(fv) => {
                    // `Ty::Float` inner: mirrors `Scalar::Float`'s own arm
                    // above exactly (same `UNE` predicate, same
                    // zero-extend), rather than inventing a new mechanism.
                    let zero = context.f64_type().const_float(0.0);
                    let cond = builder
                        .build_float_compare(
                            FloatPredicate::UNE,
                            fv,
                            zero,
                            "opt_payload_float_truthy",
                        )
                        .expect("build_float_compare should not fail for two f64 operands");
                    builder
                        .build_int_z_extend(
                            cond,
                            context.i8_type(),
                            "opt_payload_bool_from_float_truthy",
                        )
                        .expect("build_int_z_extend should not fail widening i1 to i8")
                }
                // `i8`-width payload: `Ty::Bool` inner. Already the exact
                // `0`/`1` truthiness value -- `Scalar::Bool`'s own arm
                // above (`Scalar::Bool(v) => v`) treats a bare bool
                // identically, with no runtime call.
                inkwell::values::BasicValueEnum::IntValue(iv)
                    if iv.get_type().get_bit_width() == 8 =>
                {
                    iv
                }
                // Any other int width: `Ty::Int` inner, a D-141-encoded
                // `i64` -- the pre-existing behavior, delegated to
                // `pycc_rt_int_truthy` exactly like `Scalar::Int`'s own arm
                // above.
                inkwell::values::BasicValueEnum::IntValue(iv) => builder
                    .build_call(rt.int_truthy, &[iv.into()], "opt_payload_truthy")
                    .expect("build_call should not fail for a well-formed truthiness check")
                    .try_as_basic_value()
                    .expect_basic("pycc_rt_int_truthy returns a non-void i8")
                    .into_int_value(),
                // Deliberately printing the payload's *type* rather than the
                // payload value itself (e.g. via `{other:?}`, which reaches
                // inkwell's `Value::Debug` impl and calls
                // `LLVMPrintValueToString`): either printer returns an
                // `inkwell::support::LLVMString`, whose `Drop` impl calls
                // `LLVMDisposeMessage` and crashes on Windows for this LLVM
                // release (D-029) unless routed through
                // `llvm_string_to_owned`. Printing the type instead of the
                // value additionally avoids handing the printer a value
                // freshly extracted from a constant struct inside a
                // function whose block is not yet terminated (see PR #812).
                other => panic!(
                    "pycc_codegen: internal error: an Optional[_] payload had an unsupported LLVM representation ({}) -- pycc_types::check (T0049) should have rejected this inner type before codegen",
                    llvm_string_to_owned(other.get_type().print_to_string())
                ),
            };
            builder
                .build_and(present, payload_truthy, "opt_truthy")
                .expect("build_and should not fail for two i8 operands")
        }
        // D-244, Part 3 of #1026 (PR 3a of #1082): CPython's own `bool(x)`
        // for an arbitrary object consults `__bool__`/`__len__`, which is
        // arbitrary Python code, so this delegates to the shim's
        // `pycc_ext_obj_truthy` -- and takes `foreign_fail.rs`'s failure
        // edge when that code raises. This arm is what closed Part 2's R11 hole:
        // `pycc_types` used to refuse a `Ty::Object` condition at ten
        // condition-position sites *because* there was no answer here, and
        // PR 3a deleted all ten (`crates/pycc_types/src/foreign.rs`).
        Scalar::Object(ptr) => {
            let bit = foreign_len::emit_truthy(context, builder, module, rt, ptr);
            builder
                .build_int_z_extend(bit, context.i8_type(), "bool_from_object_truthy")
                .expect("build_int_z_extend should not fail widening i1 to i8")
        }
        // Defensive, not a capability gap (Part 2 of #1027): `if b:` is a
        // bare read of a `memoryview`-bound name, refused with `C0001` by
        // `reject_memoryview_read` for a wrapper-borrowed parameter and by
        // `owned_buffer_use_unsupported` for the artifact-owned storage of
        // Part 2a of #1142 (#1165). CPython would answer `len(b) != 0`;
        // `len(b)` is implemented (#1116) but the bare read is not, so this
        // arm stays unreachable.
        Scalar::MemoryView(_) => {
            panic!("pycc_codegen: truthiness of a memoryview value is not supported yet")
        }
    };
    builder
        .build_int_compare(
            IntPredicate::NE,
            as_i8,
            context.i8_type().const_int(0, false),
            "truthy",
        )
        .expect("build_int_compare should not fail comparing two i8 operands")
}
