//! Compile-time materialization of `int` constants (D-061 encoding, D-178
//! bigint fallback).
//!
//! Narrow carve out of `lib.rs` under AGENTS.md's decomposability rule: this
//! is exactly the unit #148 touches, and nothing else is relocated.

use super::*;

/// Mirrors `pycc_rt::fits_smallint` exactly (compile-time constant folding
/// of the same encoding, see D-061): `Some(tagged)` when `n` round-trips
/// through the 63-bit tagged representation, `None` when it needs a heap
/// bigint instead.
pub(super) fn fits_tagged_smallint(n: i64) -> Option<i64> {
    let tagged = (n << 1) | 1;
    ((tagged >> 1) == n).then_some(tagged)
}

/// Emits an `int` constant. Values that fit D-061's tagged range are folded
/// into an immediate exactly as before; everything else is materialized at
/// run time by `pycc_rt_int_from_i64` (D-178).
///
/// Materialized per evaluation rather than cached in a module global,
/// mirroring `emit_string_literal`: a bigint literal evaluated in a loop
/// therefore allocates one `BigIntObj` per iteration.
///
/// That allocation is no longer leaked in the general case. #146 Part 1
/// (D-180) narrowed D-058's blanket "never freed" concession to a listed
/// residual set, on which an *unbound temporary* -- exactly what this value
/// is until something binds it to a name -- was the largest entry. Part 2
/// ([#625](https://github.com/rotnov/pycc/issues/625), D-181) retires the
/// birth reference at each site that consumes such a word and discards it,
/// so a literal in a loop is now released for the same reason `a + b`
/// discarded mid-expression is.
///
/// Two shapes still leak the word this call produces, and only these two:
/// a fresh `int` stored into a `MirExpr::TupleLiteral` element, and an
/// operand whose release is skipped because a D-173 exception edge branched
/// out of the expression first. Both are enumerated in D-181.
pub(super) fn emit_int_constant<'ctx>(
    context: &'ctx Context,
    builder: &inkwell::builder::Builder<'ctx>,
    rt: &RtFns<'ctx>,
    n: i64,
) -> IntValue<'ctx> {
    match fits_tagged_smallint(n) {
        Some(tagged) => context.i64_type().const_int(tagged as u64, true),
        None => builder
            .build_call(
                rt.int_from_i64,
                &[context.i64_type().const_int(n as u64, true).into()],
                "int_lit",
            )
            .expect("build_call should not fail for a well-formed int literal materialization")
            .try_as_basic_value()
            .expect_basic("pycc_rt_int_from_i64 returns a non-void encoded int")
            .into_int_value(),
    }
}

/// The builder-less constant-initializer path. `declare_module_globals`
/// builds an LLVM constant initializer -- there is no `Builder` and no
/// insertion point, so it cannot call a runtime function and cannot use
/// `emit_int_constant`. Its only argument is the literal `0`, so the panic
/// arm has no reachable caller from generated code; it is kept as a
/// defensive arm in this crate's established style and is covered by a
/// direct `#[should_panic]` unit test.
pub(super) fn tag_smallint_const(context: &Context, n: i64) -> IntValue<'_> {
    let Some(tagged) = fits_tagged_smallint(n) else {
        panic!(
            "pycc_codegen: integer literal {n} is too large for the constant-only \
             module-global initializer path"
        );
    };
    context.i64_type().const_int(tagged as u64, true)
}
