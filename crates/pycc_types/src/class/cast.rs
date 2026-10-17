//! `typing.cast` compatibility and `isinstance`/`issubclass` checks.

use super::*;

/// #767: Maps a validated `cast` target name to the `Ty` the call
/// expression has. The four builtin scalar names map to their scalar `Ty`;
/// every other name is a user-defined class name and maps to
/// `Ty::Instance`. Shared by `check_cast` (validation pass) and the
/// constraint solver's own `cast` mirror, so the two passes cannot drift.
/// The caller is responsible for having validated that a non-builtin name
/// really names a known class — this function only performs the mapping.
pub(crate) fn cast_target_ty(name: &str) -> Ty {
    match name {
        "int" => Ty::Int,
        "float" => Ty::Float,
        "bool" => Ty::Bool,
        "str" => Ty::Str,
        other => Ty::Instance(Box::new(other.to_string())),
    }
}

/// #767: Validates the environment-independent half of a `cast(T, value)`
/// call shape — exactly two arguments, the first of them a bare name — and
/// returns that target name. Shared by `check_cast` (validation pass) and
/// the constraint solver's own `cast` mirror, which has no class table but
/// can and should report these two malformed-shape diagnostics itself:
/// returning "no type term" for them instead would surface the solver's
/// generic "cannot infer return type of private helper" in place of the
/// accurate message whenever the call is in a return-type-inferred helper.
pub(crate) fn cast_target_name(args: &[HirExpr]) -> Result<&str, Diagnostic> {
    if args.len() != 2 {
        return Err(Diagnostic::error(
            "T0021",
            format!("`cast` expects exactly 2 arguments, got {}", args.len()),
            Span::new(0, 0),
        )
        .with_help("pass exactly 2 arguments: the target type and the value"));
    }
    let HirExpr::Name(target) = &args[0] else {
        return Err(Diagnostic::error(
            "C0001",
            "`cast`'s first argument must be a bare type name in this pycc version -- \
             subscripted generics and other type expressions are not supported yet",
            Span::new(0, 0),
        )
        .with_help(
            "pass `int`, `float`, `bool`, `str`, or the name of a class defined in this module",
        ));
    };
    Ok(target)
}

