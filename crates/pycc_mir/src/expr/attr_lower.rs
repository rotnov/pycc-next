//! Lowering of attribute reads and method calls.

use super::*;

// D-154 (Part 1 of #375): `base.attr` -- resolved to a compile-time
// slot index against the base's class's `HirClassDef`, per the
// class-instance-layout ADR (never a runtime string-keyed lookup).
// #377: if `attr` is a `@property`, the access is rewritten to an
// ordinary `MirExpr::Call` to the getter's mangled name (with `base`
// as `self`), reusing the existing method-call/codegen infrastructure
// with no new MIR/codegen variant.
// #432: property and attribute resolution walk the MRO, so an
// attribute or property declared in a base class is found when
// accessed on a derived class instance. The slot index is computed
// from the MRO's flat attribute layout (`mro_attrs`).
/// v0.9: the attributes a compiled instance answers through its carrier in
/// non-strict mode: its class and its `__dict__` snapshot.
pub(crate) fn is_carrier_dunder(attr: &str) -> bool {
    pycc_hir::is_non_strict_program() && matches!(attr, "__class__" | "__dict__")
}

pub(crate) fn lower_attr_get_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::AttrGet { base, attr } = expr else {
        unreachable!()
    };
    // #433: `super().attr` — resolve the attribute starting from
    // the next class in the current class's MRO, using `self` (the
    // current function's first parameter) as the instance.
    // #587: only a `@property` resolves this way. A `super` object
    // proxies class-level attributes and descriptors, not the
    // instance's own attributes, so `super().<instance attr>` no
    // longer lowers to a slot read against `self` — the type
    // checker rejects it with `T0047` first.
    if matches!(base.as_ref(), HirExpr::Super) {
        let current = match current_class {
            Some(c) => c,
            None => panic!(
                "pycc_mir: internal error: `HirExpr::Super` reached lower_expr outside a \
                 method body -- pycc_hir::lower_expr should have rejected this with C0001"
            ),
        };
        let self_expr = self_expr(scopes);
        // #1337 (D-254): the MRO `super()` continues is the
        // *receiver's* -- a copy's `self` is typed as the subclass it
        // was compiled for -- after the body's own class, as in
        // CPython.
        let self_ty = self_expr.ty();
        let receiver = receiver_class(&self_ty).unwrap_or(current);
        let class_def = &classes[receiver];
        let current_pos = class_def
            .mro
            .iter()
            .position(|c| c == current)
            .expect("pycc_mir: internal error: class not found in its receiver's MRO");
        let super_mro = &class_def.mro[current_pos + 1..];
        // #915: one pass over the slice, checking every class-level
        // member kind on each class before moving to the next --
        // mirroring `pycc_types::class::resolve_super_attr_get`,
        // where the reasoning lives: a `super` object resolves
        // against one class `__dict__` at a time, so MRO position
        // decides, not member kind.
        //
        // A class attribute (#911) folds to its literal through the
        // same `fold_class_attr` helper the `Base.X`, `self.X` and
        // `d.X` sites use, so `super().X` produces MIR identical to
        // `Base.X` and `pycc_codegen` needs no change. The
        // `self_expr` receiver computed above is deliberately
        // discarded on that path: a class attribute has no runtime
        // storage, so the read needs no instance, and `super()` is
        // side-effect-free, so dropping it loses nothing.
        for mro_class in super_mro {
            let mro_def = &classes[mro_class.as_str()];
            if let Some(prop) = mro_def.properties.iter().find(|p| p.name == *attr) {
                let getter =
                    exact_callee(receiver, mro_class, prop.getter.clone(), scopes, classes);
                let ty = lookup(scopes, &format!("$fn:{getter}"));
                return MirExpr::Call {
                    callee: getter,
                    args: vec![self_expr],
                    ty,
                };
            }
            if let Some(folded) = fold_class_attr(mro_def, attr) {
                return folded;
            }
        }
        // #587: nothing else resolves through `super()`. A `super`
        // object proxies class-level attributes and descriptors
        // along the MRO, not the instance's own attributes, so an
        // `AttrGet` naming an instance attribute (or a name declared
        // nowhere in the MRO at all) is rejected by
        // `pycc_types::class::resolve_super_attr_get` with `T0047`
        // or `T0044` before any HIR reaches this crate. Properties
        // and class attributes are the only class-level members pycc
        // models today, so the two loops above are exhaustive for
        // well-typed input.
        panic!(
            "pycc_mir: internal error: `super().{attr}` is not a property or class \
             attribute on any class after `{current}` in its MRO -- pycc_types::check \
             should have rejected this HIR with T0047 or T0044 before it reached pycc_mir"
        );
    }
    // #379 (PR-19): `Color.RED` — accessing an enum member by name
    // on the enum class. The base is `HirExpr::Name` referring to
    // an enum class (a class with non-empty `enum_members`), and
    // `attr` is a member name. Lower to `MirExpr::Name` reading a
    // synthetic global (`<Class>.<Member>.enum_member`) that
    // codegen initializes once at module-init time with the
    // member's singleton instance. The `.enum_member` suffix
    // ensures the name cannot collide with a real Python
    // identifier (which cannot contain `.`). The subsequent
    // `.value`/`.name` read on this result is a separate
    // `AttrGet` that resolves to a slot via the enum class's
    // `attrs = [("value", Int), ("name", Str)]` table.
    // A value binding of the enum class's own name shadows the
    // class here exactly as it does for the class-attribute fold
    // below, so this interception is guarded the same way -- without
    // it `def f(Color: D) -> int: return Color.RED` would read the
    // enum member's singleton instead of the parameter's slot.
    if !matches!(base.as_ref(), HirExpr::Name(name)
        if scopes.iter().any(|scope| scope.contains_key(name)))
        && let Some(enum_member_expr) =
            try_lower_enum_member_attr(base.as_ref(), attr.as_str(), classes)
    {
        return enum_member_expr;
    }
    // #911 (Part 1 of #885): `W.MIN_WIDTH` -- a class-level attribute
    // read through the class name. Intercepted before `lower_expr`
    // for the same reason the enum-member read above is: the base is
    // a class name, not a value binding, so lowering it as an
    // expression would fail.
    //
    // #974: the walk runs over `class_def.mro`, most-derived first,
    // so an *inherited* class attribute (`Derived.LIMIT`) folds too.
    // It stops at the first class that declares `attr` in any other
    // class-level namespace, because CPython binds that declaration
    // instead -- `declares_name_outside_class_attrs` is the shared
    // predicate, and `pycc_types::class::lookup_class_attr_by_class_name`
    // runs the identical walk over it so the type the checker
    // reported and the constant folded here can never disagree (the
    // #960 failure mode). This is deliberately *not* the instance
    // arm's walk further below: that one consults instance slots
    // first, and a class object has no instance `__dict__`.
    // An active value binding of that name shadows the class, so
    // `B.X` reads the parameter's instance slot rather than the class
    // attribute -- `pycc_types`' own class-name `AttrGet` arm applies
    // the matching binding/local guard, and this scope check is the
    // MIR-side spelling of it (the same one the `__class_getitem__`
    // interception above already uses). Without it the shadowed name
    // would reach the internal-error panic below.
    if let HirExpr::Name(class_name) = base.as_ref()
        && !scopes.iter().any(|scope| scope.contains_key(class_name))
        && let Some(class_def) = classes.get(class_name.as_str())
    {
        // Part 1 of #1284: `C.name` where the winning class-level
        // binding is `name = staticmethod(<foreign callable>)` reads
        // the foreign callable itself.
        if let Some(target) = foreign_static::class_name_target(class_def, classes, attr) {
            return lower_expr(&target.read_expr(), scopes, classes, current_class);
        }
        for mro_class in &class_def.mro {
            let mro_def = mro_class_def(mro_class, classes);
            if let Some(folded) = fold_class_attr(mro_def, attr) {
                return folded;
            }
            if declares_name_outside_class_attrs(mro_def, attr) {
                break;
            }
        }
        // The base really is a class name -- `classes.get`
        // succeeded and the scope guard above proved no value binding
        // shadows it -- so there is nothing to fall back to and
        // `lower_expr` below would fail on it. A name that is *not* a
        // class (the ordinary `d.LIMIT` local or parameter), and a
        // class name shadowed by a binding, both fall through.
        panic!(
            "pycc_mir: internal error: attribute `{attr}` is not a class attribute \
             reachable through class `{class_name}`'s MRO -- pycc_types::check should \
             have rejected this HIR with T0044 before it reached pycc_mir"
        );
    }
    let base = lower_expr(base, scopes, classes, current_class);
    // Part 2 of #1026: `numpy.pi` -- the base is a CPython object,
    // so there is no `HirClassDef` to resolve `attr` against and no
    // slot index to compute. The attribute name survives lowering
    // as a `String` (D-105's String-keyed-node precedent) and the
    // load becomes a runtime `pycc_ext_obj_getattr` call.
    //
    // Placed immediately after `lower_expr` rather than beside the
    // enum-member and class-name interceptions above because a
    // foreign module's local name *is* a value binding in `scopes`
    // (`pycc_mir::build` binds it as `Ty::Object`), so both of those
    // interceptions already decline it on their own scope guards.
    // It must come before `class_def_of`, whose `Ty::Object` input
    // would reach `class.rs`'s internal-error panic.
    // v0.9: `self.__class__` / `self.__dict__` of a compiled instance, read
    // through its carrier (`pycc_types::class::resolve_attr_get`).
    if matches!(base.ty(), Ty::Instance(_)) && is_carrier_dunder(attr) {
        return MirExpr::ObjAttrGet {
            base: Box::new(MirExpr::ObjectBox(Box::new(base))),
            attr: attr.clone(),
            ty: Ty::Object,
        };
    }
    if base.ty() == Ty::Object {
        return MirExpr::ObjAttrGet {
            base: Box::new(base),
            attr: attr.clone(),
            ty: Ty::Object,
        };
    }
    let class_def = class_def_of(&base, classes);
    // Part 1 of #1284: `x.name` reaching a
    // `staticmethod(<foreign callable>)` class attribute reads the
    // foreign callable. The receiver is not part of the read, but a
    // non-name receiver is still evaluated first (#1346).
    if let Some(target) = foreign_static::instance_target(class_def, classes, attr) {
        let value = lower_expr(&target.read_expr(), scopes, classes, current_class);
        return sequence_after(base, value);
    }
    // #432: walk the MRO for property lookup first (matching
    // CPython's descriptor protocol precedence), then for regular
    // attribute slots using the flat MRO layout.
    for mro_class in &class_def.mro {
        let mro_def = mro_class_def(mro_class, classes);
        if let Some(prop) = mro_def.properties.iter().find(|p| p.name == *attr) {
            let getter = exact_callee(
                &class_def.name,
                mro_class,
                prop.getter.clone(),
                scopes,
                classes,
            );
            let ty = lookup(scopes, &format!("$fn:{getter}"));
            // v0.9: a subclass overriding the property (`crate::dispatch`).
            let getter = crate::dispatch::route_member(
                class_def,
                attr,
                getter,
                scopes,
                classes,
                crate::dispatch::getter_of,
            );
            return MirExpr::Call {
                callee: getter,
                args: vec![base],
                ty,
            };
        }
    }
    // #960: the instance-slot lookup runs *before* the
    // class-attribute fold below, matching
    // `pycc_types::class::resolve_attr_get`'s own properties ->
    // instance `attrs` -> `class_attrs` order and, through it,
    // CPython's precedence: an instance `__dict__` entry shadows a
    // non-data-descriptor class attribute. Two independent sibling
    // bases can contribute an instance slot and a same-named class
    // attribute to one derived class without
    // `pycc_hir::class::attrs::reject_class_attr_collisions` ever
    // comparing them (it only walks a class's own `class_attrs`
    // against its own MRO), so the two really can meet here, and
    // folding first made `pycc_mir` disagree with the type checker
    // -- silently printing the class attribute's value, and aborting
    // codegen outright when the two declared types differ.
    //
    // A class attribute still deliberately never enters
    // `mro_attrs`, so it occupies no instance slot
    // and changes no other attribute's slot index or the allocation
    // size of the class's instances. The one cost of the order is
    // that `mro_attrs` now runs for a class-attribute read too,
    // where it previously never ran; MRO chains are shallow and this
    // is compile-time only. It adds no new panic path either: the
    // property walk above already resolves every entry of
    // `class_def.mro` through `mro_class_def`.
    let flat_attrs = mro_attrs(class_def, classes);
    if let Some((slot, (_, ty))) = flat_attrs
        .iter()
        .enumerate()
        .find(|(_, (name, _))| name == attr)
    {
        return MirExpr::AttrGet {
            base: Box::new(base),
            slot,
            ty: ty.clone(),
        };
    }
    // #911 (Part 1 of #885): `w.MIN_WIDTH` / `self.MIN_WIDTH` -- a
    // class-level attribute read through an instance, reached only
    // when the slot lookup above missed. Folded to its constant
    // *before* the panic below, which is the tail for a name that is
    // neither a slot nor a class attribute anywhere in the MRO.
    //
    // Discarding the already-lowered `base` here is sound because
    // `pycc_types` restricts a class-attribute read's base to a bare
    // name -- evaluating it has no observable effect.
    for mro_class in &class_def.mro {
        let mro_def = mro_class_def(mro_class, classes);
        if let Some(folded) = fold_class_attr(mro_def, attr) {
            return folded;
        }
    }
    // Non-strict mode: a method read as a value (`get = self.as_rgb`) is
    // the function value of the method with the receiver bound
    // (`pycc_types`' `bound_method_ty`).
    if let Some((owner, mangled)) = class_def.mro.iter().find_map(|mro_class| {
        mro_class_def(mro_class, classes)
            .methods
            .iter()
            .find(|(name, _)| name == attr)
            .map(|(_, mangled)| (mro_class, mangled.clone()))
    }) {
        let mangled = exact_callee(&class_def.name, owner, mangled, scopes, classes);
        let Ty::Callable(sig) = lookup(scopes, &format!("$callable:{mangled}")) else {
            unreachable!("`$callable:` keys hold function value types")
        };
        return MirExpr::MakeClosure {
            function: mangled,
            captures: vec![base],
            ty: Ty::Callable(Box::new((sig.0[1..].to_vec(), sig.1))),
        };
    }
    panic!(
        "pycc_mir: internal error: attribute `{attr}` not declared on class `{}` \
         or any base in its MRO -- pycc_types::check should have rejected this \
         HIR before it reached pycc_mir",
        class_def.name
    )
}

