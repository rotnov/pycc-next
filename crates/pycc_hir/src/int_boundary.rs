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
use crate::{BinOpKind, HirExpr};
use pycc_ast::Int;
use pycc_diag::{Diagnostic, Span};

/// Bits per chunk of [`lower_int_literal`]'s expansion. Every chunk, and
/// the multiplier `2**CHUNK_BITS`, fits the tagged smallint range.
const CHUNK_BITS: u32 = 30;

/// The magnitude of an integer literal as base-`2**CHUNK_BITS` chunks,
/// most significant first. `ruff` keeps a value that fits `u64` as a number
/// and anything larger as its source token (decimal digits with the
/// underscores already stripped, or a `0x`/`0o`/`0b` token as written).
fn literal_chunks(value: &Int) -> Vec<u64> {
    let text = match value.as_u64() {
        Some(small) => small.to_string(),
        None => value.to_string(),
    };
    let lower = text.to_ascii_lowercase();
    let (radix, digits) = match lower.get(..2) {
        Some("0x") => (16, &lower[2..]),
        Some("0o") => (8, &lower[2..]),
        Some("0b") => (2, &lower[2..]),
        _ => (10, lower.as_str()),
    };
    // Little-endian chunks, built by multiply-and-add per digit.
    let mut chunks: Vec<u64> = vec![0];
    for digit in digits.chars().filter(|c| *c != '_') {
        let mut carry = u64::from(
            digit
                .to_digit(radix)
                .expect("the lexer validated the literal"),
        );
        for chunk in &mut chunks {
            let wide = *chunk * u64::from(radix) + carry;
            *chunk = wide & ((1 << CHUNK_BITS) - 1);
            carry = wide >> CHUNK_BITS;
        }
        if carry > 0 {
            chunks.push(carry);
        }
    }
    chunks.reverse();
    chunks
}

/// Lowers an `int` literal of any size, `negate`d for a source-level `-`.
///
/// A value that fits `i64` is an ordinary [`HirExpr::IntLiteral`]. A larger
/// one has no single-word HIR form, so it lowers to the arithmetic that
/// builds it from `2**30` chunks, `((c0 * 2**30) + c1) * 2**30 + c2 ...`,
/// every chunk carrying the sign. Every literal in the expansion fits the
/// tagged range, and the runtime's arbitrary-precision `int` evaluates it
/// to exactly the literal's value. [`check_boundary_literal`] recognizes the
/// shape, so a too-large literal in a boundary position is still `T0051`.
pub(crate) fn lower_int_literal(value: &Int, negate: bool) -> HirExpr {
    let chunks = literal_chunks(value);
    if chunks.len() <= 3 {
        let magnitude = chunks
            .iter()
            .fold(0i128, |acc, chunk| (acc << CHUNK_BITS) | i128::from(*chunk));
        let signed = if negate { -magnitude } else { magnitude };
        if let Ok(fits) = i64::try_from(signed) {
            return HirExpr::IntLiteral(fits);
        }
    }
    let sign = if negate { -1 } else { 1 };
    let mut chunks = chunks
        .into_iter()
        .map(|chunk| HirExpr::IntLiteral(sign * chunk as i64));
    let first = chunks.next().expect("a literal has at least one chunk");
    chunks.fold(first, |acc, chunk| HirExpr::BinOp {
        op: BinOpKind::Add,
        left: Box::new(HirExpr::BinOp {
            op: BinOpKind::Mul,
            left: Box::new(acc),
            right: Box::new(HirExpr::IntLiteral(1 << CHUNK_BITS)),
        }),
        right: Box::new(chunk),
    })
}

/// The value of a constant `+`/`*` tree of integer literals -- the shape
/// [`lower_int_literal`] produces -- or `None` when `expr` is anything else
/// or the value overflows `i128` (which is then certainly out of range).
fn constant_value(expr: &HirExpr) -> Option<Option<i128>> {
    match expr {
        HirExpr::IntLiteral(n) => Some(Some(i128::from(*n))),
        HirExpr::BinOp {
            op: op @ (BinOpKind::Add | BinOpKind::Mul),
            left,
            right,
        } => {
            let (left, right) = (constant_value(left)?, constant_value(right)?);
            Some(match (left, right) {
                (Some(l), Some(r)) if *op == BinOpKind::Add => l.checked_add(r),
                (Some(l), Some(r)) => l.checked_mul(r),
                _ => None,
            })
        }
        _ => None,
    }
}

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
    n: &str,
    span: std::ops::Range<u32>,
    position: &str,
) -> Diagnostic {
    Diagnostic::error(
        "T0051",
        format!(
            "integer literal `{n}` is out of range for a {position} -- pycc's compiled `int` \
             representation only supports a literal here within D-061's 63-bit tagged smallint \
             range; a bigint value reaching this position through arithmetic raises at run \
             time instead, only a literal is rejected at compile time"
        ),
        Span::new(span.start, span.end),
    )
    .with_help(
        "use a literal within the tagged smallint range -- this position has no safe compile-time \
         workaround for a larger value: an out-of-range value reaching it through arithmetic \
         instead of a literal still compiles, and raises `OverflowError` or `IndexError` at run \
         time",
    )
}

