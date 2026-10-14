//! Bound-name lookup and function-local name collection.

use super::*;

/// Looks up a bare name's already-bound type, producing the same
/// "unbound local" vs. "not defined" distinction `HirExpr::Name` itself
/// uses in `infer_expr_in` below. `HirStmt::ForList`'s `list` field and
/// `HirExpr::ListAppend`'s `list` field are both plain `String`s rather
/// than `HirExpr::Name` nodes (D-105's HIR shape), so they can't go
/// through `infer_expr_in`'s own `Name` arm and need this helper instead.
pub(crate) fn lookup_bound_name(
    env: &Environment,
    local_names: &[&str],
    name: &str,
) -> Result<Ty, Diagnostic> {
    lookup_bound_name_inner(env, local_names, name, false)
}

/// [`lookup_bound_name`] with the `memoryview` read refusal suppressed, for
/// the **one** position that admits a buffer-bound name: the target of an
/// element store, `b[i] = v` (Part 1 of #1142).
///
/// A separate entry point and not a relaxation of the refusal itself. Every
/// other difference from [`lookup_bound_name`] would be a language-wide
/// regression -- `reject_object_read`, the possibly-unbound arm, the
/// unbound-local arm and the "name is not defined" `T0021` all apply to
/// `d[k] = v` for every base -- so the two share one body and differ by
/// exactly the one line this flag guards.
pub(crate) fn lookup_bound_name_for_store(
    env: &Environment,
    local_names: &[&str],
    name: &str,
) -> Result<Ty, Diagnostic> {
    lookup_bound_name_inner(env, local_names, name, true)
}

pub(crate) fn lookup_bound_name_inner(
    env: &Environment,
    local_names: &[&str],
    name: &str,
    admit_buffer: bool,
) -> Result<Ty, Diagnostic> {
    // Issue #118 Part 1: three-way distinction -- definitely bound -> ok, maybe
    // bound -> T0041, unbound -> T0021 (local) or "not defined" (global).
    match env.binding_state(name) {
        Some(BindingState::Definitely(ty)) => {
            // Part 1 of #1026, choke point 2: `for x in numpy:` and
            // `[e for x in numpy]` lower to `HirStmt::ForList` /
            // `HirExpr::Comprehension`, whose iterable is a plain `String`
            // (D-105), so they reach the binding through this helper
            // rather than through `infer_expr_in`'s `Name` arm.
            crate::foreign::reject_object_read(name, ty)?;
            // Part 1 of #1027, the same choke point for the same reason: a
            // `memoryview` parameter reached by `for x in v` bypasses the
            // `Name` arm's own guard, and without this call the iteration is
            // refused as a `T0033` type error instead of the `C0001`
            // capability gap the type actually is.
            if !admit_buffer {
                // Part 2a of #1142 (#1165): provenance comes from the same
                // environment the binding did, so an artifact-owned name
                // iterated with `for x in a` gets the owned refusal rather
                // than one that calls it a borrowed parameter.
                reject_memoryview_read(name, ty, env.owned_buffers.contains(name))?;
            }
            Ok(ty.clone())
        }
        // A function's read of a module global bound on some paths only is
        // checked at run time (`NameError`), as `expr::name_infer`'s read
        // is; so is any such read in non-strict mode
        // (`crate::non_strict::admits_maybe_read`).
        Some(BindingState::Maybe(ty))
            if (env.in_function_body && !is_local(local_names, name))
                || crate::non_strict::admits_maybe_read(name) =>
        {
            if !admit_buffer {
                reject_memoryview_read(name, ty, env.owned_buffers.contains(name))?;
            }
            Ok(ty.clone())
        }
        Some(BindingState::Maybe(_)) => Err(possibly_unbound(name)),
        None => {
            if is_local(local_names, name) {
                Err(unbound_local(name))
            } else {
                Err(Diagnostic::error(
                    "T0021",
                    format!("name `{name}` is not defined"),
                    Span::new(0, 0),
                ))
            }
        }
    }
}

pub(crate) fn function_local_names<'a>(
    params: &'a [(String, Ty)],
    body: &'a [HirStmt],
) -> Vec<&'a str> {
    let mut names = params
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    collect_local_names(body, &mut names);
    names
}

