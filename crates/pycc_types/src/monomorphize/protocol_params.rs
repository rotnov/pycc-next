//! Specializing protocol-typed parameters and rewriting their calls.

use super::*;

/// #380 (PR-20): Monomorphizes functions with protocol-typed parameters.
/// Scans `items` for calls to functions that have `Ty::Protocol` parameters,
/// and for each call site, creates a specialized version with the protocol
/// type substituted by the concrete argument's class type. The original
/// function is dropped (only specializations reach MIR/codegen). Returns
/// the updated items list with monomorphized functions appended and call
/// sites rewritten.
///
/// The second return value is parallel to `items`: `kept[i]` says whether
/// input item `i` survived into the returned list. Dropping an item shifts
/// every later item's position, and the `ForeignImportSite::Item` index of each `ImportBinding::Foreign`
/// records a position in this very list, so the caller needs the mask to
/// recompute those positions (PR 1c of #1080 review finding 1).
pub(crate) fn monomorphize_protocol_params(
    items: Vec<HirItem>,
    env: &Environment,
    _new_class_defs: &mut Vec<(String, HirClassDef)>,
) -> (Vec<HirItem>, Vec<bool>) {
    // Collect functions with protocol-typed parameters (cloned, so we
    // can move `items` below without a borrow conflict).
    let protocol_funcs: HashMap<String, HirItem> = items
        .iter()
        .filter_map(|item| {
            if let HirItem::Function { name, params, .. } = item
                && has_protocol_param(params)
            {
                return Some((name.clone(), item.clone()));
            }
            None
        })
        .collect();
    if protocol_funcs.is_empty() {
        let kept = vec![true; items.len()];
        return (items, kept);
    }
    let mut kept = Vec::with_capacity(items.len());
    let mut new_items = Vec::new();
    let mut specializations: Vec<HirItem> = Vec::new();
    let mut seen: HashSet<String> = HashSet::default();
    // #953: a module-level statement's own bindings have to accumulate as
    // the loop walks them, exactly as `bind_local_types_in_body` does for
    // a function body. Without this, a module-level `p: P = C()` leaves
    // `p` untyped for the following `p.same(C())`, no specialization is
    // created, and `pycc_mir` panics on the dropped `$fn:C.same`.
    let mut module_env = env.clone();
    for item in items {
        match item {
            HirItem::Function {
                ref name,
                ref params,
                ref return_ty,
                ..
            } if protocol_funcs.contains_key(name.as_str()) => {
                // Drop the original protocol-parameter function (only
                // specializations are kept). But if the function has no
                // call sites (no specializations), it would be silently
                // dropped — this is fine, matching how generic functions
                // are dropped.
                let _ = (params, return_ty);
                kept.push(false);
            }
            HirItem::TopLevelStmt(ref stmt) => {
                bind_local_types_in_stmt(&mut module_env, &[], stmt);
                let mut new_stmt = stmt.clone();
                rewrite_protocol_calls_in_stmt(
                    &mut new_stmt,
                    &protocol_funcs,
                    &module_env,
                    &[],
                    &mut specializations,
                    &mut seen,
                );
                new_items.push(HirItem::TopLevelStmt(new_stmt));
                kept.push(true);
            }
            HirItem::Function {
                name,
                params,
                return_ty,
                body,
            } => {
                let local_names: Vec<String> = function_local_names(&params, &body)
                    .into_iter()
                    .map(String::from)
                    .collect();
                let local_names_refs: Vec<&str> = local_names.iter().map(|s| s.as_str()).collect();
                let mut fn_env = env.child_for_function(&local_names_refs);
                for (param_name, param_ty) in &params {
                    fn_env.bind(param_name.clone(), param_ty.clone());
                }
                bind_local_types_in_body(&mut fn_env, &local_names_refs, &body);
                let mut new_body = body;
                for stmt in new_body.iter_mut() {
                    rewrite_protocol_calls_in_stmt(
                        stmt,
                        &protocol_funcs,
                        &fn_env,
                        &local_names_refs,
                        &mut specializations,
                        &mut seen,
                    );
                }
                new_items.push(HirItem::Function {
                    name,
                    params,
                    return_ty,
                    body: new_body,
                });
                kept.push(true);
            }
        }
    }
    // Rewrite calls inside specializations recursively. A specialized
    // body may itself call another protocol-parameter function, which
    // needs its own specialization. Process by index because rewriting
    // may push new specializations, extending the vector we're walking.
    //
    // The per-item work is extracted into `rewrite_protocol_calls_in_specialization`
    // so that the defense-in-depth non-`Function` arm (all specializations
    // are `HirItem::Function` in practice) is a `match` arm in a standalone
    // function that a direct unit test can cover, rather than an `if let`
    // whose never-taken false branch would carry an uncovered continuation
    // region under the 100%-coverage gate (D-014).
    let mut all_specializations = std::mem::take(&mut specializations);
    let mut i = 0;
    while i < all_specializations.len() {
        rewrite_protocol_calls_in_specialization(
            env,
            &mut all_specializations[i],
            &protocol_funcs,
            &mut specializations,
            &mut seen,
        );
        i += 1;
        if !specializations.is_empty() {
            all_specializations.append(&mut specializations);
        }
    }
    new_items.extend(all_specializations);
    (new_items, kept)
}

