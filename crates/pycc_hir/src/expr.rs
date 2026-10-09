//! Expression lowering: `lower_expr` and its comprehension/range helper
//! cluster, the recursive-descent core that turns a `pycc_ast::Expr` into a
//! `HirExpr` (or a capability / context-invalidity diagnostic).
//!
//! Extracted from `lib.rs` per AGENTS.md's file-decomposition rule (issue
//! #361, mirroring #141's `stmt.rs` extraction, D-148): `lower_expr` and
//! every helper it needs formed one cohesive, already-separable unit, called
//! only from `lib.rs`, `stmt.rs` (a sibling module), and two unit tests in the
//! crate root's `mod tests` (`tests.rs`, moved out of `lib.rs` by issue #547)
//! that call `lower_comprehension_header` and `rename_name_in_expr` directly,
//! bypassing the public `lower_checked` entry point -- unlike `stmt.rs`'s own
//! extraction, this one is not fully test-transparent, so those two tests
//! stayed with the crate root's test module, which reaches them through an
//! explicit `use crate::expr::{...}` instead of them moving here.
//!
//! `in_function: bool` (D-149) is this module's new piece of state, mirroring
//! `in_loop`'s existing shape (`stmt.rs`, D-148) but on the expression side:
//! `true` exactly when the expression being lowered is (transitively) inside
//! a real function body reached via `lower_function`'s own dispatch; `false`
//! at module scope. It exists so `Expr::Yield`/`Expr::YieldFrom` can
//! distinguish a context-invalid occurrence (no enclosing function -- CPython
//! raises `SyntaxError`, so this reuses `L0001`, the same D-148 precedent) from
//! a valid-but-unimplemented occurrence (a real enclosing function -- still
//! `C0001`, generator codegen remains out of scope). Every position lexically
//! *inside a comprehension*'s own scope (`if`-filter `cond`, `elt`, `key`,
//! `value`, and -- as a documented, narrower exception, see
//! `comprehension::lower_comprehension_iter` -- the comprehension's outermost
//! iterable) instead hardcodes a literal `true`, deliberately preserving
//! today's exact `C0001`-in-both-scopes behavior for a comprehension-internal
//! `yield`/`yield from`: CPython's real rule there is a third,
//! scope-independent classification (`'yield' inside list comprehension`)
//! this issue does not implement (see D-149 for the full rationale). This is
//! why the comprehension helpers in `comprehension.rs` need no new parameter at
//! all -- they never forward the ambient `in_function` value, only the one
//! literal that reproduces current behavior.

mod bin_op_kind;
mod comprehension;
mod container_call;
pub(crate) mod keyword_bind;
pub(crate) mod object_keyword_call;
mod receiver_class_call;
pub(crate) mod receiver_dispatch;
mod std_receiver;
mod subscript_call;
pub(crate) mod unobservable;

pub(crate) use bin_op_kind::bin_op_kind;
pub(crate) use comprehension::{
    comp_assign_stmt, lower_dict_comp, lower_list_comp, lower_set_comp,
};
#[cfg(test)]
pub(crate) use comprehension::{lower_comprehension_header, rename_name_in_expr};
use keyword_bind::SignatureTable;

use crate::boolop::{fold_bool_op, mark_truth_context};
use crate::compare_chain::{lower_cmp_op, lower_compare_chain};
use crate::int_boundary::check_boundary_literal;
use crate::{
    BinOpKind, BoolOpKind, FStringPart, HirExpr, ImportBinding, Ty, UnaryOpKind, context_invalid,
    unsupported,
};
use pycc_ast::{BoolOp, Expr, Int, Number, UnaryOp};
use pycc_diag::Diagnostic;
pub use receiver_dispatch::receiver_takes_method_path;
use std_receiver::describe_module;
pub(crate) use std_receiver::std_receiver;

/// Resolves a PEP 695 generic-class type argument (the `int` in `C[int]`)
/// to a `Ty`. PEP 695 generic class instantiation is scoped to scalar-only
/// types (D-133/D-134), so only `int`/`float`/`bool`/`str` are recognized —
/// matching `annotation_to_ty`'s own bare-name-to-`Ty` mapping for those
/// four scalars, without needing the full `aliases`/`type_param`/`class_name`
/// context `annotation_to_ty` threads for method annotations.
fn type_arg_name_to_ty(slice: &Expr) -> Result<Ty, Diagnostic> {
    let Expr::Name(name) = slice else {
        return Err(unsupported(
            "a generic class type argument must be a bare type name (int/float/bool/str) \
             so far -- subscript expressions are not supported yet",
            pycc_ast::expr_range(slice),
        ));
    };
    match name.id.as_str() {
        "int" => Ok(Ty::Int),
        "float" => Ok(Ty::Float),
        "bool" => Ok(Ty::Bool),
        "str" => Ok(Ty::Str),
        other => Err(unsupported(
            format!(
                "a generic class type argument `{other}` is not supported yet \
                 -- only int/float/bool/str are (D-133/D-134 scalar-only scope)"
            ),
            pycc_ast::expr_range(slice),
        )),
    }
}

/// #433: Recognizes a zero-arg `super()` call expression — `Expr::Call`
/// whose `func` is `Expr::Name("super")` and whose argument list is empty.
/// Used by `lower_expr`'s `Expr::Call` and `Expr::Attribute` arms to detect
/// `super().method(args)` and `super().attr` respectively, lowering both
/// to a `HirExpr::Super` base instead of letting the `super` name fall
/// through to the ordinary (unsupported-builtin) `Call` path.
pub(crate) fn is_zero_arg_super_call(expr: &Expr) -> bool {
    let Expr::Call(call) = expr else {
        return false;
    };
    let Expr::Name(name) = call.func.as_ref() else {
        return false;
    };
    name.id.as_str() == "super"
        && call.arguments.keywords.is_empty()
        && call.arguments.args.is_empty()
}