/// #767 (D-198): Whether erasing a `cast(T, value)` to `value` alone leaves
/// the emitted code's representation *and* attribute layout agreeing with
/// the static type the rest of the program is then checked against.
///
/// `Ok(())` for a cast to the value's own type. For two distinct classes,
/// `Ok(())` only when `to` is one of `from`'s ancestors in its MRO — an
/// up-cast — and no class from `from` up to (but excluding) `to` overrides a
/// method `to` also defines or inherits (see the method-dispatch paragraph
/// below). Rejected for every other pair, including a genuine down-cast
/// (`to` is a strict descendant of `from`) and every scalar-representation
/// change such as `bool` -> `int`.
///
/// The class case looks like a pure representation question at first —
/// every class instance is one heap-object pointer regardless of class
/// (`ty_to_basic_type` in `pycc_codegen`), so a pointer reinterpretation
/// alone would never crash. But representation is not the whole story here:
/// `pycc_mir` erases `cast(...)` to `value` alone with no wrapper node (see
/// `check_cast`'s doc comment), so nothing downstream ever learns the
/// checker-verified target type `to` — MIR keeps tracking the expression's
/// *runtime* class, `from`. A down-cast the checker accepted as `to` would
/// then have the type checker validate `.attr`/`.method()` access against
/// `to`'s (larger) attribute set while MIR/codegen still resolve the access
/// against `from`'s (smaller) instance-slot layout: a `pycc_mir` panic for
/// an unannotated binding or inline access (`class_def_of` can't find the
/// attribute in `from`'s MRO), or, if an `AnnAssign` re-anchors the MIR
/// type to `to`, an out-of-bounds `pycc_rt` instance-slot abort at runtime,
/// since the object was never actually allocated with `to`'s extra slots.
/// An up-cast or identity cast has the opposite shape and stays sound on
/// *attribute layout*: `to`'s attribute set is a subset of (or equal to)
/// `from`'s, so whatever slot layout MIR keeps tracking already has every
/// attribute the checker will let code reach through the cast result.
///
/// Attribute layout is not the only thing erasure puts at risk, though.
/// `pycc_mir` also resolves *method calls* statically from the same
/// MIR-tracked runtime type `from` (no vtable -- see the comment at
/// `crates/pycc_mir/src/expr.rs:608`), walking `from`'s own MRO to find the
/// implementation. An `AnnAssign` re-anchors that MIR-tracked type to the
/// declared annotation for any non-protocol annotation
/// (`crates/pycc_mir/src/stmt.rs`'s `bind_ty` computation), so
/// `b: Base = cast(Base, d)` makes every later `b.m()` dispatch through
/// `Base`'s MRO even though the allocated object is `d`'s real class. If some
/// class between `from` and `to` (inclusive of `from`, exclusive of `to`)
/// overrides a method that `to` itself already defines or inherits, that
/// static resolution silently returns `to`'s (or an intermediate ancestor's)
/// implementation instead of the override CPython's dynamic dispatch would
/// have called -- a wrong answer with no diagnostic and no crash, which is
/// strictly worse than the attribute-layout failures above. So an up-cast is
/// sound only when no class strictly more derived than `to` (down to and
/// including `from`) overrides a method reachable from `to`'s own MRO.
/// `__init__` is excluded from this check on both sides: it runs once at
/// construction, before any `cast` of the resulting object could apply, so
/// every subclass defining its own `__init__` -- the ordinary case, not an
/// exception -- would otherwise make nearly every up-cast of a real class
/// hierarchy unsound by this rule for a call that can never actually be
/// re-dispatched through the cast result.
///
/// There is no `Ty::Protocol` case. A protocol name is not reachable as a
/// `cast` target (`cast_target_ty` maps every non-builtin bare name to
/// `Ty::Instance`), and a protocol-typed parameter is monomorphized to its
/// concrete class before its function body is checked.
///
/// Deliberately not `is_assignable_env`: that is a *subtyping* test, and it
/// admits `bool` -> `int` (a representation change) while restricting
/// `Instance` -> `Instance` to protocol conformance rather than MRO
/// ancestry.
///
/// Returns `Ok(())` when `to` is sound, or a [`CastMismatch`] describing
/// which of the three distinct failure shapes applies -- `check_cast` uses
/// the variant to report a message specific to the actual cause instead of
/// one fused string that would be true (and therefore uninformative) for
/// any of the three.
pub(crate) fn cast_compatibility(
    env: &Environment,
    from: &Ty,
    to: &Ty,
) -> Result<(), CastMismatch> {
    if from == to {
        return Ok(());
    }
    let (Ty::Instance(from_name), Ty::Instance(to_name)) = (from, to) else {
        return Err(CastMismatch::Representation);
    };
    let Some(from_def) = env.lookup_class(from_name) else {
        return Err(CastMismatch::Representation);
    };
    let Some(to_pos) = from_def
        .mro
        .iter()
        .position(|m| m.as_str() == to_name.as_str())
    else {
        return Err(CastMismatch::Layout);
    };
    let Some(to_def) = env.lookup_class(to_name) else {
        return Err(CastMismatch::Layout);
    };
    let target_methods: crate::fx::HashSet<&str> = to_def
        .mro
        .iter()
        .filter_map(|name| env.lookup_class(name))
        .flat_map(|def| def.methods.iter().map(|(name, _)| name.as_str()))
        .filter(|name| *name != "__init__")
        .collect();
    for def in from_def.mro[..to_pos]
        .iter()
        .filter_map(|name| env.lookup_class(name))
    {
        if let Some((name, _)) = def
            .methods
            .iter()
            .filter(|(name, _)| name != "__init__")
            .find(|(name, _)| target_methods.contains(name.as_str()))
        {
            return Err(CastMismatch::OverriddenMethod(name.clone()));
        }
    }
    Ok(())
}

