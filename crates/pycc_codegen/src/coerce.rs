//! Scalar coercions: int tagging/untagging, numeric widening, `coerce_scalar_to_type`,
//! range operands, float/str conversion, string literals and truthiness.

use super::*;

/// Converts a numeric `Int`/`Bool` scalar to D-141's encoded-int carrier.
/// An existing `Int` passes through unchanged, whether it contains an odd
/// ordinary smallint, a bool-identity marker, or a bigint pointer. A
/// standalone `Bool` is being consumed numerically at these call sites, so
/// it becomes an ordinary D-061 smallint via a zero-extend and shift-and-or
/// matching `pycc_rt::tag_smallint`. Panics for `Float`, which is never
/// `int`-coercible -- `pycc_types`'
/// `numeric_result_type` always promotes an expression with any `float`
/// operand to `Ty::Float`, so no real MIR can reach this arm with a
/// `Float` operand (see this task's own defensive-panic test exercising
/// it via deliberately malformed MIR, matching this file's existing
/// convention for such arms).
pub(crate) fn to_numeric_encoded_int<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    scalar: Scalar<'ctx>,
) -> IntValue<'ctx> {
    match scalar {
        Scalar::Generator(_) | Scalar::Closure(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got generator")
        }
        Scalar::Int(v) => v,
        Scalar::Bool(v) => {
            let widened = builder
                .build_int_z_extend(v, context.i64_type(), "bool_to_i64")
                .expect("build_int_z_extend should not fail widening i8 to i64");
            let shifted = builder
                .build_left_shift(widened, context.i64_type().const_int(1, false), "tag_shl")
                .expect("build_left_shift should not fail for a constant shift amount");
            builder
                .build_or(shifted, context.i64_type().const_int(1, false), "tag_or")
                .expect("build_or should not fail for two i64 operands")
        }
        Scalar::Float(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got float")
        }
        Scalar::Str(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got str")
        }
        // Defensive, exactly like the two arms above -- not a feature gap
        // (D-107): `pycc_types`' `numeric_result_type` maps no `Ty::List`
        // to a numeric type, so any arithmetic with a list operand is
        // already rejected as `T0021` before codegen runs. Its own arm
        // rather than folding into `Str`'s, so the message names the type
        // it actually got.
        Scalar::List(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got list")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // `List` arm directly above, extended to `dict[K, V]` (D-107's
        // reasoning, per D-124): no arithmetic operand is ever `Ty::Dict`,
        // so this is never reached by real, type-checked source.
        Scalar::Dict(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got dict")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // `List`/`Dict` arms above, extended to `set[T]` (D-107's
        // reasoning, per D-124): no arithmetic operand is ever `Ty::Set`,
        // so this is never reached by real, type-checked source.
        Scalar::Set(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got set")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // `List`/`Dict`/`Set` arms above, extended to `tuple[...]` (D-107's
        // reasoning, per D-116): no arithmetic operand is ever `Ty::Tuple`,
        // so this is never reached by real, type-checked source.
        Scalar::Tuple(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got tuple")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // `List`/`Dict`/`Set`/`Tuple` arms above, extended to a class
        // instance (D-154, Part 1 of #375, mirroring D-107/D-124's
        // reasoning): `pycc_types` rejects arithmetic on `Ty::Instance` as
        // `T0021` before codegen runs (see `numeric_result_type`'s own
        // `as_numeric` closure), so this is never reached by real,
        // type-checked source.
        Scalar::Instance(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got instance")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // arms above, extended to `Optional[int]` (D-197, #763, Part 1 of
        // #747, extending D-107/D-124's reasoning): `as_numeric` maps no
        // `Ty::Optional` to a numeric type either, so any arithmetic with
        // an `Optional[int]` operand is already rejected as `T0021` before
        // codegen runs -- unwrapping an `Optional[int]` to use its payload
        // numerically requires narrowing (Part 2+ of #747), which produces
        // a plain `Ty::Int` value, not this type.
        Scalar::Optional(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got optional")
        }
        // Defensive for the same reason as every arm above, extended to a
        // foreign CPython object (D-244, Part 2 of #1026): `pycc_types`
        // refuses arithmetic on a `Ty::Object` operand at the consumer
        // sites the Part 2 refusal migration added, so no type-checked
        // program reaches this arm.
        Scalar::Object(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got object")
        }
        // Defensive for the same reason as every arm above, extended to a
        // `memoryview` (Part 2 of #1027): `numeric_result_type` maps no
        // `Ty::MemoryView` to a numeric type, and the bare name an
        // arithmetic operand would have to be is refused whichever
        // provenance it has -- `reject_memoryview_read` for a wrapper-
        // borrowed parameter, and since Part 2a of #1142 (#1165)
        // `pycc_types::buffer::owned_buffer_use_unsupported` for
        // artifact-owned storage. Its own arm rather than folding into
        // `Object`'s, so the message names the type it actually got.
        Scalar::MemoryView(_) => {
            panic!("pycc_codegen: internal error: expected an int-or-bool operand, got memoryview")
        }
    }
}

/// Converts an int-compatible scalar for storage at a statically-`int`
/// boundary while preserving whether the source object was `False` or
/// `True`. Ordinary integer inputs already use the D-061 encoding. The two
/// even marker words are disjoint from odd smallints and aligned bigint
/// pointers; `pycc_rt` owns their interpretation.
pub(crate) fn to_encoded_int<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    scalar: Scalar<'ctx>,
) -> IntValue<'ctx> {
    match scalar {
        Scalar::Int(value) => value,
        Scalar::Bool(value) => {
            let widened = builder
                .build_int_z_extend(value, context.i64_type(), "bool_identity_i64")
                .expect("build_int_z_extend should not fail widening i8 to i64");
            let shifted = builder
                .build_left_shift(
                    widened,
                    context.i64_type().const_int(2, false),
                    "bool_identity_shl",
                )
                .expect("build_left_shift should not fail for a constant shift amount");
            builder
                .build_or(
                    shifted,
                    context.i64_type().const_int(2, false),
                    "bool_identity_or",
                )
                .expect("build_or should not fail for two i64 operands")
        }
        other => to_numeric_encoded_int(context, builder, other),
    }
}

/// Tags an already-known-in-range raw counter as an ordinary smallint.
/// D-141 leaves this only for user-visible lengths -- `range()` operands
/// stopped using it in #147 (D-179), where normalization moved into
/// `pycc_rt_range_normalize_operand` so a bigint operand survives it.
/// Container elements/values carry their encoded words unchanged and never
/// use this helper on read.
pub(crate) fn raw_i64_to_tagged_int<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    raw: IntValue<'ctx>,
) -> IntValue<'ctx> {
    let shifted = builder
        .build_left_shift(raw, context.i64_type().const_int(1, false), "list_tag_shl")
        .expect("build_left_shift should not fail for a constant shift amount");
    builder
        .build_or(
            shifted,
            context.i64_type().const_int(1, false),
            "list_tag_or",
        )
        .expect("build_or should not fail for two i64 operands")
}

