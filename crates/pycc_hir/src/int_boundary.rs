//! Issue #618: reject an out-of-range `int` *literal* in one of D-141's
//! runtime `int`-boundary positions at compile time, restoring `pycc check`
//! as the catch point that D-178 (PR #617, closing #148) knowingly moved to
//! run time for the literal case.
//!
//! ## Why this lives entirely in `pycc_hir`
//!
//! D-178's own deferral note speculated the fix "needs a boundary-position
//! notion threaded across those 14 sites in three passes (HIR, MIR,
//! codegen)". D-179 already removed the `range()` operand from that
//! fourteen-position inventory before this issue was even filed --
//! `range()` is fully bigint-capable (bounds, step, and a mid-loop-promoting
//! induction variable, via `pycc_rt_range_normalize_operand`/
//! `pycc_rt_range_continue`), so an out-of-range literal there is ordinary
//! supported behavior, not a boundary failure, and this module deliberately
//! does not check it (see `crate::expr::lower_range_call`'s own comment).
//! That leaves 13 positions, and investigation for this issue found the
//! "three passes" estimate inaccurate for all of them: every one is
//! resolved to a dedicated HIR node (or a dedicated argument slot of one)
//! purely syntactically, during HIR lowering itself, with no type
//! information from `pycc_types` or MIR needed to identify the position --
//! `list.append`/`dict.get`/`set.add` are already recognized by bare-name
//! receiver + attribute name (see `crate::expr::lower_expr`'s `Expr::Call`
//! arm), and `list`/`dict`/`set` literals, subscripts, and slices are
//! recognized directly from the AST shape. `pycc
//! check` itself runs only `pycc_hir::lower_all` followed by
//! `pycc_types::check` (see `src/frontend.rs`'s `lower_frontend`) -- neither
//! step reaches MIR or codegen -- so a check that fires during HIR lowering
//! is sufficient on its own to make `pycc check` catch the defect, without
//! opening up `pycc_mir` or `pycc_codegen` at all.
//!
//! The last of those 13 positions -- `str` repeat count (`<str> *
//! <int-literal>` or `<int-literal> * <str>`) -- is intentionally narrower:
//! `pycc_hir` has no
//! type information, so it cannot tell a `str`-typed *variable* multiplied
//! by an oversized literal from ordinary `int * int` multiplication (which
//! is not a boundary position at all after D-178 -- an oversized literal
//! there just materializes as a heap bigint, exactly as intended). This
//! module therefore only recognizes the case where the string side is
//! itself a string *literal*, which needs no type inference and is
//! resolvable from the AST shape alone, in `crate::expr::lower_expr`'s
//! `Expr::BinOp` arm. A `str`-typed variable multiplied by an oversized
//! literal keeps today's runtime-abort behavior; catching that case too
//! would require threading a span-bearing type facts into `pycc_types`
//! (which currently has none -- see its pervasively reused
//! `Span::new(0, 0)`), a materially larger, separate architectural change
//! this issue's own completion criteria do not ask for.
//!
//! ## Why the range check is duplicated a third time
//!
//! `fits_tagged_smallint` below mirrors `pycc_rt::fits_smallint` and
//! `pycc_codegen::int_const::fits_tagged_smallint` exactly (same D-061
//! encoding, same round-trip check). `pycc_hir` cannot depend on
//! `pycc_codegen` (codegen depends on `pycc_mir`, which depends on
//! `pycc_hir` -- the reverse dependency would be circular), and it cannot
//! depend on `pycc_rt` either (no crate in the compiler's own build links
//! against the target runtime as an ordinary Rust dependency; `pycc_codegen`
//! already mirrors it for the same reason, per its own `int_const.rs` doc
//! comment). A third small, independently tested mirror of a five-line
//! invariant is judged cheaper here than adding a dependency edge purely to
//! share it.
use crate::HirExpr;
use pycc_diag::{Diagnostic, Span};

/// Mirrors `pycc_rt::fits_smallint`/`pycc_codegen::int_const::fits_tagged_smallint`
/// exactly (D-061): `true` when `n` round-trips through the 63-bit tagged
/// smallint representation, `false` when it would need a heap bigint.
pub(crate) fn fits_tagged_smallint(n: i64) -> bool {
    let tagged = n.wrapping_shl(1) | 1;
    (tagged >> 1) == n
}

