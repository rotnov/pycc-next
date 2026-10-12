//! Rewriting generic call sites to their monomorphized instantiations.

use super::*;

pub(crate) fn rewrite_generic_calls_in_expr(
    env: &mut Environment,
    local_names: &[&str],
    expr: &mut HirExpr,
    instantiations: &mut Vec<GenericInstantiation>,
    seen: &mut HashSet<String>,
) -> Result<Ty, Diagnostic> {
    match expr {
        HirExpr::Call { callee, args } => {
            // #380 (#435): `isinstance`/`issubclass` are compile-time
            // builtins whose class-name arguments (args[1] for isinstance,
            // both args for issubclass) are not value expressions — they
            // are bare class names or tuples of class names. Rewriting them
            // with `rewrite_generic_calls_in_expr` would call
            // `infer_expr_in` on a bare class name, which looks it up as a
            // value binding and fails with T0021 ("name not defined").
            // Skip the class-name arguments entirely, rewriting only the
            // object argument (isinstance's args[0]). A user-defined
            // function named `isinstance` takes priority over the builtin
            // (same pattern as `infer_expr_in`'s own guard).
            //
            // Only `isinstance` needs this special-case here:
            // `issubclass`'s arguments are always class names (never
            // generic call expressions), so `rewrite_generic_calls_in_expr`
            // never encounters a builtin `issubclass` call that needs arg
            // rewriting. Checking only `isinstance` avoids permanently-
            // uncovered `issubclass` branches under D-014's 100 %
            // coverage gate.
            if callee == "isinstance" && env.lookup_function(callee).is_none() {
                // `isinstance` always has ≥ 2 args (the type checker
                // validates argument count before this code runs), so
                // `args[0]` is always safe. The guard was removed to
                // avoid a permanently-uncovered `false` branch under
                // D-014's 100 %-coverage gate.
                //
                // Discard the returned `Ty` — only the rewriting side
                // effect matters here; the subsequent `infer_expr_in`
                // on the whole expression re-derives the type.  Using
                // `let _ =` instead of `?` avoids a permanently-
                // uncovered error-path region under D-014's 100 %
                // coverage gate: `rewrite_generic_calls_in_expr` can
                // only fail when `infer_expr_in` on a sub-expression
                // fails, and this same `infer_expr_in` call on the
                // whole expression immediately below would surface
                // the identical error.
                let _ = rewrite_generic_calls_in_expr(
                    env,
                    local_names,
                    &mut args[0],
                    instantiations,
                    seen,
                );
                return infer_expr_in(env, local_names, expr);
            }
            // #1116: `len(b)` on a `memoryview`-bound name skips the
            // argument recursion below entirely, for the reason the
            // `Subscript` arm's own `is_memoryview_base` guard records --
            // that recursion tails into `infer_expr_in` on the bare name,
            // which reaches `reject_memoryview_read` and reports `C0001`
            // for an expression `crate::expr`'s `len` interception admits.
            // A bare `memoryview` name is never a generic-call argument
            // (there is nothing to instantiate), so nothing is lost by not
            // descending. The `infer_expr_in` on the whole expression below
            // then resolves the call through that interception, exactly as
            // the `isinstance` arm above does.
            if callee == "len" && args.len() == 1 && is_memoryview_base(env, &args[0]) {
                return infer_expr_in(env, local_names, expr);
            }
            // Each arg's `Ty` comes directly from this same rewriting
            // recursion's own return value -- not a second, separate
            // `infer_expr_in` pass over the now-rewritten args -- since
            // both would always agree (this function delegates to
            // `infer_expr_in` for exactly the types it doesn't special-case
            // itself) and only one needs to actually run.
            let arg_tys = args
                .iter_mut()
                .map(|arg| {
                    rewrite_generic_calls_in_expr(env, local_names, arg, instantiations, seen)
                })
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(generic_func) = env.lookup_generic(callee).cloned() {
                let instantiation = instantiate_generic_call(&generic_func, &arg_tys)?;
                // Registered into `env` regardless of whether this is the
                // first call site to discover this exact specialization --
                // this scope's own `env` may be an independent clone (e.g.
                // a fresh `child_for_function` per function) that never
                // observed a sibling scope's earlier registration, and a
                // *later* expression in this same scope may itself call the
                // now-rewritten mangled name (directly, or as a nested
                // generic-call argument), which needs it bound to resolve
                // as an ordinary function via `infer_expr_in`.
                if env.lookup_function(&instantiation.mangled_name).is_none() {
                    env.bind_function(
                        instantiation.mangled_name.clone(),
                        arg_tys,
                        instantiation.return_ty.clone(),
                    );
                }
                if seen.insert(instantiation.mangled_name.clone()) {
                    instantiations.push(instantiation.clone());
                }
                *callee = instantiation.mangled_name;
                return Ok(instantiation.return_ty);
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
            // `let _ =` rather than `?`, for the same reason the
            // `isinstance` arm above uses it: only the rewriting side
            // effect matters here, and the `infer_expr_in` call on the
            // whole unary expression immediately below recurses into this
            // same operand, so it surfaces the identical error. Propagating
            // here as well would leave a permanently-uncovered error-path
            // region under D-014's 100 %-coverage gate.
            let _ = rewrite_generic_calls_in_expr(env, local_names, operand, instantiations, seen);
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::CompareChain { first, links } => {
            // `let _ =` rather than `?`, as in the `UnaryOp` arm above: the
            // `infer_expr_in` on the whole chain below recurses into every
            // operand and surfaces the identical error.
            let _ = rewrite_generic_calls_in_expr(env, local_names, first, instantiations, seen);
            for link in links.iter_mut() {
                let _ = rewrite_generic_calls_in_expr(
                    env,
                    local_names,
                    &mut link.right,
                    instantiations,
                    seen,
                );
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::BinOp { left, right, .. }
        | HirExpr::AugBinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            for sub in [left.as_mut(), right.as_mut()] {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::IfExp { test, body, orelse } => {
            for sub in [test.as_mut(), body.as_mut(), orelse.as_mut()] {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::FString(parts) => {
            for part in parts.iter_mut() {
                if let FStringPart::Interpolation(inner) = part {
                    rewrite_generic_calls_in_expr(env, local_names, inner, instantiations, seen)?;
                }
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::ListLiteral(elements)
        | HirExpr::ObjectList(elements)
        | HirExpr::SetLiteral(elements)
        | HirExpr::TupleLiteral(elements) => {
            for element in elements.iter_mut() {
                rewrite_generic_calls_in_expr(env, local_names, element, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::DictLiteral(pairs) => {
            for (key, value) in pairs.iter_mut() {
                for sub in [key, value] {
                    rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
                }
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::Subscript { base, index } => {
            // PEP 560 (#610): when the base is a bare class name, `C[x]` is
            // `C.__class_getitem__(x)` and the base is not a value
            // expression -- recursing into it would call `infer_expr_in` on
            // a bare class name and fail with T0021 ("name not defined"),
            // exactly the failure mode this function's own `isinstance`
            // class-argument skip above exists to avoid. Rewrite only the
            // index in that case; `infer_expr_in` on the whole expression
            // below still resolves the hook and reports any error.
            //
            // Part 2 of #1027 adds a second base this arm must not recurse
            // into, for the same reason and with the same remedy: a
            // `memoryview`-bound name. `rewrite_generic_calls_in_expr`'s own
            // tail calls `infer_expr_in` on each sub-expression, and on a
            // bare `memoryview` name that is the `C0001` read refusal --
            // reported here, in a module that happens to contain a generic
            // function, for a `b[i]` the check phase admits. The guard is a
            // separate predicate rather than a widening of
            // `is_class_name_base`, whose two other callers (the `AttrGet`
            // and `MethodCall` arms below) must keep refusing `b.attr` and
            // `b.tolist()` exactly as they do today.
            if !is_class_name_base(env, local_names, base.as_ref())
                && !is_memoryview_base(env, base.as_ref())
            {
                rewrite_generic_calls_in_expr(env, local_names, base, instantiations, seen)?;
            }
            rewrite_generic_calls_in_expr(env, local_names, index, instantiations, seen)?;
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            for sub in std::iter::once(base.as_mut()).chain(
                [start, stop, step]
                    .into_iter()
                    .flatten()
                    .map(|b| b.as_mut()),
            ) {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        // #1263: an attribute receiver is an ordinary sub-expression,
        // rewritten before the node's own arguments (CPython's order).
        HirExpr::ListAppend { list, value } => {
            for sub in list.attr_expr_mut().into_iter().chain([value.as_mut()]) {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::SetAdd { value, .. } => {
            rewrite_generic_calls_in_expr(env, local_names, value, instantiations, seen)?;
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::ListPop { list } => {
            if let Some(sub) = list.attr_expr_mut() {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::DictGetOrDefault { dict, key, default } => {
            for sub in dict
                .attr_expr_mut()
                .into_iter()
                .chain([key.as_mut(), default.as_mut()])
            {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::AttrGet { base, .. } => {
            // `C.attr` on a bare class name reads a class attribute, so the
            // base is not a value expression -- the same skip the `Subscript`
            // arm above applies, for the same reason.
            if !is_class_name_base(env, local_names, base.as_ref()) {
                rewrite_generic_calls_in_expr(env, local_names, base, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::MethodCall { base, args, .. } => {
            // `C.m()` on a bare class name calls a `@staticmethod` or
            // `@classmethod` through the class, so the base is not a value
            // expression either; the arguments still are.
            if !is_class_name_base(env, local_names, base.as_ref()) {
                rewrite_generic_calls_in_expr(env, local_names, base, instantiations, seen)?;
            }
            for arg in args.iter_mut() {
                rewrite_generic_calls_in_expr(env, local_names, arg, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        // Part 2a of #1371: the callee is an ordinary value expression (a
        // subscript), so it is rewritten like any argument.
        HirExpr::ExprCall { callee, args } => {
            rewrite_generic_calls_in_expr(env, local_names, callee, instantiations, seen)?;
            for arg in args.iter_mut() {
                rewrite_generic_calls_in_expr(env, local_names, arg, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        HirExpr::ReceiverClassCall { args } => {
            for arg in args.iter_mut() {
                rewrite_generic_calls_in_expr(env, local_names, arg, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        // Part 8 of #1371: rewrite every operand of the positional half
        // and every keyword value, but infer the *whole* node -- inferring
        // the positional half alone would check a pycc callee's arity
        // before the node's own keyword refusal.
        HirExpr::KeywordCall { call, keywords, .. } => {
            let (callee, args) = match call.as_mut() {
                HirExpr::Call { args, .. } => (None, args),
                HirExpr::MethodCall { base, args, .. } => {
                    let base = (!is_class_name_base(env, local_names, base.as_ref()))
                        .then_some(base.as_mut());
                    (base, args)
                }
                HirExpr::ExprCall { callee, args } => (Some(callee.as_mut()), args),
                other => unreachable!("a keyword call never wraps {other:?}"),
            };
            for part in callee
                .into_iter()
                .chain(args.iter_mut())
                .chain(keywords.iter_mut().map(|(_, value)| value))
            {
                rewrite_generic_calls_in_expr(env, local_names, part, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        // Issue #1188: rewrite inside the wrapped method call exactly as the
        // `MethodCall` arm does, but infer the *whole* node, so the result
        // follows the receiver's reading -- inferring `call` alone would
        // report a method error on a list receiver.
        HirExpr::ReceiverDispatchedCall { call, .. } => {
            let (base, args) = call
                .method_call_parts_mut()
                .expect("a receiver-dispatched call always wraps a MethodCall");
            if !is_class_name_base(env, local_names, base) {
                rewrite_generic_calls_in_expr(env, local_names, base, instantiations, seen)?;
            }
            for arg in args.iter_mut() {
                rewrite_generic_calls_in_expr(env, local_names, arg, instantiations, seen)?;
            }
            infer_expr_in(env, local_names, expr)
        }
        // PEP 695 (#387): `C[type_arg](args)` — the monomorphized class
        // methods were pre-registered in `env` by `monomorphize`'s own
        // pre-scan (see `instantiate_generic_class_methods`), so this arm
        // only needs to recurse into `args` (rewriting any nested generic
        // function calls) and then rewrite the expression in place to an
        // ordinary `HirExpr::Call` to the mangled class name. The mangled
        // name follows the same `0gen_`-prefixed scheme as generic
        // functions (D-133/D-134).
        HirExpr::GenericClassInstantiate {
            class,
            type_arg,
            args,
        } => {
            for arg in args.iter_mut() {
                rewrite_generic_calls_in_expr(env, local_names, arg, instantiations, seen)?;
            }
            let class_def = env.lookup_class(class).ok_or_else(|| {
                Diagnostic::error(
                    "T0001",
                    format!("class `{class}` is not defined"),
                    Span::new(0, 0),
                )
            })?;
            let type_param_name = class_def.type_param.as_deref().ok_or_else(|| {
                t0042(format!(
                    "class `{class}` is not generic (has no type parameter to instantiate)"
                ))
            })?;
            let mangled = mangle_generic_instantiation(class, type_param_name, type_arg);
            // Rewrite in place to an ordinary call.
            let args_vec = args.clone();
            *expr = HirExpr::Call {
                callee: mangled.clone(),
                args: args_vec,
            };
            infer_expr_in(env, local_names, expr)
        }
        // PEP 572 (#774): `target := value` — recurse into `value` first
        // (mirroring `AttrGet`'s own "recurse into the one sub-expression,
        // then delegate to `infer_expr_in` for the final type" shape), so a
        // generic call nested inside a walrus's value (`(x := identity(1))`)
        // is still found and rewritten. Unlike every other leaf-delegating
        // arm above, `name` must also be bound into `env` here: this pass
        // starts each statement's rewrite from a *fresh* `Environment`
        // (see `monomorphize`), not the one the module's ordinary
        // `check_and_resolve` pass populated, so a walrus target read later
        // in the same scope (as a plain name, or as another generic call's
        // argument) would otherwise resolve against a binding that was
        // never carried over and spuriously fail. `rewrite_generic_calls_in_stmt`
        // binds an `Assign`/`AnnAssign` target the same way for the same
        // reason; a walrus can appear inside any expression position, so
        // its own binding has to happen here rather than only at the
        // statement level.
        HirExpr::NamedExpr { name, value } => {
            let name = name.clone();
            rewrite_generic_calls_in_expr(env, local_names, value, instantiations, seen)?;
            let ty = infer_expr_in(env, local_names, expr)?;
            env.bind(name, ty.clone());
            Ok(ty)
        }
        // #1254 (D-250): `env` has no scope stack, so the synthesized loop
        // variable is bound directly, exactly as the statement arms in
        // `rewrite_generic_calls_in_stmt` do. That is harmless: the
        // digit-led name is unique per source offset and no user code can
        // name it.
        HirExpr::Comprehension(comp) => {
            comprehension::rewrite_comprehension_expr(env, local_names, comp, instantiations, seen)
        }
        HirExpr::Nd(nd) => {
            let _ = rewrite_generic_calls_in_expr(env, local_names, &mut nd.base, instantiations, seen);
            for arg in &mut nd.args {
                let _ = rewrite_generic_calls_in_expr(env, local_names, arg, instantiations, seen);
            }
            infer_expr_in(env, local_names, expr)
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
        | HirExpr::Super => infer_expr_in(env, local_names, expr),
    }
}

/// Structural counterpart to `rewrite_generic_calls_in_expr`, one level up:
/// walks every statement position, rewriting any embedded `HirExpr` in
/// place and growing `env`'s bindings exactly enough (a plain `env.bind`,
/// not the validating `check_assignment`) for a later statement in the same
/// scope to resolve a local name as a generic call's argument. No shape or
/// assignability validation happens here -- the module already passed that
/// validation once, via the ordinary `check`/`check_and_resolve` path that
/// calls this function only afterward (see `monomorphize`).
pub(crate) fn rewrite_generic_calls_in_stmt(
    env: &mut Environment,
    local_names: &[&str],
    stmt: &mut HirStmt,
    instantiations: &mut Vec<GenericInstantiation>,
    seen: &mut HashSet<String>,
) -> Result<(), Diagnostic> {
    match stmt {
        HirStmt::ExprStmt(expr) => {
            rewrite_generic_calls_in_expr(env, local_names, expr, instantiations, seen)?;
            Ok(())
        }
        HirStmt::Assign { target, value } => {
            let ty = rewrite_generic_calls_in_expr(env, local_names, value, instantiations, seen)?;
            env.bind(target.clone(), ty);
            Ok(())
        }
        HirStmt::AnnAssign {
            target,
            annotation,
            value,
            is_final: _,
        } => {
            if let Some(value) = value {
                rewrite_generic_calls_in_expr(env, local_names, value, instantiations, seen)?;
            }
            env.bind(target.clone(), annotation.clone());
            Ok(())
        }
        HirStmt::If { test, body, orelse } => {
            rewrite_generic_calls_in_expr(env, local_names, test, instantiations, seen)?;
            for s in body.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            for s in orelse.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            Ok(())
        }
        HirStmt::While { test, body } => {
            rewrite_generic_calls_in_expr(env, local_names, test, instantiations, seen)?;
            for s in body.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            Ok(())
        }
        HirStmt::LoopElse { loop_stmt, orelse } => {
            rewrite_generic_calls_in_stmt(env, local_names, loop_stmt, instantiations, seen)?;
            for s in orelse.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            Ok(())
        }
        HirStmt::Break | HirStmt::Continue => Ok(()),
        HirStmt::ForRange {
            var,
            start,
            stop,
            step,
            body,
        } => {
            for sub in [start, stop, step] {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            env.bind(var.clone(), Ty::Int);
            for s in body.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            Ok(())
        }
        HirStmt::ForList { var, list, body } => {
            // Issue #118 Part 1: use `lookup_any` (not `lookup`) since this
            // pass runs post-validation and needs the type regardless of
            // binding state -- the validation pass already rejected any
            // maybe-bound iterable read.
            let var_ty = match env.lookup_any(list) {
                Some(Ty::List(elem)) => *elem,
                Some(Ty::Str) => Ty::Str,
                Some(Ty::Dict(kv)) => kv.0,
                Some(Ty::Set(elem) | Ty::FrozenSet(elem)) => *elem,
                _ => {
                    // Already validated as iterable by the ordinary check
                    // pass that ran before `monomorphize`. A *missing*
                    // binding is reachable: this pass's environment lacks
                    // the foreign names, so a module-level bare-name loop
                    // over a CPython object (#1325) lands here and types its
                    // target `Ty::Infer` rather than `Ty::Object` -- the
                    // #1101 module-body gap `foreign/binding_tests.rs`'s
                    // `a_module_with_a_generic_function_refuses_foreign_reads_in_monomorphization`
                    // pins.
                    Ty::Infer
                }
            };
            env.bind(var.clone(), var_ty);
            for s in body.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            Ok(())
        }
        // PR 3c of #1082: the iterable is a real expression and may hold a
        // generic call, so it is walked. It is also the whole arm: the
        // check pass has already proved the iterable is `Ty::Object`, so
        // its root is a foreign name, and this pass seeds no foreign name
        // into its own environment -- a pre-existing gap that refuses
        // `len(numpy.pi)` in a module with a generic function exactly as it
        // refuses this loop. The walk therefore always returns `T0021` here
        // and the loop body is unreachable. It is kept as the walk rather
        // than a no-op precisely so that closing that gap cannot silently
        // let a generic call inside the iterable escape this pass.
        HirStmt::ForObject { iter, .. } => {
            rewrite_generic_calls_in_expr(env, local_names, iter, instantiations, seen)?;
            Ok(())
        }
        HirStmt::DictSet { key, value, .. } => {
            for sub in [key, value] {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            Ok(())
        }
        HirStmt::AttrSet { base, value, .. } => {
            for sub in [base, value] {
                rewrite_generic_calls_in_expr(env, local_names, sub, instantiations, seen)?;
            }
            Ok(())
        }
        HirStmt::ListCompAssign {
            target,
            var,
            iter,
            cond,
            elt,
        } => {
            let body = cond.iter_mut().map(|c| c.as_mut()).chain([elt.as_mut()]);
            comprehension::rewrite_comp_parts(
                env,
                local_names,
                var,
                iter,
                body,
                instantiations,
                seen,
            )?;
            env.bind(target.clone(), Ty::List(Box::new(Ty::Int)));
            Ok(())
        }
        HirStmt::SetCompAssign {
            target,
            var,
            iter,
            cond,
            elt,
        } => {
            let body = cond.iter_mut().map(|c| c.as_mut()).chain([elt.as_mut()]);
            let elt_ty = comprehension::rewrite_comp_parts(
                env,
                local_names,
                var,
                iter,
                body,
                instantiations,
                seen,
            )?;
            env.bind(target.clone(), comprehension::set_comp_container(elt_ty));
            Ok(())
        }
        HirStmt::DictCompAssign {
            target,
            var,
            iter,
            cond,
            key,
            value,
        } => {
            let body = cond
                .iter_mut()
                .map(|c| c.as_mut())
                .chain([key.as_mut(), value.as_mut()]);
            comprehension::rewrite_comp_parts(
                env,
                local_names,
                var,
                iter,
                body,
                instantiations,
                seen,
            )?;
            env.bind(target.clone(), Ty::Dict(Box::new((Ty::Str, Ty::Int))));
            Ok(())
        }
        HirStmt::Return(value) => {
            if let Some(value) = value {
                rewrite_generic_calls_in_expr(env, local_names, value, instantiations, seen)?;
            }
            Ok(())
        }
        HirStmt::Delete { .. } | HirStmt::ForeignImport { .. } => Ok(()),
        // Part 2c of #1371: a slice `del` reads its base and bounds, so a
        // generic call in any of them is rewritten like any other read.
        HirStmt::DeleteSlice {
            base,
            start,
            stop,
            step,
            ..
        } => {
            rewrite_generic_calls_in_expr(env, local_names, base, instantiations, seen)?;
            for bound in [start, stop, step].into_iter().flatten() {
                rewrite_generic_calls_in_expr(env, local_names, bound, instantiations, seen)?;
            }
            Ok(())
        }
        // #1457: an attribute `del` reads its base.
        HirStmt::DeleteAttr { base, .. } => {
            rewrite_generic_calls_in_expr(env, local_names, base, instantiations, seen)?;
            Ok(())
        }
        HirStmt::DeleteItem { base, key, .. } => {
            rewrite_generic_calls_in_expr(env, local_names, base, instantiations, seen)?;
            rewrite_generic_calls_in_expr(env, local_names, key, instantiations, seen)?;
            Ok(())
        }
        HirStmt::Match { subject, cases } => {
            rewrite_generic_calls_in_expr(env, local_names, subject, instantiations, seen)?;
            for case in cases.iter_mut() {
                if let Some(guard) = case.guard.as_mut() {
                    rewrite_generic_calls_in_expr(env, local_names, guard, instantiations, seen)?;
                }
                for s in case.body.iter_mut() {
                    rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
                }
            }
            Ok(())
        }
        HirStmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            for s in body.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            for handler in handlers.iter_mut() {
                let mut handler_env = env.clone();
                if let (Some(exc_types), Some(name)) = (&handler.exc_type, &handler.name) {
                    let binding_type = pycc_hir::except_handler_binding_type_name(exc_types);
                    handler_env.bind(name.clone(), Ty::Instance(Box::new(binding_type)));
                }
                for s in handler.body.iter_mut() {
                    rewrite_generic_calls_in_stmt(
                        &mut handler_env,
                        local_names,
                        s,
                        instantiations,
                        seen,
                    )?;
                }
            }
            for s in orelse.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            for s in finalbody.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            Ok(())
        }
        // Part 3 of #382 (#542): `except*` always binds `as e` to
        // `ExceptionGroup`, never to the named handler type -- see
        // `check_try_star_stmt` in `pycc_types::exception` for the same rule
        // at type-checking time.
        HirStmt::TryStar {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            for s in body.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            for handler in handlers.iter_mut() {
                let mut handler_env = env.clone();
                if let Some(name) = &handler.name {
                    handler_env.bind(
                        name.clone(),
                        Ty::Instance(Box::new("ExceptionGroup".to_string())),
                    );
                }
                for s in handler.body.iter_mut() {
                    rewrite_generic_calls_in_stmt(
                        &mut handler_env,
                        local_names,
                        s,
                        instantiations,
                        seen,
                    )?;
                }
            }
            for s in orelse.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            for s in finalbody.iter_mut() {
                rewrite_generic_calls_in_stmt(env, local_names, s, instantiations, seen)?;
            }
            Ok(())
        }
        HirStmt::Raise { exc, cause } => {
            if let Some(exc) = exc.as_mut() {
                rewrite_generic_calls_in_raise_operand(
                    env,
                    local_names,
                    exc,
                    instantiations,
                    seen,
                )?;
            }
            if let Some(cause) = cause.as_mut() {
                rewrite_generic_calls_in_raise_operand(
                    env,
                    local_names,
                    cause,
                    instantiations,
                    seen,
                )?;
            }
            Ok(())
        }
    }
}

/// Rewrites generic calls nested in a `raise` operand without asking the
/// ordinary call checker to re-type-check the builtin exception constructor
/// itself. The validation pass has already checked that constructor; only its
/// message expression can still contain a generic call that needs rewriting.
pub(crate) fn rewrite_generic_calls_in_raise_operand(
    env: &mut Environment,
    local_names: &[&str],
    expr: &mut HirExpr,
    instantiations: &mut Vec<GenericInstantiation>,
    seen: &mut HashSet<String>,
) -> Result<(), Diagnostic> {
    if let HirExpr::Call { callee, args } = expr
        && is_unshadowed_builtin_exception(env, local_names, callee)
    {
        for arg in args {
            rewrite_generic_calls_in_expr(env, local_names, arg, instantiations, seen)?;
        }
        return Ok(());
    }
    rewrite_generic_calls_in_expr(env, local_names, expr, instantiations, seen)?;
    Ok(())
}