/// What a machine-word `int` decoded by [`build_untag_checked`] is for,
/// which picks the exception a heap bigint raises (the runtime's
/// `UNTAG_*` constants).
#[derive(Clone, Copy)]
pub(crate) enum UntagUse {
    /// Stored as a container element (`OverflowError`, a recorded gap).
    Element = 0,
    /// A sequence index (`IndexError`, as CPython).
    Index = 1,
    /// A size, count or slice bound (`OverflowError`).
    Size = 2,
}

/// Calls D-141's runtime-owned classifier/decoder, then checks the pending
/// flag: a heap bigint raises (it does not fit a machine word) and the
/// sentinel result is never consumed. Container-value ingress uses the call
/// only to validate bigint exclusion and then stores the original encoded
/// word; index, slice, and `str`-repeat-count sites consume its decoded raw
/// result. Raising instead of aborting keeps an `--ext` module's host
/// interpreter alive when a caller passes a large `int`.
pub(crate) fn build_untag_checked<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    use_kind: UntagUse,
    tagged: IntValue<'ctx>,
    name: &str,
) -> IntValue<'ctx> {
    let kind = context.i32_type().const_int(use_kind as u64, false);
    let raw = builder
        .build_call(rt.int_untag_checked, &[tagged.into(), kind.into()], name)
        .expect("build_call should not fail for a well-formed untag")
        .try_as_basic_value()
        .expect_basic("pycc_rt_int_untag_checked returns a non-void i64")
        .into_int_value();
    guard_statement_effects(context, builder, rt);
    raw
}