/// `T0051`: an `int` literal in one of D-141's runtime boundary positions is
/// outside the range `pycc_rt_int_untag_checked` accepts, and would abort at
/// run time (D-178) instead of compiling. `position` names the boundary
/// position for the diagnostic message (e.g. `"list index"`, `` "`.append()`
/// value" ``) -- always a `&'static str` literal supplied at the call site,
/// never derived from user input.
fn int_literal_boundary_diagnostic(
    n: i64,
    span: std::ops::Range<u32>,
    position: &str,
) -> Diagnostic {
    Diagnostic::error(
        "T0051",
        format!(
            "integer literal `{n}` is out of range for a {position} -- pycc's compiled `int` \
             representation only supports a literal here within D-061's 63-bit tagged smallint \
             range; a bigint value reaching this position through arithmetic still runs (and \
             aborts) exactly as before, only a literal is rejected at compile time"
        ),
        Span::new(span.start, span.end),
    )
    .with_help(
        "use a literal within the tagged smallint range -- this position has no safe compile-time \
         workaround for a larger value: an out-of-range value reaching it through arithmetic \
         instead of a literal still compiles, but aborts at run time (exit 101) exactly as before",
    )
}

/// Checks one already-lowered boundary-position sub-expression: if it is an
/// out-of-range `HirExpr::IntLiteral`, returns `Err` with a spanned `T0051`
/// diagnostic naming `position`; otherwise (not a literal at all, or a
/// literal that fits) returns `Ok(())` unchanged. `span` is the *source*
/// span of the sub-expression that produced `lowered` -- `HirExpr` itself
/// carries no span (see this module's own doc comment), so callers must
/// capture it from the AST node they are about to lower, before lowering
/// discards it.
pub(crate) fn check_boundary_literal(
    lowered: &HirExpr,
    span: std::ops::Range<u32>,
    position: &'static str,
) -> Result<(), Diagnostic> {
    if let HirExpr::IntLiteral(n) = lowered
        && !fits_tagged_smallint(*n)
    {
        return Err(int_literal_boundary_diagnostic(*n, span, position));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_tagged_smallint_accepts_zero_and_rejects_i64_extremes() {
        assert!(fits_tagged_smallint(0));
        assert!(fits_tagged_smallint(1));
        assert!(fits_tagged_smallint(-1));
        assert!(!fits_tagged_smallint(i64::MAX));
        assert!(!fits_tagged_smallint(i64::MIN));
    }

    #[test]
    fn fits_tagged_smallint_accepts_the_negative_boundary_value() {
        // -4611686018427387904 == -(1i64 << 62): the most negative value
        // that still round-trips through D-061's 63-bit tagged encoding,
        // mirroring the positive-boundary case already covered by the
        // higher-level `expr.rs`/`stmt.rs` boundary-position tests.
        assert!(fits_tagged_smallint(-4611686018427387904));
    }

    #[test]
    fn fits_tagged_smallint_rejects_just_past_the_negative_boundary() {
        // One less than the negative boundary above: the first value that
        // no longer round-trips through the tagged encoding.
        assert!(!fits_tagged_smallint(-4611686018427387905));
    }

    #[test]
    fn check_boundary_literal_accepts_a_non_literal_expression() {
        assert!(
            check_boundary_literal(&HirExpr::Name("x".to_string()), 0..1, "test position").is_ok()
        );
    }

    #[test]
    fn check_boundary_literal_accepts_an_in_range_literal() {
        assert!(check_boundary_literal(&HirExpr::IntLiteral(42), 0..1, "test position").is_ok());
    }

    #[test]
    fn check_boundary_literal_rejects_an_out_of_range_literal_with_a_spanned_t0051() {
        let err = check_boundary_literal(&HirExpr::IntLiteral(i64::MAX), 3..9, "test position")
            .expect_err("i64::MAX must not fit the tagged smallint range");
        assert_eq!(err.code, "T0051");
        assert_eq!(err.span, Some(Span::new(3, 9)));
        assert!(err.message.contains("test position"));
    }
}
