//! Assignability, assignment checks, range operands and walrus bindings.

use super::*;

/// Whether a value of type `found` is a dict key of type `key`: the same
/// type (keys compare exactly, so `bool` is no `int` key here), or a
/// fixed-arity tuple of `T`s for a `tuple[T, ...]` key -- the same tuple
/// object, hashed and compared by value.
pub(crate) fn is_key_of(found: &Ty, key: &Ty) -> bool {
    found == key
        || matches!((found, key), (Ty::Tuple(items), Ty::VarTuple(element)) if pycc_hir::tuple_fits_var_tuple(items, element))
        // An `int | float` key is any number (`d[1]`, `d[2.5]`), hashed and
        // compared by value as CPython's are.
        || (*key == Ty::Num && matches!(found, Ty::Int | Ty::Bool | Ty::Float))
}

pub(crate) fn is_assignable(from: Ty, to: Ty) -> bool {
    from == to
    // A fixed-arity tuple of `T`s is a `tuple[T, ...]` (`Ty::VarTuple`):
    // the same boxed tuple object at run time.
    || matches!((&from, &to), (Ty::Tuple(items), Ty::VarTuple(element)) if pycc_hir::tuple_fits_var_tuple(items, element))
    || (from == Ty::Bool && to == Ty::Int) // bool is a subtype of int, TYPE_SYSTEM.md's representation table
    // `int | float` holds an `int` (a `bool` included) or a `float` as it is.
    || (to == Ty::Num && matches!(from, Ty::Int | Ty::Bool | Ty::Float))
    // PEP 695 (#387): a `Ty::Param` in a generic class's method signature
    // is accepted as matching any concrete scalar type during type checking,
    // because `monomorphize` will substitute the type parameter with the
    // correct concrete type before MIR/codegen. The `GenericClassInstantiate`
    // expression already validates that the type argument is a scalar
    // (int/float/bool/str) at HIR-lowering time, so this is safe.
    // Both directions are needed: `to == Ty::Param` for `self.v = arg` where
    // the attribute slot is `Ty::Param`, and `from == Ty::Param` for
    // `return self.v` where the attribute read yields `Ty::Param` and the
    // function's return type is a concrete scalar. A non-generic function
    // (e.g. `Use.fetch`) that reads a generic class instance's attribute
    // (`b.v` where `b: Box[T]`) and returns it as `int` also relies on the
    // `from == Ty::Param` direction: during type checking `b.v` is still
    // `Ty::Param` (monomorphization has not yet substituted it).
    //
    // The `from == Ty::Param` direction is, however, unsafe for a *generic
    // function's own* type parameter: `def bad[T](x: T) -> int: return x`
    // would pass here (T assignable to int) then panic at codegen once T is
    // substituted with `str` at the call site. That case is rejected
    // separately in `check_generic_function_in` / `check_function_in` via
    // the threaded `own_type_param` context (see `check_stmt_in_function`'s
    // `Return` arm), not by narrowing this clause -- `is_assignable` has no
    // call-site context to distinguish a class-owned `Ty::Param` from a
    // function-owned one.
    || matches!(to, Ty::Param(_)) && matches!(from, Ty::Int | Ty::Float | Ty::Bool | Ty::Str)
    || matches!(from, Ty::Param(_)) && matches!(to, Ty::Int | Ty::Float | Ty::Bool | Ty::Str)
    // `T | None` (PEP 604, D-197, #763, Part 1 of #747): both a bare `inner`
    // value and a bare `None` widen to `Ty::Optional(inner)` -- matching
    // D-086's "no implicit widening OR narrowing" stance means this is the
    // *only* direction: `Ty::Optional(inner)` is deliberately NOT assignable
    // back to a bare `inner` here. No flow-sensitive narrowing exists yet
    // anywhere in this crate (`is None`/`is not None` only ever produces a
    // `Ty::Bool` presence result, see `expr.rs`'s `Compare` arm) -- an
    // `Optional[int]` stays `Optional[int]` on every path, including inside
    // an `if x is not None:` branch. Narrowing is tracked as Part 2 follow-up
    // work (issue #747), deliberately deferred out of this PR: the
    // `Optional[int]` representation and `is`/`is not` presence test are
    // independently useful and verifiable (see the conformance fixture at
    // `tests/fixtures/pep_0604_union.py`, which reads the presence result
    // directly rather than an unwrapped payload) without it.
    || matches!(&to, Ty::Optional(inner) if from == Ty::None || is_assignable(from.clone(), (**inner).clone()))
}