/// Calls #1165's non-aborting length decoder
/// (`pycc_rt_buffer_alloc_untag_len`) for the `ndarray(n)` producer.
///
/// Unlike `build_untag_checked` above, this decoder also rejects a negative
/// length. It raises `OverflowError` (D-173) and returns the type-valid
/// sentinel `0`, which the `guard_statement_effects` the call site emits
/// immediately afterwards branches away on, so the sentinel never reaches
/// `pycc_rt_buffer_f64_alloc`.
pub(crate) fn build_buffer_alloc_untag_len<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    tagged: IntValue<'ctx>,
    name: &str,
) -> IntValue<'ctx> {
    builder
        .build_call(rt.buffer_alloc_untag_len, &[tagged.into()], name)
        .expect("build_call should not fail for a well-formed untag")
        .try_as_basic_value()
        .expect_basic("pycc_rt_buffer_alloc_untag_len returns a non-void i64")
        .into_int_value()
}

/// Applies the representation-changing assignment conversions accepted by
/// this type system: standalone `i8` bool to D-141's identity-preserving
/// int-compatible `i64`, and (D-197, #763, Part 1 of #747) a bare
/// `inner`-typed value or `None` widening into an `Optional[inner]` slot's
/// `{ inner, i8 }` present/absent struct. All other assignable pairs already
/// share a representation.
///
/// This is the *only* place `Optional[inner]`'s representation is built --
/// driven entirely by the target slot's own declared type, not by anything
/// MIR-lowering-time. That single-site design (rather than an `IntBoundary`-
/// style MIR wrapper node) is deliberate: it uniformly covers both the
/// first, `AnnAssign`-introduced binding (`x: int | None = 5`) and every
/// later plain reassignment to the same name (`x = None`), which
/// `pycc_mir::stmt::lower_stmt`'s `Assign` arm never wraps -- see that
/// arm's own doc comment on why a wrapper node is unnecessary.
pub(crate) fn coerce_scalar_to_type<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    scalar: Scalar<'ctx>,
    target_ty: pycc_mir::Ty,
) -> Scalar<'ctx> {
    match (target_ty, scalar) {
        (pycc_mir::Ty::Int, Scalar::Bool(value)) => {
            Scalar::Int(to_encoded_int(context, builder, Scalar::Bool(value)))
        }
        // A `Scalar::Optional` arriving here is either a *real*
        // `Optional[inner]` value (e.g. `w: int | None = y` where `y` is
        // itself `Optional[int]`, read back out via `MirExpr::Name`'s own
        // `Ty::Optional` arm), or `MirExpr::NoneLiteral`'s own `{ i8, i8 }`
        // all-zero placeholder standing in for the bare `None` literal
        // (`emit_expr_unchecked`'s `MirExpr::NoneLiteral` arm has no target
        // `inner` type to build a real struct against). Distinguished by
        // LLVM struct type, not by any Rust-level tag: an anonymous struct
        // type is uniqued per-`Context` by its field types, so the real
        // `{ i64, i8 }` (or whatever `inner` actually is) and the
        // placeholder's fixed `{ i8, i8 }` compare unequal whenever they
        // are not the exact same shape, and equal (and therefore pass
        // through unchanged) whenever the source already has the target's
        // exact representation.
        //
        // #809 risk, verified: for `inner == Ty::Bool`, `ty_to_basic_type`'s
        // `Ty::Optional` arm produces the real `Optional[bool]` shape `{
        // i8, i8 }` -- structurally, and therefore (LLVM struct types being
        // uniqued per-field-type-list within a `Context`) *literally the
        // same `StructType` value as* the `NoneLiteral` placeholder's own
        // fixed `{ i8, i8 }`. So this arm's `v.get_type() == struct_ty`
        // check cannot distinguish "this is genuinely the untyped None
        // placeholder" from "this is a real, already-correctly-shaped
        // `Optional[bool]` value" when `inner == Ty::Bool` -- both take the
        // `v.get_type() == struct_ty` branch and pass `v` through as-is.
        // This is harmless rather than a bug: taking that branch for the
        // placeholder returns `{ payload: 0, present: 0 }` unchanged, which
        // is *exactly* the correct representation for an absent
        // `Optional[bool]` too -- `Ty::Bool`'s payload carries no tagged
        // encoding analogous to `Ty::Int`'s D-141 word (see `truthy`'s and
        // `MirExpr::OptionalUnwrap`'s own `Ty::Bool` handling below/in
        // `lib.rs`, both of which read the payload only as a plain `0`/`1`
        // and never as anything requiring a specific "valid" bit pattern
        // the way `classify_encoded_int` does for `Ty::Int`), so an
        // arbitrary (here, zero) payload byte is always a safe stand-in
        // when `present == 0`. Every consumer of an `Optional[bool]`
        // payload (`truthy`'s `Scalar::Optional` arm, `OptionalUnwrap`'s
        // codegen arm) is itself only ever reached with `present == 1`
        // guarding real use of the payload's *value* (narrowing proves
        // presence before an unwrap; `truthy` ANDs the payload's
        // truthiness with `present` rather than trusting it alone) -- so no
        // observer can tell the difference between "the None placeholder,
        // never coerced" and "a real, coerced, present `Optional[bool]`"
        // for the one case (`present == 0`) where the collision could ever
        // matter. `optional_bool_none_placeholder_and_real_absent_value_are_the_same_llvm_struct_type`
        // and
        // `optional_bool_absent_value_truthiness_and_narrowed_unwrap_are_both_correct`
        // (`pycc_codegen::tests`) pin both halves of this finding: the
        // literal type-identity collision, and that it produces correct
        // end-to-end behavior for `x: bool | None = None` through both
        // `truthy` and an `if x is not None:` narrowed unwrap.
        (pycc_mir::Ty::Optional(inner), Scalar::Optional(v)) => {
            let struct_ty =
                ty_to_basic_type(context, pycc_mir::Ty::Optional(inner.clone())).into_struct_type();
            if v.get_type() == struct_ty {
                Scalar::Optional(v)
            } else {
                // The placeholder's field 0 must be a value of field 0's
                // *actual* declared type (`struct_ty`'s first field, sized
                // by `inner`), not unconditionally a D-141-encoded int:
                // `struct_ty.get_undef()` is `inner`-shaped (`{ i64, i8 }`
                // for `Ty::Int`, `{ f64, i8 }` for `Ty::Float`, `{ i8, i8 }`
                // for `Ty::Bool`), and `build_insert_value` performs no
                // type-matching validation of its own -- inserting a
                // mismatched-type constant (e.g. an `i64` into a `Ty::Float`
                // struct's `f64` field 0) builds a malformed
                // `ConstantStruct` that both `module.verify()` and LLVM's
                // own IR printer accept without complaint (neither
                // re-validates a `ConstantAggregate`'s element types against
                // its declared struct type), so this was previously a
                // latent miscompile rather than a build-time or
                // verification failure. For `Ty::Int`,
                // `tag_smallint_const(context, 0)` encodes `0` the same way
                // `to_encoded_int` would (`truthy`'s own `Scalar::Optional`
                // arm unconditionally calls `pycc_rt_int_truthy` on field 0
                // and ANDs the result with the present flag rather than
                // branching around it, so an absent-but-invalid payload
                // would trip `classify_encoded_int`'s fail-closed panic even
                // though the AND makes the payload's truth value
                // irrelevant). `Ty::Float` and `Ty::Bool` carry no such
                // tagged encoding, so a plain zero of the field's own type
                // is exact.
                let payload0: inkwell::values::BasicValueEnum = match inner.as_ref() {
                    pycc_mir::Ty::Int => tag_smallint_const(context, 0).into(),
                    pycc_mir::Ty::Float => context.f64_type().const_zero().into(),
                    pycc_mir::Ty::Bool => context.i8_type().const_zero().into(),
                    // `T0049` (`crates/pycc_hir/src/func.rs`) rejects every
                    // `Optional[T]` annotation for `T` outside `{int, float,
                    // bool}` before this value could ever be constructed.
                    other => panic!(
                        "pycc_codegen: internal error: an Optional[_] placeholder targeted an unsupported inner type ({other:?}) -- pycc_types::check (T0049) should have rejected this before codegen"
                    ),
                };
                let with_payload = builder
                    .build_insert_value(struct_ty.get_undef(), payload0, 0, "opt_none_payload")
                    .expect(
                        "build_insert_value should not fail inserting field 0 of a fresh struct",
                    )
                    .into_struct_value();
                let with_flag = builder
                    .build_insert_value(
                        with_payload,
                        context.i8_type().const_zero(),
                        1,
                        "opt_none_flag",
                    )
                    .expect(
                        "build_insert_value should not fail inserting field 1 of a fresh struct",
                    )
                    .into_struct_value();
                Scalar::Optional(with_flag)
            }
        }
        // A bare `inner`-typed (or `inner`-assignable, e.g. `bool` under
        // `Optional[int]`) value widening into an `Optional[inner]` slot:
        // recurse to apply any further representation change the payload
        // itself needs (e.g. `bool -> int`, the arm above), then wrap the
        // result as the struct's present (`i8 = 1`) payload field.
        (pycc_mir::Ty::Optional(inner), bare) => {
            let coerced = coerce_scalar_to_type(context, builder, bare, (*inner).clone());
            // #809 (Part 3 of #747): `T0049`'s widened gate
            // (`crates/pycc_hir/src/func.rs`) now also admits
            // `Optional[float]`/`Optional[bool]`, so this match is keyed on
            // *both* the declared inner type and the coerced `Scalar`
            // variant, not the `Scalar` variant alone -- matching on the
            // variant alone would accept e.g. a stray `Scalar::Float`
            // widening into an `Optional[int]` slot (that combination
            // falls through `coerce_scalar_to_type`'s own recursive call
            // unchanged, since there is no `float -> int` widening arm),
            // silently building a struct whose field 0 both has the wrong
            // LLVM type for `Ty::Optional(Int)`'s `{ i64, i8 }` shape and
            // is never a value any real `Optional[int]`-typed source could
            // produce (`pycc_types` never assigns a bare `float` to an
            // `int`-annotated slot). Pairing the target inner type with the
            // coerced variant keeps each of the three real widenings exact
            // and every mismatched combination falling to the panic below.
            let payload: inkwell::values::BasicValueEnum = match (inner.as_ref(), coerced) {
                (pycc_mir::Ty::Int, Scalar::Int(v)) => v.into(),
                // Each passes through as-is: `ty_to_basic_type`'s own
                // `Ty::Optional` arm already sizes field 0 from the inner
                // type, so no further representation change is needed here.
                (pycc_mir::Ty::Float, Scalar::Float(v)) => v.into(),
                (pycc_mir::Ty::Bool, Scalar::Bool(v)) => v.into(),
                // `T0049` (`crates/pycc_hir/src/func.rs`) rejects every
                // `Optional[T]` annotation for `T` outside `{int, float,
                // bool}` before this value could ever be constructed, and
                // `pycc_types::check`'s own assignability rule only ever
                // widens a bare value into the *matching* inner type (or
                // `bool -> int`, handled by the recursive call above), so
                // an inner/coerced-variant mismatch reaching here means one
                // of those upstream gates accepted something it should
                // have rejected.
                _ => panic!(
                    "pycc_codegen: internal error: an Optional[int|float|bool] assignment's payload did not evaluate to int, float, or bool -- pycc_types::check (T0049) should have rejected this before codegen"
                ),
            };
            let struct_ty =
                ty_to_basic_type(context, pycc_mir::Ty::Optional(inner)).into_struct_type();
            let with_payload = builder
                .build_insert_value(struct_ty.get_undef(), payload, 0, "opt_payload")
                .expect("build_insert_value should not fail inserting field 0 of a fresh struct")
                .into_struct_value();
            let with_flag = builder
                .build_insert_value(
                    with_payload,
                    context.i8_type().const_int(1, false),
                    1,
                    "opt_present",
                )
                .expect("build_insert_value should not fail inserting field 1 of a fresh struct")
                .into_struct_value();
            Scalar::Optional(with_flag)
        }
        (_, scalar) => scalar,
    }
}