/// PEP 572 (#774): walks `expr` for every `HirExpr::NamedExpr { name, .. }`
/// node at any depth and pushes each `name` into `names` (skipping one
/// already recorded, mirroring every other arm in `collect_local_names`
/// below). A walrus target binds into the *enclosing function scope*
/// (`lower_stmt`'s own placement restriction limits where a `NamedExpr` can
/// appear to an `if`/`while` test or a bare expression statement -- never
/// inside a nested function/comprehension scope), so this walk does not need
/// to worry about crossing a scope boundary the way a general free-variable
/// analysis would.
pub(crate) fn collect_named_expr_names_in_expr<'a>(expr: &'a HirExpr, names: &mut Vec<&'a str>) {
    match expr {
        HirExpr::NamedExpr { name, value } => {
            collect_named_expr_names_in_expr(value, names);
            if !is_local(names, name) {
                names.push(name);
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
        HirExpr::ListPop { list } => {
            if let Some(receiver) = list.attr_expr() {
                collect_named_expr_names_in_expr(receiver, names);
            }
        }
        // #1254 (D-250): lowering refuses a walrus inside a comprehension,
        // and its loop variable is node-scoped, not a local of the function.
        HirExpr::Comprehension(_) => {}
        HirExpr::Nd(_) => {
            for sub in pycc_hir::ndarray::expr_children(expr) {
                collect_named_expr_names_in_expr(sub, names);
            }
        }
        HirExpr::Call { args, .. } => {
            for arg in args {
                collect_named_expr_names_in_expr(arg, names);
            }
        }
        HirExpr::CompareChain { first, links } => {
            for operand in pycc_hir::compare_chain_operands(first, links) {
                collect_named_expr_names_in_expr(operand, names);
            }
        }
        HirExpr::BinOp { left, right, .. }
        | HirExpr::AugBinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            collect_named_expr_names_in_expr(left, names);
            collect_named_expr_names_in_expr(right, names);
        }
        HirExpr::IfExp { test, body, orelse } => {
            for part in [test, body, orelse] {
                collect_named_expr_names_in_expr(part, names);
            }
        }
        HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
            collect_named_expr_names_in_expr(operand, names)
        }
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(inner) = part {
                    collect_named_expr_names_in_expr(inner, names);
                }
            }
        }
        HirExpr::ListLiteral(es)
        | HirExpr::ObjectList(es)
        | HirExpr::SetLiteral(es)
        | HirExpr::TupleLiteral(es) => {
            for e in es {
                collect_named_expr_names_in_expr(e, names);
            }
        }
        HirExpr::Subscript { base, index } => {
            collect_named_expr_names_in_expr(base, names);
            collect_named_expr_names_in_expr(index, names);
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            collect_named_expr_names_in_expr(base, names);
            for bound in [start, stop, step].into_iter().flatten() {
                collect_named_expr_names_in_expr(bound, names);
            }
        }
        HirExpr::ListAppend { list, value } => {
            if let Some(receiver) = list.attr_expr() {
                collect_named_expr_names_in_expr(receiver, names);
            }
            collect_named_expr_names_in_expr(value, names);
        }
        HirExpr::SetAdd { value, .. } => collect_named_expr_names_in_expr(value, names),
        HirExpr::DictLiteral(pairs) => {
            for (k, v) in pairs {
                collect_named_expr_names_in_expr(k, names);
                collect_named_expr_names_in_expr(v, names);
            }
        }
        HirExpr::DictGetOrDefault { dict, key, default } => {
            if let Some(receiver) = dict.attr_expr() {
                collect_named_expr_names_in_expr(receiver, names);
            }
            collect_named_expr_names_in_expr(key, names);
            collect_named_expr_names_in_expr(default, names);
        }
        HirExpr::AttrGet { base, .. } => collect_named_expr_names_in_expr(base, names),
        HirExpr::MethodCall { base, args, .. } => {
            collect_named_expr_names_in_expr(base, names);
            for arg in args {
                collect_named_expr_names_in_expr(arg, names);
            }
        }
        HirExpr::ReceiverDispatchedCall { call, .. } => {
            collect_named_expr_names_in_expr(call, names)
        }
        HirExpr::KeywordCall { call, keywords, .. } => {
            collect_named_expr_names_in_expr(call, names);
            for (_, value) in keywords {
                collect_named_expr_names_in_expr(value, names);
            }
        }
        HirExpr::GenericClassInstantiate { args, .. } => {
            for arg in args {
                collect_named_expr_names_in_expr(arg, names);
            }
        }
        HirExpr::ExprCall { callee, args } => {
            collect_named_expr_names_in_expr(callee, names);
            for arg in args {
                collect_named_expr_names_in_expr(arg, names);
            }
        }
        HirExpr::ReceiverClassCall { args } => {
            for arg in args {
                collect_named_expr_names_in_expr(arg, names);
            }
        }
    }
}

