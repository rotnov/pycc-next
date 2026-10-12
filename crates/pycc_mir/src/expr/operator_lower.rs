//! Lowering of unary/binary operators, comparisons, unpacking and f-strings.

use super::*;

// #603 (Part 2 of #573): `-x`/`+x` over a non-literal operand is
// rewritten into the equivalent binary expression rather than
// getting a `MirExpr` variant of its own -- pycc's MIR deliberately
// has no `USub`/`UAdd`/`Invert` unary node (see the #604 comment
// below for `not x`'s own dedicated `MirExpr::Not` exception to
// that rule, which also supersedes the "negated `__eq__` call"
// comparison this comment used to draw to the dataclass `!=`
// lowering further below -- that lowering now itself uses
// `MirExpr::Not` rather than illustrating "no unary node exists").
//
// The rewrite is representation-sensitive, so it is not a single
// uniform shape:
//
// * `int`/`bool` become `0 - x` / `0 + x`. `pycc_rt`'s `int_sub`
//   and `int_add` both handle an already-promoted bigint operand
//   and the smallint boundary, so negation inherits arbitrary
//   precision for free. `x * -1` would *not*: `int_mul` decodes
//   its operands inline and raises `OverflowError` on a bigint
//   (Part C of #1038).
// * `float` becomes `x * -1.0` / `x * 1.0`, a plain LLVM `fmul`.
//   `0.0 - x` would be wrong for `-0.0` (it yields `+0.0`), and
//   multiplication is exact for infinities and NaN too.
//
// `UAdd` is not folded away, because it is not the identity: on a
// `bool` operand it crosses into `int` (`+True` is `1`), which the
// `0 + x` / `x * 1.0` shapes deliver through the same typing rules
// as `USub`. Any non-numeric operand is already rejected by
// `pycc_types::unop::unary_result_type` and never reaches here.
//
// #604 (Part 3 of #573) adds `not x` and `~x`. `~x` follows the
// exact same "rewrite into a `BinOp`, no new MIR node" strategy as
// `USub`/`UAdd`: `~x == -x - 1` for every `int`/`bool` operand (the
// only operands `pycc_types::unop::unary_result_type` accepts for
// `Invert`), so it reuses the identical `0 - x` shape and its
// bigint-precision guarantee, then subtracts a further literal
// `1` -- both legs going through `int_sub`. `not x` genuinely has
// no binary-expression equivalent (its truthiness rule spans
// types `BinOp`'s own typing has no notion of), so it is the one
// case that *does* need a dedicated `MirExpr::Not` node, built
// directly on `pycc_codegen`'s existing `truthy` helper (the same
// one `if`/`while` conditions already call).
pub(crate) fn lower_unary_op_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::UnaryOp { op, operand } = expr else {
        unreachable!()
    };
    let operand = lower_expr(operand, scopes, classes, current_class);
    // Non-strict mode (#3): `-o`, `+o`, `~o` run CPython's own operator.
    if let Some(object_unary) = super::super::obj_arith::lower_object_unary(*op, &operand) {
        return object_unary;
    }
    match op {
        UnaryOpKind::USub | UnaryOpKind::UAdd => {
            let (bin_op, left, right) = if operand.ty() == Ty::Float {
                let factor = if matches!(op, UnaryOpKind::USub) {
                    -1.0
                } else {
                    1.0
                };
                (BinOpKind::Mul, operand, MirExpr::FloatLiteral(factor))
            } else {
                let bin_op = if matches!(op, UnaryOpKind::USub) {
                    BinOpKind::Sub
                } else {
                    BinOpKind::Add
                };
                (bin_op, MirExpr::IntLiteral(0), operand)
            };
            let ty = binop_result_ty(bin_op, left.ty(), right.ty());
            MirExpr::BinOp {
                op: bin_op,
                left: Box::new(left),
                right: Box::new(right),
                ty,
            }
        }
        UnaryOpKind::Invert => {
            let negated_ty = binop_result_ty(BinOpKind::Sub, Ty::Int, operand.ty());
            let negated = MirExpr::BinOp {
                op: BinOpKind::Sub,
                left: Box::new(MirExpr::IntLiteral(0)),
                right: Box::new(operand),
                ty: negated_ty.clone(),
            };
            let ty = binop_result_ty(BinOpKind::Sub, negated_ty, Ty::Int);
            MirExpr::BinOp {
                op: BinOpKind::Sub,
                left: Box::new(negated),
                right: Box::new(MirExpr::IntLiteral(1)),
                ty,
            }
        }
        UnaryOpKind::Not => MirExpr::Not(Box::new(operand)),
    }
}