/// #602: applies a source-level unary sign to an integer literal's own
/// magnitude, in the literal's arbitrary-precision form, *before* the `i64`
/// range check.
///
/// The order matters for exactly one value. `ruff`'s `Int` stores its
/// magnitude as a `u64`, and `-9223372036854775808` parses as `USub` applied
/// to the literal `9223372036854775808` -- a magnitude that does not fit in
/// an `i64` even though its negation is precisely `i64::MIN`. Range-checking
/// the operand first (what `lower_expr`'s own unsigned `Number::Int` arm
/// does, correctly, for an unsigned literal) would reject that source as out
/// of range. Checking after the sign is applied accepts it.
pub(crate) fn fold_int_literal_sign(
    value: &Int,
    negate: bool,
    range: std::ops::Range<u32>,
) -> Result<i64, Diagnostic> {
    let magnitude = value.as_u64();
    let folded = match magnitude {
        // `i64::MIN`'s magnitude is one past `i64::MAX`, so it is
        // representable only when the sign is actually negative.
        Some(m) if negate && m == (i64::MAX as u64) + 1 => Some(i64::MIN),
        Some(m) => i64::try_from(m).ok().map(|v| if negate { -v } else { v }),
        None => None,
    };
    folded.ok_or_else(|| {
        unsupported(
            format!("integer literal does not fit in i64: {value:?}"),
            range,
        )
    })
}

