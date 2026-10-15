//! Inference of calls to user functions, builtins and stdlib symbols.

use super::*;

pub(crate) fn infer_call_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let HirExpr::Call { callee, args } = expr else {
        unreachable!()
    };
    // D-110 (#133): a call target resolves through the active value
    // binding before any builtin or function-registry fallback -- a
    // module `helper = 1` shadows both a same-named `def` and a
    // builtin at every later call site, and every value binding in
    // the current subset is a primitive, so a shadowed target is
    // non-callable -- with one exception: a module-body `object`
    // binding (a foreign import or a `for` loop target) is callable
    // since #1313, admitted by the first branch inside the gate. The
    // gate is deliberately callee-first (before argument inference),
    // uniform with how the local gate below always behaved. Local diagnostics are preserved exactly:
    // a value-bound local reported `non_callable_binding` before this
    // reordering too, and a local without a binding still falls
    // through to `unbound_local`. In pass 3 the environment is the
    // final module environment (D-041), so a body call is rejected
    // when the callee is value-bound anywhere at top level -- D-110
    // records that consequence (it rejects some later-rebind programs
    // CPython's dynamic order would run) as deliberate; source-order
    // visibility questions stay #22's scope.
    // A nested function used as a value (`pycc_hir::CLOSURE_MARKER`).
    if callee == pycc_hir::CLOSURE_MARKER {
        return crate::closure::closure_value(env, local_names, args);
    }
    if callee == pycc_hir::YIELD_MARKER {
        return crate::generator::yield_value(env, local_names, args);
    }
    if callee == pycc_hir::YIELD_FROM_MARKER {
        return crate::generator::check_yield_from(env, local_names, &args[0], false);
    }
    if let Some(ty) = env.lookup(callee)
        && !env.def_rebound.contains(callee)
    {
        // A call of a variable holding a function value.
        if let Ty::Callable(sig) = ty {
            return crate::closure::call_value(env, local_names, callee, &sig, args);
        }
        // #1313: a direct call of an `object`-typed name (a foreign
        // binding or a `for` loop target) in a module body is an
        // `object` producer under the shared object-call argument
        // rule (`crate::foreign::check_object_call_args`). `lookup` answers `None` for a maybe-bound name, so a
        // one-arm-`if` import falls through to the `T0041` below.
        // #1316 and Part 1 of #1333 admit the same call in a
        // function body for any `object` callee, exactly as the
        // `Name` arm admits its read.
        if matches!(ty, Ty::Object) {
            let arg_tys = args
                .iter()
                .map(|arg| infer_expr_in(env, local_names, arg))
                .collect::<Result<Vec<_>, _>>()?;
            crate::foreign::check_object_call_args(env, args, &arg_tys, "call")?;
            return Ok(Ty::Object);
        }
        return Err(non_callable_binding(callee));
    }
    // Issue #118 Part 1: a maybe-bound callee is not callable -- it
    // may not be bound on every path reaching this call. Reject with
    // T0041 before the `is_local` unbound check below, since a
    // maybe-bound local is "possibly unbound," not "never bound."
    if matches!(env.binding_state(callee), Some(BindingState::Maybe(_))) {
        return Err(possibly_unbound(callee));
    }
    if is_local(local_names, callee) {
        return Err(unbound_local(callee));
    }
    // #435: `isinstance`/`issubclass` are compile-time-evaluated
    // builtins. They must be intercepted BEFORE the generic arg
    // inference loop below, because the class argument (args[1] for
    // isinstance, both args for issubclass) is a class name or tuple
    // of class names — not a value expression. Inferring a bare class
    // name as a regular expression would fail with "name not defined"
    // (class names are registered in `env.classes`, not
    // `env.bindings`). The object argument (isinstance's args[0]) IS
    // inferred normally.
    // A user-defined function named `isinstance`/`issubclass` takes
    // priority over the builtin (same pattern as `float` — see the
    // `a_user_defined_float_function_takes_priority_over_the_builtin`
    // test and its identical guard in the constraint solver).
    if callee == "isinstance" && !env.lookup_function(callee).is_some() {
        return class::check_isinstance(env, local_names, args);
    }
    if callee == "issubclass" && !env.lookup_function(callee).is_some() {
        return class::check_issubclass(env, args);
    }
    // #767: `typing.cast(T, value)` is a compile-time-only construct
    // — a runtime no-op in CPython whose sole effect is to declare
    // `value`'s static type as `T`. Like `isinstance`/`issubclass`
    // above it must be intercepted BEFORE the generic argument
    // inference loop, because `args[0]` is a *type* expression (a
    // bare class or builtin-scalar name, not a value binding) that
    // ordinary inference would reject as an undefined name. A
    // user-defined function named `cast` takes priority over the
    // special case, exactly as for `isinstance`/`issubclass`.
    //
    // The interception does not verify that the module actually wrote
    // `from typing import cast`: `pycc_types` has no access to
    // `HirModule::imports` at all, so no bare-name stdlib symbol is
    // import-gated today (the `Final`/`Annotated`/`Enum` markers behave
    // the same way). Closing that gap uniformly is tracked by #768;
    // `cast_without_its_import_is_currently_accepted` in
    // `tests/typing_cast.rs` pins the present behavior so that fix has
    // to invert it deliberately.
    if callee == "cast" && !env.lookup_function(callee).is_some() {
        return class::check_cast(env, local_names, args);
    }
    // For a callee that survives D-110's binding gate above, preserve
    // the established diagnostic order by inferring every
    // argument before validating arity or compatibility. Most Python
    // calls are small, so keep up to four inferred types on the stack
    // and reserve a heap vector only for wider calls.
    // #1116: `len(b)` on a name bound to a `memoryview` is the
    // buffer's `Ty::Int` element count -- the second read of such a
    // name this compiler admits, after Part 2's `b[i]`. Like the
    // `Subscript` interception further below it must run *before*
    // the argument inference that follows, because `infer_expr_in`'s
    // own `Name` arm calls `reject_memoryview_read`, so inferring
    // the argument first would report the `C0001` capability gap for
    // the very expression that closes it.
    //
    // Only the one-argument shape is claimed, so `len(b, x)` still
    // reaches the `callee == "len"` block's own `T0033` arity
    // refusal below. There is deliberately no `$fn:len` shadow check
    // here, for the reason that block records: D-105 point 3 makes
    // `len` a hand-recognized builtin, not a user-declarable
    // signature.
    if args.len() == 1
        && callee == "len"
        && let HirExpr::Name(buffer_name) = &args[0]
        && matches!(
            env.binding_state(buffer_name),
            Some(BindingState::Definitely(Ty::MemoryView))
        )
    {
        return Ok(Ty::Int);
    }
    const INLINE_ARG_TYPES: usize = 4;
    let mut inline_arg_tys = [const { Ty::Infer }; INLINE_ARG_TYPES];
    let heap_arg_tys;
    let arg_tys: &[Ty] = if args.len() <= INLINE_ARG_TYPES {
        for (slot, arg) in inline_arg_tys.iter_mut().zip(args) {
            *slot = infer_expr_in(env, local_names, arg)?;
        }
        &inline_arg_tys[..args.len()]
    } else {
        heap_arg_tys = args
            .iter()
            .map(|arg| infer_expr_in(env, local_names, arg))
            .collect::<Result<Vec<_>, _>>()?;
        &heap_arg_tys
    };
    // A generator passed to a builtin that consumes it: `next(g)`, and the
    // consumers typed as if given the list of its values (`pycc_mir`
    // materializes it, or loops over it for `any`/`all`).
    let user_callee = env.lookup_function(callee).is_some();
    if !user_callee && let Some(ty) = crate::generator::next_call(callee, arg_tys, env)? {
        return Ok(ty);
    }
    // `any(g)` / `all(g)` of a generator of CPython objects: each value's
    // truth is CPython's (`pycc_mir`'s `GenAnyAll`).
    if !user_callee
        && matches!(callee.as_str(), "any" | "all")
        && let [Ty::Generator(sig)] = arg_tys
        && sig.0 == Ty::Object
    {
        return Ok(Ty::Bool);
    }
    let materialized;
    let arg_tys: &[Ty] =
        if !user_callee && let Some(listed) = crate::generator::consumer_arg_tys(callee, arg_tys) {
            materialized = listed;
            &materialized
        } else {
            arg_tys
        };
    // An `IntEnum` member passed to a builtin reading an `int` is its value
    // (`pycc_hir::int_enum`).
    let read_ints: Vec<Ty>;
    let arg_tys: &[Ty] = if !user_callee
        && pycc_hir::int_enum::reads_int_args(callee)
        && arg_tys.iter().any(pycc_hir::int_enum::is_int_enum)
    {
        read_ints = arg_tys.iter().map(pycc_hir::int_enum::as_int).collect();
        &read_ints
    } else {
        arg_tys
    };
    // A stdlib function the builtin table types (`math.log`) is still
    // refused when a local shadows its module's spelling.
    if let Some(symbol) = std_qualified_symbol(callee)
        && let Some(shadowed) =
            shadowed_std_receiver(symbol.module, &env.std_module_aliases, |receiver| {
                is_std_receiver_bound(env, local_names, receiver)
            })
    {
        return Err(std_receiver_shadowed(shadowed, symbol.module));
    }
    // `str(x)` / `repr(x)` of an instance whose class defines `__str__` or
    // `__repr__` (`pycc_mir` calls the method).
    if env.lookup_function(callee).is_none()
        && env.lookup_class(callee).is_none()
        && let Some(resolved) = crate::string_conversion::instance_text_call(env, callee, arg_tys)
    {
        return resolved;
    }
    // `sorted`/`min`/`max` of instances (`crate::instance_order`).
    if env.lookup_function(callee).is_none()
        && env.lookup_class(callee).is_none()
        && let Some(resolved) = crate::instance_order::call_ty(env, callee, arg_tys)
    {
        return resolved;
    }
    if env.lookup_function(callee).is_none()
        && env.lookup_class(callee).is_none()
        && env.lookup_generic(callee).is_none()
        && let Some(resolved) = crate::builtin_call::resolve_builtin_call(callee, arg_tys)
    {
        return resolved;
    }
    if callee == "print" {
        // print's own signature isn't user-declarable in v0.1.
        // #977 (D-237): both arg-inference branches above funnel
        // into `arg_tys`, so this is the single place the
        // string-conversion gate sees every argument -- see the
        // `FString` arm and `string_conversion.rs`. The expression
        // form re-infers each argument (already inferred into
        // `arg_tys` above) so it can look under an erased `cast`.
        for arg in args {
            reject_unrenderable_expr(env, local_names, arg, StringConversionSite::PrintArgument)?;
        }
        return Ok(Ty::None);
    }
    if callee == "len" {
        // D-105 point 3: `len(lst)` is a hand-recognized builtin
        // call, same as `print` above -- not a user-declarable
        // signature. Generic over any scalar element type (`T0034`
        // already gates non-`int` lists further upstream, at the
        // point a list literal is constructed), reusing T0033 for
        // both failure shapes (wrong arity, non-list argument) --
        // the same "value does not support list operations" shape
        // already established for `ForList`/`Subscript`/`ListAppend`.
        // PR-11 Task 3 (D-123): also accepts `Ty::Dict` (gated the
        // same way by `T0036`), so `len(d)` type-checks too. PR-11
        // Task 7 (D-123): also accepts `Ty::Set` (gated the same way
        // by `T0038`), so `len(s)` type-checks too.
        if arg_tys.len() != 1 {
            return Err(Diagnostic::error(
                "T0033",
                format!("`len` expects exactly 1 argument, got {}", arg_tys.len()),
                Span::new(0, 0),
            )
            .with_help("pass exactly 1 argument"));
        }
        // Part 3 of #1026 (PR 3a of #1082): `Ty::Object` joins them.
        // A CPython object's length is whatever `PyObject_Size`
        // answers at run time, so the refusal moves from compile
        // time to the host -- `len(o)` on an operand with no
        // `__len__` raises `TypeError` there. The diagnostic text
        // deliberately stays as it is: `object` is not spellable in
        // an annotation (only a foreign-imported class resolves to
        // it, Part 1 of #1367), so naming it in the message a user
        // sees for `len(5)` would point at a type they cannot write.
        // A tuple bound to a name: its length is its type's arity
        // (`pycc_mir` folds the call to the constant; reading a name has
        // no effect to keep).
        let named_tuple =
            matches!(arg_tys[0], Ty::Tuple(_)) && matches!(args.first(), Some(HirExpr::Name(_)));
        if !named_tuple
            && !matches!(
                arg_tys[0],
                Ty::List(_) | Ty::Dict(_) | Ty::Set(_) | Ty::FrozenSet(_) | Ty::Object
            )
        {
            return Err(Diagnostic::error(
                "T0033",
                format!(
                    "`len` expects a `list[T]`, `dict[K, V]`, `set[T]`, or `frozenset[T]` argument, got `{}`",
                    arg_tys[0].name()
                ),
                Span::new(0, 0),
            ).with_help("pass a `list[T]`, `dict[K, V]`, `set[T]`, or `frozenset[T]` value"));
        }
        return Ok(Ty::Int);
    }
    if let Some(symbol) = std_qualified_symbol(callee) {
        // See `shadowed_std_receiver`'s own doc comment -- a real
        // local/parameter named `math` (or an alias of it, #962)
        // shadows the stdlib module.
        if let Some(shadowed) =
            shadowed_std_receiver(symbol.module, &env.std_module_aliases, |receiver| {
                is_std_receiver_bound(env, local_names, receiver)
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
        if arg_tys.len() != expected_arg_tys.len() {
            return Err(Diagnostic::error(
                "T0021",
                format!(
                    "`{callee}` expects {} argument(s), got {}",
                    expected_arg_tys.len(),
                    arg_tys.len()
                ),
                Span::new(0, 0),
            )
            .with_help(format!(
                "pass exactly {} argument(s)",
                expected_arg_tys.len()
            )));
        }
        for (arg_ty, expected) in arg_tys.iter().zip(expected_arg_tys) {
            if *arg_ty != std_scalar_to_ty(*expected) {
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
        return Ok(std_scalar_to_ty(ret_ty));
    }
    if callee == "round"
        && env.lookup_function(callee).is_none()
        && env.lookup_class(callee).is_none()
    {
        return crate::round::round_call_ty(&arg_tys);
    }
    if callee == "float" && env.lookup_function(callee).is_none() {
        // D-086's own remedy for the int-to-float boundary: a hand-recognized
        // builtin, same as `len` above, not a user-declarable signature --
        // except, unlike `print`/`len`, a program can predate this builtin's
        // introduction with its own `def float(...)`. Reviewer finding
        // (post-merge review): unlike `print`/`len`, which have been
        // hand-recognized since before this compiler could compile
        // user-declared functions at all, `float` was undefined until
        // this issue, so a user-defined `float` was a valid, working
        // program on `main` immediately before this change landed --
        // silently reinterpreting it as this builtin would be a real
        // regression, not an inherited, already-accepted precedent.
        // `env.lookup_function` takes priority; only fall through to the
        // builtin when no such definition exists.
        // Always returns `Ty::Float` regardless of the argument's own type,
        // once that argument is numeric-like -- unlike `len`, there is no
        // homogeneity/element-type question to defer.
        if arg_tys.len() != 1 {
            return Err(Diagnostic::error(
                "T0021",
                format!("`float` expects exactly 1 argument, got {}", arg_tys.len()),
                Span::new(0, 0),
            )
            .with_help("pass exactly 1 argument"));
        }
        // Part 4 of #1026 (PR 4a of #1083) admits `Ty::Object`: an
        // explicit `float(o)` runs CPython's own `PyNumber_Float`
        // protocol through the shim. That is not the implicit
        // thunk-seam crossing D-244 rule 7 closes -- the author named
        // the destination type -- see `docs/TYPE_SYSTEM.md`'s `object`
        // row.
        //
        // The message below deliberately does *not* enumerate
        // `object`: outside an `--ext` module it is unspellable in
        // an annotation (`docs/TYPE_SYSTEM.md`'s `object` row) and
        // exists only because a foreign `import` bound it -- or,
        // since Part 1 of #1367, a class such an import binds
        // annotated it -- so in a native build "pass an `object`" is
        // advice nobody can act on by writing a type. (Under D-258 an
        // ext module can spell it as `Any`/`object`; the message is
        // shared by both modes and keeps the native wording.) `tests::float_of_a_str_is_rejected_as_t0021` pins the
        // exact text, so the omission reads as deliberate rather than
        // as an oversight.
        //
        // Reviewer finding (codex, PR 4a of #1083): the `Ty::Object`
        // admission must not preempt a user-defined `class float`.
        // MIR resolves a call whose callee names a class as an
        // instantiation, so without this guard a module defining
        // `class float` and calling `float(o)` passes `pycc check`
        // with `Ty::Float` and then panics in codegen. The guard is
        // deliberately confined to the `Ty::Object` arm this change
        // introduces: `class float` with an `int` argument already
        // diverges the same way on `main` (`pycc check` accepts,
        // `pycc build` panics) and is tracked separately, so widening
        // the guard to the whole arm would change behavior this change
        // does not own.
        let object_admitted =
            matches!(arg_tys[0], Ty::Object) && env.lookup_class(callee).is_none();
        if !(matches!(arg_tys[0], Ty::Int | Ty::Float | Ty::Bool) || object_admitted) {
            return Err(Diagnostic::error(
                "T0021",
                format!(
                    "`float` expects an `int`, `float`, or `bool` argument, got `{}`",
                    arg_tys[0].name()
                ),
                Span::new(0, 0),
            )
            .with_help("pass an `int`, `float`, or `bool` value"));
        }
        return Ok(Ty::Float);
    }
    if callee == "bool"
        && env.lookup_function(callee).is_none()
        && env.lookup_class(callee).is_none()
    {
        // Part 4 of #1026 (PR 4a of #1083): `bool` is admitted for a
        // `Ty::Object` argument *only*. Every other argument type falls
        // through to `unsupported_callable_builtin` below and keeps its
        // C0001 verbatim, so `bool(1)` is refused exactly as it was --
        // the residual incoherence (`bool(o)` compiles, `bool(1)` does
        // not) is stated openly in `docs/TYPE_SYSTEM.md` and tracked by
        // #1017/#1018, which own the general builtin-conversion story.
        //
        // The user-defined-function guard is `float`'s, for `float`'s
        // reason: a `def bool(...)` is a valid, working program on
        // `main` today, so `env.lookup_function` takes priority. The
        // class guard is codex's PR 4a finding, also `float`'s: MIR
        // resolves a call naming a class as an instantiation, so a
        // module defining `class bool` would otherwise pass
        // `pycc check` here and panic in codegen. It sits on the whole
        // arm rather than on the `Ty::Object` test alone because this
        // arm admits nothing else -- every other argument already falls
        // through to `C0001` -- so the two placements are equivalent
        // and the outer one reads plainly.
        if arg_tys.len() == 1 && matches!(arg_tys[0], Ty::Object) {
            return Ok(Ty::Bool);
        }
    }
    if (callee == "int" || callee == "str")
        && env.lookup_function(callee).is_none()
        && env.lookup_class(callee).is_none()
    {
        // Part 4 of #1026 (PR 4b of #1083): `int` and `str` are
        // admitted for a `Ty::Object` argument *only*, exactly as
        // `bool` above is. Every other argument type falls through to
        // `unsupported_callable_builtin` and keeps its `C0001`
        // verbatim, so `int(1)` and `str(1)` are refused precisely as
        // they were -- the residual asymmetry (`str(o)` compiles,
        // `str(1)` does not) is stated openly in `docs/TYPE_SYSTEM.md`
        // and left to #1017/#1018, which own the general
        // builtin-conversion story.
        //
        // Both guards are `bool`'s, for `bool`'s reasons: a
        // `def int(...)`/`def str(...)` is a valid working program on
        // `main` today and must keep winning, and MIR resolves a call
        // naming a class as an instantiation, so a module defining
        // `class int` would otherwise type-check here and panic in
        // codegen.
        //
        // **Placement of the class guard.** It sits on the whole arm,
        // like `bool`'s and unlike `float`'s. `float`'s is confined to
        // its `Ty::Object` admission because that arm already admitted
        // `int`/`float`/`bool` on `main`, and widening the guard would
        // have changed behavior PR 4a did not own. These two arms are
        // new and admit nothing else, so every other argument already
        // falls through to `C0001` and the two placements are
        // equivalent -- the outer one simply reads plainly. (The
        // general precedence divergence between a class-shadowed
        // builtin in `pycc check` and in `pycc build` is #1107's, not
        // this change's.)
        if arg_tys.len() == 1 && matches!(arg_tys[0], Ty::Object) {
            return Ok(if callee == "int" { Ty::Int } else { Ty::Str });
        }
        // `str(xs)` of a list: the runtime's `repr` rendering.
        if callee == "str" && arg_tys.len() == 1 && matches!(arg_tys[0], Ty::List(_)) {
            crate::list_ops::check_list_renderable(&arg_tys[0])?;
            return Ok(Ty::Str);
        }
    }
    // `abs`/`divmod`/`int` over native numbers (`crate::int_builtins`),
    // under `int`'s guards: a user function or class of the name wins.
    if crate::int_builtins::NAMES.contains(&callee.as_str())
        && env.lookup_function(callee).is_none()
        && env.lookup_class(callee).is_none()
        && let Some(result) = crate::int_builtins::result_ty(callee, arg_tys)
    {
        return result;
    }
    // D-154 (Part 1 of #375): `ClassName(args)` (instantiation)
    // reuses this same generic `HirExpr::Call` node -- there is no
    // dedicated HIR shape for it (`pycc_hir::class`'s own doc
    // comment) -- so it is resolved here, checked before the
    // ordinary ("ClassName" as a plain function) lookup below, the
    // same precedence a generic-function call already gets just
    // above. `pycc_hir::lower_all` (`lower_checked` wraps it) enforces
    // that a class name can never collide with a top-level function,
    // type-alias, or import name in this compiler's flat,
    // single-namespace model -- both directions are rejected with
    // `C0001` at HIR-lowering time, before `env` is ever built (D-068
    // review finding on #385; `crates/pycc_hir/src/module.rs`'s
    // `lower_top_level_item`) -- so trying the class table first here
    // is unambiguous, not merely an assumption.
    if env.lookup_class(callee).is_some() {
        return class::resolve_instantiation(env, callee, args, arg_tys);
    }
    // D-133/D-134: a call to a PEP 695 generic function is resolved
    // through call-site substitution, not through the ordinary
    // `functions` signature -- `env.lookup_function(callee)` would
    // otherwise see a signature still carrying `Ty::Param` and
    // reject every real (concrete) argument as an assignability
    // mismatch. Checked before the ordinary lookup below so this
    // takes precedence for every generic function, including one
    // reached recursively while inferring a nested call's own
    // arguments (e.g. `print(identity(1))`).
    // #1316: a type parameter admits any argument, so a CPython
    // object passed to a generic is refused before substitution
    // (an ordinary user function refuses it in its argument loop
    // below; a constructor or method parameter is always declared
    // or refused as uninferable, so `class::check_call_args`'s own
    // mismatch already refuses an `object` there).
    if let Some(generic_func) = env.lookup_generic(callee) {
        crate::foreign::reject_object_arguments(arg_tys)?;
        return Ok(instantiate_generic_call(generic_func, arg_tys)?.return_ty);
    }
    // Part 2b of #1142 (#1164): a buffer-returning function is
    // callable from the CPython host through its generated wrapper
    // and from nowhere else. `crate::buffer`'s own diagnostic
    // carries the reason; refusing here is what keeps
    // `crates/pycc_codegen/src/call_result.rs`'s `Ty::MemoryView`
    // panic unreachable from source.
    if let Some((_, Ty::MemoryView)) = env.lookup_function(callee) {
        return Err(crate::buffer::buffer_returning_call_unsupported(callee));
    }
    let Some((param_tys, return_ty)) = env.lookup_function(callee) else {
        // Issue #142: before falling back to T0021 ("call to undefined
        // function"), check whether `callee` is a known Python 3.14
        // callable builtin that this compiler version does not implement
        // (e.g. `ValueError`, `Exception`, `int`, `range`). Such a call
        // is valid Python -- the builtin genuinely exists -- so it is a
        // capability gap (`C0001`), not a name-resolution failure
        // (`T0021`). This check is deliberately *after* the user-defined
        // function lookup, the `print`/`len`/`float` special cases, the
        // stdlib-qualified symbol lookup, the class-instantiation lookup,
        // and the generic-function lookup, so a user `def
        // ValueError(...)` always takes priority over this classification.
        // Part 2a of #1142 (#1165): the buffer producer. Placed
        // here, and not early like `len`'s own interception, so
        // D-244's #1129 statement (h) falls out of the ordering
        // rather than needing a second precedence rule: the class
        // table, the generic table and the user-function table have
        // all been consulted above, so a program's own `class
        // ndarray` or `def NDArray` keeps its meaning and never
        // reaches this line.
        //
        // A refusal rather than a type: `ndarray(n)` is admitted
        // only as an assignment's whole right-hand side, which
        // `crate::buffer_producer_assignment` handles before the
        // value ever reaches this walk. Every other position --
        // including the bare `HirStmt::ExprStmt` whose arm discards
        // the inferred type, and which would otherwise leak one
        // allocation per call -- arrives here.
        //
        // The alias gate is statement (h)'s fifth arm, in the
        // position `is_local` holds further above: `import math as
        // ndarray` binds the spelling to the `math` module, and a
        // module alias lives in no table the lookups above consult,
        // so without it this line refused the program's *own* call
        // with the producer's position message. Declining leaves the
        // `T0021` below, which is what an aliased spelling that is
        // not a producer (`import math as m`, then `m(4)`) already
        // reports. `crate::buffer::producer_assignment_ty` owns the
        // full reason; `docs/TYPE_SYSTEM.md`'s `memoryview` row is
        // the canonical enumeration.
        if crate::buffer::is_producer_spelling(callee)
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
        // Part 1 of #1319: the `frozenset(...)` builtin, placed here
        // for the same statement-(h) reason as the buffer producer
        // above -- the class, generic and user-function tables have
        // all been consulted, so a program's own `class frozenset`
        // or `def frozenset` keeps its meaning.
        if callee == crate::frozenset::FROZENSET {
            return crate::frozenset::check_call(arg_tys);
        }
        // #1331: `hash(...)`, placed here for `frozenset`'s reason
        // directly above. It also yields to a stdlib module alias
        // spelled `hash` (`import math as hash`), which lives in no
        // table consulted above; that call keeps the known-builtin
        // `C0001` below instead of being typed as the builtin.
        if callee == crate::hash::HASH
            && !env
                .std_module_aliases
                .iter()
                .any(|(alias, _)| alias == callee)
        {
            return crate::hash::check_call(arg_tys, env);
        }
        // Part 11 of #1371: `type(o)` on a CPython object, placed
        // here for `frozenset`'s reason above (`foreign::type_call`).
        // A bare narrowed `object` read is the object itself
        // (#1476), as `pycc_mir`'s split reads it.
        let narrowed_object = matches!(&args[..], [arg]
            if crate::narrow::is_bare_narrowed_object_read(env, arg));
        if crate::foreign::type_call::is_object_type_call(
            &env.std_module_aliases,
            callee,
            if narrowed_object {
                &[Ty::Object]
            } else {
                arg_tys
            },
        ) {
            return Ok(Ty::Object);
        }
        // Non-strict mode: an empty `set()`/`dict()` the empty-container
        // pass left unresolved is the CPython container (`pycc_mir`).
        if crate::non_strict::is_non_strict()
            && (pycc_hir::empty_object_constructor(callee, args.len()).is_some()
                || (callee == "set" && arg_tys == [Ty::Object])
                || pycc_hir::builtins::is_object_format(callee, arg_tys))
        {
            return Ok(Ty::Object);
        }
        if is_known_callable_builtin(callee) {
            return Err(unsupported_callable_builtin(callee));
        }
        return Err(Diagnostic::error(
            "T0021",
            format!("call to undefined function `{callee}`"),
            Span::new(0, 0),
        ));
    };
    // Issue #22: in top-level code, a call to a function whose
    // `def` has not been encountered yet in source order is a
    // static error -- CPython raises `NameError` at runtime for
    // the same case. Function bodies are exempt (all functions
    // are marked defined in `child_for_function`) because Python
    // evaluates a function body at call time, by which point all
    // module-level `def`s have typically executed.
    if !env.defined_functions.contains(callee.as_str()) {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "cannot call function `{callee}` before its definition \
                 (NameError in CPython: name '{callee}' is not defined)"
            ),
            Span::new(0, 0),
        ));
    }
    if arg_tys.len() != param_tys.len() {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "`{callee}` expects {} argument(s), got {}",
                param_tys.len(),
                arg_tys.len()
            ),
            Span::new(0, 0),
        )
        .with_help(format!("pass exactly {} argument(s)", param_tys.len())));
    }
    for (i, (arg_ty, param_ty)) in arg_tys.iter().zip(param_tys.iter()).enumerate() {
        // An `IntEnum` member passed for an `int` parameter is its value
        // (`pycc_hir::int_enum`; `pycc_mir` passes `member.value`).
        let read_int;
        let arg_ty = if *param_ty == Ty::Int {
            read_int = pycc_hir::int_enum::as_int(arg_ty);
            &read_int
        } else {
            arg_ty
        };
        if !class::is_assignable_env(env, arg_ty, param_ty)
            && !crate::object_box::admits_value(env, &args[i], arg_ty, param_ty)?
            && !crate::non_strict::admits_coercion(arg_ty, param_ty)
        {
            // #380 (PR-20): if the mismatch involves a protocol,
            // produce a detailed T0046 conformance error.
            let diag = if matches!(param_ty, Ty::Protocol(_)) || matches!(arg_ty, Ty::Protocol(_)) {
                class::assignable_error(env, arg_ty, param_ty)
            } else {
                Diagnostic::error(
                    "T0021",
                    format!(
                        "argument {} of `{callee}` expects `{}`, got `{}`",
                        i + 1,
                        param_ty.name(),
                        arg_ty.name()
                    ),
                    Span::new(0, 0),
                )
                .with_help(format!("pass a `{}` value", param_ty.name()))
            };
            return Err(crate::locate::place_at(
                diag,
                crate::locate::expr_span(&args[i]),
            ));
        }
        // Part 1 of #1333: an `object` argument that got this far
        // met a parameter it is assignable to -- an unannotated
        // private helper's solver-inferred `object` parameter -- and
        // is passed through as a borrowed pointer.
    }
    Ok(return_ty.clone())
}