/// Recomputes every `ImportBinding::Foreign`'s recorded item position for
/// an item list monomorphization has rewritten (PR 1c of #1080 review
/// finding 1).
///
/// A `ForeignImportSite::Item` index is the item *count* at the moment the
/// `import` lowered, so the import belongs immediately before that original
/// item --
/// which means the new position is simply how many of the original items
/// strictly before it still exist. `survives[j]` says whether original
/// item `j` reached the returned list; a trailing import records
/// `hir.items.len()`, which the clamp maps to "after everything that
/// survived". Items monomorphization *appends* (generic instantiations,
/// protocol specializations) all land after every original item, so they
/// never shift a recomputed position. The result stays ascending, which is
/// what `pycc_mir::splice_foreign_imports`'s running-offset insertion
/// assumes.
pub(crate) fn remap_foreign_import_positions(
    imports: &[ImportBinding],
    survives: &[bool],
) -> Vec<ImportBinding> {
    imports
        .iter()
        .map(|binding| match binding {
            ImportBinding::Foreign {
                local_name,
                module_path,
                from,
                site,
                span,
            } => ImportBinding::Foreign {
                local_name: local_name.clone(),
                module_path: module_path.clone(),
                from: from.clone(),
                // A block import has no item position (#1291).
                site: match site {
                    ForeignImportSite::Item(index) => ForeignImportSite::Item(
                        survives[..(*index).min(survives.len())]
                            .iter()
                            .filter(|kept| **kept)
                            .count(),
                    ),
                    site @ (ForeignImportSite::Block { .. } | ForeignImportSite::Local { .. }) => *site,
                },
                span: *span,
            },
            other => other.clone(),
        })
        .collect()
}

/// #380 (PR-20): Per-item worker for the specialization rewrite loop.
/// Rewrites protocol-parameter function calls inside one already-created
/// specialization's body, pushing any newly discovered specializations.
/// Extracted from `monomorphize_protocol_params`'s loop so the
/// defense-in-depth non-`Function` arm is a `match` arm coverable by a
/// direct unit test (D-014).
pub(crate) fn rewrite_protocol_calls_in_specialization(
    env: &Environment,
    spec: &mut HirItem,
    protocol_funcs: &HashMap<String, HirItem>,
    specializations: &mut Vec<HirItem>,
    seen: &mut HashSet<String>,
) {
    let (params, body) = match spec {
        HirItem::Function { params, body, .. } => (params, body),
        // All specializations are `HirItem::Function` (created by
        // `rewrite_protocol_calls_in_expr`). This arm is defense-in-depth,
        // covered by a direct unit test.
        _ => return,
    };
    let local_names: Vec<String> = function_local_names(params, body)
        .into_iter()
        .map(String::from)
        .collect();
    let local_names_refs: Vec<&str> = local_names.iter().map(|s| s.as_str()).collect();
    let mut spec_env = env.child_for_function(&local_names_refs);
    for (param_name, param_ty) in params.iter() {
        spec_env.bind(param_name.clone(), param_ty.clone());
    }
    bind_local_types_in_body(&mut spec_env, &local_names_refs, body);
    for stmt in body.iter_mut() {
        rewrite_protocol_calls_in_stmt(
            stmt,
            protocol_funcs,
            &spec_env,
            &local_names_refs,
            specializations,
            seen,
        );
    }
}