/// The canonical `T0025`: an annotated assignment whose initializer type is
/// not assignable to the declared annotation.
///
/// One function rather than one construction per arm because three call
/// sites now raise it -- `check_stmt`'s and `check_stmt_in_function`'s
/// `AnnAssign` arms, and `constraints::reject_producer_annotation_mismatch`,
/// the solver's mirror of the second (Part 2a of #1142, issue #1165).
/// AGENTS.md's no-paraphrase rule makes that decisive: the solver's answer
/// beats the check phase's under `crate::module::merge_solver_first`, so a
/// message that drifted from this one would be the message the user sees.
pub(crate) fn annotation_initializer_mismatch(
    target: &str,
    inferred: &Ty,
    annotation: &Ty,
) -> Diagnostic {
    Diagnostic::error(
        "T0025",
        format!(
            "cannot assign `{}` to `{target}: {}`, initializer does not match the declared annotation",
            inferred.name(),
            annotation.name()
        ),
        Span::new(0, 0),
    ).with_help(foreign::object_into_scalar_help(inferred, annotation).unwrap_or_else(|| format!("change the value to `{}` (the expected/declared type), or the declaration/annotation to `{}` (the actual type)", annotation.name(), inferred.name())))
}

/// The canonical PEP 591 `T0045`: a second assignment to a `Final` name.
///
/// Shared with `constraints::reject_final_rebinding`, the solver's mirror of
/// the [`check_assignment`] refusal above, for the reason
/// [`annotation_initializer_mismatch`] is shared: the solver's answer for a
/// function displaces the check phase's, so the two must not paraphrase each
/// other apart.
pub(crate) fn final_reassignment(target: &str) -> Diagnostic {
    Diagnostic::error(
        "T0045",
        format!("cannot reassign `Final` name `{target}`"),
        Span::new(0, 0),
    )
}

pub(crate) fn numeric_or_bool_compatible(a: Ty, b: Ty) -> bool {
    let is_numeric_like = |t: &Ty| matches!(t, Ty::Int | Ty::Float | Ty::Num | Ty::Bool);
    (is_numeric_like(&a) && is_numeric_like(&b)) || (a == Ty::Str && b == Ty::Str)
}

pub(crate) fn check_range_operand(
    env: &Environment,
    position: &str,
    expr: &HirExpr,
) -> Result<(), Diagnostic> {
    check_range_operand_in(env, &[], position, expr)
}

pub(crate) fn check_range_operand_in(
    env: &Environment,
    local_names: &[&str],
    position: &str,
    expr: &HirExpr,
) -> Result<(), Diagnostic> {
    // An `IntEnum` member is its `int` value (`pycc_hir::int_enum`).
    let actual = pycc_hir::int_enum::as_int(&infer_expr_in(env, local_names, expr)?);
    // Non-strict mode (#3): an object is read through `__index__`, as
    // `range()` reads it (`pycc_mir::MirExpr::ObjectIndex`).
    if is_assignable(actual.clone(), Ty::Int) || crate::non_strict::admits_index(&actual) {
        Ok(())
    } else {
        let diagnostic = Diagnostic::error(
            "T0021",
            format!("range {position} expects `int`, got `{}`", actual.name()),
            Span::new(0, 0),
        )
        .with_help("pass an `int` value");
        Err(crate::locate::place_expr(diagnostic, expr))
    }
}

pub(crate) fn check_assignment(
    env: &mut Environment,
    target: &str,
    ty: Ty,
) -> Result<(), Diagnostic> {
    check_assignment_of(env, target, ty, None)
}