/// Prepares one `range()` operand for the induction phi: the result is an
/// *encoded* int-compatible word (D-061/D-141), normalized so the
/// bool-identity markers become the ordinary smallints `0`/`1` while a
/// smallint or a heap bigint passes through unchanged.
///
/// Renamed from `range_operand_to_tagged_int` in #147 (D-179): the old name
/// described the old mechanism, which decoded to a raw `i64` and re-tagged,
/// and therefore could not represent a bigint operand at all.
pub(crate) fn range_operand_to_normalized_int<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    scalar: Scalar<'ctx>,
    position: &str,
) -> IntValue<'ctx> {
    match scalar {
        Scalar::Generator(_) | Scalar::Closure(_) => {
            panic!("pycc_codegen: internal error: a generator is never a range() operand")
        }
        scalar @ (Scalar::Int(_) | Scalar::Bool(_)) => {
            let encoded = to_encoded_int(context, builder, scalar);
            // #147 (D-179): normalize instead of decode-and-re-tag. The old
            // `build_untag_checked` + `raw_i64_to_tagged_int` pair enforced
            // D-141's bool normalization by round-tripping through a raw
            // `i64`, which made every bigint operand abort. The runtime
            // normalizer keeps that bool contract and lets a bigint through.
            int_fast::emit_range_normalize(context, builder, rt, encoded)
        }
        // `List`/`Dict`/`Set`/`Tuple` join this arm's existing or-pattern
        // rather than getting their own (D-107, extended to dict/set by
        // D-124 and to tuple by D-116): unlike `to_numeric_encoded_int`/`to_float`
        // above and below, this message never names the offending type, so
        // it stays exactly as honest for a list, dict, set, or tuple
        // operand as for a `float` or `str` one -- and folding adds no
        // separate, permanently-unexecutable region under this crate's
        // 100%-region gate (D-014). `range()` arguments are type-checked by
        // `pycc_types` before codegen, so this whole arm is defensive
        // either way.
        Scalar::Float(_)
        | Scalar::Str(_)
        | Scalar::List(_)
        | Scalar::Dict(_)
        | Scalar::Set(_)
        | Scalar::Tuple(_)
        // D-154 (Part 1 of #375): a class instance joins this same
        // or-pattern for the identical reason `List`/`Dict`/`Set`/`Tuple`
        // already do -- no new instrumented region.
        | Scalar::Instance(_)
        // D-197, #763, Part 1 of #747: `Optional[int]` joins this same
        // or-pattern for the identical `numeric_result_type`/`as_numeric`
        // reason -- `range()` operands are type-checked as plain numeric
        // types before codegen, and an `Optional[int]` is never one.
        | Scalar::Optional(_)
        // D-244, Part 2 of #1026: a foreign CPython object joins this same
        // or-pattern for the identical reason -- `range()` operands are
        // type-checked as plain numeric types before codegen, and a
        // `Ty::Object` never is. Folded in rather than given its own arm
        // so it adds no separate, permanently-unexecutable region.
        // Part 2 of #1027: a `memoryview` joins this same or-pattern for
        // the identical reason -- `range()` operands are type-checked as
        // plain numeric types before codegen, and a `Ty::MemoryView` never
        // is, under either provenance (Part 2a of #1142, #1165).
        | Scalar::MemoryView(_)
        | Scalar::Object(_) => {
            panic!("pycc_codegen: internal error: range() {position} did not evaluate to int")
        }
    }
}

