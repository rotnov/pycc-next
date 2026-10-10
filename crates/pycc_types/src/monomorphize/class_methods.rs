//! Instantiating generic class methods.

use super::*;

/// PEP 695 (#387): Pre-scans `hir` for every `GenericClassInstantiate`
/// expression, collecting the unique `(class_name, type_arg)` pairs, then
/// monomorphizes each generic class's methods by substituting the class's
/// type parameter with the concrete `type_arg` — reusing PR-13's own
/// `substitute_ty`/`substitute_body`/`mangle_generic_instantiation`
/// infrastructure (D-133/D-134). Each monomorphized method is registered
/// in `env` (as an ordinary function signature) and collected in
/// `instantiations` (as a full `HirItem::Function` body for `pycc_mir` to
/// register). The monomorphized class itself is also registered in
/// `env.classes` under its mangled name, with mangled method names, so
/// `infer_expr_in`'s class-instantiation and method-call resolution work
/// transparently on the specialized class.
pub(crate) fn instantiate_generic_class_methods(
    hir: &HirModule,
    env: &mut Environment,
    instantiations: &mut Vec<GenericInstantiation>,
    seen: &mut HashSet<String>,
    new_class_defs: &mut Vec<(String, HirClassDef)>,
) {
    // Collect all unique (class_name, type_arg) pairs from the entire module.
    let mut pairs: Vec<(String, Ty)> = Vec::new();
    for item in &hir.items {
        match item {
            HirItem::TopLevelStmt(stmt) => {
                collect_generic_class_instantiations_from_stmt(stmt, &mut pairs);
            }
            HirItem::Function { body, .. } => {
                for stmt in body {
                    collect_generic_class_instantiations_from_stmt(stmt, &mut pairs);
                }
            }
        }
    }

    for (class_name, type_arg) in &pairs {
        let Some(class_def) = hir.class_defs.iter().find(|(name, _)| name == class_name) else {
            continue;
        };
        let (_, class_def) = class_def;
        let Some(type_param_name) = &class_def.type_param else {
            continue;
        };
        let mangled_class = mangle_generic_instantiation(class_name, type_param_name, type_arg);

        // Monomorphize each method: find its HirItem::Function in hir.items
        // by its mangled name (e.g. "C.__init__"), substitute T with
        // type_arg, rename to the new mangled class name, and register.
        // #386 rebind semantics: class lowering retains multiple same-named
        // method items (the latest definition wins at runtime), and the
        // method table entry was replaced to point at the latest mangled
        // name on redefinition. Since the mangled name is identical across
        // redefinitions, `rfind` (reverse find) specializes the *last*
        // matching `HirItem::Function` -- the one whose body actually runs
        // -- rather than `find`'s first match, which would specialize a
        // stale, shadowed definition.
        let mut mangled_methods: Vec<(String, String)> = Vec::new();
        // Part 1 of #1191: (origin mangled, specialized mangled) for
        // re-keying `method_defaults` below.
        let mut method_renames: Vec<(String, String)> = Vec::new();
        for (method_name, original_mangled) in &class_def.methods {
            // The `rfind` filter already guarantees the item is a
            // `HirItem::Function`, so the destructuring cannot fail — the
            // `else { continue }` handles only the `None` case (method not
            // found in `hir.items`).
            let Some(HirItem::Function {
                name: _,
                params,
                return_ty,
                body,
            }) = hir.items.iter().rfind(|item| {
                matches!(
                    item,
                    HirItem::Function { name, .. } if name == original_mangled
                )
            })
            else {
                continue;
            };
            let substituted_params = params
                .iter()
                .map(|(pn, ty)| {
                    (
                        pn.clone(),
                        substitute_ty_with_class(
                            ty,
                            type_param_name,
                            type_arg,
                            class_name,
                            &mangled_class,
                        ),
                    )
                })
                .collect::<Vec<_>>();
            let substituted_return = substitute_ty_with_class(
                return_ty,
                type_param_name,
                type_arg,
                class_name,
                &mangled_class,
            );
            let substituted_body = substitute_body_with_class(
                body,
                type_param_name,
                type_arg,
                class_name,
                &mangled_class,
            );
            let new_mangled = format!("{mangled_class}.{method_name}");
            let param_tys = substituted_params
                .iter()
                .map(|(_, ty)| ty.clone())
                .collect::<Vec<_>>();
            // Register the method's signature in env so infer_expr_in can
            // resolve calls to it.
            env.bind_function(new_mangled.clone(), param_tys, substituted_return.clone());
            // substitute_body_with_class (called above) already rewrites
            // both `Ty::Param` → `type_arg` and `Ty::Instance(class_name)`
            // → `Ty::Instance(mangled_class)` in the method body's
            // annotations. Self/class-name resolution happened at
            // annotation_to_ty time (HIR lowering), where `Self` and the
            // class name both became `Ty::Instance(class_name)`, and
            // substitute_body_with_class rewrites those to the mangled
            // monomorphized class name. No further substitution is needed
            // here.
            let specialized = HirItem::Function {
                name: new_mangled.clone(),
                params: substituted_params,
                return_ty: substituted_return,
                body: substituted_body,
            };
            if seen.insert(new_mangled.clone()) {
                instantiations.push(GenericInstantiation {
                    mangled_name: new_mangled.clone(),
                    specialized,
                    return_ty: Ty::Instance(Box::new(mangled_class.clone())),
                });
            }
            method_renames.push((original_mangled.clone(), new_mangled.clone()));
            mangled_methods.push((method_name.clone(), new_mangled));
        }

        // #377: Monomorphize properties. Each property's getter and setter
        // are ordinary `HirItem::Function` items with mangled names like
        // `Box.v` and `Box.v.setter`. They are NOT in the class's `methods`
        // table (they're in `properties`), so the method monomorphization
        // loop above does not handle them. We monomorphize them here by
        // finding their `HirItem::Function` in `hir.items`, substituting the
        // type parameter, and renaming to the mangled class name. The
        // monomorphized property entries point at the new mangled names.
        let mut monomorphized_properties: Vec<PropertyDef> = Vec::new();
        for prop in &class_def.properties {
            let new_getter = format!("{mangled_class}.{}", prop.name);
            if let Some(HirItem::Function {
                name: _,
                params,
                return_ty,
                body,
            }) = hir.items.iter().rfind(|item| {
                matches!(
                    item,
                    HirItem::Function { name, .. } if name == &prop.getter
                )
            }) {
                let substituted_params = params
                    .iter()
                    .map(|(pn, ty)| {
                        (
                            pn.clone(),
                            substitute_ty_with_class(
                                ty,
                                type_param_name,
                                type_arg,
                                class_name,
                                &mangled_class,
                            ),
                        )
                    })
                    .collect::<Vec<_>>();
                let substituted_return = substitute_ty_with_class(
                    return_ty,
                    type_param_name,
                    type_arg,
                    class_name,
                    &mangled_class,
                );
                let substituted_body = substitute_body_with_class(
                    body,
                    type_param_name,
                    type_arg,
                    class_name,
                    &mangled_class,
                );
                let param_tys = substituted_params
                    .iter()
                    .map(|(_, ty)| ty.clone())
                    .collect::<Vec<_>>();
                env.bind_function(new_getter.clone(), param_tys, substituted_return.clone());
                let specialized = HirItem::Function {
                    name: new_getter.clone(),
                    params: substituted_params,
                    return_ty: substituted_return,
                    body: substituted_body,
                };
                if seen.insert(new_getter.clone()) {
                    instantiations.push(GenericInstantiation {
                        mangled_name: new_getter.clone(),
                        specialized,
                        return_ty: Ty::Instance(Box::new(mangled_class.clone())),
                    });
                }
            }
            let new_setter = prop.setter.as_ref().map(|s| {
                let new_s = format!("{mangled_class}.{}.setter", prop.name);
                if let Some(HirItem::Function {
                    name: _,
                    params,
                    return_ty,
                    body,
                }) = hir.items.iter().rfind(|item| {
                    matches!(
                        item,
                        HirItem::Function { name, .. } if name == s
                    )
                }) {
                    let substituted_params = params
                        .iter()
                        .map(|(pn, ty)| {
                            (
                                pn.clone(),
                                substitute_ty_with_class(
                                    ty,
                                    type_param_name,
                                    type_arg,
                                    class_name,
                                    &mangled_class,
                                ),
                            )
                        })
                        .collect::<Vec<_>>();
                    let substituted_return = substitute_ty_with_class(
                        return_ty,
                        type_param_name,
                        type_arg,
                        class_name,
                        &mangled_class,
                    );
                    let substituted_body = substitute_body_with_class(
                        body,
                        type_param_name,
                        type_arg,
                        class_name,
                        &mangled_class,
                    );
                    let param_tys = substituted_params
                        .iter()
                        .map(|(_, ty)| ty.clone())
                        .collect::<Vec<_>>();
                    env.bind_function(new_s.clone(), param_tys, substituted_return.clone());
                    let specialized = HirItem::Function {
                        name: new_s.clone(),
                        params: substituted_params,
                        return_ty: substituted_return,
                        body: substituted_body,
                    };
                    if seen.insert(new_s.clone()) {
                        instantiations.push(GenericInstantiation {
                            mangled_name: new_s.clone(),
                            specialized,
                            return_ty: Ty::Instance(Box::new(mangled_class.clone())),
                        });
                    }
                }
                new_s
            });
            monomorphized_properties.push(PropertyDef {
                name: prop.name.clone(),
                getter: new_getter,
                setter: new_setter,
            });
        }

        // #436: Monomorphize static methods. Each static method's
        // mangled name uses a `.static` suffix; the monomorphized name
        // replaces the class prefix with the mangled class name. Static
        // methods have no `self`/`cls` parameter, so only the type
        // parameter substitution applies.
        let mut mangled_static_methods: Vec<(String, String)> = Vec::new();
        for (method_name, original_mangled) in &class_def.static_methods {
            let Some(HirItem::Function {
                name: _,
                params,
                return_ty,
                body,
            }) = hir.items.iter().rfind(|item| {
                matches!(
                    item,
                    HirItem::Function { name, .. } if name == original_mangled
                )
            })
            else {
                continue;
            };
            let substituted_params = params
                .iter()
                .map(|(pn, ty)| {
                    (
                        pn.clone(),
                        substitute_ty_with_class(
                            ty,
                            type_param_name,
                            type_arg,
                            class_name,
                            &mangled_class,
                        ),
                    )
                })
                .collect::<Vec<_>>();
            let substituted_return = substitute_ty_with_class(
                return_ty,
                type_param_name,
                type_arg,
                class_name,
                &mangled_class,
            );
            let substituted_body = substitute_body_with_class(
                body,
                type_param_name,
                type_arg,
                class_name,
                &mangled_class,
            );
            let new_mangled = format!("{mangled_class}.{method_name}.static");
            let param_tys = substituted_params
                .iter()
                .map(|(_, ty)| ty.clone())
                .collect::<Vec<_>>();
            env.bind_function(new_mangled.clone(), param_tys, substituted_return.clone());
            let specialized = HirItem::Function {
                name: new_mangled.clone(),
                params: substituted_params,
                return_ty: substituted_return,
                body: substituted_body,
            };
            if seen.insert(new_mangled.clone()) {
                instantiations.push(GenericInstantiation {
                    mangled_name: new_mangled.clone(),
                    specialized,
                    return_ty: Ty::Instance(Box::new(mangled_class.clone())),
                });
            }
            mangled_static_methods.push((method_name.clone(), new_mangled));
        }

        // #436: Monomorphize class methods. Each class method's mangled
        // name uses a `.classmethod` suffix; the monomorphized name
        // replaces the class prefix with the mangled class name. Class
        // methods have a `cls` parameter typed `Ty::Instance(class_name)`,
        // which `substitute_body_with_class` rewrites to
        // `Ty::Instance(mangled_class)`.
        let mut mangled_class_methods: Vec<(String, String)> = Vec::new();
        for (method_name, original_mangled) in &class_def.class_methods {
            let Some(HirItem::Function {
                name: _,
                params,
                return_ty,
                body,
            }) = hir.items.iter().rfind(|item| {
                matches!(
                    item,
                    HirItem::Function { name, .. } if name == original_mangled
                )
            })
            else {
                continue;
            };
            let substituted_params = params
                .iter()
                .map(|(pn, ty)| {
                    (
                        pn.clone(),
                        substitute_ty_with_class(
                            ty,
                            type_param_name,
                            type_arg,
                            class_name,
                            &mangled_class,
                        ),
                    )
                })
                .collect::<Vec<_>>();
            let substituted_return = substitute_ty_with_class(
                return_ty,
                type_param_name,
                type_arg,
                class_name,
                &mangled_class,
            );
            let substituted_body = substitute_body_with_class(
                body,
                type_param_name,
                type_arg,
                class_name,
                &mangled_class,
            );
            let new_mangled = format!("{mangled_class}.{method_name}.classmethod");
            let param_tys = substituted_params
                .iter()
                .map(|(_, ty)| ty.clone())
                .collect::<Vec<_>>();
            env.bind_function(new_mangled.clone(), param_tys, substituted_return.clone());
            let specialized = HirItem::Function {
                name: new_mangled.clone(),
                params: substituted_params,
                return_ty: substituted_return,
                body: substituted_body,
            };
            if seen.insert(new_mangled.clone()) {
                instantiations.push(GenericInstantiation {
                    mangled_name: new_mangled.clone(),
                    specialized,
                    return_ty: Ty::Instance(Box::new(mangled_class.clone())),
                });
            }
            mangled_class_methods.push((method_name.clone(), new_mangled));
        }

        // Register the monomorphized class in env.classes with mangled
        // method names and substituted attribute types.
        let substituted_attrs = class_def
            .attrs
            .iter()
            .map(|(attr_name, ty)| {
                (
                    attr_name.clone(),
                    substitute_ty(ty, type_param_name, type_arg),
                )
            })
            .collect::<Vec<_>>();
        // #911: a class attribute is a compile-time constant with no
        // instance slot, so monomorphization carries it through unchanged
        // -- its type is already a non-`Param` scalar (a `Param(_)`
        // annotation is rejected at lowering), so there is nothing to
        // substitute. Omitting the field here would silently drop every
        // class attribute from a monomorphized generic class.
        // #977 (D-237): a `@dataclass` generic class keeps its dataclass
        // identity in every specialization. `mangled_methods` above already
        // carries the synthesized `__init__`/`__eq__`/`__repr__` under the
        // mangled class name, so `pycc_mir::class::rewrite_instance_to_repr`
        // can render the specialization exactly as it renders the origin --
        // but only if `is_dataclass` says so. Dropping the flag here made
        // `check` accept `print(Box[int](1))` (the origin `Box` is a
        // dataclass) while `build` re-inferred the rewritten call against
        // the specialization and rejected it with `C0001`. The field list
        // is substituted the same way `attrs` is, so a `T`-typed field
        // reports its concrete type on the specialization.
        let substituted_dataclass_fields = class_def
            .dataclass_fields
            .iter()
            .map(|(field_name, ty)| {
                (
                    field_name.clone(),
                    substitute_ty(ty, type_param_name, type_arg),
                )
            })
            .collect::<Vec<_>>();
        let new_class_def = HirClassDef {
            class_attrs: class_def.class_attrs.clone(),
            exception_type_tag: None,
            name: mangled_class.clone(),
            bases: Vec::new(),
            mro: vec![mangled_class.clone()],
            attrs: substituted_attrs,
            methods: mangled_methods,
            type_param: None, // The monomorphized class is not generic.
            properties: monomorphized_properties,
            static_methods: mangled_static_methods,
            class_methods: mangled_class_methods,
            is_enum: false,
            implicit_object_init: false,
            method_defaults: method_defaults::rekey_method_defaults(
                &class_def.method_defaults,
                &method_renames,
            ),
            enum_members: Vec::new(),
            is_dataclass: class_def.is_dataclass,
            dataclass_fields: substituted_dataclass_fields,
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        };
        // `bind_class` clears any synthetic marking for the name it binds
        // (Part 1 of #541, D-188), which is harmless here: `mangled_class`
        // always carries `mangle_generic_instantiation`'s `0gen_` prefix, so
        // it can never equal one of the seven `BUILTIN_EXCEPTION_CLASSES`
        // names and can never unmark a synthetic entry `bind_classes` made.
        env.bind_class(mangled_class.clone(), new_class_def.clone());
        new_class_defs.push((mangled_class, new_class_def));
    }
}