/// [`check_assignment`] for a plain `name = value` rebinding, the one
/// binding seam whose MIR lowering wraps the value in
/// `MirExpr::ObjectBox` (Part 2 of #1387, #1475): rebinding a name already
/// inferred as `object` to a native value boxes it, under
/// [`object_box::admits_value`] (which also refuses a class method's own
/// `cls`, `I0404`). Every other binder -- a `for` target, a walrus, a
/// `match` capture, a comprehension variable, an annotation on a name
/// already bound -- stores its value unboxed, so it keeps the strict
/// [`check_assignment`] and a native value there stays `T0023`.
pub(crate) fn check_assignment_boxing(
    env: &mut Environment,
    target: &str,
    ty: Ty,
    value: &HirExpr,
) -> Result<(), Diagnostic> {
    // #1476 (Part 3 of #1387): a local's first binding fixes its type, so a
    // bare read of an `object` name narrowed by an `isinstance` guard
    // declares the local `object`, as it did before the guard narrowed
    // anything. Otherwise `if isinstance(o, int): y = o` / `else: y = o`
    // would turn a program that compiles into a `T0023`. Every other use of
    // the narrowed name keeps its native type.
    let ty = if env.lookup_any(target).is_none() && narrow::is_bare_narrowed_object_read(env, value)
    {
        Ty::Object
    } else {
        ty
    };
    check_assignment_of(env, target, ty, Some(value))
}