/// #767: Type-checks `cast(T, value)` — `typing.cast` is a runtime no-op in
/// CPython whose only effect is to declare `value`'s static type as `T`, so
/// the call's type is `T` and `pycc_mir` lowers the whole expression to
/// `value` alone (no call is emitted, and codegen never sees `cast`).
///
/// `T` must be a bare name: one of the four builtin scalar type names or a
/// class defined in this module. Subscripted generics (`cast(list[int], x)`)
/// and every other type expression are outside the current subset and are
/// rejected with `C0001`, the versioned capability code, rather than a
/// by-design rejection — the same classification `check_isinstance` uses for
/// in-scope-but-unimplemented call shapes.
///
/// `value` is inferred normally so its own errors are still reported.
///
/// Unlike CPython's `typing.cast`, a target whose *runtime representation*
/// differs from the value's own is rejected (D-198). `pycc_mir` elides the
/// whole `cast(...)` call to `value` alone (see the module doc above), so
/// nothing at MIR/codegen time ever applies a representation conversion. If
/// the two representations differed — `cast(str, 5)`, `cast(int, flag)` —
/// the checker would validate the rest of the program against `T` while the
/// emitted code still carried the value's real representation: a codegen
/// panic in a debug build (the `pycc_codegen` local-type-drift guard is a
/// `debug_assert_eq!`) or silently misinterpreted bits in a release one.
/// Per `docs/TYPE_SYSTEM.md`'s representation table each of `int` (`i64`),
/// `float` (`f64`), `bool` (`i8`), `str` (heap pointer) has its own
/// distinct representation, so `bool` -> `int` is a representation change
/// like any other despite `bool` being an `int` subtype in the static type
/// system.
///
/// What remains permitted is a cast to the value's own type, and an
/// up-cast between two class instances (`to` is `from` or one of `from`'s
/// MRO ancestors) that crosses no method-override boundary. A genuine
/// down-cast (`to` is a strict descendant of `from`) is rejected: unlike a
/// representation change, an unsound down-cast cannot be caught by any
/// runtime guard, because MIR erasure (see `cast_compatibility`'s doc
/// comment) never threads `to`'s verified attribute layout through to the
/// object `value` actually names, so `cast`'s single most common legitimate
/// use — narrowing after an `isinstance` check — is deferred to a version
/// that either keeps a runtime class tag or otherwise verifies layout
/// compatibility instead of relying on pure erasure. An up-cast that
/// overrides a method along the way is rejected for the same erasure
/// reason, on the *dispatch* axis rather than the layout axis (see
/// `cast_compatibility`'s doc comment). `cast` does not otherwise verify
/// the nominal relationship between the two — CPython's and mypy's `cast`
/// are deliberately unchecked assertions, and pycc preserves that for the
/// subset it does erase; the checks above are codegen-soundness limits, not
/// a re-introduction of static checking on the assertion itself.
///
/// This check runs only on the validation-pass route (`infer_expr_in`,
/// reached for an annotated function's body and for module-level/`AnnAssign`
/// statements). `constraints.rs`'s solver mirror — reached only for a
/// return-type-inferred private helper — has no resolved `Ty` for the value
/// at that point and does not perform it, mirroring `check_isinstance`'s own
/// documented asymmetry between the two passes.
///
/// Two guards `check_isinstance` carries are deliberately absent here.
/// There is no side-effect guard on the value operand: `isinstance` needs
/// one because it discards its operand and evaluates to a compile-time
/// constant, whereas `cast` *is* its value operand after lowering, so a
/// call expression there is evaluated exactly once, as CPython does. And
/// there is no `@runtime_checkable` gate on a protocol target: `isinstance`
/// needs one because it performs a runtime class test, whereas `cast`
/// performs none — `cast(P, x)` only renames the static type and emits no
/// code, which is exactly what `typing.cast` means for a protocol in
/// CPython too.
pub(crate) fn check_cast(
    env: &Environment,
    local_names: &[&str],
    args: &[HirExpr],
) -> Result<Ty, Diagnostic> {
    // A CPython object stays one (v0.8 accept 4): CPython's `cast` returns
    // its value unchanged, so `cast(str, token["raw"])` or
    // `cast(Callable[..., str], f)` is the object, whatever the target names
    // and however it is spelled; the target is not evaluated
    // (`docs/DEVIATIONS.md`).
    if args.len() == 2 && infer_expr_in(env, local_names, &args[1])? == Ty::Object {
        return Ok(Ty::Object);
    }
    let target = cast_target_name(args)?;
    validate_class_name(env, target)?;
    let target_ty = cast_target_ty(target);
    let value_ty = infer_expr_in(env, local_names, &args[1])?;
    if let Err(mismatch) = cast_compatibility(env, &value_ty, &target_ty) {
        let (message, help) = match mismatch {
            CastMismatch::Representation => (
                format!(
                    "`cast({target}, ...)` would change the value's runtime representation \
                     (`{}` to `{}`), which this pycc version does not support",
                    value_ty.name(),
                    target_ty.name()
                ),
                "pycc erases `cast` at compile time and emits no conversion, so the target \
                 type must already share the value's runtime representation: cast to the \
                 value's own type, or to one of its base classes",
            ),
            CastMismatch::Layout => (
                format!(
                    "`cast({target}, ...)` would narrow the value's attribute layout (`{}` \
                     to `{}`), which this pycc version does not support",
                    value_ty.name(),
                    target_ty.name()
                ),
                "pycc erases `cast` at compile time and never re-derives the target's \
                 attribute layout, so the target type must already be a subset of the \
                 value's own: cast to the value's own type, or to one of its base classes",
            ),
            CastMismatch::OverriddenMethod(method) => (
                format!(
                    "`cast({target}, ...)` would let a later call to `{method}` on the \
                     result statically resolve to `{}`'s implementation instead of `{}`'s \
                     override, which this pycc version does not support",
                    target_ty.name(),
                    value_ty.name()
                ),
                "pycc resolves method calls statically from the cast result's declared type, \
                 so casting across a class that overrides a base method is unsound: cast to \
                 the value's own type, or to a base class not separated from the value's class \
                 by any subclass that overrides one of that base class's methods",
            ),
        };
        return Err(Diagnostic::error("C0001", message, Span::new(0, 0)).with_help(help));
    }
    Ok(target_ty)
}