pub(crate) fn lower_method_call_expr(
    expr: &HirExpr,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    current_class: Option<&str>,
) -> MirExpr {
    let HirExpr::MethodCall { base, method, args } = expr else {
        unreachable!()
    };
    // #433: `super().method(args)` — resolve the method starting
    // from the next class in the current class's MRO, using `self`
    // (the current function's first parameter) as the instance.
    // Lowers to a direct `MirExpr::Call` to the resolved method's
    // mangled name, with `self` prepended as the first argument —
    // no vtable, no runtime dispatch (D-006 static-dispatch framing,
    // per the #433 ADR).
    if matches!(base.as_ref(), HirExpr::Super) {
        let current = match current_class {
            Some(c) => c,
            None => panic!(
                "pycc_mir: internal error: `HirExpr::Super` reached lower_expr outside a \
                 method body -- pycc_hir::lower_expr should have rejected this with C0001"
            ),
        };
        let self_expr = self_expr(scopes);
        // #1337 (D-254): the receiver's MRO after the body's own
        // class, as in the `super().attr` arm above.
        let self_ty = self_expr.ty();
        let receiver = receiver_class(&self_ty).unwrap_or(current);
        let class_def = &classes[receiver];
        let current_pos = class_def
            .mro
            .iter()
            .position(|c| c == current)
            .expect("pycc_mir: internal error: class not found in its receiver's MRO");
        let super_mro = &class_def.mro[current_pos + 1..];
        // #966: `super().__init__()` ranks constructors exactly as
        // `Instantiate` does, so it takes the same skip -- for
        // `class C(A, B)` with `A` init-less, `super().__init__()`
        // must reach `B.__init__` rather than `A`'s D-225 stub.
        // The skip is gated on the method name because every other
        // `super().m()` resolution is name-agnostic and must stay
        // that way; the `or_else` pass is mandatory, since
        // `class A: pass` / `class C(A)` calling `super().__init__()`
        // has nothing but the implicit constructor to reach and
        // would otherwise panic here.
        let skip_implicit_init = method == "__init__";
        let super_method = |skip_implicit: bool| {
            super_mro.iter().find_map(|mro_class| {
                let mro_def = &classes[mro_class.as_str()];
                if skip_implicit && mro_def.implicit_object_init {
                    return None;
                }
                mro_def
                    .methods
                    .iter()
                    .find(|(name, _)| name == method)
                    .map(|(_, mangled)| (mro_class, mangled.clone()))
            })
        };
        let (owner, mangled) = super_method(skip_implicit_init)
            .or_else(|| super_method(false))
            .expect(
                "pycc_mir: internal error: method not declared on class or any base in its \
             MRO after the current class -- pycc_types::check should have rejected this \
             HIR before it reached pycc_mir",
            );
        let mangled = exact_callee(receiver, owner, mangled, scopes, classes);
        let ty = lookup(scopes, &format!("$fn:{mangled}"));
        let mut call_args = Vec::with_capacity(args.len() + 1);
        call_args.push(self_expr);
        call_args.extend(
            args.iter()
                .map(|a| lower_expr(a, scopes, classes, current_class)),
        );
        return MirExpr::Call {
            callee: mangled,
            args: call_args,
            ty,
        };
    }
    // #436: `ClassName.static_method(args)` or
    // `ClassName.class_method(args)` — a method call on a class
    // name (not an instance). The base is `HirExpr::Name` referring
    // to a registered class. `lower_expr` on a bare class name
    // would panic (class names are not in the scope), so intercept
    // here before lowering the base.
    // A value binding of that name shadows the class, so
    // `B.m()` calls the parameter's instance method rather than the
    // class's static or class method -- `pycc_types`' own
    // `MethodCall` class-name arm applies the matching
    // binding/local guard, and this scope check is the MIR-side
    // spelling of it, the same one the class-attribute fold and the
    // enum-member interception already use.
    if let HirExpr::Name(class_name) = base.as_ref()
        && receiver_dispatch::is_unshadowed_class_name(class_name, scopes, classes)
    {
        let class_def = &classes[class_name.as_str()];
        // Part 1 of #1284: `C.name(args)` reaching a
        // `staticmethod(<foreign callable>)` class attribute calls
        // the foreign callable.
        if let Some(target) = foreign_static::class_name_target(class_def, classes, method) {
            return lower_expr(
                &target.call_expr(args.clone()),
                scopes,
                classes,
                current_class,
            );
        }
        let static_mangled = class_def.mro.iter().find_map(|mro_class| {
            let mro_def = mro_class_def(mro_class, classes);
            mro_def
                .static_methods
                .iter()
                .find(|(name, _)| name == method)
                .map(|(_, mangled)| mangled.clone())
        });
        if let Some(mangled) = static_mangled {
            let ty = lookup(scopes, &format!("$fn:{mangled}"));
            let call_args: Vec<MirExpr> = args
                .iter()
                .map(|a| lower_expr(a, scopes, classes, current_class))
                .collect();
            return MirExpr::Call {
                callee: mangled,
                args: call_args,
                ty,
            };
        }
        let class_mangled = class_def.mro.iter().find_map(|mro_class| {
            let mro_def = mro_class_def(mro_class, classes);
            mro_def
                .class_methods
                .iter()
                .find(|(name, _)| name == method)
                .map(|(_, mangled)| (mro_class, mangled.clone()))
        });
        if let Some((owner, mangled)) = class_mangled {
            let mangled = exact_callee(class_name, owner, mangled, scopes, classes);
            let ty = lookup(scopes, &format!("$fn:{mangled}"));
            let mut call_args = Vec::with_capacity(args.len() + 1);
            call_args.push(MirExpr::NullInstance {
                ty: Ty::Instance(Box::new(class_name.clone())),
            });
            call_args.extend(
                args.iter()
                    .map(|a| lower_expr(a, scopes, classes, current_class)),
            );
            return MirExpr::Call {
                callee: mangled,
                args: call_args,
                ty,
            };
        }
    }
    let base_hir = base;
    let base = lower_expr(base, scopes, classes, current_class);
    if let Ty::Generator(sig) = base.ty() {
        return crate::generator::lower_generator_method(
            base,
            &sig,
            method,
            args,
            scopes,
            classes,
            current_class,
        );
    }
    // Part 2 of #1026 (PR 2b of #1081): a method call on a foreign
    // CPython object, which has no `HirClassDef` and so no mangled
    // symbol to dispatch to. Placed exactly where the
    // `HirExpr::AttrGet` arm places its own `ObjAttrGet` branch:
    // after `lower_expr` (a foreign module's local name *is* a value
    // binding in `scopes`, so lowering it is correct and necessary)
    // and before `class_def_of`, whose `Ty::Object` input would hit
    // `class.rs`'s "not a class instance" panic. Every earlier
    // branch in this arm is unreachable for such a base: `Super` is
    // a distinct HIR node, and the class-name guard requires a name
    // that is *not* in `scopes`, which a foreign import's local name
    // always is.
    // `xs.sort()` of instances (`pycc_types::instance_order`).
    let base = match super::instance_order::lower_sort_method(base, method, args, classes, scopes) {
        Ok(sort) => return sort,
        Err(base) => base,
    };
    let lowered_args: Vec<MirExpr> = args
        .iter()
        .map(|a| lower_expr(a, scopes, classes, current_class))
        .collect();
    let base = match builtin_call::lower_builtin_method(base, method, lowered_args) {
        Ok(call) => return call,
        // Non-strict mode: a `str` method given a CPython object argument,
        // or one pycc does not compile, which `pycc_types` admitted as
        // CPython's own method.
        Err((base, lowered_args)) if base.ty() == Ty::Str => {
            return MirExpr::ObjMethodCall {
                base: Box::new(MirExpr::ObjectBox(Box::new(base))),
                method: method.clone(),
                args: lowered_args
                    .into_iter()
                    .map(crate::obj_arith::object_arg)
                    .collect(),
            };
        }
        Err((base, _)) => base,
    };
    // `xs.insert(i, v)` / `xs.pop(i)`: the method calls on a list
    // receiver `pycc_types::list_ops::list_method_ty` admits.
    if let Ty::List(elem_ty) = base.ty() {
        let receiver = match base {
            MirExpr::Name { name, .. } => MirContainerReceiver::Name(name),
            other => MirContainerReceiver::Attr(Box::new(other)),
        };
        let mut args = args
            .iter()
            .map(|a| Box::new(lower_expr(a, scopes, classes, current_class)));
        return match (method.as_str(), args.next(), args.next()) {
            ("insert", Some(index), Some(value)) => MirExpr::ListInsert {
                list: receiver,
                index,
                value,
            },
            ("pop", index, None) => MirExpr::ListPop {
                list: receiver,
                index,
                ty: *elem_ty,
            },
            _ => panic!(
                "pycc_mir: internal error: `list.{method}()` -- pycc_types::list_ops \
                 should have rejected this HIR before it reached pycc_mir"
            ),
        };
    }
    if base.ty() == Ty::Object {
        return MirExpr::ObjMethodCall {
            base: Box::new(base),
            method: method.clone(),
            args: args
                .iter()
                .map(|a| {
                    crate::obj_arith::object_arg(lower_expr(a, scopes, classes, current_class))
                })
                .collect(),
        };
    }
    let class_def = class_def_of(&base, classes);
    // `obj.f(args)` where `f` is an instance attribute holding a function
    // value and no class of the MRO defines a method `f`: a call of the
    // attribute's value (`pycc_types::class::method_call`).
    let defines_method = class_def.mro.iter().any(|mro_class| {
        let mro_def = mro_class_def(mro_class, classes);
        mro_def.methods.iter().any(|(name, _)| name == method)
            || mro_def
                .static_methods
                .iter()
                .any(|(name, _)| name == method)
            || mro_def.class_methods.iter().any(|(name, _)| name == method)
    });
    let holds_function = class_def.mro.iter().any(|mro_class| {
        mro_class_def(mro_class, classes)
            .attrs
            .iter()
            .any(|(name, ty)| name == method && matches!(ty, Ty::Callable(_) | Ty::Object))
    });
    if !defines_method && holds_function {
        let call = HirExpr::ExprCall {
            callee: Box::new(HirExpr::AttrGet {
                base: base_hir.clone(),
                attr: method.clone(),
            }),
            args: args.clone(),
        };
        return super::call_lower::lower_expr_call_expr(&call, scopes, classes, current_class);
    }
    // Part 1 of #1284: `x.name(args)` reaching a
    // `staticmethod(<foreign callable>)` class attribute calls the
    // foreign callable. The receiver is not an argument, but a
    // non-name receiver is still evaluated first, before the
    // arguments (#1346).
    if let Some(target) = foreign_static::instance_target(class_def, classes, method) {
        let value = lower_expr(
            &target.call_expr(args.clone()),
            scopes,
            classes,
            current_class,
        );
        return sequence_after(base, value);
    }
    // #436: check static_methods and class_methods before regular
    // method resolution. Static methods can be called on both
    // classes and instances; class methods can too. When called on
    // an instance, a class method receives the instance as `cls`,
    // while a static method receives nothing -- its receiver is
    // evaluated for its effects only (#1346).
    let static_mangled = class_def.mro.iter().find_map(|mro_class| {
        let mro_def = mro_class_def(mro_class, classes);
        mro_def
            .static_methods
            .iter()
            .find(|(name, _)| name == method)
            .map(|(_, mangled)| mangled.clone())
    });
    if let Some(mangled) = static_mangled {
        let ty = lookup(scopes, &format!("$fn:{mangled}"));
        let call_args: Vec<MirExpr> = args
            .iter()
            .map(|a| lower_expr(a, scopes, classes, current_class))
            .collect();
        let call = MirExpr::Call {
            callee: mangled,
            args: call_args,
            ty,
        };
        return sequence_after(base, call);
    }
    let class_mangled = class_def.mro.iter().find_map(|mro_class| {
        let mro_def = mro_class_def(mro_class, classes);
        mro_def
            .class_methods
            .iter()
            .find(|(name, _)| name == method)
            .map(|(_, mangled)| (mro_class, mangled.clone()))
    });
    if let Some((owner, mangled)) = class_mangled {
        let mangled = exact_callee(&class_def.name, owner, mangled, scopes, classes);
        let ty = lookup(scopes, &format!("$fn:{mangled}"));
        let mut call_args = Vec::with_capacity(args.len() + 1);
        call_args.push(base);
        call_args.extend(
            args.iter()
                .map(|a| lower_expr(a, scopes, classes, current_class)),
        );
        return MirExpr::Call {
            callee: mangled,
            args: call_args,
            ty,
        };
    }
    regular_method::lower_regular_method_call(
        base,
        class_def,
        method,
        args,
        scopes,
        classes,
        current_class,
    )
}