pub(crate) fn check_assignment_of(
    env: &mut Environment,
    target: &str,
    ty: Ty,
    boxed_value: Option<&HirExpr>,
) -> Result<(), Diagnostic> {
    // #1325 / Part 1 of #1333: binding a CPython object to a name is
    // admitted in both scopes, so no refusal sits here. The rest of this
    // function applies unchanged: rebinding the name to another type is
    // `T0023`, an annotation it does not satisfy is `T0025`, and a one-armed
    // `if` leaves it possibly unbound.
    //
    // At module scope the name is an ordinary module global
    // (`pycc_codegen`'s `collect_module_bindings`). Since Part 1 of #1499
    // that global owns its reference: a borrowed value (`y = x`) is retained
    // on bind and a rebinding releases the old one (`pycc_codegen`'s
    // `object_slot.rs`). In a function body the name is a function-local
    // slot, which still never releases: a rebinding leaks the old value
    // (#1092) until Part 2 of #1499 (#1502), because a parameter or alias
    // there is stored with no incref (`docs/RUNTIME.md`).
    // Part 2a of #1142 (#1165): assigning to a name bound to a buffer
    // *parameter* is refused. See `buffer::buffer_parameter_rebinding` for
    // the two independent grounds; the one that matters most here is that
    // the flat, flow-insensitive `owned_buffers` set below is sound only
    // because no name can acquire *parameter* provenance mid-function.
    // Artifact-owned provenance can be lost mid-function, which the
    // solver's `ConstraintEnvironment::rebind_over_owned_buffer` handles
    // (#1166 round-11 review finding 2); this phase needs no mirror of it,
    // because every way a name stops denoting its owned buffer is a
    // reassignment this function has already refused by the time such a
    // stale marker could be read -- `T0023` here for an incompatible
    // rebinding, and at a join for a name the branches bind differently. Reassigning an
    // artifact-*owned* buffer stays admitted -- codegen frees the previous
    // allocation before the store (D-074) -- so the guard keys on
    // provenance, not on the type alone.
    if matches!(env.lookup_any(target), Some(Ty::MemoryView)) && !env.owned_buffers.contains(target)
    {
        return Err(buffer::buffer_parameter_rebinding(target));
    }
    // PEP 591 (#383): reject reassignment of a `Final` name. The `finals`
    // set is populated *after* the initial assignment's `check_assignment`
    // call returns (in `check_stmt`/`check_stmt_in_function`'s `AnnAssign`
    // arm), so the initial binding does not trigger this check — only a
    // subsequent `Assign` or valued `AnnAssign` to the same name does. The
    // `bindings.contains_key` guard distinguishes a reassignment (the name
    // already has a runtime value) from the value-less `Final` declaration
    // case (`x: Final[int]` then `x = 1`): a value-less declaration puts the
    // name in `declared`, not `bindings`, so the first real assignment is
    // the *initial* assignment and must be allowed.
    if env.finals.contains(target) && env.bindings.contains_key(target) {
        return Err(final_reassignment(target));
    }
    // Every value assignment re-shadows a same-named `def` (D-110),
    // including a compatible-type reassignment of a name that already has a
    // representation record -- that branch below returns without calling
    // `bind()`, so clearing only inside `bind()` left the def-rebound flag
    // permanently stuck for any name with a pre-`def` binding (PR #252's
    // round-5 review caught `helper = 1; def helper() -> int: ...;
    // helper = 2; helper()` resolving the function where CPython raises
    // `TypeError`). Cleared here unconditionally, matching the solver's own
    // unconditional clearing in its Assign arm. Function bodies operate on
    // `child_for_function` clones, so a body-local assignment clears only
    // that body's view, never the module-level fact.
    env.def_rebound.remove(target);
    // Issue #769 (Part 2 of #747): any assignment to `target` kills its
    // narrowing overlay entry from this point forward -- the overlay
    // records a fact about the value `target` held at narrowing time
    // (`x is not None`), which a reassignment (even to a value that
    // happens to be compatible with the narrowed type) invalidates.
    // Unconditional and target-only: this fires for every assignment path
    // (`Assign`, `AnnAssign`, a `for` loop's own target, ...) since they all
    // route through this single function, and never touches any other
    // name's overlay entry.
    env.narrowed.remove(target);
    // Issue #118 Part 1: use `lookup_any` (not `lookup`) so a maybe-bound name
    // being reassigned on the current path becomes definite, while the
    // first-assignment-wins representation (type) from the maybe-binding is
    // retained -- `lookup` would return `None` for a `Maybe` binding, wrongly
    // treating the reassignment as a fresh first binding.
    if let Some(previous) = env.lookup_any(target) {
        let boxed = match boxed_value {
            Some(value) => object_box::admits_value(env, value, &ty, &previous)?,
            None => false,
        };
        // Non-strict mode: an object rebinding a scalar parameter is
        // converted (`pycc_codegen`'s `object_coerce`).
        if non_strict::admits_rebinding_coercion(env, target, &ty, &previous) {
            return Ok(());
        }
        if !class::is_assignable_env(env, &ty, &previous) && !boxed {
            non_strict::request_widen(target, &previous, &ty, |t| {
                non_strict::widens_to_object(env, t)
            });
            // A native container rebound to a fresh container of another
            // type (`t = list(...)` then `t = {x}`): the name becomes an
            // `object` holding CPython copies, which no alias can observe
            // -- the widened bindings must all box (`admits_value`), or the
            // re-check refuses them as before.
            if let Some(value) = boxed_value
                && is_container_ty(&previous)
                && is_container_ty(&ty)
                && (object_box::display_boxable(value, &ty)
                    || object_box::fresh_container(env, value, &ty))
            {
                non_strict::request_widen(target, &previous, &ty, |_| true);
            }
            // #380 (PR-20): if the mismatch involves a protocol,
            // produce a detailed T0046 conformance error.
            let diag = if matches!(previous, Ty::Protocol(_)) || matches!(ty, Ty::Protocol(_)) {
                class::assignable_error(env, &ty, &previous)
            } else {
                Diagnostic::error(
                    "T0023",
                    format!(
                        "cannot assign `{}` to `{target}`, previously inferred as `{}`",
                        ty.name(),
                        previous.name()
                    ),
                    Span::new(0, 0),
                ).with_help(format!("change the value to `{}` (the expected/declared type), or the declaration/annotation to `{}` (the actual type)", previous.name(), ty.name()))
            };
            return Err(crate::locate::place_labeled(
                diag,
                boxed_value.and_then(crate::locate::expr_span),
                format!("expected `{}`, found `{}`", previous.name(), ty.name()),
            ));
        }
        // Issue #118 Part 1: a compatible reassignment on the current path
        // upgrades a `Maybe` binding to `Definitely` (the name is now
        // definitely assigned on this path). The first-assignment-wins
        // representation (type) is retained -- `bind` with the *existing*
        // type, not `ty`, would be wrong here because `bind` takes the
        // passed type; instead, directly insert `Definitely(previous)` to
        // keep the sticky representation while upgrading the binding state.
        if matches!(env.binding_state(target), Some(BindingState::Maybe(_))) {
            env.bind(target.to_string(), previous);
        }
        return Ok(());
    }
    // issue #245: a value-less `AnnAssign` (`x: int`) records a declared
    // type without binding one. The first real assignment/valued
    // redeclaration reaching this point is validated against that
    // declaration, and -- on success -- the *declared* type becomes the
    // sticky representation, not `ty` itself, exactly as an
    // `AnnAssign{value: Some}` already makes its own annotation (not the
    // initializer's inferred type) the sticky representation just above in
    // `check_stmt`/`check_stmt_in_function`. Worked examples: `x: int; x =
    // 1` binds `Int` (matches); `x: int; x: bool = True` binds `Int` (the
    // *earlier* declaration wins, `bool` is merely assignable to it); `x:
    // int; x = "hello"` rejects with `T0026`, distinct from `T0023`
    // (nothing was "previously inferred" here -- it was declared, never
    // assigned).
    if let Some(declared) = env.declared_ty(target) {
        if !is_assignable(ty.clone(), declared.clone()) {
            return Err(Diagnostic::error(
                "T0026",
                format!(
                    "cannot assign `{}` to `{target}`, previously declared as `{target}: {}`",
                    ty.name(),
                    declared.name()
                ),
                Span::new(0, 0),
            ).with_help(format!("change the value to `{}` (the expected/declared type), or the declaration/annotation to `{}` (the actual type)", declared.name(), ty.name())));
        }
        env.declared.remove(target);
        env.bind(target.to_string(), declared);
        return Ok(());
    }
    env.bind(target.to_string(), ty);
    Ok(())
}