pub(crate) fn collect_local_names<'a>(body: &'a [HirStmt], names: &mut Vec<&'a str>) {
    for stmt in body {
        match stmt {
            HirStmt::Assign { target, .. } => {
                if !is_local(names, target) {
                    names.push(target);
                }
            }
            // #1244: `del x` makes `x` local to the function exactly as an
            // assignment does (CPython's `UnboundLocalError` when the
            // function never binds it first).
            HirStmt::AnnAssign { target, .. } | HirStmt::Delete { name: target } => {
                if !is_local(names, target) {
                    names.push(target);
                }
            }
            // An `import` binds its local names as an assignment does
            // (#1291; `pycc_hir` produces this node only at module level).
            HirStmt::ForeignImport { bindings, .. } => {
                for (target, _, _) in bindings {
                    if !is_local(names, target) {
                        names.push(target);
                    }
                }
            }
            HirStmt::If { test, body, orelse } => {
                collect_named_expr_names_in_expr(test, names);
                collect_local_names(body, names);
                collect_local_names(orelse, names);
            }
            HirStmt::While { test, body } => {
                collect_named_expr_names_in_expr(test, names);
                collect_local_names(body, names);
            }
            HirStmt::LoopElse { loop_stmt, orelse } => {
                collect_local_names(std::slice::from_ref(loop_stmt.as_ref()), names);
                collect_local_names(orelse, names);
            }
            HirStmt::ForRange { var, body, .. } => {
                if !is_local(names, var) {
                    names.push(var);
                }
                collect_local_names(body, names);
            }
            HirStmt::ForList { var, body, .. } | HirStmt::ForObject { var, body, .. } => {
                if !is_local(names, var) {
                    names.push(var);
                }
                collect_local_names(body, names);
            }
            // PR-12 Task 3 (D-117): a comprehension introduces two new local
            // names where a plain `for` loop introduces one -- its own
            // `target` (the comprehension's result) and its synthesized
            // `var` (the loop variable, already collision-proof by
            // construction, see `pycc_hir`'s `synthesize_comp_var_name`).
            // Neither has a body to recurse into (a comprehension is not a
            // nested block of statements).
            HirStmt::ListCompAssign { target, var, .. }
            | HirStmt::SetCompAssign { target, var, .. }
            | HirStmt::DictCompAssign { target, var, .. } => {
                if !is_local(names, target) {
                    names.push(target);
                }
                if !is_local(names, var) {
                    names.push(var);
                }
            }
            // `d[k] = v` (PR-11 Task 3) reassigns an existing binding's
            // contents, not a name -- unlike `Assign`/`AnnAssign`/`ForList`
            // above, it introduces no new local name to collect.
            // `base.attr = value` (D-154) is the same shape: it mutates an
            // existing instance's attribute slot, never binds a new local
            // name.
            HirStmt::ExprStmt(expr) => collect_named_expr_names_in_expr(expr, names),
            // A slice or attribute `del` (Part 2c of #1371, #1457) binds no
            // name, and `pycc_hir` refuses a walrus in its operands.
            HirStmt::Return(_)
            | HirStmt::DictSet { .. }
            | HirStmt::AttrSet { .. }
            | HirStmt::DeleteSlice { .. }
            | HirStmt::SetSlice { .. }
            | HirStmt::DeleteAttr { .. }
            | HirStmt::DeleteItem { .. }
            | HirStmt::Break
            | HirStmt::Continue
            | HirStmt::Raise { .. } => {}
            HirStmt::Match { cases, .. } => {
                for case in cases {
                    collect_pattern_capture_names(&case.pattern, names);
                    collect_local_names(&case.body, names);
                }
            }
            // Part 3 of #382 (#542): `except*` collects local names exactly
            // like plain `try`/`except` -- its `as` binding introduces a
            // local the same way, whatever the bound type turns out to be.
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
                collect_local_names(body, names);
                for handler in handlers {
                    if let Some(name) = &handler.name
                        && !is_local(names, name)
                    {
                        names.push(name);
                    }
                    collect_local_names(&handler.body, names);
                }
                collect_local_names(orelse, names);
                collect_local_names(finalbody, names);
            }
        }
    }
}

/// PEP 634-636 (#381, PR-21): collects all capture names introduced by a
/// pattern (recursively), for `collect_local_names`'s pre-pass.
pub(crate) fn collect_pattern_capture_names<'a>(pattern: &'a HirPattern, names: &mut Vec<&'a str>) {
    match pattern {
        HirPattern::Wildcard
        | HirPattern::Literal(_)
        | HirPattern::Singleton(_)
        | HirPattern::NoneSingleton => {}
        HirPattern::Capture(name) => {
            if !is_local(names, name) {
                names.push(name);
            }
        }
        HirPattern::Sequence(subs) | HirPattern::Or(subs) => {
            for sub in subs {
                collect_pattern_capture_names(sub, names);
            }
        }
        HirPattern::SequenceStar(subs, rest) => {
            for sub in subs {
                collect_pattern_capture_names(sub, names);
            }
            if let Some(rest) = rest
                && !is_local(names, rest)
            {
                names.push(rest);
            }
        }
        HirPattern::Mapping(pairs, rest) => {
            for (_, sub) in pairs {
                collect_pattern_capture_names(sub, names);
            }
            if let Some(rest) = rest
                && !is_local(names, rest)
            {
                names.push(rest);
            }
        }
        HirPattern::Class {
            positional,
            keyword,
            ..
        } => {
            for sub in positional {
                collect_pattern_capture_names(sub, names);
            }
            for (_, sub) in keyword {
                collect_pattern_capture_names(sub, names);
            }
        }
        HirPattern::As(inner, name) => {
            collect_pattern_capture_names(inner, names);
            if !is_local(names, name) {
                names.push(name);
            }
        }
    }
}

pub(crate) fn is_local(local_names: &[&str], name: &str) -> bool {
    local_names.contains(&name)
}