/// PEP 695 (#387): Pass 2b per-item worker — rewrites any
/// `GenericClassInstantiate` expressions that survived inside one
/// monomorphized generic-class method body. Extracted from `monomorphize`'s
/// Pass 2b loop so that the defense-in-depth non-`Function` arm (all
/// instantiations are `HirItem::Function` in practice) is a `match` arm in
/// a standalone function that a direct unit test can cover, rather than an
/// `if let` whose never-taken false branch would carry an uncovered
/// continuation region under the 100%-coverage gate (D-014).
///
/// Clones the function's params/body out of `instantiations[i]` so the
/// immutable borrow ends before `rewrite_generic_calls_in_stmt` takes
/// `&mut instantiations` (it may push new instantiations for a GCI inside
/// the body that triggers another class monomorphization).
pub(crate) fn rewrite_generic_calls_in_instantiation(
    env: &mut Environment,
    i: usize,
    instantiations: &mut Vec<GenericInstantiation>,
    seen: &mut HashSet<String>,
) -> Result<(), Diagnostic> {
    let (name, params, return_ty, body) = match &instantiations[i].specialized {
        HirItem::Function {
            name,
            params,
            return_ty,
            body,
        } => (
            name.clone(),
            params.clone(),
            return_ty.clone(),
            body.clone(),
        ),
        // All instantiations are `HirItem::Function` (created by
        // `instantiate_generic_call` or `instantiate_generic_class_methods`).
        // This arm is defense-in-depth, covered by a direct unit test.
        _ => return Ok(()),
    };
    let local_names: Vec<String> = params.iter().map(|(n, _)| n.clone()).collect();
    let local_names_refs: Vec<&str> = local_names.iter().map(|s| s.as_str()).collect();
    let mut fn_env = env.child_for_function(&local_names_refs);
    for (param_name, param_ty) in &params {
        fn_env.bind(param_name.clone(), param_ty.clone());
    }
    let mut new_body = body;
    for stmt in new_body.iter_mut() {
        rewrite_generic_calls_in_stmt(&mut fn_env, &local_names_refs, stmt, instantiations, seen)?;
    }
    // Direct assignment avoids a second `if let` (whose never-taken false
    // branch would carry an uncovered region under D-014). All instantiations
    // are `HirItem::Function`, so this reconstruction is always valid.
    instantiations[i].specialized = HirItem::Function {
        name,
        params,
        return_ty,
        body: new_body,
    };
    Ok(())
}