/// The `T0051` [`check_boundary_literal`] reports for `lowered` in
/// `position`, without a source span: for a check `pycc_types` makes once
/// it knows the position is native (a non-strict `.append()` value).
pub fn boundary_literal_diagnostic(lowered: &HirExpr, position: &'static str) -> Option<Diagnostic> {
    check_boundary_literal(lowered, 0..0, position).err()
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
        return Err(int_literal_boundary_diagnostic(
            &n.to_string(),
            span,
            position,
        ));
    }
    // A literal beyond `i64` arrives as `lower_int_literal`'s expansion.
    if matches!(lowered, HirExpr::BinOp { .. })
        && let Some(value) = constant_value(lowered)
    {
        let fits = value
            .and_then(|v| i64::try_from(v).ok())
            .is_some_and(fits_tagged_smallint);
        if !fits {
            let text = value.map_or_else(|| "beyond 2**127".to_string(), |v| v.to_string());
            return Err(int_literal_boundary_diagnostic(&text, span, position));
        }
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

    fn int_of(text: &str) -> Int {
        text.parse().unwrap()
    }

    #[test]
    fn lower_int_literal_keeps_i64_values_as_plain_literals() {
        assert_eq!(
            lower_int_literal(&int_of("42"), true),
            HirExpr::IntLiteral(-42)
        );
        assert_eq!(
            lower_int_literal(&int_of("9223372036854775808"), true),
            HirExpr::IntLiteral(i64::MIN)
        );
        assert_eq!(
            lower_int_literal(&int_of("9223372036854775807"), false),
            HirExpr::IntLiteral(i64::MAX)
        );
    }

    #[test]
    fn lower_int_literal_expands_a_wider_value_into_its_exact_chunks() {
        for (text, negate, expected) in [
            ("9223372036854775808", false, Some(1i128 << 63)),
            ("18446744073709551615", true, Some(-(u64::MAX as i128))),
            ("0x1_0000_0000_0000_0000", false, Some(1i128 << 64)),
            ("0o2000000000000000000000", false, Some(1i128 << 64)),
            (
                "0B10000000000000000000000000000000000000000000000000000000000000000",
                false,
                Some(1i128 << 64),
            ),
        ] {
            let parsed: Int = match text.get(..2) {
                Some("0x") => Int::from_str_radix(&text[2..].replace('_', ""), 16, text).unwrap(),
                Some("0o") => Int::from_str_radix(&text[2..], 8, text).unwrap(),
                Some("0B") => Int::from_str_radix(&text[2..], 2, text).unwrap(),
                _ => int_of(text),
            };
            let lowered = lower_int_literal(&parsed, negate);
            assert_eq!(constant_value(&lowered), Some(expected), "{text}");
        }
        let huge = lower_int_literal(&int_of(&"9".repeat(60)), false);
        assert_eq!(constant_value(&huge), Some(None));
    }

    #[test]
    fn check_boundary_literal_rejects_an_expanded_literal_but_not_an_in_range_tree() {
        let wide = lower_int_literal(&int_of("9223372036854775808"), false);
        let err = check_boundary_literal(&wide, 1..2, "list index").unwrap_err();
        assert_eq!(err.code, "T0051");
        assert!(
            err.message.contains("9223372036854775808"),
            "{}",
            err.message
        );
        let huge = lower_int_literal(&int_of(&"9".repeat(60)), false);
        let err = check_boundary_literal(&huge, 1..2, "list index").unwrap_err();
        assert!(err.message.contains("beyond 2**127"), "{}", err.message);
        let small_tree = HirExpr::BinOp {
            op: BinOpKind::Add,
            left: Box::new(HirExpr::IntLiteral(3)),
            right: Box::new(HirExpr::IntLiteral(4)),
        };
        assert!(check_boundary_literal(&small_tree, 1..2, "list index").is_ok());
        let not_constant = HirExpr::BinOp {
            op: BinOpKind::Sub,
            left: Box::new(HirExpr::IntLiteral(3)),
            right: Box::new(HirExpr::IntLiteral(4)),
        };
        assert!(check_boundary_literal(&not_constant, 1..2, "list index").is_ok());
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