// Part 1 of #891: a native tuple's arity was checked statically by
// `pycc_types`, so the tuple itself is the unpacked value; a CPython
// object is unpacked at run time by `MirExpr::ObjUnpack`.
pub(crate) fn lower_unpack_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::Unpack { value, arity } = expr else {
        unreachable!()
    };
    let value = lower_expr(value, scopes, classes, current_class);
    if value.ty() == Ty::Object {
        MirExpr::ObjUnpack {
            value: Box::new(value),
            arity: *arity,
        }
    } else if let Ty::List(_) = value.ty() {
        builtin_call::lower_list_unpack(value, *arity)
    } else {
        value
    }
}

pub(crate) fn lower_bin_op_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let (HirExpr::BinOp { op, left, right } | HirExpr::AugBinOp { op, left, right }) = expr else {
        unreachable!()
    };
    let left = lower_expr(left, scopes, classes, current_class);
    let right = lower_expr(right, scopes, classes, current_class);
    // `int ** <negative int literal>` is `float(int) ** float(literal)`,
    // as CPython's `long_pow` defers to `float_pow`.
    let (left, right) = match (&right, left.ty()) {
        (MirExpr::IntLiteral(exponent), Ty::Int | Ty::Bool)
            if *op == BinOpKind::Pow && *exponent < 0 =>
        {
            // The mixed `int ** float` converts the base as `float(n)`
            // does (`OverflowError` beyond range).
            #[allow(clippy::cast_precision_loss)]
            let exponent = MirExpr::FloatLiteral(*exponent as f64);
            (left, exponent)
        }
        _ => (left, right),
    };
    // Non-strict mode (#3): an object operand runs CPython's operator,
    // the in-place one for an augmented assignment.
    let in_place = matches!(expr, HirExpr::AugBinOp { .. });
    let (left, right) =
        match super::super::obj_arith::lower_object_binop(*op, in_place, left, right) {
            Ok(object_binop) => return object_binop,
            Err(operands) => *operands,
        };
    if let Some(sig) =
        pycc_hir::builtins::set_operator(op.as_str(), in_place, &left.ty(), &right.ty())
    {
        let mut args = vec![left, right];
        if sig.params.len() == 3 {
            args.push(MirExpr::IntLiteral(pycc_hir::builtins::set_inplace_index(
                op.as_str(),
            )));
        }
        return MirExpr::RtCall(Box::new(crate::RtCallExpr { sig, args }));
    }
    if in_place
        && let Some(sig) =
            pycc_hir::builtins::list_inplace_operator(op.as_str(), &left.ty(), &right.ty())
    {
        return MirExpr::RtCall(Box::new(crate::RtCallExpr {
            sig,
            args: vec![left, right],
        }));
    }
    let ty = binop_result_ty(*op, left.ty(), right.ty());
    MirExpr::BinOp {
        op: *op,
        left: Box::new(left),
        right: Box::new(right),
        ty,
    }
}