/// Lowers an expression whose value is consumed only for its truth: an
/// `if`/`elif`/`while` test, a comprehension `if` filter, or the operand of
/// `not` (#1211). Identical to [`lower_expr`] except that an `and`/`or` at
/// the top, or under further `and`/`or`/`not` nodes, is marked truth-only
/// (see [`crate::boolop`]).
pub(crate) fn lower_condition(
    expr: &Expr,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirExpr, Diagnostic> {
    let mut lowered = lower_expr(expr, in_function, class_name, imports, signatures)?;
    mark_truth_context(&mut lowered);
    Ok(lowered)
}

/// Lowers one expression.
///
/// `imports` is the enclosing module's import table as it stands at the
/// point the enclosing item is lowered (Part 1 of #883, #962). It is
/// threaded through every lowering function that transitively reaches
/// this one and is read in exactly two places: [`std_receiver`], which
/// resolves an aliased stdlib receiver (`import math as m` then
/// `m.sqrt(x)`) at the call-shaped and bare-attribute stdlib arms below,
/// and `stmt::is_type_checking_guard`, which lets `t.TYPE_CHECKING` fold
/// when `t` aliases `typing`. Every other arm ignores it.
///
/// `signatures` is the enclosing module's keyword-bindable signature table
/// (Part 1 of #884, #1125), collected from its top-level `def`s before any
/// item is lowered and threaded exactly as `imports` is. It is read in
/// exactly one place: the `Expr::Call` arm below, which uses it to decide
/// whether a keyword call is bindable at all, to bind one that is, and --
/// since Part 2 of #884 (#1189) -- to decide whether a zero-keyword call
/// short of its callee's arity must have trailing defaults filled.
/// Every other arm ignores it.
pub(crate) fn lower_expr(
    expr: &Expr,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirExpr, Diagnostic> {
    let lowered = match expr {
        Expr::NumberLiteral(lit) => match &lit.value {
            Number::Int(i) => {
                let Some(value) = i.as_i64() else {
                    return Err(unsupported(
                        format!("integer literal does not fit in i64: {i:?}"),
                        lit.range,
                    ));
                };
                HirExpr::IntLiteral(value)
            }
            Number::Float(f) => HirExpr::FloatLiteral(*f),
            other => {
                return Err(unsupported(
                    format!("numeric literal kind not supported yet: {other:?}"),
                    lit.range,
                ));
            }
        },
        // #602 (Part 1 of #573): a source-level negative number is not a
        // negative literal in the AST -- `-5` parses as `UnaryOp { op: USub,
        // operand: NumberLiteral(5) }`. `HirExpr::IntLiteral` is already an
        // `i64` and `FloatLiteral` an `f64`, and every downstream crate
        // already handles negative values in them, so folding the sign into
        // the literal here needs no new HIR variant and no change to
        // `pycc_types`, `pycc_mir`, or `pycc_codegen`. Only a *literal*
        // operand folds: `-x` for a variable needs a real `HirExpr` variant
        // and downstream arms, which the next arm supplies (#603, Part 2);
        // `not x` and `~x` (#604, Part 3) get no such fold either, since
        // neither is part of Python's numeric-literal grammar.
        Expr::UnaryOp(unary) => match (unary.op, unary.operand.as_ref()) {
            (UnaryOp::USub | UnaryOp::UAdd, Expr::NumberLiteral(lit)) => {
                let negate = matches!(unary.op, UnaryOp::USub);
                match &lit.value {
                    Number::Int(i) => HirExpr::IntLiteral(fold_int_literal_sign(
                        i,
                        negate,
                        pycc_ast::expr_range(expr),
                    )?),
                    Number::Float(f) => HirExpr::FloatLiteral(if negate { -*f } else { *f }),
                    other => {
                        return Err(unsupported(
                            format!("numeric literal kind not supported yet: {other:?}"),
                            lit.range,
                        ));
                    }
                }
            }
            // #603 (Part 2 of #573): every `-`/`+` whose operand is not a
            // numeric literal. The literal arm above still runs first, so
            // `-1` keeps folding into a signed `IntLiteral` with no node at
            // all; this arm covers only what folding cannot reach (`-x`,
            // `-f(y)`, `-(a + b)`).
            (UnaryOp::USub | UnaryOp::UAdd, operand) => HirExpr::UnaryOp {
                op: if matches!(unary.op, UnaryOp::USub) {
                    UnaryOpKind::USub
                } else {
                    UnaryOpKind::UAdd
                },
                operand: Box::new(lower_expr(
                    operand,
                    in_function,
                    class_name,
                    imports,
                    signatures,
                )?),
            },
            // #604 (Part 3 of #573): `not x` and `~x`. Neither operator has
            // a literal-folding arm the way `USub`/`UAdd` do above --
            // `not 5` and `~5` are not part of Python's numeric-literal
            // grammar the way a source-level `-5` is, so every operand
            // (literal or not) lowers into the same `HirExpr::UnaryOp` node
            // and is typed/rewritten downstream.
            //
            // The operand of `not` is a truth position (#1211): lowering it
            // through `lower_condition` marks an `and`/`or` operand
            // truth-only, so `not (n and s)` needs no common type.
            (UnaryOp::Not, operand) => HirExpr::UnaryOp {
                op: UnaryOpKind::Not,
                operand: Box::new(lower_condition(
                    operand,
                    in_function,
                    class_name,
                    imports,
                    signatures,
                )?),
            },
            (UnaryOp::Invert, operand) => HirExpr::UnaryOp {
                op: UnaryOpKind::Invert,
                operand: Box::new(lower_expr(
                    operand,
                    in_function,
                    class_name,
                    imports,
                    signatures,
                )?),
            },
        },
        // #1211 (Part 3 of #1018): `and`/`or`, right-folded by
        // `crate::boolop::fold_bool_op`. A walrus is admitted only in the
        // first operand, which always executes: every binding walker
        // (`pycc_types`' and `pycc_mir`'s `collect_named_expr_bindings`,
        // `pre_bind_named_expr_targets`) binds unconditionally on the premise
        // that the enclosing test runs, and a later operand may be
        // short-circuited away.
        Expr::BoolOp(bool_op) => {
            let op = match bool_op.op {
                BoolOp::And => BoolOpKind::And,
                BoolOp::Or => BoolOpKind::Or,
            };
            let mut operands = Vec::with_capacity(bool_op.values.len());
            for (index, value) in bool_op.values.iter().enumerate() {
                let lowered = lower_expr(value, in_function, class_name, imports, signatures)?;
                if index > 0 && contains_named_expr(&lowered) {
                    return Err(unsupported(
                        "a walrus assignment (`:=`) in a short-circuited `and`/`or` operand is not supported",
                        pycc_ast::expr_range(value),
                    ));
                }
                operands.push(lowered);
            }
            fold_bool_op(op, operands)
        }
        // #1395: `body if test else orelse`. `test` always runs, first, and
        // only for its truth, so it is a truth position and may hold a
        // walrus; either branch may not run, so a walrus there is refused
        // for the same reason as in a later `and`/`or` operand.
        Expr::If(if_exp) => {
            let mut test = lower_expr(&if_exp.test, in_function, class_name, imports, signatures)?;
            mark_truth_context(&mut test);
            let mut branches = Vec::with_capacity(2);
            for branch in [&if_exp.body, &if_exp.orelse] {
                let lowered = lower_expr(branch, in_function, class_name, imports, signatures)?;
                if contains_named_expr(&lowered) {
                    return Err(unsupported(
                        "a walrus assignment (`:=`) in a conditional expression branch is not supported",
                        pycc_ast::expr_range(branch),
                    ));
                }
                branches.push(lowered);
            }
            let orelse = branches.pop().expect("two branches were pushed");
            let body = branches.pop().expect("two branches were pushed");
            HirExpr::IfExp {
                test: Box::new(test),
                body: Box::new(body),
                orelse: Box::new(orelse),
            }
        }
        Expr::Name(name) => HirExpr::Name(name.id.as_str().to_string()),
        Expr::List(list) => HirExpr::ListLiteral(
            list.elts
                .iter()
                .map(|e| {
                    let lowered = lower_expr(e, in_function, class_name, imports, signatures)?;
                    check_boundary_literal(
                        &lowered,
                        pycc_ast::expr_range(e),
                        "list-literal element",
                    )?;
                    Ok(lowered)
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Expr::Dict(dict) => HirExpr::DictLiteral(
            dict.items
                .iter()
                .map(|item| {
                    let Some(key) = &item.key else {
                        return Err(unsupported(
                            "dict-unpacking (`**expr`) inside a dict literal is not supported yet",
                            pycc_ast::expr_range(&item.value),
                        ));
                    };
                    let key = lower_expr(key, in_function, class_name, imports, signatures)?;
                    let value =
                        lower_expr(&item.value, in_function, class_name, imports, signatures)?;
                    check_boundary_literal(
                        &value,
                        pycc_ast::expr_range(&item.value),
                        "dict-literal value",
                    )?;
                    Ok((key, value))
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Expr::Set(set) => HirExpr::SetLiteral(
            set.elts
                .iter()
                .map(|e| {
                    let lowered = lower_expr(e, in_function, class_name, imports, signatures)?;
                    check_boundary_literal(
                        &lowered,
                        pycc_ast::expr_range(e),
                        "set-literal element",
                    )?;
                    Ok(lowered)
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Expr::Tuple(tuple) => HirExpr::TupleLiteral(
            tuple
                .elts
                .iter()
                .map(|e| lower_expr(e, in_function, class_name, imports, signatures))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Expr::Subscript(sub) => match sub.slice.as_ref() {
            // A colon-containing subscript (`xs[a:b:c]`) parses its `slice`
            // field as `Expr::Slice`, distinct from the plain single-
            // expression `slice` an ordinary index (`xs[0]`) produces
            // (PR-12, D-118). Each bound is independently optional in real
            // Python's own grammar, so each is lowered through
            // `Option::map`/`.transpose()` rather than assumed present.
            Expr::Slice(slice) => {
                let lower_bound = |e: &Expr| -> Result<HirExpr, Diagnostic> {
                    let lowered = lower_expr(e, in_function, class_name, imports, signatures)?;
                    check_boundary_literal(&lowered, pycc_ast::expr_range(e), "slice bound")?;
                    Ok(lowered)
                };
                HirExpr::Slice {
                    base: Box::new(lower_expr(
                        &sub.value,
                        in_function,
                        class_name,
                        imports,
                        signatures,
                    )?),
                    start: slice
                        .lower
                        .as_deref()
                        .map(lower_bound)
                        .transpose()?
                        .map(Box::new),
                    stop: slice
                        .upper
                        .as_deref()
                        .map(lower_bound)
                        .transpose()?
                        .map(Box::new),
                    step: slice
                        .step
                        .as_deref()
                        .map(lower_bound)
                        .transpose()?
                        .map(Box::new),
                }
            }
            _ => {
                let base = Box::new(lower_expr(
                    &sub.value,
                    in_function,
                    class_name,
                    imports,
                    signatures,
                )?);
                let index = lower_expr(&sub.slice, in_function, class_name, imports, signatures)?;
                // #618/D-207 (finding from PR #827 review): a tuple base has
                // no D-141 runtime `int`-boundary position at all -- tuple
                // indexing is resolved entirely at compile time in
                // `pycc_types::check_expr`'s own `Ty::Tuple` arm, which
                // already rejects an out-of-range literal index with T0040
                // ("non-negative literal within range"). Emitting T0051
                // unconditionally here, before the base's type is known,
                // would preempt that existing T0040 check and mislabel the
                // position as a "list index" for a tuple. HIR lowering can
                // only recognize a tuple base syntactically when it is
                // itself a tuple *literal* (`(1, 2)[huge]`); a tuple value
                // held in a variable is indistinguishable from a list at
                // this stage without type information `pycc_hir` does not
                // have, so that case is an accepted, documented gap
                // mirroring the `str * int` repeat-count narrowing
                // elsewhere in this module -- see `crate::int_boundary`'s
                // doc comment.
                if !matches!(sub.value.as_ref(), Expr::Tuple(_)) {
                    check_boundary_literal(&index, pycc_ast::expr_range(&sub.slice), "list index")?;
                }
                HirExpr::Subscript {
                    base,
                    index: Box::new(index),
                }
            }
        },
        Expr::Call(call) => {
            // Part 1 of #884 (#1125) narrows this rejection in place rather
            // than moving it: the call-shape arms that follow (method calls,
            // `super().m()`, container and stdlib intrinsics, generic class
            // instantiation, subscript callees) never inspect `keywords`, so
            // relocating the guard past them would silently erase keyword
            // arguments on every one of those shapes. `is_bindable_call`
            // answers `true` only for the one shape the tail of this arm can
            // actually bind.
            //
            // Part 8 of #1371: a keyword call the binder cannot bind is no
            // longer refused outright -- `object_keyword_call` keeps it as a
            // `KeywordCall` for the shapes whose callee can be a CPython
            // object, and refuses every other shape with the same `C0001`.
            if !call.arguments.keywords.is_empty()
                && !keyword_bind::is_bindable_call(signatures, call)
            {
                return object_keyword_call::lower(
                    call,
                    in_function,
                    class_name,
                    imports,
                    signatures,
                );
            }
            if let Expr::Attribute(attr) = call.func.as_ref() {
                // #433: `super().method(args)` — recognize a zero-arg
                // `super()` call as the receiver of a method call, before
                // any container-method or stdlib fast path. Lower to
                // `HirExpr::MethodCall { base: Super, method, args }` so
                // the type checker and MIR lowering can resolve `method`
                // starting from the next class in the MRO (D-006 static
                // dispatch, per the #433 ADR). A `super()` outside a
                // method body (`class_name` is `None`) is rejected here
                // with C0001, matching CPython's own `RuntimeError: super()
                // no arguments` / `NameError: __class__` for the same shape.
                if is_zero_arg_super_call(&attr.value) {
                    if class_name.is_none() {
                        return Err(unsupported(
                            "`super()` outside a method body is not supported",
                            pycc_ast::expr_range(&attr.value),
                        ));
                    }
                    let args = call
                        .arguments
                        .args
                        .iter()
                        .map(|e| lower_expr(e, in_function, class_name, imports, signatures))
                        .collect::<Result<Vec<_>, _>>()?;
                    return Ok(HirExpr::MethodCall {
                        base: Box::new(HirExpr::Super),
                        method: attr.attr.to_string(),
                        args,
                    });
                }
                // Issue #1188: in a module from which a user class defining
                // a method of this name is reachable, keep both readings and
                // let the receiver's static type choose between them.
                if signatures.dispatches_on_receiver(attr.attr.as_str()) {
                    return receiver_dispatch::lower_receiver_dispatched_call(
                        call,
                        attr,
                        in_function,
                        class_name,
                        imports,
                        signatures,
                    );
                }
                if let Some(lowered) = container_call::lower_container_method_call(
                    call,
                    attr,
                    in_function,
                    class_name,
                    imports,
                    signatures,
                ) {
                    return lowered;
                }
                // `math.sqrt(x)`-shaped stdlib intrinsic call (D-136/D-137).
                // The receiver is resolved by `std_receiver`: an alias bound
                // by `import math as m` in the module's import table first
                // (Part 1 of #883, #962), then the textual
                // `pycc_std::resolve_module` fallback, the same precedent
                // this file already uses for `X: TypeAlias` (see
                // `lower_legacy_type_alias_ann_assign`'s doc comment). The
                // textual fallback is still not import-gated: `math.sqrt(x)`
                // lowers without an `import math` in scope, a real,
                // deliberate D-136 scope trim from a fully import-gated
                // design (#768 tracks closing it). A receiver that is a
                // *local* binding of the same name is caught downstream by
                // `pycc_types`' alias-aware shadow check, the first stage
                // with binding-scope information -- but only when the
                // accessed symbol itself resolves: an unregistered symbol
                // is rejected right here, before `pycc_types` runs, so a
                // local `m` whose method is not a registered `math` symbol
                // is a false reject (D-231 residual (c)). The emitted
                // callee is always the module's canonical spelling, never
                // the alias.
                if let Expr::Name(receiver) = attr.value.as_ref()
                    && let Some(module) = std_receiver(receiver.id.as_str(), imports)
                {
                    // Unlike the generic `MethodCall` fallback below, a
                    // receiver that *is* a resolvable stdlib module keeps
                    // its existing "not registered" rejection even when the
                    // called symbol itself doesn't resolve -- falling
                    // through to `MethodCall` here would silently turn
                    // `math.tan(1.0)` (a real module, an unregistered
                    // symbol) into "call method `tan` on `math`", losing
                    // the far more precise stdlib diagnostic.
                    let Some(symbol) = pycc_std::resolve_symbol(module, attr.attr.as_str()) else {
                        return Err(unsupported(
                            format!(
                                "module {} has no importable symbol named `{}`",
                                describe_module(module, receiver.id.as_str()),
                                attr.attr
                            ),
                            call.range,
                        ));
                    };
                    let args = call
                        .arguments
                        .args
                        .iter()
                        .map(|e| lower_expr(e, in_function, class_name, imports, signatures))
                        .collect::<Result<Vec<_>, _>>()?;
                    return Ok(HirExpr::Call {
                        callee: format!("{}.{}", pycc_std::module_name(module), symbol.name),
                        args,
                    });
                }
                // `base.method(args)` (D-154, Part 1 of #375): the generic
                // instance-method-call fallback, tried only after every
                // hand-recognized container method and the stdlib-module
                // call above -- both must keep winning first, or e.g.
                // `xs.append(1)` would start lowering as a `MethodCall`
                // instead of the dedicated `ListAppend` node, silently
                // breaking every existing container-method conformance
                // fixture. `base` is lowered generically (mirroring
                // `Expr::Attribute`'s own instance-attribute-read fallback
                // below): this lowering step has no type information to
                // narrow it further, so `pycc_types` is the one that
                // rejects a method call on a non-instance-typed receiver or
                // an unknown method name.
                return receiver_dispatch::lower_method_call(
                    call,
                    attr,
                    in_function,
                    class_name,
                    imports,
                    signatures,
                );
            }
            // PEP 695 (#387): `C[int](args)` — a generic class
            // instantiation — or, since Part 2a of #1371, a call of a
            // subscript result (`table[k](args)`). `subscript_call` owns the
            // rule that tells the two apart.
            if let Expr::Subscript(sub) = call.func.as_ref() {
                return subscript_call::lower(
                    call,
                    sub,
                    in_function,
                    class_name,
                    imports,
                    signatures,
                );
            }
            // #1411: `type(self)(args)` -- `receiver_class_call` owns the rule.
            if let Some(inner) = receiver_class_call::type_call_callee(call) {
                return receiver_class_call::lower(
                    call,
                    inner,
                    in_function,
                    class_name,
                    imports,
                    signatures,
                );
            }
            // #433: a bare `super()` not used as a method-call or
            // attribute-access base (e.g. `x = super()`) has no useful
            // static-dispatch lowering on its own — reject it here with
            // C0001 rather than letting `super` fall through to the
            // known-but-unsupported-builtin path, which would produce a
            // less precise diagnostic. `super().method()` and `super().attr`
            // are already handled above (in the `Expr::Attribute` arm of
            // this `Expr::Call` match), so a bare `super()` reaching here
            // is genuinely a standalone use.
            if is_zero_arg_super_call(expr) {
                return Err(unsupported(
                    "a bare `super()` expression is not supported — use `super().method()` or `super().attr`",
                    call.range,
                ));
            }
            let Expr::Name(callee) = call.func.as_ref() else {
                return Err(unsupported(
                    // The callee here is never a `Name`, `Attribute`, or
                    // `Subscript` -- those are handled or rejected earlier
                    // in this arm -- so the dominant case, `g()()`, reads
                    // "got a call whose callee is a call expression".
                    format!(
                        "only calling a bare name is supported so far, got a call whose callee is {}",
                        pycc_ast::expr_kind_name(&call.func)
                    ),
                    pycc_ast::expr_range(&call.func),
                ));
            };
            let args = call
                .arguments
                .args
                .iter()
                .map(|e| lower_expr(e, in_function, class_name, imports, signatures))
                .collect::<Result<Vec<_>, _>>()?;
            let args = if call.arguments.keywords.is_empty() {
                // Part 2 of #884 (#1189): a zero-keyword call that is short
                // of its callee's arity is routed through the same binder,
                // which fills each unsupplied defaulted parameter. Every
                // other all-positional call keeps the unbound path, so the
                // arity diagnostic for an over-long call stays where it was.
                if keyword_bind::needs_default_fill(signatures, callee.id.as_str(), args.len()) {
                    keyword_bind::bind_keyword_arguments(
                        signatures,
                        callee.id.as_str(),
                        args,
                        Vec::new(),
                        std::ops::Range::<u32>::from(call.range),
                    )?
                } else {
                    args
                }
            } else {
                let mut supplied = Vec::with_capacity(call.arguments.keywords.len());
                for keyword in &call.arguments.keywords {
                    let name = keyword
                        .arg
                        .as_ref()
                        .expect("`is_bindable_call` rejected `**kwargs` unpacking");
                    supplied.push((
                        name.as_str().to_string(),
                        std::ops::Range::<u32>::from(keyword.range),
                        lower_expr(&keyword.value, in_function, class_name, imports, signatures)?,
                    ));
                }
                keyword_bind::bind_keyword_arguments(
                    signatures,
                    callee.id.as_str(),
                    args,
                    supplied,
                    std::ops::Range::<u32>::from(call.range),
                )?
            };
            HirExpr::Call {
                callee: callee.id.as_str().to_string(),
                args,
            }
        }
        Expr::BinOp(bin_op) => {
            let Some(op) = bin_op_kind(bin_op.op) else {
                return Err(unsupported(
                    format!(
                        "binary operator `{}` is not supported yet",
                        bin_op.op.as_str()
                    ),
                    bin_op.range,
                ));
            };
            let left = lower_expr(&bin_op.left, in_function, class_name, imports, signatures)?;
            let right = lower_expr(&bin_op.right, in_function, class_name, imports, signatures)?;
            // #618: `str` repeat count. Only the case where the *string*
            // side is itself a string literal is recognized here -- see
            // `crate::int_boundary`'s doc comment for why a `str`-typed
            // variable multiplied by an oversized literal is a documented,
            // narrower out-of-scope gap rather than a missed case.
            if op == BinOpKind::Mul {
                if matches!(bin_op.left.as_ref(), Expr::StringLiteral(_)) {
                    check_boundary_literal(
                        &right,
                        pycc_ast::expr_range(&bin_op.right),
                        "`str` repeat count",
                    )?;
                } else if matches!(bin_op.right.as_ref(), Expr::StringLiteral(_)) {
                    check_boundary_literal(
                        &left,
                        pycc_ast::expr_range(&bin_op.left),
                        "`str` repeat count",
                    )?;
                }
            }
            HirExpr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            }
        }
        Expr::BooleanLiteral(lit) => HirExpr::BoolLiteral(lit.value),
        Expr::StringLiteral(lit) => HirExpr::StringLiteral(lit.value.to_str().to_string()),
        Expr::NoneLiteral(_) => HirExpr::NoneLiteral,
        Expr::FString(fstring) => {
            let parts = fstring
                .value
                .elements()
                .map(|element| -> Result<FStringPart, Diagnostic> {
                    Ok(match element {
                        pycc_ast::InterpolatedStringElement::Literal(lit) => {
                            FStringPart::Literal(lit.value.to_string())
                        }
                        pycc_ast::InterpolatedStringElement::Interpolation(interp) => {
                            if interp.debug_text.is_some() {
                                // #720: the `=` debug specifier (`f"{n=}"`) renders the
                                // source text plus `repr(value)`, which this crate does
                                // not implement. Silently discarding `debug_text` would
                                // compile cleanly and print the wrong value, so reject
                                // it explicitly instead.
                                return Err(unsupported(
                                    "f-string debug specifier (=) is not supported yet",
                                    interp.range,
                                ));
                            }
                            if interp.conversion != pycc_ast::ConversionFlag::None {
                                return Err(unsupported(
                                    "f-string conversion flags (!r/!s/!a) are not supported yet",
                                    interp.range,
                                ));
                            }
                            if interp.format_spec.is_some() {
                                return Err(unsupported(
                                    "f-string format spec ({x:...}) is not supported yet",
                                    interp.range,
                                ));
                            }
                            FStringPart::Interpolation(Box::new(lower_expr(
                                &interp.expression,
                                in_function,
                                class_name,
                                imports,
                                signatures,
                            )?))
                        }
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            HirExpr::FString(parts)
        }
        // #1212 (Part 4 of #1018): two or more operators lower to a
        // `HirExpr::CompareChain` (see `crate::compare_chain`); a single
        // comparison stays `HirExpr::Compare`.
        Expr::Compare(cmp) if cmp.ops.len() >= 2 => {
            lower_compare_chain(cmp, in_function, class_name, imports, signatures)?
        }
        Expr::Compare(cmp) => {
            let op = lower_cmp_op(
                cmp.ops[0],
                &cmp.left,
                &cmp.comparators[0],
                cmp.range.into(),
                true,
            )?;
            HirExpr::Compare {
                op,
                left: Box::new(lower_expr(
                    &cmp.left,
                    in_function,
                    class_name,
                    imports,
                    signatures,
                )?),
                right: Box::new(lower_expr(
                    &cmp.comparators[0],
                    in_function,
                    class_name,
                    imports,
                    signatures,
                )?),
            }
        }
        // `math.pi`-shaped bare stdlib constant reference (D-136/D-137),
        // e.g. `print(math.pi)`. A call-shaped `math.sqrt(x)` is handled
        // separately inside the `Expr::Call` arm above (it needs the call
        // arguments, which this bare-attribute position never has). The
        // receiver is resolved by `std_receiver` exactly as on that arm:
        // alias binding first, textual (non-import-gated) spelling second.
        // Encoded as `HirExpr::Name("math.pi")` in the module's canonical
        // spelling: real Python identifiers can never contain `.`, so this
        // qualified spelling is an unambiguous marker `pycc_types`'
        // ordinary name lookup can special-case without any risk of
        // colliding with a real variable named `pi`.
        Expr::Attribute(attr) => {
            if let Expr::Name(receiver) = attr.value.as_ref()
                && let Some(module) = std_receiver(receiver.id.as_str(), imports)
            {
                let Some(symbol) = pycc_std::resolve_symbol(module, attr.attr.as_str()) else {
                    return Err(unsupported(
                        format!(
                            "module {} has no attribute `{}`",
                            describe_module(module, receiver.id.as_str()),
                            attr.attr
                        ),
                        pycc_ast::expr_range(expr),
                    ));
                };
                return Ok(HirExpr::Name(format!(
                    "{}.{}",
                    pycc_std::module_name(module),
                    symbol.name
                )));
            }
            // #433: `super().attr` — recognize a zero-arg `super()` call as
            // the base of an attribute access, before the generic fallback
            // below. Lower to `HirExpr::AttrGet { base: Super, attr }` so
            // the type checker and MIR lowering can resolve `attr` starting
            // from the next class in the MRO. Same `class_name.is_none()`
            // rejection as the `super().method()` arm above.
            if is_zero_arg_super_call(&attr.value) {
                if class_name.is_none() {
                    return Err(unsupported(
                        "`super()` outside a method body is not supported",
                        pycc_ast::expr_range(&attr.value),
                    ));
                }
                return Ok(HirExpr::AttrGet {
                    base: Box::new(HirExpr::Super),
                    attr: attr.attr.to_string(),
                });
            }
            // `base.attr` (D-154, Part 1 of #375): the generic
            // instance-attribute-read fallback, tried only after the
            // stdlib-module case above -- a receiver that *is* a resolvable
            // module keeps its existing "no attribute named ..." rejection
            // unchanged rather than falling through here (that error names
            // the exact reason far more precisely than a generic
            // "not a declared attribute" `pycc_types` diagnostic could).
            // Every other receiver shape -- `self`, any other bare name,
            // or an arbitrary nested expression -- lowers `base` generically
            // and defers to `pycc_types` to reject a non-instance base or an
            // attribute name the base's class never declares.
            HirExpr::AttrGet {
                base: Box::new(lower_expr(
                    &attr.value,
                    in_function,
                    class_name,
                    imports,
                    signatures,
                )?),
                attr: attr.attr.to_string(),
            }
        }
        // `yield`/`yield from` outside any function body is a CPython
        // `SyntaxError`, not "valid but unimplemented" (D-149, the
        // expression-lowering sequel to D-148's `break`/`continue`/`async
        // for` precedent) -- reused as `L0001`, matching `context_invalid`'s
        // existing convention. The match guard is deliberately `!in_function`
        // so the valid-context case (`in_function == true`, a real enclosing
        // function) falls through unchanged to the generic `other =>`
        // fallback below, preserving today's `C0001` "expression kind not
        // supported yet" classification there byte-for-byte (generator
        // codegen itself remains out of scope, D-149).
        Expr::Yield(y) if !in_function => {
            return Err(context_invalid("'yield' outside function", y.range));
        }
        Expr::YieldFrom(yf) if !in_function => {
            return Err(context_invalid("'yield from' outside function", yf.range));
        }
        // #1254 (D-250): a comprehension in any expression position. The
        // `name = <comp>` statement form is recognized earlier, in
        // `stmt::assign`, and builds its statement from the same node.
        Expr::ListComp(comp) => HirExpr::Comprehension(Box::new(lower_list_comp(
            comp, class_name, imports, signatures,
        )?)),
        Expr::SetComp(comp) => HirExpr::Comprehension(Box::new(lower_set_comp(
            comp, class_name, imports, signatures,
        )?)),
        Expr::DictComp(comp) => HirExpr::Comprehension(Box::new(lower_dict_comp(
            comp, class_name, imports, signatures,
        )?)),
        // PEP 572 (#774): `target := value`. CPython's own grammar only ever
        // parses a bare identifier as a walrus target -- there is no
        // tuple/attribute/subscript walrus target to reject here in
        // practice, but the check is kept explicit (rather than an
        // unchecked `Expr::Name` pattern) so a future `ruff_python_parser`
        // upgrade that somehow relaxed the grammar would still surface a
        // clean diagnostic instead of an `unreachable!()`/panic.
        Expr::Named(named) => {
            let Expr::Name(target) = named.target.as_ref() else {
                return Err(unsupported(
                    "a walrus assignment target must be a bare name",
                    pycc_ast::expr_range(&named.target),
                ));
            };
            HirExpr::NamedExpr {
                name: target.id.as_str().to_string(),
                value: Box::new(lower_expr(
                    &named.value,
                    in_function,
                    class_name,
                    imports,
                    signatures,
                )?),
            }
        }
        other => {
            return Err(unsupported(
                format!(
                    "expression kind not supported yet: {}",
                    pycc_ast::expr_kind_name(other)
                ),
                pycc_ast::expr_range(other),
            ));
        }
    };
    Ok(lowered)
}
/// PEP 572 (#774): whether `expr` contains a `HirExpr::NamedExpr` anywhere
/// within it, at any nesting depth. `crate::stmt::lower_stmt` calls this on
/// every expression field of a statement kind other than `Stmt::If`'s/
/// `Stmt::While`'s own `test` and a bare `Stmt::Expr`'s own value -- the
/// three placements a walrus is permitted in (#774's own explicit
/// permitted-scope-cut) -- to reject a walrus lowered anywhere else with a
/// clean diagnostic instead of leaving `pycc_types`/`pycc_mir` to either
/// silently mishandle it or panic downstream on an unbound name. An
/// exhaustive match over every `HirExpr` variant, mirroring
/// `rename_name_in_expr`'s own exhaustive structure just above, so a future
/// variant is a compile error here rather than a silently-permitted new
/// placement.
pub(crate) fn contains_named_expr(expr: &HirExpr) -> bool {
    match expr {
        HirExpr::NamedExpr { .. } => true,
        HirExpr::IntLiteral(_)
        | HirExpr::FloatLiteral(_)
        | HirExpr::BoolLiteral(_)
        | HirExpr::StringLiteral(_)
        | HirExpr::EmptyList(_)
        | HirExpr::EmptyDict(_)
        | HirExpr::NoneLiteral
        | HirExpr::NotImplemented
        | HirExpr::Name(_)
        | HirExpr::Super => false,
        HirExpr::Call { args, .. } => args.iter().any(contains_named_expr),
        HirExpr::BinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            contains_named_expr(left) || contains_named_expr(right)
        }
        HirExpr::CompareChain { first, links } => {
            contains_named_expr(first) || links.iter().any(|link| contains_named_expr(&link.right))
        }
        HirExpr::IfExp { test, body, orelse } => {
            contains_named_expr(test) || contains_named_expr(body) || contains_named_expr(orelse)
        }
        HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
            contains_named_expr(operand)
        }
        HirExpr::FString(parts) => parts.iter().any(|part| match part {
            FStringPart::Literal(_) => false,
            FStringPart::Interpolation(e) => contains_named_expr(e),
        }),
        HirExpr::ListLiteral(es)
        | HirExpr::ObjectList(es)
        | HirExpr::SetLiteral(es)
        | HirExpr::TupleLiteral(es) => es.iter().any(contains_named_expr),
        HirExpr::Subscript { base, index } => {
            contains_named_expr(base) || contains_named_expr(index)
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            contains_named_expr(base)
                || [start, stop, step]
                    .into_iter()
                    .flatten()
                    .any(|b| contains_named_expr(b))
        }
        HirExpr::ListAppend { list, value } => {
            list.attr_expr().is_some_and(contains_named_expr) || contains_named_expr(value)
        }
        HirExpr::SetAdd { value, .. } => contains_named_expr(value),
        HirExpr::DictLiteral(pairs) => pairs
            .iter()
            .any(|(k, v)| contains_named_expr(k) || contains_named_expr(v)),
        HirExpr::ListPop { list } => list.attr_expr().is_some_and(contains_named_expr),
        HirExpr::DictGetOrDefault { dict, key, default } => {
            dict.attr_expr().is_some_and(contains_named_expr)
                || contains_named_expr(key)
                || contains_named_expr(default)
        }
        HirExpr::AttrGet { base, .. } => contains_named_expr(base),
        HirExpr::MethodCall { base, args, .. } => {
            contains_named_expr(base) || args.iter().any(contains_named_expr)
        }
        HirExpr::ReceiverDispatchedCall { call, .. } => contains_named_expr(call),
        HirExpr::GenericClassInstantiate { args, .. } => args.iter().any(contains_named_expr),
        HirExpr::ExprCall { callee, args } => {
            contains_named_expr(callee) || args.iter().any(contains_named_expr)
        }
        HirExpr::ReceiverClassCall { args } => args.iter().any(contains_named_expr),
        HirExpr::KeywordCall { call, keywords, .. } => {
            contains_named_expr(call) || keywords.iter().any(|(_, v)| contains_named_expr(v))
        }
        HirExpr::Comprehension(comp) => comprehension::comprehension_contains_named_expr(comp),
    }
}

/// Parses `range(...)`'s argument list into `(start, stop, step)` `HirExpr`s,
/// defaulting `start`/`step` per Python's own `range()` overloads. Shared by
/// `Stmt::For`'s own lowering and `comprehension::lower_comprehension_iter` (PR-12) --
/// factored out rather than duplicated a second time. Callers are
/// responsible for checking the callee is actually `range` and carries no
/// keyword arguments first (their own diagnostics differ in wording between
/// a plain `for` loop and a comprehension's `for` clause), so this helper
/// only ever inspects `call.arguments.args`.
pub(crate) fn lower_range_call(
    call: &pycc_ast::ExprCall,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<(HirExpr, HirExpr, HirExpr), Diagnostic> {
    // Issue #618 (T0051) deliberately does NOT check a `range()` argument:
    // D-179 already removed `range` from D-141's runtime `int`-boundary
    // inventory. `range()` is fully bigint-capable (bounds, step, and a
    // mid-loop-promoting induction variable all work via
    // `pycc_rt_range_normalize_operand`/`pycc_rt_range_continue`), so an
    // out-of-range literal here is not a capability gap at all -- it is
    // ordinary, supported behavior, not a candidate for a boundary
    // diagnostic. See D-207 for why this position was wrongly included in
    // #618's own filed inventory (copied from D-178's pre-D-179 fourteen).
    let lower_arg = |e: &Expr| -> Result<HirExpr, Diagnostic> {
        lower_expr(e, in_function, class_name, imports, signatures)
    };
    match &*call.arguments.args {
        [stop] => Ok((
            HirExpr::IntLiteral(0),
            lower_arg(stop)?,
            HirExpr::IntLiteral(1),
        )),
        [start, stop] => Ok((lower_arg(start)?, lower_arg(stop)?, HirExpr::IntLiteral(1))),
        [start, stop, step] => Ok((lower_arg(start)?, lower_arg(stop)?, lower_arg(step)?)),
        other => Err(unsupported(
            format!("range() with {} arguments is not supported", other.len()),
            call.range,
        )),
    }
}

#[cfg(test)]
mod tests;
