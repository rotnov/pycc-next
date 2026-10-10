//! Structural protocol conformance.

use super::*;

/// #380 (PR-20): Checks whether a concrete class structurally conforms to
/// a protocol. A class conforms if, for every protocol member (method or
/// attribute), the class has a member of the same name with a compatible
/// type. Method compatibility requires matching parameter count and
/// assignable parameter/return types. Attribute compatibility requires
/// an assignable attribute type. Members are looked up through the
/// class's MRO (inherited methods and attributes count toward
/// conformance). Returns `Ok(())` if the class conforms, or a `T0046`
/// diagnostic identifying the first missing or incompatible member.
pub(crate) fn check_protocol_conformance(
    env: &Environment,
    class_name: &str,
    protocol_name: &str,
) -> Result<(), Diagnostic> {
    let class_def = env
        .lookup_class(class_name)
        .expect("pycc_types: internal error: class was not registered -- check_protocol_conformance should only be called with a known class");
    let proto_def = env
        .lookup_class(protocol_name)
        .expect("pycc_types: internal error: protocol was not registered -- check_protocol_conformance should only be called with a known protocol");
    // The protocol's members are already merged (including inherited
    // members from base protocols) at HIR-lowering time.
    for member in &proto_def.protocol_members {
        match member {
            ProtocolMember::Method {
                name: method_name,
                param_tys: proto_param_tys,
                return_ty: proto_return_ty,
            } => {
                // Look up the method through the MRO.
                let found = lookup_method_through_mro(env, &class_def.mro, method_name);
                let Some((concrete_param_tys, concrete_return_ty)) = found else {
                    return Err(Diagnostic::error(
                        "T0046",
                        format!(
                            "class `{class_name}` does not conform to protocol \
                             `{protocol_name}`: missing method `{method_name}`"
                        ),
                        Span::new(0, 0),
                    ));
                };
                // Check parameter count (excluding `self`).
                // `concrete_param_tys` includes `self`; `proto_param_tys`
                // excludes it (stripped at HIR-lowering time).
                let concrete_non_self: Vec<Ty> = concrete_param_tys
                    .iter()
                    .skip_while(|(_, n)| n == "self")
                    .map(|(t, _)| t.clone())
                    .collect();
                if concrete_non_self.len() != proto_param_tys.len() {
                    return Err(Diagnostic::error(
                        "T0046",
                        format!(
                            "class `{class_name}` does not conform to protocol \
                             `{protocol_name}`: method `{method_name}` has {} parameter(s), \
                             expected {}",
                            concrete_non_self.len(),
                            proto_param_tys.len()
                        ),
                        Span::new(0, 0),
                    ));
                }
                // Check parameter types (contravariant — a caller through
                // the protocol may pass any value of the protocol's declared
                // parameter type, so the concrete method must accept at least
                // that type: the protocol param type must be assignable to
                // the concrete param type).
                for (i, (cp, pp)) in concrete_non_self
                    .iter()
                    .zip(proto_param_tys.iter())
                    .enumerate()
                {
                    if !is_assignable(pp.clone(), cp.clone()) {
                        return Err(Diagnostic::error(
                            "T0046",
                            format!(
                                "class `{class_name}` does not conform to protocol \
                                 `{protocol_name}`: method `{method_name}` parameter {} has \
                                 type `{}`, expected `{}`",
                                i + 1,
                                cp.name(),
                                pp.name()
                            ),
                            Span::new(0, 0),
                        ));
                    }
                }
                // Check return type (covariant — concrete return must be
                // assignable to protocol return).
                if !is_assignable(concrete_return_ty.clone(), proto_return_ty.clone()) {
                    return Err(Diagnostic::error(
                        "T0046",
                        format!(
                            "class `{class_name}` does not conform to protocol \
                             `{protocol_name}`: method `{method_name}` return type is `{}`, \
                             expected `{}`",
                            concrete_return_ty.name(),
                            proto_return_ty.name()
                        ),
                        Span::new(0, 0),
                    ));
                }
            }
            ProtocolMember::Attribute {
                name: attr_name,
                ty: proto_attr_ty,
            } => {
                // Look up the attribute through the MRO.
                // #914 (follow-up to #911, Part 1 of #885): a class-level
                // attribute satisfies an attribute member exactly as an
                // instance slot or a `@property` does, matching mypy and
                // pyright. The two walks stay separate for the reason
                // `lookup_class_attr_through_mro`'s own doc comment gives
                // (a class attribute occupies no slot and folds to a
                // constant), and the *order* is deliberate: two independent
                // sibling bases can contribute an instance slot and a
                // same-named class attribute to one derived class without
                // `reject_class_attr_collisions` ever comparing them, and
                // CPython lets the instance `__dict__` entry win there.
                // Since #960 `pycc_mir`'s instance `AttrGet` arm mirrors that
                // order too, so the checker and the lowering agree on which
                // of the two a read resolves to.
                let found = lookup_attr_through_mro(env, &class_def.mro, attr_name)
                    .or_else(|| lookup_class_attr_through_mro(env, class_name, attr_name));
                // Part 1 of #1284: a `staticmethod(<foreign callable>)` class
                // attribute has no object path through a protocol-typed
                // read, so it does not satisfy an attribute member.
                if lookup_attr_through_mro(env, &class_def.mro, attr_name).is_none()
                    && foreign_static::is_foreign_static_class_attr(env, class_name, attr_name)
                {
                    return Err(Diagnostic::error(
                        "T0046",
                        format!(
                            "class `{class_name}` does not conform to protocol \
                             `{protocol_name}`: missing attribute `{attr_name}`"
                        ),
                        Span::new(0, 0),
                    )
                    .with_help(format!(
                        "`{class_name}.{attr_name}` is a `staticmethod(...)` class attribute, \
                         which does not satisfy a protocol attribute member (#1284)"
                    )));
                }
                let Some(concrete_attr_ty) = found else {
                    return Err(Diagnostic::error(
                        "T0046",
                        format!(
                            "class `{class_name}` does not conform to protocol \
                             `{protocol_name}`: missing attribute `{attr_name}`"
                        ),
                        Span::new(0, 0),
                    ));
                };
                if !is_assignable(concrete_attr_ty.clone(), proto_attr_ty.clone()) {
                    return Err(Diagnostic::error(
                        "T0046",
                        format!(
                            "class `{class_name}` does not conform to protocol \
                             `{protocol_name}`: attribute `{attr_name}` has type `{}`, \
                             expected `{}`",
                            concrete_attr_ty.name(),
                            proto_attr_ty.name()
                        ),
                        Span::new(0, 0),
                    ));
                }
            }
        }
    }
    Ok(())
}