pub(crate) fn lower_compare_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::Compare { op, left, right } = expr else {
        unreachable!()
    };
    let mut left_lowered = lower_expr(left, scopes, classes, current_class);
    let mut right_lowered = lower_expr(right, scopes, classes, current_class);
    // #1476: an identity test compares the objects themselves, never
    // a narrowed native copy, so both narrowed operands are read raw.
    if matches!(op, CmpOpKind::Is | CmpOpKind::IsNot) {
        left_lowered = super::super::object_narrow::object_operand(left_lowered);
        right_lowered = super::super::object_narrow::object_operand(right_lowered);
    }
    if matches!(op, CmpOpKind::In | CmpOpKind::NotIn)
        && left_lowered.ty() == Ty::Str
        && right_lowered.ty() == Ty::Str
    {
        return builtin_call::lower_str_contains(
            left_lowered,
            right_lowered,
            *op == CmpOpKind::NotIn,
        );
    }
    // Part 1 of #1371: an object operand takes CPython's own
    // comparison, never the native one below.
    let (left_lowered, right_lowered) =
        match super::super::obj_compare::lower_object_compare(*op, left_lowered, right_lowered) {
            Ok(object_compare) => return object_compare,
            Err(operands) => *operands,
        };
    // #378 (PR-18): `==`/`!=` between same-class dataclass instances
    // is rewritten to a `MirExpr::Call` to the class's
    // compiler-synthesized `__eq__` method. `!=` is lowered as
    // `__eq__(left, right) != True` (a `MirExpr::Compare` against a
    // `BoolLiteral`, predating #604's `MirExpr::Not`) rather than
    // negating the call result with `MirExpr::Not` directly --
    // `MirExpr::Not` computes truthiness (bool/int/float/str/
    // None/Optional) via `pycc_codegen`'s `truthy` helper, but this
    // call's result is already a `bool`, so an equality compare
    // against `true` is the simpler, equally-correct rewrite. This
    // mirrors how `@property` redirects attribute access to method
    // calls -- a MIR-level rewrite, not a new MIR node. Only `Eq`
    // and `NotEq` are rewritten, and only for dataclass classes
    // (whose synthesized `__eq__` has a known-correct signature);
    // other comparison operators (`<`, `<=`, `>`, `>=`) and
    // non-dataclass classes
    // fall through to the default `MirExpr::Compare` (the type
    // checker rejects them with `T0021` before they reach MIR
    // lowering in normal compilation, but the MIR itself stays
    // semantically correct for defense-in-depth).
    if matches!(op, pycc_hir::CmpOpKind::Eq | pycc_hir::CmpOpKind::NotEq)
        && let Some(eq_mangled) = super::super::compare_chain::dataclass_eq_callee(
            &left_lowered.ty(),
            &right_lowered.ty(),
            classes,
        )
    {
        let eq_call = MirExpr::Call {
            callee: eq_mangled,
            args: vec![left_lowered, right_lowered],
            ty: Ty::Bool,
        };
        // The `matches!` guard above limits entry to
        // `Eq`/`NotEq`, so an if/else (rather than a match
        // with a `_` arm) suffices and avoids an unreachable
        // arm that would be permanently uncovered under
        // D-014's 100% coverage gate.
        if *op == pycc_hir::CmpOpKind::Eq {
            return eq_call;
        }
        return MirExpr::Compare {
            op: pycc_hir::CmpOpKind::NotEq,
            left: Box::new(eq_call),
            right: Box::new(MirExpr::BoolLiteral(true)),
            ty: Ty::Bool,
        };
    }
    MirExpr::Compare {
        op: *op,
        left: Box::new(left_lowered),
        right: Box::new(right_lowered),
        ty: Ty::Bool,
    }
}

// #1212: every operand is lowered once, left to right; a
// same-dataclass `==`/`!=` link keeps both operands as values and
// names the synthesized `__eq__` instead of owning them in a `Call`.
pub(crate) fn lower_compare_chain_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::CompareChain { first, links } = expr else {
        unreachable!()
    };
    super::super::compare_chain::lower_compare_chain(
        lower_expr(first, scopes, classes, current_class),
        links
            .iter()
            .map(|link| {
                (
                    link.op,
                    lower_expr(&link.right, scopes, classes, current_class),
                )
            })
            .collect(),
        classes,
    )
}

pub(crate) fn lower_f_string_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::FString(parts) = expr else {
        unreachable!()
    };
    MirExpr::FString(
        parts
            .iter()
            .map(|p| match p {
                FStringPart::Literal(s) => MirFStringPart::Literal(s.clone()),
                FStringPart::Interpolation(e) => {
                    let lowered = lower_expr(e, scopes, classes, current_class);
                    // #378 (PR-18): if the interpolation is a class
                    // instance with `__repr__`, rewrite it to a call to
                    // `__repr__` so the codegen's `to_str` receives a
                    // `str` scalar, not an Instance scalar (which would
                    // panic). This mirrors how `==`/`!=` on instances is
                    // rewritten to `__eq__` calls in the Compare arm.
                    // Part 3A of #541 (#736): try the exception-message
                    // rewrite first -- see the identical `print`-argument
                    // ordering comment above for why.
                    let lowered = rewrite_exception_to_message(&lowered, classes);
                    let rewritten = rewrite_instance_to_repr(&lowered, classes);
                    MirFStringPart::Interpolation(Box::new(rewritten))
                }
            })
            .collect(),
    )
}
