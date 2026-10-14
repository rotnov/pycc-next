//! Inference of attribute reads, method calls and generic class instantiation.

use super::*;

// D-154 (Part 1 of #375): an instance attribute read/method call --
// see `class::resolve_attr_get`/`class::resolve_method_call` for the
// actual resolution (both shared with `check_stmt`'s own
// `HirStmt::AttrSet` arm, which needs the identical attribute-type
// lookup for its own assigned-value check).
pub(crate) fn infer_attr_get_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let HirExpr::AttrGet { base, attr } = expr else {
        unreachable!()
    };
    // #433: `super().attr` — resolve the attribute starting from
    // the next class in the current class's MRO, not from the
    // current class itself. The `self` instance retains its actual
    // (most-derived) type, so the slot index is still computed from
    // the full MRO's flat layout downstream.
    if matches!(base.as_ref(), HirExpr::Super) {
        return class::resolve_super_attr_get(env, attr);
    }
    if attr == "value"
        && let Some(ty) = crate::generator::stop_value_ty(env, base)
    {
        return ty;
    }
    // #436: `ClassName.attr` — accessing an attribute on a class
    // name (not an instance) is not supported. A static or class
    // method accessed without calling it (e.g. `C.create` instead
    // of `C.create()`) has no value representation in this
    // compiler's static-dispatch model. Reject with a clear error
    // rather than letting `infer_expr_in` on the class name
    // produce a confusing "name not defined" diagnostic.
    //
    // An active value binding of that same name wins over the class:
    // in `def f(B: D) -> int: return B.X`, `B` is the parameter, not
    // the class `B`, exactly as CPython resolves it. Guard with the
    // same binding/local pair the `Subscript` class-name path above
    // already uses, so a shadowed name falls through to the ordinary
    // instance path below.
    if let HirExpr::Name(class_name) = base.as_ref()
        && class_name_dispatch(env, local_names, class_name)?
        && let Some(class_def) = env.lookup_class(class_name)
    {
        // #379 (PR-19): `Color.RED` — accessing an enum member by
        // name on the enum class.
        if let Some(ty) = enum_member_attr_type(class_def, class_name, attr) {
            return Ok(ty);
        }
        // Part 1 of #1284: a winning `staticmethod(<foreign
        // callable>)` class attribute reads the foreign callable.
        if let Some(ty) =
            class::foreign_static::class_name_read(env, local_names, class_name, attr)?
        {
            return Ok(ty);
        }
        // #911 (Part 1 of #885): `W.MIN_WIDTH` -- reading a
        // class-level attribute through the class name itself. This
        // is the *only* class-name attribute read pycc supports;
        // everything else still falls through to the `T0044` below.
        //
        // #974: the lookup walks `class_name`'s MRO rather than its
        // own `class_attrs` alone, so an *inherited* class attribute
        // (`Derived.LIMIT` where a base declares `LIMIT`) resolves,
        // as it does in CPython. It deliberately does **not** reuse
        // `lookup_class_attr_through_mro`: that walk would also
        // resolve a name a derived class re-declares as a method,
        // static method, class method or property, which CPython
        // shadows -- see `lookup_class_attr_by_class_name`. It is
        // equally deliberately *not* `resolve_attr_get`'s #960
        // instance precedence: a class object has no instance
        // `__dict__`, so an instance slot does not shadow here.
        // `pycc_mir`'s fold runs the same walk over the same shared
        // predicate; the two must never drift.
        if let Some(ty) = class::lookup_class_attr_by_class_name(env, class_name, attr) {
            return Ok(ty);
        }
        return Err(Diagnostic::error(
            "T0044",
            format!(
                "class `{class_name}` has no attribute named `{attr}` -- \
                 accessing a class attribute or method without an instance is \
                 not supported (use `instance.{attr}` or `{class_name}.{attr}()` \
                 for a static/class method)"
            ),
            Span::new(0, 0),
        ));
    }
    let base_ty = infer_expr_in(env, local_names, base)?;
    // Part 1 of #1284: an instance read reaching a
    // `staticmethod(<foreign callable>)` class attribute. Must run
    // before the #911 literal-fold gate below: that gate's
    // `lookup_class_attr_through_mro` also matches a `ForeignStatic`
    // entry, so reaching it first would wrongly refuse a non-name
    // receiver (`FS().exists`, #1346) as a folded constant. Also runs
    // before `resolve_attr_get`'s property walk, matching
    // `pycc_mir`'s guard placement.
    if let Ty::Instance(class_name) = &base_ty
        && let Some(ty) = class::foreign_static::instance_read(env, local_names, class_name, attr)?
    {
        return Ok(ty);
    }
    // #911 (Part 1 of #885): a class-attribute read is folded to its
    // constant by `pycc_mir`, which *discards* the base expression.
    // That is only sound when evaluating the base has no observable
    // effect, so the base is restricted to a bare name (`w.LIMIT`,
    // `self.LIMIT`). `make_window().LIMIT` would otherwise drop the
    // call, so it is rejected here rather than silently mis-compiled.
    //
    // #960 left this gate deliberately coarse, exactly like
    // `check_attr_set`'s own class-attribute rejection: it fires when
    // a class attribute of that name exists anywhere in the MRO,
    // without asking whether a sibling base's instance slot shadows
    // it. For that one shape the read would no longer fold, so the
    // message's stated reason does not apply -- but the result is a
    // conservative rejection of a program CPython accepts, never a
    // mis-compile, and relaxing it belongs with the write-side
    // follow-up rather than with #960's read fix.
    if !matches!(base.as_ref(), HirExpr::Name(_))
        && let Ty::Instance(class_name) = &base_ty
        && class::lookup_class_attr_through_mro(env, class_name, attr).is_some()
    {
        return Err(Diagnostic::error(
            "T0044",
            format!(
                "the class-level attribute `{class_name}.{attr}` can only be read \
                 through a plain name (`obj.{attr}`, `self.{attr}`) or through the \
                 class itself (`{class_name}.{attr}`) -- it is folded to a constant \
                 at compile time, which would discard the base expression"
            ),
            Span::new(0, 0),
        ));
    }
    // Part 2 of #1026 (#1081): `numpy.pi`. A CPython object has no
    // `HirClassDef`, so `class::resolve_attr_get`'s
    // `let Ty::Instance(..) else` just below would report `T0043`
    // ("not a class instance") -- a message that describes pycc's
    // own representation rather than the user's program, and a
    // user-visible regression the moment the `Name` arm above stops
    // refusing the read. The load itself is what Part 2 implements,
    // so it is admitted here, opaquely: the result is another
    // `Ty::Object`, because pycc knows nothing about the attribute's
    // real type either.
    //
    // Placed after the `Super` (#433) and `ClassName.attr` (#436)
    // guards above so neither changes meaning: both key on the
    // *syntactic* base, which a foreign read never matches.
    if matches!(base_ty, Ty::Object) {
        return Ok(Ty::Object);
    }
    class::resolve_attr_get(env, &base_ty, attr)
}