/// PEP 572 (#774): walks `expr` for every `HirExpr::NamedExpr { name, value }`
/// node (an `if`/`while` test or a bare expression statement -- the only
/// placements `pycc_hir::stmt::lower_stmt`'s own placement restriction
/// allows a `NamedExpr` to survive lowering in) and binds each `name` into
/// `env` via [`check_assignment`], exactly as `HirStmt::Assign`'s own arm
/// does for an ordinary `target = value` assignment.
///
/// Walked in the expression's own left-to-right evaluation order, and each
/// binding is applied immediately (not batched) -- so a later sibling
/// sub-expression that reads an earlier walrus-bound name (e.g. `(a := 1) +
/// (b := a + 1)`) resolves correctly. `value`'s type is re-derived here via
/// `infer_expr_in` rather than threaded through from the walk that already
/// validated it (this statement's own `infer_expr`/`infer_expr_in` call,
/// made by the caller just before this one runs) -- `value` has no side
/// effects of its own beyond further nested walrus bindings, which this
/// same recursive walk also applies, so recomputing its type is
/// behavior-preserving, just not the cheapest possible implementation.
pub(crate) fn collect_named_expr_bindings(
    env: &mut Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<(), Diagnostic> {
    match expr {
        HirExpr::NamedExpr { name, value } => {
            collect_named_expr_bindings(env, local_names, value)?;
            let ty = infer_expr_in(env, local_names, value)?;
            check_assignment(env, name, ty)
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
        | HirExpr::Super => Ok(()),
        HirExpr::ListPop { list } => match list.attr_expr() {
            Some(receiver) => collect_named_expr_bindings(env, local_names, receiver),
            None => Ok(()),
        },
        // #1254 (D-250): no walrus can sit inside a comprehension (lowering
        // refuses it), so there is nothing to bind in the enclosing scope.
        HirExpr::Comprehension(_) => Ok(()),
        HirExpr::Nd(_) => {
            for sub in pycc_hir::ndarray::expr_children(expr) {
                collect_named_expr_bindings(env, local_names, sub)?;
            }
            Ok(())
        }
        HirExpr::Call { args, .. } => {
            for arg in args {
                collect_named_expr_bindings(env, local_names, arg)?;
            }
            Ok(())
        }
        HirExpr::CompareChain { first, links } => {
            for operand in pycc_hir::compare_chain_operands(first, links) {
                collect_named_expr_bindings(env, local_names, operand)?;
            }
            Ok(())
        }
        HirExpr::BinOp { left, right, .. }
        | HirExpr::AugBinOp { left, right, .. }
        | HirExpr::Compare { left, right, .. }
        | HirExpr::BoolOp { left, right, .. } => {
            collect_named_expr_bindings(env, local_names, left)?;
            collect_named_expr_bindings(env, local_names, right)
        }
        HirExpr::IfExp { test, body, orelse } => {
            for part in [test, body, orelse] {
                collect_named_expr_bindings(env, local_names, part)?;
            }
            Ok(())
        }
        HirExpr::UnaryOp { operand, .. } | HirExpr::Unpack { value: operand, .. } => {
            collect_named_expr_bindings(env, local_names, operand)
        }
        HirExpr::FString(parts) => {
            for part in parts {
                if let FStringPart::Interpolation(inner) = part {
                    collect_named_expr_bindings(env, local_names, inner)?;
                }
            }
            Ok(())
        }
        HirExpr::ListLiteral(es)
        | HirExpr::ObjectList(es)
        | HirExpr::SetLiteral(es)
        | HirExpr::TupleLiteral(es) => {
            for e in es {
                collect_named_expr_bindings(env, local_names, e)?;
            }
            Ok(())
        }
        HirExpr::Subscript { base, index } => {
            collect_named_expr_bindings(env, local_names, base)?;
            collect_named_expr_bindings(env, local_names, index)
        }
        HirExpr::Slice {
            base,
            start,
            stop,
            step,
        } => {
            collect_named_expr_bindings(env, local_names, base)?;
            for bound in [start, stop, step].into_iter().flatten() {
                collect_named_expr_bindings(env, local_names, bound)?;
            }
            Ok(())
        }
        HirExpr::ListAppend { list, value } => {
            if let Some(receiver) = list.attr_expr() {
                collect_named_expr_bindings(env, local_names, receiver)?;
            }
            collect_named_expr_bindings(env, local_names, value)
        }
        HirExpr::SetAdd { value, .. } => collect_named_expr_bindings(env, local_names, value),
        HirExpr::DictLiteral(pairs) => {
            for (k, v) in pairs {
                collect_named_expr_bindings(env, local_names, k)?;
                collect_named_expr_bindings(env, local_names, v)?;
            }
            Ok(())
        }
        HirExpr::DictGetOrDefault { dict, key, default } => {
            if let Some(receiver) = dict.attr_expr() {
                collect_named_expr_bindings(env, local_names, receiver)?;
            }
            collect_named_expr_bindings(env, local_names, key)?;
            collect_named_expr_bindings(env, local_names, default)
        }
        HirExpr::AttrGet { base, .. } => collect_named_expr_bindings(env, local_names, base),
        HirExpr::MethodCall { base, args, .. } => {
            collect_named_expr_bindings(env, local_names, base)?;
            for arg in args {
                collect_named_expr_bindings(env, local_names, arg)?;
            }
            Ok(())
        }
        HirExpr::ReceiverDispatchedCall { call, .. } => {
            collect_named_expr_bindings(env, local_names, call)
        }
        HirExpr::KeywordCall { call, keywords, .. } => {
            collect_named_expr_bindings(env, local_names, call)?;
            for (_, value) in keywords {
                collect_named_expr_bindings(env, local_names, value)?;
            }
            Ok(())
        }
        HirExpr::GenericClassInstantiate { args, .. } => {
            for arg in args {
                collect_named_expr_bindings(env, local_names, arg)?;
            }
            Ok(())
        }
        HirExpr::ExprCall { callee, args } => {
            collect_named_expr_bindings(env, local_names, callee)?;
            for arg in args {
                collect_named_expr_bindings(env, local_names, arg)?;
            }
            Ok(())
        }
        HirExpr::ReceiverClassCall { args } => {
            for arg in args {
                collect_named_expr_bindings(env, local_names, arg)?;
            }
            Ok(())
        }
    }
}

/// A native list, set, frozenset, dict or tuple.
pub(crate) fn is_container_ty(ty: &Ty) -> bool {
    matches!(
        ty,
        Ty::List(_) | Ty::Set(_) | Ty::FrozenSet(_) | Ty::Dict(_) | Ty::Tuple(_) | Ty::VarTuple(_)
    )
}
