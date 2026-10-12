//! Collecting generic class instantiations from expressions and statements.

use super::*;

/// PR-13 Task 3 (D-133/D-134): the monomorphization pass that turns a
/// validated module possibly containing PEP 695 generic functions into an
/// equivalent module containing only ordinary, concrete-`Ty` functions --
/// exactly what `pycc_mir::build` already expects, with no MIR/codegen
/// change needed for genericity itself. Every call site whose callee was a
/// generic function is rewritten to call that call site's mangled,
/// monomorphized specialization instead; two call sites resolving the same
/// type parameter to the same concrete `Ty` share one specialization
/// (deduplicated by `GenericInstantiation::mangled_name`, per
/// `instantiate_generic_call`'s own documented dedup guarantee). Every
/// original generic function itself is dropped -- it still carries
/// `Ty::Param`, which must never reach `pycc_mir` -- and each distinct
/// specialization is appended as an ordinary `HirItem::Function`, in the
/// order its first call site was encountered (module item order, depth
/// first through each function body).
///
/// Callers must have already validated `hir` (e.g. via `check_all` or
/// `checked_function_signatures_all`, both of which now dispatch a generic call
/// site through `instantiate_generic_call` themselves via
/// `infer_expr_in`'s own generic arm) -- this pass performs no validation
/// of its own and instead trusts that every generic call site it encounters
/// resolves successfully.
/// PEP 695 (#387): Recursively collects every `(class_name, type_arg)` pair
/// from `GenericClassInstantiate` expressions reachable from `expr`. Used by
/// `instantiate_generic_class_methods`'s pre-scan to know which generic
/// classes need monomorphization before the rewrite pass starts. `Ty` does
/// not implement `Hash`, so dedup is done linearly against this `Vec`.
pub(crate) fn collect_generic_class_instantiations_from_expr(
    expr: &HirExpr,
    out: &mut Vec<(String, Ty)>,
) {
    match expr {
        HirExpr::GenericClassInstantiate {
            class,
            type_arg,
            args,
        } => {
            let pair = (class.clone(), type_arg.clone());
            if !out.contains(&pair) {
                out.push(pair);
            }
            for arg in args {
                collect_generic_class_instantiations_from_expr(arg, out);
            }
        }
        HirExpr::Call { args, .. } => {
            for arg in args {
                collect_generic_class_instantiations_from_expr(arg, out);
            }
        }
        HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
            collect_generic_class_instantiations_from_expr(operand, out);
        }
        HirExpr::CompareChain { first, links } => {
            for operand in pycc_hir::compare_chain_operands(first, links) {
                collect_generic_class_instantiations_from_expr(operand, out);
            }
        }
        HirExpr::BinOp { left, right, .. }
        | HirExpr::AugBinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            collect_generic_class_instantiations_from_expr(left, out);
            collect_generic_class_instantiations_from_expr(right, out);
        }
        HirExpr::IfExp { test, body, orelse } => {
            for part in [test, body, orelse] {
                collect_generic_class_instantiations_from_expr(part, out);
            }
        }
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(inner) = part {
                    collect_generic_class_instantiations_from_expr(inner, out);
                }
            }
        }
        HirExpr::ListLiteral(es)
        | HirExpr::ObjectList(es)
        | HirExpr::SetLiteral(es)
        | HirExpr::TupleLiteral(es) => {
            for e in es {
                collect_generic_class_instantiations_from_expr(e, out);
            }
        }
        HirExpr::DictLiteral(pairs) => {
            for (k, v) in pairs {
                collect_generic_class_instantiations_from_expr(k, out);
                collect_generic_class_instantiations_from_expr(v, out);
            }
        }
        HirExpr::Subscript { base, index } => {
            collect_generic_class_instantiations_from_expr(base, out);
            collect_generic_class_instantiations_from_expr(index, out);
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            collect_generic_class_instantiations_from_expr(base, out);
            for bound in [start, stop, step].into_iter().flatten() {
                collect_generic_class_instantiations_from_expr(bound, out);
            }
        }
        HirExpr::ListAppend { list, value } => {
            for sub in list.attr_expr().into_iter().chain([value.as_ref()]) {
                collect_generic_class_instantiations_from_expr(sub, out);
            }
        }
        HirExpr::SetAdd { value, .. } => {
            collect_generic_class_instantiations_from_expr(value, out);
        }
        HirExpr::ListPop { list } => {
            if let Some(sub) = list.attr_expr() {
                collect_generic_class_instantiations_from_expr(sub, out);
            }
        }
        HirExpr::DictGetOrDefault { dict, key, default } => {
            for sub in dict
                .attr_expr()
                .into_iter()
                .chain([key.as_ref(), default.as_ref()])
            {
                collect_generic_class_instantiations_from_expr(sub, out);
            }
        }
        HirExpr::AttrGet { base, .. } => {
            collect_generic_class_instantiations_from_expr(base, out);
        }
        HirExpr::MethodCall { base, args, .. } => {
            collect_generic_class_instantiations_from_expr(base, out);
            for arg in args {
                collect_generic_class_instantiations_from_expr(arg, out);
            }
        }
        HirExpr::ExprCall { callee, args } => {
            collect_generic_class_instantiations_from_expr(callee, out);
            for arg in args {
                collect_generic_class_instantiations_from_expr(arg, out);
            }
        }
        HirExpr::ReceiverClassCall { args } => {
            for arg in args {
                collect_generic_class_instantiations_from_expr(arg, out);
            }
        }
        HirExpr::ReceiverDispatchedCall { call, .. } => {
            collect_generic_class_instantiations_from_expr(call, out);
        }
        HirExpr::KeywordCall { call, keywords, .. } => {
            collect_generic_class_instantiations_from_expr(call, out);
            for (_, value) in keywords {
                collect_generic_class_instantiations_from_expr(value, out);
            }
        }
        // PEP 572 (#774): `target := value` — recurse into `value` only,
        // mirroring `AttrGet`'s own single-sub-expression shape just above.
        HirExpr::NamedExpr { name: _, value } => {
            collect_generic_class_instantiations_from_expr(value, out);
        }
        HirExpr::Nd(_) => {
            for sub in pycc_hir::ndarray::expr_children(expr) {
                collect_generic_class_instantiations_from_expr(sub, out);
            }
        }
        HirExpr::Comprehension(comp) => {
            for sub in comp.sub_exprs() {
                collect_generic_class_instantiations_from_expr(sub, out);
            }
        }
        HirExpr::IntLiteral(_)
        | HirExpr::FloatLiteral(_)
        | HirExpr::BoolLiteral(_)
        | HirExpr::OpaqueBool
        | HirExpr::StringLiteral(_)
        | HirExpr::EmptyList(_)
        | HirExpr::EmptySet(_)
        | HirExpr::EmptyDict(_)
        | HirExpr::NoneLiteral
        | HirExpr::NotImplemented
        | HirExpr::Name(_)
        | HirExpr::Super => {}
    }
}