/// Evaluates a `range()`-shaped triple of bounds (`MirStmt::ForRange` and
/// its three comprehension-tail copies all share this exact preheader
/// shape) with #638 (D-208) exception-edge protection: `start`'s word is
/// protected across `stop` and `step`'s own evaluation, and `stop`'s word is
/// protected across `step`'s, mirroring the two-operand push/pop pattern
/// `BinOp`/`Compare` use above but generalized to three operands evaluated
/// in sequence. Per D-179, these bounds are legitimately owned, live
/// `IntValue`s held across each other's evaluation (unlike `Slice`/
/// `Subscript`/container-literal bounds, which abort the process on a
/// bigint before a sibling is ever reached) -- see the #638 decision
/// entry's affected-site inventory for the full boundary.
///
/// Returns the three normalized `IntValue`s; callers keep their own
/// existing retain/release and `owned_range_operands` bookkeeping unchanged
/// -- this helper only factors out the shared evaluation-with-protection
/// step, not the ownership contract built on top of it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_range_operands_with_exception_safety<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    start: &MirExpr,
    stop: &MirExpr,
    step: &MirExpr,
) -> (IntValue<'ctx>, IntValue<'ctx>, IntValue<'ctx>) {
    let start_v = range_operand_to_normalized_int(
        context,
        builder,
        rt,
        emit_expr(context, builder, module, rt, user_functions, locals, start),
        "start",
    );
    let pending_start = push_pending_int_release_if_temporary(rt, start, start_v);
    let stop_v = range_operand_to_normalized_int(
        context,
        builder,
        rt,
        emit_expr(context, builder, module, rt, user_functions, locals, stop),
        "stop",
    );
    let pending_stop = push_pending_int_release_if_temporary(rt, stop, stop_v);
    let step_v = range_operand_to_normalized_int(
        context,
        builder,
        rt,
        emit_expr(context, builder, module, rt, user_functions, locals, step),
        "step",
    );
    // LIFO pop order, matching the LIFO push order above.
    pop_pending_int_release(rt, pending_stop);
    pop_pending_int_release(rt, pending_start);
    (start_v, stop_v, step_v)
}