pub(crate) fn infer_method_call_expr(
    env: &Environment,
    local_names: &[&str],
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let HirExpr::MethodCall { base, method, args } = expr else {
        unreachable!()
    };
    // #433: `super().method(args)` — resolve the method starting
    // from the next class in the current class's MRO, with `self`
    // (the most-derived instance) as the implicit first argument.
    if matches!(base.as_ref(), HirExpr::Super) {
        let arg_tys = args
            .iter()
            .map(|arg| infer_expr_in(env, local_names, arg))
            .collect::<Result<Vec<_>, _>>()?;
        return class::resolve_super_method_call(env, method, args, &arg_tys);
    }
    // Part 1 of #1284: `C.name(args)` reaching a
    // `staticmethod(<foreign callable>)` class attribute. Runs
    // before the static/class-method table walk, which is not
    // positional against class attributes (MRO case D3).
    if let HirExpr::Name(class_name) = base.as_ref()
        && class_name_dispatch(env, local_names, class_name)?
        && let Some(ty) =
            class::foreign_static::class_name_call(env, local_names, class_name, method, args)?
    {
        return Ok(ty);
    }
    // #436: `ClassName.static_method(args)` or
    // `ClassName.class_method(args)` — a method call on a class
    // name (not an instance). The base is `HirExpr::Name` referring
    // to a registered class. Check the static_methods and
    // class_methods tables before the regular instance-method
    // resolution (which requires a `Ty::Instance` base and would
    // reject a bare class name).
    // An active value binding of that name shadows the class
    // here exactly as it does for the class-name `AttrGet` arm
    // above: in `def f(B: D) -> int: return B.m()`, `B` is the
    // parameter, so the call must reach `D.m`, not `B`'s static or
    // class method. The guard runs before the method-table walk so
    // a shadowed name short-circuits straight to the ordinary
    // instance path below.
    if let Some(class_name) =
        receiver_dispatch::static_or_class_method_receiver(env, local_names, base, method)?
    {
        let arg_tys = args
            .iter()
            .map(|arg| infer_expr_in(env, local_names, arg))
            .collect::<Result<Vec<_>, _>>()?;
        return class::resolve_static_or_class_method_call(env, class_name, method, args, &arg_tys);
    }
    let base_ty = infer_expr_in(env, local_names, base)?;
    if let Ty::Generator(sig) = &base_ty {
        return crate::generator::method_call(env, local_names, sig, method, args);
    }
    let arg_tys = args
        .iter()
        .map(|arg| infer_expr_in(env, local_names, arg))
        .collect::<Result<Vec<_>, _>>()?;
    // Part 2 of #1026 (PR 2b of #1081): a method call on a foreign
    // CPython object. Placed after `base_ty`/`arg_tys` are computed
    // -- both still need their ordinary inference, and an argument
    // that is itself an unsupported operation must report its own
    // diagnostic rather than this one -- and before
    // `resolve_method_call`, whose `Ty::Object` base reports the
    // `T0043` ("not a class instance") that this branch replaces.
    //
    // Only the packable operands can be marshalled: the four
    // scalars, another `Ty::Object` (Part 2a of #1371) and an
    // instance of a regular user class (#1435), each with a
    // `pycc_ext_obj_pack_*` helper in the shim, and `None` (Part 8
    // of #1371), passed as CPython's own `Py_None`. Anything else
    // -- a container, an enum member -- has no boundary
    // representation yet and is refused here rather than reaching
    // codegen.
    if matches!(base_ty, Ty::Object) {
        crate::foreign::check_object_call_args(env, args, &arg_tys, "method")?;
        return Ok(Ty::Object);
    }
    // Part 1 of #1284: the instance form of the call above, placed
    // before the static/class-method tables for the same reason.
    if let Ty::Instance(ref class_name) = base_ty
        && let Some(ty) =
            class::foreign_static::instance_call(env, local_names, class_name, method, args)?
    {
        return Ok(ty);
    }
    // #436: static and class methods can also be called on an
    // instance. Check the static/class method tables before the
    // regular instance-method resolution.
    if let Ty::Instance(ref class_name) = base_ty
        && class::has_static_or_class_method(env, class_name, method)
    {
        return class::resolve_static_or_class_method_call(env, class_name, method, args, &arg_tys);
    }
    // Non-strict mode: a `str` method given a CPython object argument
    // (`" ".join(o)`) is CPython's own method, on the `str` boxed.
    if base_ty == Ty::Str && arg_tys.contains(&Ty::Object) && crate::non_strict::is_non_strict() {
        crate::foreign::check_object_call_args(env, args, &arg_tys, "method")?;
        return Ok(Ty::Object);
    }
    let arg_tys = pycc_hir::builtins::materialized_method_args(&base_ty, method, &arg_tys)
        .unwrap_or(arg_tys);
    if let Some(resolved) = pycc_hir::builtins::resolve_method(&base_ty, method, &arg_tys) {
        return resolved
            .map(|sig| sig.ret)
            .map_err(|message| Diagnostic::error("T0021", message, Span::new(0, 0)));
    }
    if let Ty::List(elem) = &base_ty {
        return crate::list_ops::list_method_ty(elem, method, &arg_tys);
    }
    class::resolve_method_call(env, &base_ty, method, args, &arg_tys)
}

pub(crate) fn infer_generic_class_instantiate_expr(
    env: &Environment,
    expr: &HirExpr,
) -> Result<Ty, Diagnostic> {
    let HirExpr::GenericClassInstantiate { class, .. } = expr else {
        unreachable!()
    };
    // Verify the class exists. Genericity (the class has a type
    // parameter) is checked later during monomorphization's rewrite
    // pass (`rewrite_generic_calls_in_expr`), which rejects a
    // non-generic class used with `C[int](args)` with T0042.
    if !env.classes.contains_key(class) {
        return Err(Diagnostic::error(
            "T0001",
            format!("class `{class}` is not defined"),
            Span::new(0, 0),
        ));
    }
    Ok(Ty::Instance(Box::new(class.to_string())))
}