/// #380 (PR-20): Rewrites calls to protocol-parameter functions in a
/// statement, creating monomorphized specializations as needed.
pub(crate) fn rewrite_protocol_calls_in_stmt(
    stmt: &mut HirStmt,
    protocol_funcs: &HashMap<String, HirItem>,
    env: &Environment,
    local_names: &[&str],
    specializations: &mut Vec<HirItem>,
    seen: &mut HashSet<String>,
) {
    match stmt {
        HirStmt::ExprStmt(expr) => {
            rewrite_protocol_calls_in_expr(
                expr,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        HirStmt::Assign { value, .. } => {
            rewrite_protocol_calls_in_expr(
                value,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        HirStmt::AnnAssign {
            value: Some(value), ..
        } => {
            rewrite_protocol_calls_in_expr(
                value,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        HirStmt::Return(Some(expr)) => {
            rewrite_protocol_calls_in_expr(
                expr,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        HirStmt::If { test, body, orelse } => {
            rewrite_protocol_calls_in_expr(
                test,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            for s in body.iter_mut() {
                rewrite_protocol_calls_in_stmt(
                    s,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
            for s in orelse.iter_mut() {
                rewrite_protocol_calls_in_stmt(
                    s,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        HirStmt::While { test, body } => {
            rewrite_protocol_calls_in_expr(
                test,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            for s in body.iter_mut() {
                rewrite_protocol_calls_in_stmt(
                    s,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        HirStmt::LoopElse { loop_stmt, orelse } => {
            rewrite_protocol_calls_in_stmt(
                loop_stmt,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            for s in orelse.iter_mut() {
                rewrite_protocol_calls_in_stmt(
                    s,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        HirStmt::ForRange {
            start,
            stop,
            step,
            body,
            ..
        } => {
            rewrite_protocol_calls_in_expr(
                start,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            rewrite_protocol_calls_in_expr(
                stop,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            rewrite_protocol_calls_in_expr(
                step,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            for s in body.iter_mut() {
                rewrite_protocol_calls_in_stmt(
                    s,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        HirStmt::ForList { body, .. } => {
            for s in body.iter_mut() {
                rewrite_protocol_calls_in_stmt(
                    s,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        HirStmt::DictSet { key, value, .. } => {
            rewrite_protocol_calls_in_expr(
                key,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            rewrite_protocol_calls_in_expr(
                value,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        HirStmt::AttrSet { base, value, .. } => {
            rewrite_protocol_calls_in_expr(
                base,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            rewrite_protocol_calls_in_expr(
                value,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        // Part 2c of #1371: a slice `del` reads its base and bounds; an
        // unrewritten protocol call there would dangle once the original
        // function item is dropped.
        HirStmt::DeleteSlice {
            base,
            start,
            stop,
            step,
            ..
        } => {
            for operand in std::iter::once(base).chain([start, stop, step].into_iter().flatten()) {
                rewrite_protocol_calls_in_expr(
                    operand,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        HirStmt::SetSlice {
            base,
            start,
            stop,
            step,
            value,
            ..
        } => {
            for operand in [value, base]
                .into_iter()
                .chain([start, stop, step].into_iter().flatten())
            {
                rewrite_protocol_calls_in_expr(
                    operand,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        // #1457: an attribute `del` reads its base, for the same reason.
        HirStmt::DeleteAttr { base, .. } => {
            rewrite_protocol_calls_in_expr(
                base,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        HirStmt::DeleteItem { base, key, .. } => {
            for expr in [base, key] {
                rewrite_protocol_calls_in_expr(
                    expr,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        // #1254: the loop variable is bound in a scoped clone of `env`
        // before `cond` and the elements are walked, so a protocol call
        // whose argument reads it resolves; before, `infer_expr_in` failed
        // on the unbound name and the call was silently left unrewritten.
        HirStmt::ListCompAssign {
            var,
            iter,
            cond,
            elt,
            ..
        }
        | HirStmt::SetCompAssign {
            var,
            iter,
            cond,
            elt,
            ..
        } => {
            let mut body: Vec<&mut HirExpr> = cond.iter_mut().map(|c| c.as_mut()).collect();
            body.push(elt);
            rewrite_protocol_calls_in_comprehension(
                var,
                iter,
                body,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        HirStmt::DictCompAssign {
            var,
            iter,
            cond,
            key,
            value,
            ..
        } => {
            let mut body: Vec<&mut HirExpr> = cond.iter_mut().map(|c| c.as_mut()).collect();
            body.extend([key.as_mut(), value.as_mut()]);
            rewrite_protocol_calls_in_comprehension(
                var,
                iter,
                body,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        _ => {}
    }
}

/// #380 (PR-20): Rewrites calls to protocol-parameter functions in an
/// expression, creating monomorphized specializations as needed.
pub(crate) fn rewrite_protocol_calls_in_expr(
    expr: &mut HirExpr,
    protocol_funcs: &HashMap<String, HirItem>,
    env: &Environment,
    local_names: &[&str],
    specializations: &mut Vec<HirItem>,
    seen: &mut HashSet<String>,
) {
    match expr {
        HirExpr::Call { callee, args } => {
            // First, recurse into arguments (they may contain nested calls).
            for arg in args.iter_mut() {
                rewrite_protocol_calls_in_expr(
                    arg,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
            // Check if this is a call to a protocol-parameter function.
            if let Some(func_item) = protocol_funcs.get(callee.as_str())
                && let HirItem::Function {
                    name,
                    params,
                    return_ty,
                    body,
                } = func_item
            {
                // Collect all protocol→concrete substitutions from every
                // protocol-typed parameter's corresponding argument.
                let mut substitutions: Vec<(String, Ty)> = Vec::new();
                for (i, (_, param_ty)) in params.iter().enumerate() {
                    if let Ty::Protocol(proto_name) = param_ty
                        && i < args.len()
                    {
                        let arg_ty = infer_expr_in(env, local_names, &args[i]);
                        if let Ok(Ty::Instance(concrete_name)) = arg_ty {
                            substitutions
                                .push((proto_name.as_ref().clone(), Ty::Instance(concrete_name)));
                        }
                    }
                }
                if !substitutions.is_empty() {
                    let mangled = mangle_protocol_instantiation(name, &substitutions);
                    if seen.insert(mangled.clone()) {
                        let substituted_params: Vec<(String, Ty)> = params
                            .iter()
                            .map(|(n, ty)| (n.clone(), substitute_ty_protocols(ty, &substitutions)))
                            .collect();
                        let substituted_return = substitute_ty_protocols(return_ty, &substitutions);
                        let substituted_body = substitute_body_protocols(body, &substitutions);
                        specializations.push(HirItem::Function {
                            name: mangled.clone(),
                            params: substituted_params,
                            return_ty: substituted_return,
                            body: substituted_body,
                        });
                    }
                    *callee = mangled;
                }
            }
        }
        HirExpr::MethodCall { base, method, args } => {
            rewrite_protocol_calls_in_expr(
                base,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            for arg in args.iter_mut() {
                rewrite_protocol_calls_in_expr(
                    arg,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
            // #953: a method whose signature has a protocol-typed
            // parameter is in `protocol_funcs` under its mangled
            // `<Class>.<method>` name, so the loop above already dropped
            // its original definition from the module. Without the
            // specialization this arm creates, `$fn:<Class>.<method>` is
            // never bound and `pycc_mir::build` panics on the very call
            // the type checker just accepted.
            if let Some(rewritten) = specialize_protocol_method_call(
                base,
                method,
                args,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            ) {
                *expr = rewritten;
            }
        }
        HirExpr::AttrGet { base, .. } => {
            rewrite_protocol_calls_in_expr(
                base,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
            rewrite_protocol_calls_in_expr(
                operand,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        // #1211: `BoolOp` shares `BinOp`'s arm. Without it the `_ => {}`
        // catch-all below would skip a protocol-typed call inside `and`/`or`
        // and leave it unspecialized.
        HirExpr::BinOp { left, right, .. }
        | HirExpr::AugBinOp { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            rewrite_protocol_calls_in_expr(
                left,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            rewrite_protocol_calls_in_expr(
                right,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        // #1395: like `BoolOp` above, without this arm the `_ => {}`
        // catch-all below would leave a protocol-typed call inside a
        // conditional expression unspecialized.
        HirExpr::IfExp { test, body, orelse } => {
            for part in [test, body, orelse] {
                rewrite_protocol_calls_in_expr(
                    part,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        // #1411: without this arm the `_ => {}` catch-all below would leave
        // a protocol-typed call in the constructor arguments of
        // `type(self)(...)` unspecialized.
        HirExpr::ReceiverClassCall { args } => {
            for arg in args.iter_mut() {
                rewrite_protocol_calls_in_expr(
                    arg,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        // Part 2a of #1371: like `IfExp` above, without this arm the
        // `_ => {}` catch-all below would leave a protocol-typed call in the
        // arguments of `table[k](args)` unspecialized.
        HirExpr::ExprCall { callee, args } => {
            for part in std::iter::once(callee.as_mut()).chain(args.iter_mut()) {
                rewrite_protocol_calls_in_expr(
                    part,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        // Part 8 of #1371: the positional half and every keyword value.
        HirExpr::KeywordCall { call, keywords, .. } => {
            for part in
                std::iter::once(call.as_mut()).chain(keywords.iter_mut().map(|(_, value)| value))
            {
                rewrite_protocol_calls_in_expr(
                    part,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        // #1212: without this arm the `_ => {}` catch-all below would skip
        // a protocol-typed call inside a chained comparison and leave it
        // unspecialized.
        HirExpr::CompareChain { first, links } => {
            rewrite_protocol_calls_in_expr(
                first,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            for link in links.iter_mut() {
                rewrite_protocol_calls_in_expr(
                    &mut link.right,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        HirExpr::Compare { left, right, .. } => {
            rewrite_protocol_calls_in_expr(
                left,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            rewrite_protocol_calls_in_expr(
                right,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        HirExpr::ListLiteral(elements) | HirExpr::ObjectList(elements) => {
            for e in elements.iter_mut() {
                rewrite_protocol_calls_in_expr(
                    e,
                    protocol_funcs,
                    env,
                    local_names,
                    specializations,
                    seen,
                );
            }
        }
        // Issue #1188: rewrite inside the wrapped method call. A
        // specialization replaces `call` with a plain `Call`, and that only
        // happens for an `Instance` receiver -- the method reading -- so the
        // specialized call then stands for the whole node.
        HirExpr::ReceiverDispatchedCall { call, .. } => {
            rewrite_protocol_calls_in_expr(
                call,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
            if !matches!(call.as_ref(), HirExpr::MethodCall { .. }) {
                *expr = (**call).clone();
            }
        }
        // PEP 572 (#774), deep-review follow-up: unlike its exhaustive
        // `rewrite_generic_calls_in_expr` sibling, this function is a
        // non-exhaustive recursion over only the shapes a protocol-typed
        // call can appear inside -- the trailing `_ => {}` above is
        // deliberate for every other `HirExpr` variant. But a walrus's own
        // value is exactly such a shape (`(x := proto_fn(c))` nests a
        // protocol call inside `value`), so without a dedicated arm here it
        // fell into the catch-all: `proto_fn`'s call site was never
        // rewritten to its specialization, while `monomorphize_protocol_params`
        // unconditionally drops the original `proto_fn` item regardless,
        // leaving a dangling call to a function no longer in the module.
        HirExpr::NamedExpr { value, .. } => {
            rewrite_protocol_calls_in_expr(
                value,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        // #1254 (D-250): without this arm a protocol call inside a
        // comprehension fell into the catch-all below and was left calling
        // an item `monomorphize_protocol_params` drops (the #774 bug class).
        HirExpr::Comprehension(comp) => {
            let HirComprehension {
                var,
                iter,
                cond,
                elt,
            } = comp.as_mut();
            let mut body: Vec<&mut HirExpr> = cond.iter_mut().collect();
            match elt {
                CompElt::List(e) | CompElt::Set(e) => body.push(e),
                CompElt::Dict { key, value } => body.extend([key, value]),
            }
            rewrite_protocol_calls_in_comprehension(
                var,
                iter,
                body,
                protocol_funcs,
                env,
                local_names,
                specializations,
                seen,
            );
        }
        _ => {}
    }
}

/// Protocol-call rewriting for one comprehension, statement or expression
/// form (#1254): the range operands are walked against `env`, where they
/// are evaluated; `cond` and the elements against a clone of `env` with the
/// synthesized loop variable bound, since this walker's `env` is immutable
/// and `bind_local_types_in_stmt` never binds that variable.
#[allow(clippy::too_many_arguments)]
pub(crate) fn rewrite_protocol_calls_in_comprehension(
    var: &str,
    iter: &mut CompIter,
    body: Vec<&mut HirExpr>,
    protocol_funcs: &HashMap<String, HirItem>,
    env: &Environment,
    local_names: &[&str],
    specializations: &mut Vec<HirItem>,
    seen: &mut HashSet<String>,
) {
    let operands: Vec<&mut HirExpr> = match iter {
        CompIter::Range { start, stop, step } => vec![start, stop, step],
        CompIter::Iterable(iterable) => vec![iterable.as_mut()],
        CompIter::Name(_) => vec![],
    };
    for operand in operands {
        rewrite_protocol_calls_in_expr(
            operand,
            protocol_funcs,
            env,
            local_names,
            specializations,
            seen,
        );
    }
    let mut scoped = env.clone();
    if let Ok(var_ty) = crate::comprehension::resolve_comp_iter(env, local_names, iter) {
        scoped.bind(var.to_string(), var_ty);
    }
    for sub in body {
        rewrite_protocol_calls_in_expr(
            sub,
            protocol_funcs,
            &scoped,
            local_names,
            specializations,
            seen,
        );
    }
}