/// PEP 695 (#387): Traverses a `CompIter` for `GenericClassInstantiate`
/// expressions. `CompIter::Range` carries three `HirExpr`s (`start`, `stop`,
/// `step`) that can each contain a GCI (e.g. `range(C[int](0), 10)`), and
/// `CompIter::Iterable` (Part 1 of #1255) carries one. `CompIter::Name`
/// holds only a bare `String`, so it cannot contain one.
pub(crate) fn collect_generic_class_instantiations_from_comp_iter(
    iter: &CompIter,
    out: &mut Vec<(String, Ty)>,
) {
    match iter {
        CompIter::Range { start, stop, step } => {
            for sub in [start, stop, step] {
                collect_generic_class_instantiations_from_expr(sub, out);
            }
        }
        CompIter::Iterable(iterable) => {
            collect_generic_class_instantiations_from_expr(iterable, out)
        }
        CompIter::Name(_) => {}
    }
}

/// PEP 695 (#387): Recursively collects `(class_name, type_arg)` pairs from
/// `GenericClassInstantiate` expressions reachable from a statement.
pub(crate) fn collect_generic_class_instantiations_from_stmt(
    stmt: &HirStmt,
    out: &mut Vec<(String, Ty)>,
) {
    match stmt {
        HirStmt::ExprStmt(expr) => collect_generic_class_instantiations_from_expr(expr, out),
        HirStmt::Assign { value, .. } => collect_generic_class_instantiations_from_expr(value, out),
        HirStmt::AnnAssign { value, .. } => {
            if let Some(v) = value {
                collect_generic_class_instantiations_from_expr(v, out);
            }
        }
        HirStmt::If { test, body, orelse } => {
            collect_generic_class_instantiations_from_expr(test, out);
            for s in body {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
            for s in orelse {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
        }
        HirStmt::While { test, body } => {
            collect_generic_class_instantiations_from_expr(test, out);
            for s in body {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
        }
        HirStmt::ForRange {
            start,
            stop,
            step,
            body,
            ..
        } => {
            for sub in [start, stop, step] {
                collect_generic_class_instantiations_from_expr(sub, out);
            }
            for s in body {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
        }
        HirStmt::ForList { body, .. } => {
            // `list` is a bare `String` (variable name), not an `HirExpr`,
            // so it cannot contain a `GenericClassInstantiate`.
            for s in body {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
        }
        HirStmt::ForObject { iter, body, .. } => {
            // PR 3c of #1082: unlike `ForList`'s `list`, this iterable is an
            // `HirExpr` and can hold a `GenericClassInstantiate`.
            collect_generic_class_instantiations_from_expr(iter, out);
            for s in body {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
        }
        HirStmt::LoopElse { loop_stmt, orelse } => {
            collect_generic_class_instantiations_from_stmt(loop_stmt, out);
            for s in orelse {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
        }
        HirStmt::Break | HirStmt::Continue => {}
        HirStmt::DictSet { key, value, .. } => {
            collect_generic_class_instantiations_from_expr(key, out);
            collect_generic_class_instantiations_from_expr(value, out);
        }
        HirStmt::ListCompAssign {
            elt, cond, iter, ..
        }
        | HirStmt::SetCompAssign {
            elt, cond, iter, ..
        } => {
            collect_generic_class_instantiations_from_comp_iter(iter, out);
            collect_generic_class_instantiations_from_expr(elt, out);
            if let Some(c) = cond {
                collect_generic_class_instantiations_from_expr(c, out);
            }
        }
        HirStmt::DictCompAssign {
            key,
            value,
            cond,
            iter,
            ..
        } => {
            collect_generic_class_instantiations_from_comp_iter(iter, out);
            collect_generic_class_instantiations_from_expr(key, out);
            collect_generic_class_instantiations_from_expr(value, out);
            if let Some(c) = cond {
                collect_generic_class_instantiations_from_expr(c, out);
            }
        }
        HirStmt::Return(None) | HirStmt::Delete { .. } | HirStmt::ForeignImport { .. } => {}
        HirStmt::DeleteSlice {
            base,
            start,
            stop,
            step,
            ..
        } => {
            collect_generic_class_instantiations_from_expr(base, out);
            for bound in [start, stop, step].into_iter().flatten() {
                collect_generic_class_instantiations_from_expr(bound, out);
            }
        }
        HirStmt::DeleteAttr { base, .. } => {
            collect_generic_class_instantiations_from_expr(base, out)
        }
        HirStmt::DeleteItem { base, key, .. } => {
            collect_generic_class_instantiations_from_expr(base, out);
            collect_generic_class_instantiations_from_expr(key, out);
        }
        HirStmt::Return(Some(expr)) => collect_generic_class_instantiations_from_expr(expr, out),
        HirStmt::AttrSet { base, value, .. } => {
            collect_generic_class_instantiations_from_expr(base, out);
            collect_generic_class_instantiations_from_expr(value, out);
        }
        HirStmt::Match { subject, cases } => {
            collect_generic_class_instantiations_from_expr(subject, out);
            for case in cases {
                if let Some(guard) = &case.guard {
                    collect_generic_class_instantiations_from_expr(guard, out);
                }
                for s in &case.body {
                    collect_generic_class_instantiations_from_stmt(s, out);
                }
            }
        }
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        }
        | HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            for s in body {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
            for handler in handlers {
                for s in &handler.body {
                    collect_generic_class_instantiations_from_stmt(s, out);
                }
            }
            for s in orelse {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
            for s in finalbody {
                collect_generic_class_instantiations_from_stmt(s, out);
            }
        }
        HirStmt::Raise { exc, cause } => {
            if let Some(e) = exc {
                collect_generic_class_instantiations_from_expr(e, out);
            }
            if let Some(c) = cause {
                collect_generic_class_instantiations_from_expr(c, out);
            }
        }
    }
}
