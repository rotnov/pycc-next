//! Constraint terms of calls to user functions, builtins and stdlib symbols.

use super::*;

pub(crate) fn collect_call_constraints(
    signatures: &HashMap<String, SignatureTerms>,
    parents: &mut Vec<usize>,
    concrete: &mut Vec<Option<Ty>>,
    deferred: &mut DeferredConstraints,
    env: &ConstraintEnvironment<'_, '_>,
    expr: &HirExpr,
) -> Result<Option<TypeTerm>, Diagnostic> {
    let HirExpr::Call { callee, args } = expr else {
        unreachable!()
    };
    // Mirror of `infer_expr_in`'s D-110 call-target rule (#133): an
    // active value binding shadows builtin and function lookup. At
    // module level `local_names` is empty and `bindings` holds the
    // accumulated top-level assignments in source order; for a
    // private-helper body the environment is seeded from those module
    // globals with the helper's own local names stripped and its
    // parameters re-inserted, so this gate sees a module binding
    // exactly when Python's name resolution would. Binding-first
    // ordering preserves the local diagnostics unchanged, as in
    // `infer_expr_in`. This mirror is load-bearing on its own when
    // the bound callee is neither `print` nor any `def`: without it,
    // `signatures.get` misses, the call stays unresolved, and the
    // solver dead-ends in the misleading "cannot infer return type"
    // error *before* pass 3 ever runs. For a shadowed `print`
    // specifically, the special case below would resolve the call
    // and pass 3's own gate would still catch the shadowing later --
    // there this mirror is fail-fast defense-in-depth, not the only
    // line of defense.
    // A nested function used as a value (`pycc_hir::CLOSURE_MARKER`):
    // its captured values constrain the lifted function's leading
    // parameters as a call's arguments would.
    if callee == pycc_hir::GEN_RETURN_MARKER {
        collect_expr_constraints(signatures, parents, concrete, deferred, env, &args[0])?;
        return Ok(Some(Ok(Ty::None)));
    }
    if callee == pycc_hir::YIELD_MARKER || callee == pycc_hir::YIELD_FROM_MARKER {
        let operand =
            collect_expr_constraints(signatures, parents, concrete, deferred, env, &args[0])?;
        if callee == pycc_hir::YIELD_FROM_MARKER {
            return Ok(match operand {
                Some(Ok(Ty::Generator(sig))) => Some(Ok(sig.2)),
                _ => None,
            });
        }
        return Ok(Some(Ok(env
            .generator_sig
            .as_ref()
            .map_or(Ty::None, |sig| sig.1.clone()))));
    }
    if callee == pycc_hir::CLOSURE_MARKER
        && let [HirExpr::Name(function), captures @ ..] = args.as_slice()
    {
        let mut capture_terms = Vec::with_capacity(captures.len());
        for capture in captures {
            capture_terms.push(collect_expr_constraints(
                signatures, parents, concrete, deferred, env, capture,
            )?);
        }
        let Some(signature) = signatures.get(function) else {
            return Ok(None);
        };
        if captures.len() > signature.1.len() {
            return Err(crate::closure::too_many_captures(
                function,
                captures.len(),
                signature.1.len(),
            ));
        }
        for (index, (term, parameter)) in capture_terms.into_iter().zip(&signature.1).enumerate() {
            if let Some(term) = term {
                unify_terms(
                    parameter.clone(),
                    term,
                    parents,
                    concrete,
                    "T0021",
                    &format!(
                        "captured value {} of nested function `{function}`",
                        index + 1
                    ),
                )?;
            }
        }
        let rest: Option<Vec<Ty>> = signature.1[captures.len().min(signature.1.len())..]
            .iter()
            .map(|term| resolved_term(term.clone(), parents, concrete))
            .collect();
        let ret = resolved_term(signature.2.clone(), parents, concrete);
        return Ok(match (rest, ret) {
            (Some(rest), Some(ret)) => Some(Ok(Ty::Callable(Box::new((rest, ret))))),
            _ => None,
        });
    }
    // A call of a variable holding a function value.
    if let Some(term) = env.bindings.get(callee).cloned()
        && !env.defs_rebound.contains(callee)
        && let Some(Ty::Callable(sig)) = resolved_term(term, parents, concrete)
    {
        for (index, (arg, param)) in args.iter().zip(&sig.0).enumerate() {
            if let Some(term) =
                collect_expr_constraints(signatures, parents, concrete, deferred, env, arg)?
            {
                unify_terms(
                    Ok(param.clone()),
                    term,
                    parents,
                    concrete,
                    "T0021",
                    &format!("argument {} of `{callee}`", index + 1),
                )?;
            }
        }
        return Ok(Some(Ok(sig.1.clone())));
    }
    if let Some(term) = env.bindings.get(callee).cloned()
        && !env.defs_rebound.contains(callee)
    {
        // Part 1 of #1333: a callee whose term is concretely
        // `object` is a call of a CPython object, not a D-110
        // non-callable value. Keyed on the unresolved term, never on
        // `resolved_term`, so admission cannot depend on source
        // order (`object_lift`'s module doc).
        if let Some(result) = object_lift::call_of_object_binding(
            signatures, parents, concrete, deferred, env, &term, args,
        )? {
            return Ok(Some(result));
        }
        // Part 1 of #1027: this gate, not `infer_expr_in`'s own
        // D-110 arm, is the one a `memoryview` *parameter* reaches
        // -- the solver runs first, and a parameter's annotation is
        // already a binding here while the check phase never gets to
        // look at the call. Calling the name is a *read* of it, so
        // it is the capability gap every other use of a `memoryview`
        // is (`C0001`), not D-110's "no value in the current subset
        // is callable" (`T0021`).
        match resolved_term(term.clone(), parents, concrete) {
            Some(ty) => {
                crate::expr::reject_memoryview_read(
                    callee,
                    &ty,
                    env.owned_buffers.contains(callee),
                )?;
            }
            // A loop target over a container this pass cannot type (a list
            // of function values): the check phase types the call.
            None if env.opaque_bindings.contains(callee) => {
                for arg in args {
                    collect_expr_constraints(signatures, parents, concrete, deferred, env, arg)?;
                }
                return Ok(None);
            }
            // Non-strict mode: a name not typed yet that is called (an
            // unannotated helper's parameter, `get()`) is a CPython
            // callable: it is an object, whatever binds it.
            None if crate::non_strict::is_non_strict() => {
                unify_terms(
                    term,
                    Ok(Ty::Object),
                    parents,
                    concrete,
                    "T0021",
                    "a called name",
                )?;
                for arg in args {
                    collect_expr_constraints(signatures, parents, concrete, deferred, env, arg)?;
                }
                return Ok(Some(Ok(Ty::Object)));
            }
            None => {}
        }
        return Err(non_callable_binding(callee));
    }
    // #1313: a foreign import binds its name to a CPython object,
    // and the name deliberately stays out of `bindings` (see the
    // `Name` arm), so the gate above cannot see it. A call of it is
    // an `object` producer, exactly like the `MethodCall` arm's
    // `o.method(...)`: the arguments are collected as ordinary
    // expressions and the call answers the concrete `object` term.
    // Position is the check phase's job, as for the `Name` arm's
    // read: a call inside an unannotated private helper resolves
    // the helper's return to `object` here -- rather than leaving
    // it unresolved, which signature materialization would report
    // as a `T0021` asking for an annotation the type cannot be
    // spelled in. Since Part 1 of #1333 the check phase admits the
    // helper returning it.
    if env.foreign_objects.contains(callee.as_str()) {
        for arg in args {
            collect_expr_constraints(signatures, parents, concrete, deferred, env, arg)?;
        }
        return Ok(Some(Ok(Ty::Object)));
    }
    // A local bound to a value this pass cannot represent (`g = fs[0]`,
    // `crate::constraints::ConstraintEnvironment::opaque_bindings`): the
    // check phase types the call.
    if env.opaque_bindings.contains(callee) && !env.maybe_bindings.contains(callee) {
        for arg in args {
            collect_expr_constraints(signatures, parents, concrete, deferred, env, arg)?;
        }
        return Ok(None);
    }
    if is_local(env.local_names, callee) {
        return Err(unbound_local(callee));
    }
    // #1116, the solver half of `crate::expr`'s own `len`
    // interception: `len(b)` on a `memoryview`-bound name is a
    // `Ty::Int` element count. Like the `Subscript` interception
    // further below it must run *before* the argument recursion
    // that follows, because that recursion reaches the `Name` seam
    // above, which calls `reject_memoryview_read` and would report
    // the `C0001` capability gap for the very expression that
    // closes it. The arity is checked here rather than deferred to
    // the `callee == "len"` block below for the same ordering
    // reason -- a two-argument `len(b, x)` must still reach that
    // block's `T0033`, so only the one-argument shape is claimed.
    if args.len() == 1
        && callee == "len"
        && let HirExpr::Name(buffer_name) = &args[0]
        && let Some(term) = env.bindings.get(buffer_name).cloned()
        && let Some(Ty::MemoryView) = resolved_term(term, parents, concrete)
    {
        return Ok(Some(Ok(Ty::Int)));
    }
    // Part 2a of #1142 (#1165), the solver half of `crate::expr`'s
    // own producer refusal: `ndarray(n)`/`NDArray(n)` is admitted
    // only as an assignment's whole right-hand side, which
    // `collect_block_constraints` handles before the value reaches
    // this walk. Placed before the argument recursion for the same
    // ordering reason the `len` interception above is: the recursion
    // reaches the `Name` seam, which would report a read refusal for
    // an argument of the very expression being refused.
    //
    // The callee-bound gate further above already returned for a
    // name the module binds to a *value*, so only the two spelling
    // shadows and the module-alias binding remain to check here. The
    // last of those is statement (h)'s fifth arm: a stdlib module
    // alias binds the spelling but is a value in no table, so
    // without it this line refused the program's own call with the
    // producer's position message instead of letting the walk below
    // report it. See `crate::buffer::producer_assignment_ty`.
    if crate::buffer::is_producer_spelling(callee)
        && !signatures.contains_key(callee)
        && !env.shadowed_producers.contains(callee.as_str())
        && !env
            .std_module_aliases
            .iter()
            .any(|(alias, _)| alias == callee)
    {
        return Err(if env.in_function_body {
            crate::buffer::producer_position_unsupported(callee)
        } else {
            crate::buffer::producer_at_module_scope(callee)
        });
    }
    let mut arg_terms = Vec::with_capacity(args.len());
    for arg in args {
        arg_terms.push(collect_expr_constraints(
            signatures, parents, concrete, deferred, env, arg,
        )?);
    }
    // A generator argument of a builtin (`crate::generator`): `next(g)` is
    // its element; a consumer sees the list of its values.
    if !signatures.contains_key(callee) {
        if callee == "next"
            && let Some(Some(Ok(Ty::Generator(element)))) = arg_terms.first()
        {
            return Ok(Some(Ok(element.0.clone())));
        }
        // An argument still being inferred (a generator whose element
        // type is not known yet): no term until the next round.
        if callee == "next" && matches!(arg_terms.first(), Some(None)) {
            return Ok(None);
        }
        if crate::generator::is_generator_consumer(callee) {
            for term in arg_terms.iter_mut() {
                if let Some(Ok(Ty::Generator(element))) = term {
                    *term = Some(Ok(Ty::List(Box::new(element.0.clone()))));
                }
            }
        }
    }
    if let Some(symbol) = std_qualified_symbol(callee)
        && let Some(shadowed) =
            shadowed_std_receiver(symbol.module, &env.std_module_aliases, |receiver| {
                env.is_std_receiver_bound(receiver)
            })
    {
        return Err(std_receiver_shadowed(shadowed, symbol.module));
    }
    if !signatures.contains_key(callee)
        && let Some(term) = builtin_call::builtin_call_term(callee, &arg_terms, parents, concrete)
    {
        return Ok(Some(term));
    }
    if !signatures.contains_key(callee)
        && let Some(term) =
            builtin_call::pending_builtin_call(callee, &arg_terms, parents, concrete)
    {
        return Ok(term);
    }
    // `str(x)`/`repr(x)` of an instance: the check phase decides whether
    // its class renders (`crate::string_conversion::instance_text_call`).
    if !signatures.contains_key(callee)
        && matches!(callee.as_str(), "str" | "repr")
        && let [Some(term)] = arg_terms.as_slice()
        && matches!(
            super::unify::resolved_term(term.clone(), parents, concrete),
            Some(Ty::Instance(_))
        )
    {
        return Ok(Some(Ok(Ty::Str)));
    }
    if callee == "print" {
        return Ok(Some(Ok(Ty::None)));
    }
    if callee == "len" {
        // D-105 point 3: `len(lst)` is a hand-recognized builtin
        // call, same as `print` above, not a user-declarable
        // signature. Its own return (`Ty::Int`) never depends on
        // the list's element type, so it's always producible here
        // regardless of whether the argument's own term has
        // resolved yet -- unlike `ListLiteral`/`Subscript`/
        // `ListAppend` above, there's no homogeneity-style check
        // to defer. Only a term that is *already* a known concrete
        // type can be validated at this point in constraint
        // collection (union-find resolution hasn't run yet); an
        // unresolved argument is left to the real check pass
        // (`infer_expr_in`) below, matching this solver's existing
        // lenient-until-known pattern. PR-11 Task 3 (D-123) relaxed
        // the argument-type check below to also accept `Ty::Dict`;
        // PR-11 Task 7 (D-123) relaxes it once more to also accept
        // `Ty::Set`.
        if arg_terms.len() != 1 {
            return Err(Diagnostic::error(
                "T0033",
                format!("`len` expects exactly 1 argument, got {}", arg_terms.len()),
                Span::new(0, 0),
            )
            .with_help("pass exactly 1 argument"));
        }
        // Part 3 of #1026 (PR 3a of #1082): mirrors `expr.rs`'s own
        // `len` guard, which now also admits `Ty::Object`. The
        // `Ok(Ty::Int)` result term below was already unconditional,
        // so accepting the new argument type here is the whole
        // change the solver needs.
        if let Some(Ok(arg_ty)) = &arg_terms[0]
            && !matches!(
                arg_ty,
                Ty::List(_)
                    | Ty::VarTuple(_)
                    | Ty::Dict(_)
                    | Ty::Set(_)
                    | Ty::FrozenSet(_)
                    | Ty::Object
                    | Ty::Tuple(_)
            )
        {
            return Err(Diagnostic::error(
                "T0033",
                format!(
                    "`len` expects a `list[T]`, `dict[K, V]`, `set[T]`, or `frozenset[T]` argument, got `{}`",
                    arg_ty.name()
                ),
                Span::new(0, 0),
            ).with_help("pass a `list[T]`, `dict[K, V]`, `set[T]`, or `frozenset[T]` value"));
        }
        return Ok(Some(Ok(Ty::Int)));
    }
    // #435: `isinstance`/`issubclass` are compile-time-evaluated
    // builtins that always return `Ty::Bool`. The constraint solver
    // only needs the result type — the actual validation and
    // compile-time evaluation happen in `infer_expr_in`'s own Call
    // arm. The class arguments (bare names or tuples of bare names)
    // produce `Ok(None)` terms in the solver (class names are not
    // value bindings), which is harmless.
    // A user-defined function named `isinstance`/`issubclass` takes
    // priority over the builtin (same pattern as `float` above).
    if (callee == "isinstance" || callee == "issubclass") && !signatures.contains_key(callee) {
        return Ok(Some(Ok(Ty::Bool)));
    }
    // #767: `cast(T, value)` is a compile-time-only construct whose
    // type is exactly `T`. Mirrors `infer_expr_in`'s own `cast`
    // interception; without this mirror the callee misses
    // `signatures`, the call stays unresolved, and the solver
    // dead-ends in "cannot infer return type" before the validation
    // pass ever reports the real diagnostic. `args[0]` is a bare
    // type name, which the `Name` arm above already resolved to a
    // harmless `Ok(None)` term (a type name is not a value
    // binding), exactly as for `isinstance`'s class argument.
    //
    // A malformed call shape (wrong arity, or a first argument that
    // is not a bare name) is reported here, from the same shared
    // helper `check_cast` uses, so the two passes cannot drift.
    //
    // The class table is not consulted here (`class_defs` serves
    // the method-call and constructor-call terms only), so it produces a *term* only for the
    // four builtin scalar target names it can recognize on its own;
    // an unverified class name yields `Ok(None)` and leaves the
    // decision to `check_cast`. Producing an unverified
    // `Ty::Instance(name)` here instead was measurably worse: for
    // `cast(Nope, x)` the solver's term reached the return-type
    // comparison first and reported `T0022` ("conflicting inferred
    // types `int` and `Nope`") in place of `check_cast`'s accurate
    // `T0001` ("`Nope` is not a known class or builtin type").
    //
    // #767 review fix (D-198): unlike `check_cast`, this arm does
    // not reject a representation-changing target (`cast(str, 5)`,
    // `cast(int, flag)`) — the
    // solver has no resolved `Ty` for `value` to compare against at
    // this point, only an unsolved term. This is the same asymmetry
    // `check_isinstance`'s own doc comment already accepts between
    // the two passes: full validation runs on the validation-pass
    // route (an annotated function's body, module level,
    // `AnnAssign`) and is absent for a return-type-inferred private
    // helper, which only ever reaches this solver arm.
    if callee == "cast" && !signatures.contains_key(callee) {
        let target = crate::class::cast_target_name(args)?;
        if pycc_hir::is_builtin_type_name(target) {
            return Ok(Some(Ok(crate::class::cast_target_ty(target))));
        }
        return Ok(None);
    }
    if let Some(symbol) = std_qualified_symbol(callee) {
        // See `shadowed_std_receiver`'s own doc comment -- a real
        // local/parameter named `math` (or an alias of it, #962)
        // shadows the stdlib module.
        if let Some(shadowed) =
            shadowed_std_receiver(symbol.module, &env.std_module_aliases, |receiver| {
                env.is_std_receiver_bound(receiver)
            })
        {
            return Err(std_receiver_shadowed(shadowed, symbol.module));
        }
        let pycc_std::StdSymbolKind::Function {
            arg_tys: expected_arg_tys,
            ret_ty,
        } = symbol.kind
        else {
            return Err(if is_marker_kind(symbol.kind) {
                if matches!(symbol.kind, pycc_std::StdSymbolKind::EnumMarker) {
                    enum_marker_is_not_a_value(callee)
                } else if matches!(symbol.kind, pycc_std::StdSymbolKind::AnnotationMarker) {
                    annotation_marker_is_not_a_value(callee)
                } else if matches!(symbol.kind, pycc_std::StdSymbolKind::CastMarker) {
                    cast_marker_is_not_a_value(callee)
                } else if matches!(symbol.kind, pycc_std::StdSymbolKind::TypeCheckingMarker) {
                    type_checking_marker_is_not_a_value(callee)
                } else if matches!(symbol.kind, pycc_std::StdSymbolKind::TypingFormMarker) {
                    typing_form_is_not_a_value(callee)
                } else {
                    marker_is_not_a_value(callee)
                }
            } else {
                std_constant_is_not_callable(callee)
            });
        };
        if arg_terms.len() != expected_arg_tys.len() {
            return Err(Diagnostic::error(
                "T0021",
                format!(
                    "`{callee}` expects {} argument(s), got {}",
                    expected_arg_tys.len(),
                    arg_terms.len()
                ),
                Span::new(0, 0),
            )
            .with_help(format!(
                "pass exactly {} argument(s)",
                expected_arg_tys.len()
            )));
        }
        // Non-strict mode: called with a CPython object, the function is
        // CPython's own (`pycc_mir`'s module call), its result an object.
        if crate::non_strict::is_non_strict()
            && arg_terms
                .iter()
                .any(|term| matches!(term, Some(Ok(Ty::Object))))
        {
            return Ok(Some(Ok(Ty::Object)));
        }
        for (term, expected) in arg_terms.iter().zip(expected_arg_tys) {
            if let Some(Ok(arg_ty)) = term
                && *arg_ty != std_scalar_to_ty(*expected)
            {
                return Err(Diagnostic::error(
                    "T0021",
                    format!(
                        "`{callee}` expects `{}`, got `{}`",
                        std_scalar_to_ty(*expected).name(),
                        arg_ty.name()
                    ),
                    Span::new(0, 0),
                )
                .with_help(format!(
                    "pass a `{}` value",
                    std_scalar_to_ty(*expected).name()
                )));
            }
        }
        return Ok(Some(Ok(std_scalar_to_ty(ret_ty))));
    }
    // The solver-side mirror of `infer_expr_in`'s `round` arm; an
    // unresolved argument falls through to the builtin's `C0001`,
    // as `bool`'s does below.
    if callee == "round" && !signatures.contains_key(callee) {
        let resolved: Option<Vec<Ty>> = arg_terms
            .iter()
            .map(|term| match term {
                Some(Ok(ty)) => Some(ty.clone()),
                _ => None,
            })
            .collect();
        if let Some(arg_tys) = resolved {
            return crate::round::round_call_ty(&arg_tys).map(|ty| Some(Ok(ty)));
        }
    }
    if callee == "float" && !signatures.contains_key(callee) {
        // A user-defined `float` takes priority over the builtin -- see
        // `infer_expr_in`'s own identical guard and its comment for why
        // this differs from `len`/`print`, which need no such guard.
        // Mirrors the `len` arm immediately above for the same reason (D-105
        // point 3's rationale applies identically): `float`'s own return type
        // (`Ty::Float`) never depends on the argument's resolved type, so it
        // is always producible here regardless of whether the argument's own
        // term has resolved yet. Only an already-concretely-resolved argument
        // can be validated at this point (union-find resolution hasn't run);
        // an unresolved argument is left to the real check pass
        // (`infer_expr_in`) above, matching this solver's existing
        // lenient-until-known pattern.
        if arg_terms.len() != 1 {
            return Err(Diagnostic::error(
                "T0021",
                format!(
                    "`float` expects exactly 1 argument, got {}",
                    arg_terms.len()
                ),
                Span::new(0, 0),
            )
            .with_help("pass exactly 1 argument"));
        }
        // Part 4 of #1026 (PR 4a of #1083) admits `Ty::Object` here
        // exactly as `infer_expr_in`'s own arm does; the two must not
        // drift. Its comment carries the D-244 rule-7 reasoning and the
        // reason the message still enumerates only the three original
        // types.
        //
        // It does *not* carry that arm's user-defined-class guard: the
        // class table is not consulted here. Nor
        // does it need one -- a `Ty::Object` term can only come from a
        // foreign name, and `I0404` refuses a foreign name used in a
        // function body at all, which is the only place this solver
        // runs (issue #142's unannotated private helpers). Verified by
        // compiling a private helper returning `float(gc)`: `I0404`,
        // with and without a user-defined `class float`.
        if let Some(Ok(arg_ty)) = &arg_terms[0]
            && !matches!(arg_ty, Ty::Int | Ty::Float | Ty::Bool | Ty::Object)
        {
            return Err(Diagnostic::error(
                "T0021",
                format!(
                    "`float` expects an `int`, `float`, or `bool` argument, got `{}`",
                    arg_ty.name()
                ),
                Span::new(0, 0),
            )
            .with_help("pass an `int`, `float`, or `bool` value"));
        }
        return Ok(Some(Ok(Ty::Float)));
    }
    if callee == "bool" && !signatures.contains_key(callee) {
        // Part 4 of #1026 (PR 4a of #1083): the solver-side mirror of
        // `infer_expr_in`'s `bool` arm, which owns the reasoning --
        // `Ty::Object` only, user-defined `bool` first.
        //
        // Unlike the `float` arm above, this one does not defer an
        // unresolved term. `bool` admits exactly one argument type,
        // so every other argument -- resolved to something else, or
        // not yet resolved at all -- falls through to
        // `unsupported_callable_builtin`'s C0001 below, which is how
        // issue #142 deliberately classifies a known callable builtin
        // in the solver. `float` can be lenient only because its arm
        // ends in an unconditional `Ty::Float`, so an unresolved term
        // there still has a result to fall through to; here there is
        // none.
        //
        // The user-defined-class guard `infer_expr_in`'s arm carries is
        // absent here for the reason the `float` arm above states: no
        // class table, and `I0404` keeps `Ty::Object` out of every
        // function body this solver runs on.
        if let [Some(Ok(Ty::Object))] = arg_terms.as_slice() {
            return Ok(Some(Ok(Ty::Bool)));
        }
    }
    if (callee == "int" || callee == "str" || callee == "repr") && !signatures.contains_key(callee) {
        // Part 4 of #1026 (PR 4b of #1083): the solver-side mirrors of
        // `infer_expr_in`'s `int`/`str` arms, which own the reasoning
        // -- `Ty::Object` only, a user-defined `int`/`str` first.
        //
        // They follow the `bool` arm above rather than the `float`
        // one in not deferring an unresolved term, for the reason that
        // arm states: each admits exactly one argument type, so every
        // other argument -- resolved to something else or not yet
        // resolved at all -- falls through to
        // `unsupported_callable_builtin`'s `C0001` below, which is how
        // issue #142 deliberately classifies a known callable builtin
        // here.
        //
        // The user-defined-*class* guard `infer_expr_in`'s arms carry
        // is absent here for the reason the `float` arm above states:
        // the class table is not consulted here, and `I0404`
        // keeps a foreign object out of every function body it runs
        // on. `infer_expr_in` is the authority for a class-shadowed
        // name and refuses the program there.
        if let [Some(Ok(Ty::Object))] = arg_terms.as_slice() {
            return Ok(Some(Ok(if callee == "int" { Ty::Int } else { Ty::Str })));
        }
    }
    // The solver half of `crate::expr`'s `abs`/`divmod`/`int` arm. Only
    // fully resolved arguments are admitted; anything else falls
    // through to the same `C0001` as before.
    if crate::int_builtins::NAMES.contains(&callee.as_str()) && !signatures.contains_key(callee) {
        let resolved: Option<Vec<Ty>> = arg_terms
            .iter()
            .map(|term| match term {
                Some(Ok(ty)) => Some(ty.clone()),
                _ => None,
            })
            .collect();
        match resolved {
            Some(resolved) => {
                if let Some(result) = crate::int_builtins::result_ty(callee, &resolved) {
                    return Ok(Some(Ok(result?)));
                }
            }
            // An argument not resolved yet: `divmod` and `int` have a
            // fixed result, and `abs` has `0 + x`'s, so the deferred
            // binop rule types it; the check pass validates the
            // arguments once they resolve.
            None if callee == "abs" && arg_terms.len() == 1 => {
                let Some(operand) = arg_terms[0].clone() else {
                    // No term for the operand (an element of a container
                    // the empty-container pass types): the check pass
                    // types the call.
                    return Ok(None);
                };
                let result = fresh_term(parents, concrete);
                deferred
                    .binops
                    .push((BinOpKind::Add, Ok(Ty::Int), operand, result.clone(), None));
                return Ok(Some(result));
            }
            None if callee == "divmod" && arg_terms.len() == 2 => {
                return Ok(Some(Ok(Ty::Tuple(Box::new(vec![Ty::Int, Ty::Int])))));
            }
            None if callee == "int" && arg_terms.len() == 1 => {
                return Ok(Some(Ok(Ty::Int)));
            }
            None => {}
        }
    }
    // Part 2b of #1142 (#1164), the solver half of `crate::expr`'s
    // own buffer-returning-call refusal. The two must not drift: this
    // solver runs first, so without the mirror an intra-artifact call
    // to a buffer-returning export resolves to a `Ty::MemoryView`
    // term here and is reported as some later type error rather than
    // as the named capability gap it is.
    if let Some((_, _, Ok(Ty::MemoryView))) = signatures.get(callee) {
        return Err(crate::buffer::buffer_returning_call_unsupported(callee));
    }
    let Some(signature) = signatures.get(callee) else {
        // Issue #142: a private helper calling a known callable
        // builtin (e.g. `ValueError("x")`) gets the same `C0001`
        // classification as the final validation pass, rather than
        // deferring with `Ok(None)` -- the builtin genuinely exists
        // in Python 3.14, so it is a capability gap, not an
        // unresolved callee. A name that is neither a signature, a known
        // builtin, nor a user class still returns `Ok(None)` and
        // defers to final validation's `T0021`.
        //
        // Part 1 of #1319: the solver half of `crate::expr`'s
        // `frozenset(...)` arm. `signatures` has already missed, so a
        // `def frozenset` cannot reach here; `shadowed_producers`
        // carries a module `class frozenset`, which skips this arm and
        // falls through to `is_known_callable_builtin`'s `C0001`
        // below -- the pre-existing behavior every builtin-named
        // class shares (`class range:` reports the same), since the
        // class-constructor term below (#1342) is consulted only
        // after that `C0001`.
        if callee == crate::frozenset::FROZENSET
            && !env.shadowed_producers.contains(callee.as_str())
        {
            return Ok(Some(crate::frozenset::check_call_terms(
                &arg_terms, parents, concrete, deferred,
            )?));
        }
        // #1331: the solver half of `crate::expr`'s `hash(...)` arm,
        // guarded like the `frozenset` arm directly above and, like
        // `crate::expr`'s, against a stdlib module alias spelled
        // `hash`.
        if callee == crate::hash::HASH
            && !env.shadowed_producers.contains(callee.as_str())
            && !env
                .std_module_aliases
                .iter()
                .any(|(alias, _)| alias == callee)
        {
            return Ok(Some(Ok(crate::hash::check_call_terms(&arg_terms)?)));
        }
        // Part 11 of #1371: the solver half of `crate::expr`'s
        // `type(o)` arm, guarded like the `hash` arm above. An
        // argument whose term is still unresolved (an unannotated
        // parameter a caller passes an object to) is read as an
        // object, as `hash`'s arm reads it as hashable: the final
        // check pass re-types the call with the resolved argument
        // and refuses a native one with the known-builtin `C0001`.
        // A bare narrowed `object` read is the object itself (#1476).
        if !env.shadowed_producers.contains(callee.as_str())
            && let [Some(arg_term)] = arg_terms.as_slice()
            && crate::foreign::type_call::is_object_type_call(
                &env.std_module_aliases,
                callee,
                &[match arg_term {
                    _ if object_narrow::is_bare_narrowed_read(env, &args[0]) => Ty::Object,
                    Ok(arg_ty) => arg_ty.clone(),
                    Err(_) => Ty::Object,
                }],
            )
        {
            return Ok(Some(Ok(Ty::Object)));
        }
        if crate::non_strict::is_non_strict()
            && !env.shadowed_producers.contains(callee.as_str())
            && (pycc_hir::empty_object_constructor(callee, args.len()).is_some()
                || (callee == "set"
                    && matches!(arg_terms.as_slice(), [Some(Ok(Ty::Object))]))
                // `iter(xs)` / `tuple(xs)` of an argument the solver cannot
                // type yet (a list display of tuples): CPython's own, which
                // the check phase re-types (`pycc_hir::builtins`).
                || (matches!(callee.as_str(), "iter" | "tuple")
                    && matches!(arg_terms.as_slice(), [None | Some(Ok(Ty::Object))]))
                || matches!(arg_terms.as_slice(), [Some(Ok(first)), Some(Ok(second))]
                    if pycc_hir::builtins::is_object_format(callee, &[first.clone(), second.clone()])))
        {
            return Ok(Some(Ok(Ty::Object)));
        }
        // An empty `set()`/`dict()`/`list()` the empty-container pass has
        // not typed yet gives no term while a partial table is collected
        // (`return_fallback::partial_signatures`): the table that pass
        // reruns with may be what types it (`s.add(c.find(i))`).
        if super::return_fallback::collecting_partial()
            && args.is_empty()
            && matches!(callee.as_str(), "set" | "dict" | "list")
            && !env.shadowed_producers.contains(callee.as_str())
        {
            return Ok(None);
        }
        if is_known_callable_builtin(callee) {
            return Err(unsupported_callable_builtin(callee));
        }
        // #1342: `C(args)` on a non-generic user class answers its
        // instance; placed last so every reading above keeps
        // precedence (`constructor_call`'s module doc).
        return Ok(constructor_call::constructor_call_term(env, callee));
    };
    for (index, (arg, parameter)) in arg_terms.into_iter().zip(&signature.1).enumerate() {
        // Unify whenever either side is still an inference variable --
        // not just when the callee's own parameter is unresolved.
        // This used to only match `parameter: Err(_)`, so a concrete
        // (e.g. explicitly annotated) callee parameter never
        // constrained an unresolved *caller* argument variable in the
        // reverse direction, even though `unify_terms` itself already
        // handles that case symmetrically (self-review finding,
        // pre-merge).
        // Non-strict mode: an annotated `frozenset[str]` parameter takes a
        // CPython object too (converted, `non_strict::admits_coercion`),
        // so it does not type an argument still being inferred.
        if matches!((&arg, parameter), (Some(Err(_)), Ok(ty))
            if crate::non_strict::is_non_strict() && crate::non_strict::is_coercible_frozenset(ty))
        {
            continue;
        }
        // Nor does an `object` parameter, which takes any value it can box:
        // an argument still being inferred (a module constant built from
        // others) keeps the type its own uses give it.
        if matches!((&arg, parameter), (Some(Err(_)), Ok(Ty::Object)))
            && crate::non_strict::is_non_strict()
        {
            continue;
        }
        if let Some(arg) = arg
            && matches!((&arg, parameter), (Err(_), _) | (_, Err(_)))
        {
            // Defense in depth against a `Ty::Param` leak (finding
            // #2, PR-13 fix round): this solver has no notion of
            // `instantiate_generic_call` -- it constrains an
            // unannotated parameter's fresh inference variable
            // directly against whichever concrete side is
            // available. When that "concrete" side is actually a
            // still-generic function's own uninstantiated `x: T`
            // parameter type, or a still-generic call's `-> T`
            // return type flowing in as an argument, unifying it
            // in would bind the unannotated parameter's resolved
            // type to the raw internal `Ty::Param` representation
            // -- which then surfaces to the user in a later
            // diagnostic (e.g. an `Add`-not-defined error
            // mentioning `T` instead of a real type). Reject this
            // shape here, before that leak can happen, with a
            // clear, dedicated diagnostic instead.
            if matches!(&arg, Ok(ty) if ty_contains_param(ty))
                || matches!(parameter, Ok(ty) if ty_contains_param(ty))
            {
                return Err(Diagnostic::error(
                    "T0042",
                    format!(
                        "cannot infer the type of argument {} of private helper `{callee}` from a generic function's uninstantiated type; add an explicit type annotation",
                        index + 1
                    ),
                    Span::new(0, 0),
                ).with_help("add an explicit type annotation"));
            }
            unify_terms(
                parameter.clone(),
                arg,
                parents,
                concrete,
                "T0021",
                &format!("argument {} of private helper `{callee}`", index + 1),
            )
            .map_err(|conflict| {
                match signature.0.get(index) {
                    Some(param) if super::public_inference::reports_missing_annotation(callee) => {
                        super::public_inference::parameter_conflict(callee, param, &conflict)
                    }
                    _ => conflict,
                }
            })?;
        }
    }
    // A call of a generator function whose element type the solver is
    // still inferring yields no term until it is resolved.
    if let Ok(Ty::Generator(element)) = &signature.2
        && crate::empty_container::contains_infer(&element.0)
    {
        return Ok(None);
    }
    Ok(Some(signature.2.clone()))
}