/// #435: Type-checks `isinstance(obj, class_arg)` — validates the argument
/// count, infers the object's type, extracts and validates the class
/// argument(s), and returns `Ok(Ty::Bool)`. The compile-time boolean result
/// is computed at MIR lowering time (not here — the type checker only
/// confirms the call is well-formed).
pub(crate) fn check_isinstance(
    env: &Environment,
    local_names: &[&str],
    args: &[HirExpr],
) -> Result<Ty, Diagnostic> {
    if args.len() != 2 {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "`isinstance` expects exactly 2 arguments, got {}",
                args.len()
            ),
            Span::new(0, 0),
        )
        .with_help("pass exactly 2 arguments: the object and the class"));
    }
    // #435 review fix (P1): `isinstance` is a compile-time predicate in
    // pycc's static-dispatch model — the result is a `BoolLiteral` constant
    // computed from the operand's declared type. A side-effecting operand
    // (a function call or class instantiation) would have its effects
    // silently discarded, changing standard Python semantics. Reject such
    // operands with `C0001` rather than silently dropping the call.
    if let HirExpr::Call { .. } = &args[0] {
        return Err(Diagnostic::error(
            "C0001",
            "`isinstance` is a compile-time predicate in pycc and cannot evaluate a \
             call expression as its first argument (side effects would be lost)",
            Span::new(0, 0),
        )
        .with_help(
            "assign the call result to a variable first, then pass the variable to \
             `isinstance`",
        ));
    }
    // Infer the object's type normally.
    let obj_ty = infer_expr_in(env, local_names, &args[0])?;
    // Part 1 of #1371: an object first argument is a run-time
    // `PyObject_IsInstance`, never the compile-time fold below, whose
    // catch-all would answer a constant `False` where CPython answers
    // `True` and drop the operand's side effect (see
    // `foreign::compare::check_object_isinstance`).
    // #1476: a narrowed `object` name is still tested at run time, as
    // `pycc_mir`'s `class::lower_isinstance` does, so a nested guard over a
    // narrowed name never folds over its narrowed type.
    if obj_ty == Ty::Object || crate::narrow::is_bare_narrowed_object_read(env, &args[0]) {
        return crate::foreign::compare::check_object_isinstance(env, local_names, &args[1]);
    }
    // Extract class names from the second argument (do NOT infer it as a
    // regular expression — class names are not value bindings).
    let class_names = extract_class_names(&args[1]).map_err(|_| {
        Diagnostic::error(
            "T0021",
            "`isinstance`'s second argument must be a class name or a tuple of class names",
            Span::new(0, 0),
        )
        .with_help("pass a class name (e.g. `int`) or a tuple of class names (e.g. `(int, str)`)")
    })?;
    // Validate each class name.
    for name in &class_names {
        validate_class_name(env, name)?;
        // #380 (PR-20, PEP 544): `isinstance` against a protocol class is
        // only valid if the protocol is `@runtime_checkable`. A
        // non-runtime-checkable protocol used with `isinstance` is
        // rejected with `C0001`.
        if let Some(class_def) = env.lookup_class(name)
            && class_def.is_protocol
            && !class_def.runtime_checkable
        {
            return Err(Diagnostic::error(
                "C0001",
                format!(
                    "`isinstance` against protocol `{name}` is not valid -- the protocol is \
                     not `@runtime_checkable`; add `@runtime_checkable` to the protocol \
                     class declaration to use it with `isinstance`"
                ),
                Span::new(0, 0),
            ));
        }
    }
    // The result is always `Ty::Bool`. The actual compile-time boolean value
    // is computed by `eval_isinstance_single` at MIR lowering time.
    // (We could compute it here too, but the type checker's job is just
    // validation — the MIR computes the constant.)
    // An object first argument returned above, before this fold.
    Ok(Ty::Bool)
}