/// Promotes any numeric `Scalar` to `f64`: an existing `Float` passes
/// through; `Int` goes through `pycc_rt_int_to_float` (never a raw LLVM
/// cast -- the value is D-141 encoded, so only `pycc_rt` may interpret its
/// bits); `Bool` uses a plain unsigned-int-to-float conversion
/// (unambiguous for a 0/1 value, no tagging involved).
pub(crate) fn to_float<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    scalar: Scalar<'ctx>,
) -> FloatValue<'ctx> {
    match scalar {
        Scalar::Float(v) => v,
        Scalar::Generator(_) | Scalar::Closure(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got generator")
        }
        Scalar::Int(v) => int_fast::emit_int_to_float(context, builder, rt, v),
        Scalar::Bool(v) => builder
            .build_unsigned_int_to_float(v, context.f64_type(), "bool_to_float")
            .expect("build_unsigned_int_to_float should not fail for an i8 0/1 value"),
        Scalar::Str(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got str")
        }
        // Defensive for the same `numeric_result_type` reason as
        // `to_numeric_encoded_int`'s own `List` arm above (D-107), and separate from
        // `Str`'s for the same message-honesty reason.
        Scalar::List(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got list")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // `List` arm directly above, extended to `dict[K, V]` (D-107's
        // reasoning, per D-124).
        Scalar::Dict(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got dict")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // `List`/`Dict` arms above, extended to `set[T]` (D-107's
        // reasoning, per D-124).
        Scalar::Set(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got set")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // `List`/`Dict`/`Set` arms above, extended to `tuple[...]` (D-107's
        // reasoning, per D-116).
        Scalar::Tuple(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got tuple")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // `List`/`Dict`/`Set`/`Tuple` arms above, extended to a class
        // instance (D-154, Part 1 of #375).
        Scalar::Instance(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got instance")
        }
        // Defensive for the identical `numeric_result_type` reason as the
        // arms above, extended to `Optional[int]` (D-197, #763, Part 1 of
        // #747).
        Scalar::Optional(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got optional")
        }
        // Defensive for the same reason as every arm above, extended to a
        // foreign CPython object (D-244, Part 2 of #1026): `pycc_types`
        // refuses arithmetic on a `Ty::Object` operand.
        Scalar::Object(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got object")
        }
        // Defensive for the same reason as every arm above, extended to a
        // `memoryview` (Part 2 of #1027). A buffer *element* is a `float`
        // and reaches this function freely; the buffer itself never does,
        // whether it is a wrapper-borrowed parameter or the artifact-owned
        // storage Part 2a of #1142 (#1165) added.
        Scalar::MemoryView(_) => {
            panic!("pycc_codegen: internal error: expected a numeric operand, got memoryview")
        }
    }
}

/// Shared by the `BinOp` arm's `FloorDiv`/`Mod`/`Pow` cases under a
/// `Ty::Float` result: each of these three (unlike `Add`/`Sub`/`Mul`/`Div`,
/// which map directly to an LLVM float instruction) needs a `pycc_rt_float_*`
/// runtime call instead -- this is the one piece of call-building logic all
/// three share, parameterized only by which `RtFns` field to call.
pub(crate) fn build_float_rt_binop<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    rt_fn: FunctionValue<'ctx>,
    l: FloatValue<'ctx>,
    r: FloatValue<'ctx>,
) -> FloatValue<'ctx> {
    builder
        .build_call(rt_fn, &[l.into(), r.into()], "float_binop")
        .expect("build_call should not fail for a well-formed float binop")
        .try_as_basic_value()
        .expect_basic("pycc_rt_float_* functions all return a non-void f64")
        .into_float_value()
}

/// `round(x)` / `round(x, ndigits)` (`pycc_types::round` admits the
/// shapes): an `int` argument is its own rounding, a `float` one goes to
/// `pycc_rt_float_round_to_int` / `pycc_rt_float_round`.
pub(crate) fn emit_round<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    module: &inkwell::module::Module<'ctx>,
    rt: &RtFns<'ctx>,
    user_functions: &HashMap<&str, UserFunction<'ctx>>,
    locals: &HashMap<String, StorageSlot<'ctx>>,
    args: &[MirExpr],
) -> Scalar<'ctx> {
    let emit = |arg: &MirExpr| emit_expr(context, builder, module, rt, user_functions, locals, arg);
    let x = emit(&args[0]);
    let (callee, call_args): (
        FunctionValue<'ctx>,
        Vec<inkwell::values::BasicMetadataValueEnum<'ctx>>,
    ) = match (x, args.get(1)) {
        (Scalar::Float(x), None) => (rt.float_round_to_int, vec![x.into()]),
        (Scalar::Float(x), Some(ndigits)) => {
            let ndigits = emit(ndigits);
            let ndigits = list::raw_index(context, builder, rt, ndigits);
            (rt.float_round, vec![x.into(), ndigits.into()])
        }
        (int, _) => return Scalar::Int(to_numeric_encoded_int(context, builder, int)),
    };
    let result = builder
        .build_call(callee, &call_args, "round")
        .expect("build_call should not fail for a declared runtime function")
        .try_as_basic_value()
        .expect_basic("the round runtime functions return a value");
    if args.len() == 2 {
        Scalar::Float(result.into_float_value())
    } else {
        guard_statement_effects(context, builder, rt);
        Scalar::Int(result.into_int_value())
    }
}
