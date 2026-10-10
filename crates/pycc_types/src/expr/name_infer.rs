//! Inference of name reads, walrus expressions and f-strings.

use super::*;

pub(crate) fn infer_f_string_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let HirExpr::FString(parts) = expr else {
        unreachable!()
    };
    for part in parts {
        if let FStringPart::Interpolation(expr) = part {
            // #977 (D-237): the backend renders scalars, `@dataclass`
            // instances and caught builtin exceptions; every other
            // instance and every protocol-typed value is rejected
            // here with `C0001` before it can reach the codegen
            // `to_str` panic. See `string_conversion.rs`.
            reject_unrenderable_expr(
                env,
                local_names,
                expr,
                StringConversionSite::FStringInterpolation,
            )?;
        }
    }
    Ok(Ty::Str)
}

pub(crate) fn infer_name_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let HirExpr::Name(name) = expr else {
        unreachable!()
    };
    if let Some(symbol) = std_qualified_symbol(name) {
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
        return match symbol.kind {
            pycc_std::StdSymbolKind::Constant { ty } => Ok(std_scalar_to_ty(ty)),
            pycc_std::StdSymbolKind::Function { .. } => Err(std_function_used_as_a_value(name)),
            pycc_std::StdSymbolKind::EnumMarker => Err(enum_marker_is_not_a_value(name)),
            pycc_std::StdSymbolKind::AnnotationMarker => {
                Err(annotation_marker_is_not_a_value(name))
            }
            pycc_std::StdSymbolKind::CastMarker => Err(cast_marker_is_not_a_value(name)),
            pycc_std::StdSymbolKind::TypeCheckingMarker => {
                Err(type_checking_marker_is_not_a_value(name))
            }
            pycc_std::StdSymbolKind::TypingFormMarker => Err(typing_form_is_not_a_value(name)),
            pycc_std::StdSymbolKind::ProtocolMarker
            | pycc_std::StdSymbolKind::AbcMarker
            | pycc_std::StdSymbolKind::DecoratorMarker
            | pycc_std::StdSymbolKind::EnumAutoMarker => Err(marker_is_not_a_value(name)),
        };
    }
    // Issue #118 Part 1: three-way distinction -- definitely bound ->
    // ok, maybe bound -> T0041, unbound -> T0021 (local) or "not
    // defined" (global). The stdlib-qualified check above already
    // handled `math.sqrt`-style names.
    match env.binding_state(name) {
        // Issue #769 (Part 2 of #747): a *read* of a definitely-bound
        // name inside a narrowed region resolves to the narrowed
        // (Optional's inner) type instead of the name's real,
        // still-Optional binding -- consulted here only, never by
        // `check_assignment`'s target checking (see `env.narrowed`'s
        // own doc comment).
        Some(BindingState::Definitely(ty)) => {
            // Part 2 of #1026 (#1081) removed Part 1's
            // unconditional `reject_object_read` call from this
            // arm. A read of a foreign binding at *module* scope
            // now yields `Ty::Object` like any other read: this
            // function is context-free, so it cannot tell a
            // `numpy.pi` base apart from a `print(numpy)` operand,
            // and Part 2 must admit the first. Every *consumer*
            // refuses on its own instead -- `crate::foreign`'s
            // module doc carries the full rule.
            let ty = env.narrowed_ty(name).unwrap_or_else(|| ty.clone());
            // The read is admitted in both scopes, for any
            // definitely-bound `object` name -- a foreign import, a
            // module-level `x = <object>` global, a function local
            // or a solver-inferred `object` parameter (#1316, Part 1
            // of #1333). A read that runs before a module-level
            // binding at run time raises `NameError` from codegen's
            // unbound-global branch, since D-041 cannot prove call
            // order here, and a failed operation bridges CPython's
            // exception into pycc's pending state and branches to
            // the innermost exception target (`pycc_codegen`'s
            // `foreign_fail.rs`).
            reject_memoryview_read(name, &ty, env.owned_buffers.contains(name))?;
            Ok(ty)
        }
        Some(BindingState::Maybe(_)) => Err(possibly_unbound(name)),
        None => {
            if is_local(local_names, name) {
                Err(unbound_local(name))
            } else {
                Err(Diagnostic::error(
                    "T0021",
                    format!("name `{name}` is not defined"),
                    Span::new(0, 0), // real span threading through HIR is out of scope for this task -- see Task 15's follow-up note
                ))
            }
        }
    }
}

// PEP 572 (#774): `target := value`'s own type is simply `value`'s
// type -- mirroring an assignment statement's own RHS typing rule.
// The persistent binding of `name` into the *enclosing* scope's
// `Environment` cannot happen here: this function only ever holds a
// shared `&Environment` reference, never a mutable one. That
// mutation instead happens one level up, at the statement boundary
// that calls into this expression (`crate::collect_named_expr_bindings`,
// invoked from `check_stmt`/`check_stmt_in_function` before the
// statement's own body/orelse/next-statement checking, so a name
// bound by a walrus in an `if`/`while` test or a bare expression
// statement is visible to every statement that follows it in the
// same scope) -- this arm only ever reports the type, never binds.
// #774's own scope note: codegen's `MirExpr::NamedExpr` yields the
// stored value by re-reading the slot it was just stored into (the
// "assignment followed by a name-load" shape the issue calls for),
// exactly mirroring a plain `Name` read's own refcount contract. That
// contract is only worked out (`pycc_codegen::bigint_rc`'s
// `int_value_is_a_duplicate_reference`) for `Ty::Int`; a
// reference-counted `Ty::Str`/`Ty::List`/`Ty::Dict`/`Ty::Set`/
// `Ty::Tuple`/`Ty::Instance`/`Ty::Protocol` value would need the same
// "yielded value borrows the slot's reference" treatment
// `str_value_is_a_duplicate_reference` and its container/instance
// counterparts don't yet carry for this new node, and getting that
// wrong silently reproduces the exact D-154 Part 1 use-after-free
// class already fixed once for plain reads. Rather than widen every
// one of those classifiers for a PR whose own fixture never exercises
// them, T0050 restricts a walrus's value to the non-reference-counted
// scalar types (`int`, `float`, `bool`, `None`, and `Optional` of
// those) up front -- a `core` gap recorded in the conformance-breadth
// manifest, not a silent one, and revisited only alongside a real
// audit of every refcounted `MirExpr` classifier this crate over.
// (T0050, not T0048/T0049: those two codes are already registered
// for the unrelated PEP 604 general-union-annotation gap -- see
// `pycc_diag::explain`'s own registry -- so this diagnostic takes
// the next free code instead of colliding with them.)
pub(crate) fn infer_named_expr_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let HirExpr::NamedExpr { name: _, value } = expr else {
        unreachable!()
    };
    let ty = infer_expr_in(env, local_names, value)?;
    if !is_walrus_value_ty_supported(&ty) {
        return Err(Diagnostic::error(
            "T0050",
            format!(
                "a walrus assignment (`:=`) value of type `{}` is not yet supported — \
                 only `int`, `float`, `bool`, and `None` (including `Optional` of those) \
                 are currently allowed as a walrus value (#774)",
                ty.name()
            ),
            Span::new(0, 0),
        ));
    }
    Ok(ty)
}