/// #435: Type-checks `issubclass(cls_arg, class_arg)` — validates the
/// argument count, extracts and validates both class arguments, and returns
/// `Ok(Ty::Bool)`. Neither argument is inferred as a regular expression
/// (both are class references, not values).
pub(crate) fn check_issubclass(env: &Environment, args: &[HirExpr]) -> Result<Ty, Diagnostic> {
    if args.len() != 2 {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "`issubclass` expects exactly 2 arguments, got {}",
                args.len()
            ),
            Span::new(0, 0),
        )
        .with_help("pass exactly 2 arguments: the class and the target class"));
    }
    // The first argument must be a bare class name (not a tuple).
    let cls_name = match &args[0] {
        HirExpr::Name(name) => name.clone(),
        _ => {
            return Err(Diagnostic::error(
                "T0021",
                "`issubclass`'s first argument must be a class name",
                Span::new(0, 0),
            )
            .with_help("pass a bare class name (e.g. `int` or `MyClass`)"));
        }
    };
    // Extract class names from the second argument.
    let target_names = extract_class_names(&args[1]).map_err(|_| {
        Diagnostic::error(
            "T0021",
            "`issubclass`'s second argument must be a class name or a tuple of class names",
            Span::new(0, 0),
        )
        .with_help("pass a class name (e.g. `int`) or a tuple of class names (e.g. `(int, str)`)")
    })?;
    // Validate the first argument's class name.
    validate_class_name(env, &cls_name)?;
    // #380 (PR-20, PEP 544): `issubclass` against a protocol class is
    // rejected — protocols use structural typing, not nominal
    // inheritance, so `issubclass` does not apply.
    if let Some(class_def) = env.lookup_class(&cls_name)
        && class_def.is_protocol
    {
        return Err(Diagnostic::error(
            "C0001",
            format!(
                "`issubclass` with protocol `{cls_name}` as the first argument is not valid \
                 -- protocols use structural typing, not nominal inheritance; use \
                 `isinstance` with a `@runtime_checkable` protocol instead"
            ),
            Span::new(0, 0),
        ));
    }
    // Validate each target class name.
    for name in &target_names {
        validate_class_name(env, name)?;
        // #380 (PR-20): `issubclass` against a protocol as the target is
        // also rejected.
        if let Some(class_def) = env.lookup_class(name)
            && class_def.is_protocol
        {
            return Err(Diagnostic::error(
                "C0001",
                format!(
                    "`issubclass` against protocol `{name}` is not valid -- protocols use \
                     structural typing, not nominal inheritance; use `isinstance` with a \
                     `@runtime_checkable` protocol instead"
                ),
                Span::new(0, 0),
            ));
        }
    }
    Ok(Ty::Bool)
}

/// The three distinct ways `cast_compatibility` rejects a target, each
/// reported with its own message by `check_cast`: a scalar or unrelated-type
/// representation change, a genuine down-cast or unrelated-class layout
/// narrowing, and an up-cast that crosses a method-override boundary (see
/// `cast_compatibility`'s doc comment for why the third is unsound too).
#[derive(Debug, PartialEq)]
pub(crate) enum CastMismatch {
    Representation,
    Layout,
    OverriddenMethod(String),
}
