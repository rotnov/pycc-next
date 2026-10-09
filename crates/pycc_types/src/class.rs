//! Class-body type-checking (D-154, Part 1 of #375): resolving instance
//! instantiation, attribute access, and method calls against
//! `Environment`'s class table.
//!
//! Unlike a hand-rolled per-class checking pass, every method (including
//! `__init__`) is already an ordinary `HirItem::Function` under its mangled
//! `<ClassName>.<method_name>` name (see `pycc_hir::class`'s own doc
//! comment) -- so `check_function_in`/`check_generic_function_in` and the
//! constraint solver already check a method body exactly like any other
//! function, with `self` bound to `Ty::Instance(class_name)` by the same
//! ordinary parameter-binding logic every other parameter goes through. The
//! functions in this module are only the *additional* pieces that ordinary
//! function-checking has no shape for: resolving `ClassName(...)`
//! (instantiation), `base.attr` (an instance attribute read or write), and
//! `base.method(...)` (an instance method call) against the class's
//! declared shape.

mod attr_set;
mod binding;
pub(crate) mod foreign_static;
mod method_call;
mod method_defaults;
mod receiver_class_call;
mod static_call;
mod super_call;

use crate::{Environment, infer_expr_in, is_assignable};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{
    HirExpr, ProtocolMember, Ty, declares_name_outside_class_attrs, extract_class_names,
    is_builtin_type_name,
};

// A private import, not a re-export: it keeps this module's own unqualified
// `expect_class` call sites compiling now that the function itself lives in
// `class/binding.rs`.
use binding::expect_class;
// A re-export, so every existing `class::bind_classes` /
// `class::resolve_instantiation` call site (and the doc comments in `lib.rs`
// naming those paths) keeps working across the `class/binding.rs` extraction.
pub(crate) use binding::{bind_classes, resolve_instantiation};
// A re-export, so every existing `class::check_attr_set` call site keeps
// working across the `class/attr_set.rs` extraction (#1219).
pub(crate) use attr_set::check_attr_set;
// A re-export, so every existing `class::resolve_method_call` call site
// (`expr.rs`'s `HirExpr::MethodCall` arm) keeps working across the
// `class/method_call.rs` extraction (#815, Part 1 of #737).
pub(crate) use method_call::resolve_method_call;
// #1411: `expr.rs`'s `HirExpr::ReceiverClassCall` arm.
pub(crate) use receiver_class_call::infer_receiver_class_call;
// Re-exports for the same reason, across the `class/static_call.rs` and
// `class/super_call.rs` extractions (#549): `expr.rs` and
// `crates/pycc_types/src/tests/init_rank.rs` both name these through
// `class::`.
pub(crate) use static_call::{has_static_or_class_method, resolve_static_or_class_method_call};
pub(crate) use super_call::{resolve_super_attr_get, resolve_super_method_call};

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

/// Looks up a method through the MRO, returning its parameter types
/// (including `self`) and return type. Walks the MRO most-derived-first
/// and returns the first match.
fn lookup_method_through_mro(
    env: &Environment,
    mro: &[String],
    method_name: &str,
) -> Option<(Vec<(Ty, String)>, Ty)> {
    for mro_class in mro {
        // Every class in the MRO was defined earlier in the module and
        // is present in the `Environment`'s class table — the C3 MRO is
        // built from already-lowered class definitions. Using `.expect`
        // (whose panic path lives in libcore, outside this crate's
        // instrumented regions) avoids a permanently-uncovered `?`-None
        // region under D-014's 100 %-coverage gate.
        let mro_def = env
            .lookup_class(mro_class)
            .expect("MRO classes are always in the environment");
        if let Some(mangled) = mro_def
            .methods
            .iter()
            .find(|(name, _)| name == method_name)
            .map(|(_, mangled)| mangled.as_str())
            && let Some((param_tys, return_ty)) = env.lookup_function(mangled)
        {
            // `param_tys` is `Vec<Ty>` (just types, no names). We
            // pair each with an empty name since conformance
            // checking only needs the types. The `self` parameter
            // is identified by position (first), not by name.
            let named: Vec<(Ty, String)> = param_tys
                .iter()
                .enumerate()
                .map(|(i, ty)| {
                    (
                        ty.clone(),
                        if i == 0 {
                            "self".to_string()
                        } else {
                            String::new()
                        },
                    )
                })
                .collect();
            return Some((named, return_ty.clone()));
        }
    }
    None
}

/// Looks up an attribute type through the MRO. Walks the MRO
/// most-derived-first and returns the first match.
fn lookup_attr_through_mro(env: &Environment, mro: &[String], attr_name: &str) -> Option<Ty> {
    for mro_class in mro {
        // Same invariant as `lookup_method_through_mro` above: every
        // class in the MRO is present in the `Environment`'s class table.
        let mro_def = env
            .lookup_class(mro_class)
            .expect("MRO classes are always in the environment");
        if let Some((_, ty)) = mro_def.attrs.iter().find(|(name, _)| name == attr_name) {
            return Some(ty.clone());
        }
        // A @property satisfies an attribute requirement just as a direct
        // attribute does — `eval_isinstance_protocol` checks both, and the
        // conformance check must be consistent with it (#380 W2). Since #914
        // a class-level attribute is a third such source; it is matched by
        // `check_protocol_conformance`'s own fallback to
        // `lookup_class_attr_through_mro` rather than here, because this
        // walk is also the instance-attribute path.
        if let Some(prop) = mro_def.properties.iter().find(|p| p.name == attr_name) {
            let (_, return_ty) = env.lookup_function(&prop.getter).unwrap_or_else(|| {
                panic!(
                    "pycc_types: internal error: property getter `{}` is in class `{mro_class}`'s \
                     own property table but was not registered as an ordinary function",
                    prop.getter
                )
            });
            return Some(return_ty.clone());
        }
    }
    None
}

/// #380 (PR-20): Checks assignability with protocol conformance support.
/// Delegates to `is_assignable` for non-protocol cases. When `to` is
/// `Ty::Protocol(name)` and `from` is `Ty::Instance(class_name)`, checks
/// structural conformance via `check_protocol_conformance`. Returns
/// `true` if assignable, `false` otherwise. For detailed error messages,
/// call `check_protocol_conformance` directly.
pub(crate) fn is_assignable_env(env: &Environment, from: &Ty, to: &Ty) -> bool {
    // Protocol conformance: Instance -> Protocol
    if let (Ty::Instance(class_name), Ty::Protocol(protocol_name)) = (from, to) {
        return check_protocol_conformance(env, class_name, protocol_name).is_ok();
    }
    // Protocol to Protocol: same protocol or inherited
    if let (Ty::Protocol(from_name), Ty::Protocol(to_name)) = (from, to) {
        if from_name == to_name {
            return true;
        }
        // Check if from_name's MRO includes to_name
        if let Some(from_def) = env.lookup_class(from_name)
            && from_def.mro.iter().any(|m| m.as_str() == to_name.as_str())
        {
            return true;
        }
        return false;
    }
    // For all other cases, use the plain `is_assignable`.
    is_assignable(from.clone(), to.clone())
}

/// #380 (PR-20): Returns a `T0046` diagnostic for a protocol conformance
/// failure, or a `T0021` for a general type mismatch. Called when
/// `is_assignable_env` returns `false` to produce a detailed error.
pub(crate) fn assignable_error(env: &Environment, from: &Ty, to: &Ty) -> Diagnostic {
    if let (Ty::Instance(class_name), Ty::Protocol(protocol_name)) = (from, to) {
        // `assignable_error` is only called when `is_assignable_env`
        // returned `false`, which means `check_protocol_conformance`
        // already returned `Err`.  Using `.expect()` (whose panic path
        // lives in libcore, outside this crate's instrumented regions)
        // avoids a permanently-uncovered `unwrap_or_else` closure under
        // D-014's 100 %-region coverage gate.
        return check_protocol_conformance(env, class_name, protocol_name)
            .expect_err("is_assignable_env already returned false, so check_protocol_conformance must return Err");
    }
    if let (Ty::Protocol(from_name), Ty::Protocol(to_name)) = (from, to)
        && from_name != to_name
    {
        return Diagnostic::error(
            "T0046",
            format!("protocol `{from_name}` does not conform to protocol `{to_name}`"),
            Span::new(0, 0),
        );
    }
    Diagnostic::error(
        "T0021",
        format!(
            "type mismatch: `{}` is not assignable to `{}`",
            from.name(),
            to.name()
        ),
        Span::new(0, 0),
    )
}

fn t0043_not_an_instance(action: &str, ty: &Ty) -> Diagnostic {
    Diagnostic::error(
        "T0043",
        format!(
            "cannot {action} on `{}`: it is not a class instance",
            ty.name()
        ),
        Span::new(0, 0),
    )
}

fn t0044_unknown_member(kind: &str, class_name: &str, member: &str) -> Diagnostic {
    Diagnostic::error(
        "T0044",
        format!("class `{class_name}` has no {kind} named `{member}`"),
        Span::new(0, 0),
    )
}

/// #587: `super().<attr>` naming an *instance* attribute — one established
/// by `self.<attr> = ...` inside `__init__`, and therefore living in the
/// instance's own slot rather than on any class in the MRO. CPython's
/// `super` object proxies class-level attributes and descriptors only, so
/// this raises `AttributeError` there; pycc used to resolve it against
/// `self`'s slot and return a value instead.
fn t0047_super_instance_attr(attr: &str, declaring_class: &str) -> Diagnostic {
    Diagnostic::error(
        "T0047",
        format!(
            "`super().{attr}` is not readable: `{attr}` is an instance attribute of class \
             `{declaring_class}`, and `super()` proxies class-level attributes and \
             descriptors along the MRO, not the instance's own attributes -- CPython raises \
             `AttributeError: 'super' object has no attribute '{attr}'` here"
        ),
        Span::new(0, 0),
    )
    // #1181: the receiver's *source* spelling never crosses into this crate
    // -- `pycc_hir::class::receiver` lowers it under the canonical name
    // `self` precisely so nothing here has to know it -- so the help names
    // the receiver without asserting how it is spelled. Naming `self` would
    // tell a user whose receiver is spelled `this` to write the one thing
    // that crate's own occurrence guard forbids.
    .with_help(format!(
        "read it through the method's own receiver instead: `<receiver>.{attr}`"
    ))
}

/// Validates a call's arguments against a resolved `(param_tys, return_ty)`
/// signature, reusing this crate's own existing "call to undefined
/// function"-adjacent diagnostic shape (`T0021`, `infer_expr_in`'s own
/// `HirExpr::Call` arm) rather than inventing a class-specific arity/type
/// mismatch code -- an instantiation call and a method call are both, at
/// their core, "call this mangled function with these arguments," the same
/// shape an ordinary function call already validates.
///
/// #953: `structural` selects the assignability predicate applied to each
/// argument. `true` uses the environment-aware
/// [`is_assignable_env`], so a concrete class instance is accepted for a
/// protocol-typed parameter when it structurally conforms (PEP 544);
/// `false` keeps the plain nominal [`is_assignable`]. Only the
/// instance-method-call sites (`class/method_call.rs`,
/// `class/method_defaults.rs`) pass `true`: the
/// static-method, class-method, `super()`, constructor and exception-
/// constructor sites still pass `false`, because accepting a conforming
/// concrete argument there needs protocol monomorphization support that
/// does not exist yet for those lowering shapes, and flipping one of them
/// without that support produces a `pycc_mir`/`pycc_codegen` panic rather
/// than a working program (see the #953 follow-up issue). The diagnostics
/// themselves -- both the arity `T0021` and the per-argument `T0021` --
/// are identical either way: a non-conforming argument keeps today's
/// message, and `T0046` (`assignable_error`) stays reserved for the
/// assignment/conformance positions that already use it.
///
/// Independently of `structural`, every site admits a native value boxed
/// into an `object` parameter ([`crate::object_box::admits_value`], Part 2
/// of #1387): codegen's one argument-marshalling helper boxes it for every
/// call shape, so no site needs a lowering of its own. `args` are the
/// call's argument expressions, in `arg_tys` order, so a class method's
/// own `cls` is refused (`I0404`) at every call shape. It is shorter than
/// `arg_tys` when `method_defaults` filled trailing defaults in; a filled
/// default is appended unboxed, so it never boxes here and must be
/// assignable as it stands.
pub(crate) fn check_call_args(
    env: &Environment,
    callee: &str,
    args: &[HirExpr],
    arg_tys: &[Ty],
    param_tys: &[Ty],
    structural: bool,
) -> Result<(), Diagnostic> {
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
        let assignable = if structural {
            is_assignable_env(env, arg_ty, param_ty)
        } else {
            is_assignable(arg_ty.clone(), param_ty.clone())
        } || args.get(i).map_or(Ok(false), |arg| {
            crate::object_box::admits_value(env, arg, arg_ty, param_ty)
        })?;
        if !assignable {
            return Err(Diagnostic::error(
                "T0021",
                format!(
                    "argument {} of `{callee}` expects `{}`, got `{}`",
                    i + 1,
                    param_ty.name(),
                    arg_ty.name()
                ),
                Span::new(0, 0),
            )
            .with_help(format!("pass a `{}` value", param_ty.name())));
        }
    }
    Ok(())
}

/// Resolves `base.attr` (an instance attribute read) against `base_ty`.
/// Shared by `infer_expr_in`'s `HirExpr::AttrGet` arm and `check_stmt`'s
/// `HirStmt::AttrSet` arm (which also needs `base`'s attribute type, to
/// check the assigned value against it).
///
/// #377: a `@property` getter is checked *before* the regular attribute
/// slot table -- `obj.x` where `x` is a property resolves to the getter
/// method's return type, not a slot type. This mirrors CPython's own
/// observable behavior, where a property descriptor intercepts attribute
/// access before the instance's `__dict__`/slot table is consulted.
///
/// #432: walks the class's MRO (C3 linearization) in order, checking each
/// class's property table and attribute slots. The first class in the MRO
/// that declares the attribute (or a property of that name) wins, matching
/// CPython's own MRO-based attribute resolution.
pub(crate) fn resolve_attr_get(
    env: &Environment,
    base_ty: &Ty,
    attr: &str,
) -> Result<Ty, Diagnostic> {
    // #380 (PR-20, PEP 544): protocol-typed variable attribute access.
    // When `base_ty` is `Ty::Protocol(P)`, look up the attribute in
    // `P`'s `protocol_members`.
    if let Ty::Protocol(protocol_name) = base_ty {
        let proto_def = expect_class(env, protocol_name);
        for member in &proto_def.protocol_members {
            if let ProtocolMember::Attribute { name, ty } = member
                && name == attr
            {
                return Ok(ty.clone());
            }
        }
        return Err(t0044_unknown_member("attribute", protocol_name, attr));
    }
    let Ty::Instance(class_name) = base_ty else {
        return Err(t0043_not_an_instance("read an attribute", base_ty));
    };
    let class_def = expect_class(env, class_name);
    // #432/#377: walk the MRO for property lookup first (matching CPython's
    // descriptor protocol precedence — a property descriptor intercepts
    // attribute access before `__dict__`), across ALL classes in the MRO,
    // then fall back to regular attribute slots. This matches the MIR
    // lowering's own properties-first-across-full-MRO logic exactly,
    // avoiding a type-checker/MIR disagreement when a derived class has a
    // regular attr with the same name as a base class property.
    for mro_class in &class_def.mro {
        let mro_def = expect_class(env, mro_class);
        if let Some(prop) = mro_def.properties.iter().find(|p| p.name == attr) {
            let (_, return_ty) = env.lookup_function(&prop.getter).unwrap_or_else(|| {
                panic!(
                    "pycc_types: internal error: property getter `{}` is in class `{mro_class}`'s \
                     own property table but was not registered as an ordinary function",
                    prop.getter
                )
            });
            return Ok(return_ty.clone());
        }
    }
    // No property matched in any class — now check regular attribute slots
    // by walking the MRO in order (most-derived first, so a re-declared
    // attr uses the most-derived type).
    for mro_class in &class_def.mro {
        let mro_def = expect_class(env, mro_class);
        if let Some((_, ty)) = mro_def.attrs.iter().find(|(name, _)| name == attr) {
            return Ok(ty.clone());
        }
    }
    // #911 (Part 1 of #885): a class-level attribute (`MIN_WIDTH: int =
    // -1024`) read through an instance. Checked *after* the instance slots
    // so a real slot always wins, which is CPython's own precedence -- an
    // instance `__dict__` entry shadows a non-data-descriptor class
    // attribute.
    //
    // #960: that ordering is load-bearing, not merely defensive. `pycc_hir`'s
    // `reject_class_attr_collisions` walks only a class's *own* `class_attrs`
    // against its own MRO, so two independent sibling bases -- one
    // contributing an instance slot, the other a same-named class attribute
    // -- reach this function without ever having been compared. `pycc_mir`'s
    // instance `AttrGet` arm mirrors this exact order; folding first is what
    // #960 was.
    if let Some(ty) = lookup_class_attr_through_mro(env, class_name, attr) {
        return Ok(ty);
    }
    Err(t0044_unknown_member("attribute", class_name, attr))
}

/// #911 (Part 1 of #885): looks a class-level attribute up through
/// `class_name`'s MRO, most-derived first, returning its declared type.
///
/// A class attribute occupies no instance slot -- `pycc_mir` folds a read of
/// it to the constant recorded in `HirClassDef::class_attrs` -- so this walk
/// is deliberately separate from the `attrs` walk above rather than merged
/// into it. Since #960 that fold is reached only when the instance-slot walk
/// missed, on both sides: a same-named slot contributed by a sibling MRO base
/// wins here and in `pycc_mir` alike.
pub(crate) fn lookup_class_attr_through_mro(
    env: &Environment,
    class_name: &str,
    attr: &str,
) -> Option<Ty> {
    let class_def = expect_class(env, class_name);
    for mro_class in &class_def.mro {
        let mro_def = expect_class(env, mro_class);
        if let Some((_, ty, _)) = mro_def.class_attrs.iter().find(|(name, _, _)| name == attr) {
            return Some(ty.clone());
        }
    }
    None
}

/// #974: looks a class-level attribute up through `class_name`'s MRO for a
/// **class-name-qualified** read (`Derived.LIMIT`), most-derived first,
/// returning its declared type.
///
/// This is deliberately *not* [`lookup_class_attr_through_mro`], and the two
/// must not be merged. That function serves the instance read path and its
/// four other callers, none of which want the rule below; it iterates
/// `class_attrs` alone, so on
///
/// ```python
/// class A:
///     x: int = 2
///
///
/// class B(A):
///     @staticmethod
///     def x() -> int:
///         return 1
/// ```
///
/// it would answer `int` for `B.x` and let `pycc_mir` fold the read to `2`,
/// turning a program that compiles correctly today into a wrong-value
/// mis-compile. CPython binds `B`'s own `x` and never reaches `A`'s.
///
/// The rule implemented here instead: walk `class_name`'s MRO most-derived
/// first and return the class attribute's type only when the **first** class
/// that declares `attr` at all declares it as a class attribute. When some
/// other class-level namespace claims the name first, the read stays
/// unresolved (`None`) and the caller reports `T0044` -- pycc does not model
/// a bare, uncalled class-name-qualified method read.
/// [`declares_name_outside_class_attrs`] is the shadowing predicate, shared
/// verbatim with `pycc_mir`'s fold so the two cannot drift; its own doc
/// comment records which namespaces are checked and why instance slots and
/// protocol members are excluded.
///
/// Excluding instance slots is what makes this path differ from
/// [`resolve_attr_get`]'s #960 precedence, on purpose: a class object has no
/// instance `__dict__`. The divergence is CPython's own -- see
/// `docs/TYPE_SYSTEM.md`'s "Class-level attributes" section for the measured
/// `c.x` / `C.x` pair.
pub(crate) fn lookup_class_attr_by_class_name(
    env: &Environment,
    class_name: &str,
    attr: &str,
) -> Option<Ty> {
    let class_def = expect_class(env, class_name);
    for mro_class in &class_def.mro {
        let mro_def = expect_class(env, mro_class);
        if let Some((_, ty, _)) = mro_def.class_attrs.iter().find(|(name, _, _)| name == attr) {
            return Some(ty.clone());
        }
        if declares_name_outside_class_attrs(mro_def, attr) {
            return None;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Issue #435: compile-time `isinstance`/`issubclass` type checking.
//
// pycc uses static dispatch (D-006) — every variable's runtime type is
// exactly its declared static type, so `isinstance`/`issubclass` can always
// be evaluated at compile time. These functions validate the call's
// arguments and return `Ty::Bool` (the result is computed at MIR lowering
// time, not here — the type checker only needs to confirm the call is
// well-formed and returns `bool`).
// ---------------------------------------------------------------------------

/// Validates a class name argument to `isinstance`/`issubclass`: it must be
/// either a registered user-defined class (including protocols) or one of
/// the builtin scalar type names (`int`, `str`, `float`, `bool`). Returns
/// `Ok(())` if valid, or a `T0001` diagnostic if the name is unknown.
fn validate_class_name(env: &Environment, name: &str) -> Result<(), Diagnostic> {
    if env.lookup_class(name).is_some() || is_builtin_type_name(name) {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "T0001",
            format!("`{name}` is not a known class or builtin type"),
            Span::new(0, 0),
        ))
    }
}

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
fn cast_compatibility(env: &Environment, from: &Ty, to: &Ty) -> Result<(), CastMismatch> {
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
    let target_methods: std::collections::HashSet<&str> = to_def
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

/// The three distinct ways `cast_compatibility` rejects a target, each
/// reported with its own message by `check_cast`: a scalar or unrelated-type
/// representation change, a genuine down-cast or unrelated-class layout
/// narrowing, and an up-cast that crosses a method-override boundary (see
/// `cast_compatibility`'s doc comment for why the third is unsound too).
#[derive(Debug, PartialEq)]
enum CastMismatch {
    Representation,
    Layout,
    OverriddenMethod(String),
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

#[cfg(test)]
mod tests {
    use crate::{check, check_and_resolve};
    use pycc_hir::{BinOpKind, HirItem, HirModule, HirStmt, Ty};
    use pycc_hir::{HirClassDef, HirExpr};

    /// Builds a minimal `Point` class module: `__init__(self, x: int, y:
    /// int)` sets both attributes from its own parameters; `bump(self) ->
    /// None` reads and mutates `self.x`. `extra_items`/`extra_stmts` let
    /// each test append its own instantiation/attribute/method-call
    /// exercise without duplicating this fixture.
    fn point_module(extra_items: Vec<HirItem>) -> HirModule {
        let self_ty = Ty::Instance(Box::new("Point".to_string()));
        let init = HirItem::Function {
            name: "Point.__init__".to_string(),
            params: vec![
                ("self".to_string(), self_ty.clone()),
                ("x".to_string(), Ty::Int),
                ("y".to_string(), Ty::Int),
            ],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "x".to_string(),
                    value: HirExpr::Name("x".to_string()),
                },
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "y".to_string(),
                    value: HirExpr::Name("y".to_string()),
                },
                HirStmt::Return(None),
            ],
        };
        let bump = HirItem::Function {
            name: "Point.bump".to_string(),
            params: vec![("self".to_string(), self_ty.clone())],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "x".to_string(),
                    value: HirExpr::BinOp {
                        op: BinOpKind::Add,
                        left: Box::new(HirExpr::AttrGet {
                            base: Box::new(HirExpr::Name("self".to_string())),
                            attr: "x".to_string(),
                        }),
                        right: Box::new(HirExpr::IntLiteral(1)),
                    },
                },
                HirStmt::Return(None),
            ],
        };
        let mut items = vec![init, bump];
        items.extend(extra_items);
        HirModule {
            seeded_builtin_exception_classes: false,
            items,
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: vec![(
                "Point".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "Point".to_string(),
                    bases: Vec::new(),
                    mro: vec!["Point".to_string()],
                    attrs: vec![("x".to_string(), Ty::Int), ("y".to_string(), Ty::Int)],
                    methods: vec![
                        ("__init__".to_string(), "Point.__init__".to_string()),
                        ("bump".to_string(), "Point.bump".to_string()),
                    ],
                    type_param: None,
                    properties: Vec::new(),
                    static_methods: Vec::new(),
                    class_methods: Vec::new(),
                    is_enum: false,
                    implicit_object_init: false,
                    method_defaults: Vec::new(),
                    enum_members: Vec::new(),
                    is_dataclass: false,
                    dataclass_fields: Vec::new(),
                    is_protocol: false,
                    runtime_checkable: false,
                    protocol_members: Vec::new(),
                    abstract_methods: Vec::new(),
                    is_abstract: false,
                },
            )],
        }
    }

    fn top_level(stmt: HirStmt) -> HirItem {
        HirItem::TopLevelStmt(stmt)
    }

    #[test]
    fn instantiation_attribute_read_and_method_call_all_type_check() {
        let hir = point_module(vec![
            top_level(HirStmt::Assign {
                target: "p".to_string(),
                value: HirExpr::Call {
                    callee: "Point".to_string(),
                    args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("p".to_string())),
                method: "bump".to_string(),
                args: vec![],
            })),
            top_level(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("p".to_string())),
                    attr: "x".to_string(),
                }],
            })),
        ]);
        check(&hir).expect(
            "a well-typed class instantiation/method-call/attribute-read program should check",
        );
    }

    #[test]
    fn instantiating_with_the_wrong_argument_count_is_rejected() {
        let hir = point_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "Point".to_string(),
            args: vec![HirExpr::IntLiteral(1)],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn instantiating_with_a_wrong_argument_type_is_rejected() {
        let hir = point_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "Point".to_string(),
            args: vec![
                HirExpr::IntLiteral(1),
                HirExpr::StringLiteral("y".to_string()),
            ],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn reading_an_attribute_on_a_non_instance_value_is_rejected() {
        let hir = point_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::AttrGet {
                base: Box::new(HirExpr::IntLiteral(1)),
                attr: "x".to_string(),
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0043");
    }

    #[test]
    fn reading_an_undeclared_attribute_is_rejected() {
        let hir = point_module(vec![
            top_level(HirStmt::Assign {
                target: "p".to_string(),
                value: HirExpr::Call {
                    callee: "Point".to_string(),
                    args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("p".to_string())),
                    attr: "z".to_string(),
                }],
            })),
        ]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0044");
    }

    #[test]
    fn calling_a_method_on_a_non_instance_value_is_rejected() {
        let hir = point_module(vec![top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
            base: Box::new(HirExpr::IntLiteral(1)),
            method: "bump".to_string(),
            args: vec![],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0043");
    }

    #[test]
    fn calling_an_undeclared_method_is_rejected() {
        let hir = point_module(vec![
            top_level(HirStmt::Assign {
                target: "p".to_string(),
                value: HirExpr::Call {
                    callee: "Point".to_string(),
                    args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("p".to_string())),
                method: "fly".to_string(),
                args: vec![],
            })),
        ]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0044");
    }

    #[test]
    fn calling_a_method_with_a_wrong_argument_count_is_rejected() {
        let hir = point_module(vec![
            top_level(HirStmt::Assign {
                target: "p".to_string(),
                value: HirExpr::Call {
                    callee: "Point".to_string(),
                    args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("p".to_string())),
                method: "bump".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            })),
        ]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn assigning_a_wrong_typed_value_to_an_attribute_is_rejected() {
        let hir = point_module(vec![
            top_level(HirStmt::Assign {
                target: "p".to_string(),
                value: HirExpr::Call {
                    callee: "Point".to_string(),
                    args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
                },
            }),
            top_level(HirStmt::AttrSet {
                base: HirExpr::Name("p".to_string()),
                attr: "x".to_string(),
                value: HirExpr::StringLiteral("nope".to_string()),
            }),
        ]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn setting_an_attribute_on_a_non_instance_value_is_rejected() {
        let hir = point_module(vec![top_level(HirStmt::AttrSet {
            base: HirExpr::IntLiteral(1),
            attr: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        })]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0043");
    }

    #[test]
    fn attribute_set_propagates_an_ill_typed_base_s_error() {
        // Exercises `check_attr_set`'s own `?` on `base`'s own inference --
        // as opposed to `setting_an_attribute_on_a_non_instance_value_is_rejected`
        // above, which only exercises `base` resolving successfully to a
        // non-instance type.
        let hir = point_module(vec![top_level(HirStmt::AttrSet {
            base: HirExpr::Name("undefined".to_string()),
            attr: "x".to_string(),
            value: HirExpr::IntLiteral(1),
        })]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn attribute_set_propagates_an_ill_typed_value_s_error() {
        // Exercises `check_attr_set`'s own `?` on `value`'s own inference.
        let hir = point_module(vec![
            top_level(HirStmt::Assign {
                target: "p".to_string(),
                value: HirExpr::Call {
                    callee: "Point".to_string(),
                    args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
                },
            }),
            top_level(HirStmt::AttrSet {
                base: HirExpr::Name("p".to_string()),
                attr: "x".to_string(),
                value: HirExpr::Name("undefined".to_string()),
            }),
        ]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn a_class_instance_is_rejected_as_a_numeric_operand() {
        // Task 3's own explicit rule: reject arithmetic on `Ty::Instance`
        // (this crate's existing D-116/D-124-style precedent), exercised
        // through the ordinary `numeric_result_type` catch-all (no
        // class-specific code needed there -- see that function's own
        // `as_numeric` closure).
        let hir = point_module(vec![
            top_level(HirStmt::Assign {
                target: "p".to_string(),
                value: HirExpr::Call {
                    callee: "Point".to_string(),
                    args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::BinOp {
                op: BinOpKind::Add,
                left: Box::new(HirExpr::Name("p".to_string())),
                right: Box::new(HirExpr::IntLiteral(1)),
            })),
        ]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn attribute_read_propagates_an_ill_typed_base_s_error() {
        // Exercises `infer_expr_in`'s own `HirExpr::AttrGet` arm's `?` on
        // `base`'s own inference -- as opposed to
        // `reading_an_attribute_on_a_non_instance_value_is_rejected` above,
        // which only exercises `base` itself resolving successfully to a
        // non-instance type.
        let hir = point_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("undefined".to_string())),
                attr: "x".to_string(),
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn method_call_propagates_an_ill_typed_base_s_error() {
        // Exercises `infer_expr_in`'s own `HirExpr::MethodCall` arm's `?`
        // on `base`'s own inference.
        let hir = point_module(vec![top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
            base: Box::new(HirExpr::Name("undefined".to_string())),
            method: "bump".to_string(),
            args: vec![],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn method_call_propagates_an_ill_typed_argument_s_error() {
        // Exercises `infer_expr_in`'s own `HirExpr::MethodCall` arm's `?`
        // on its own argument-collection loop -- distinct from the `base`
        // propagation test above.
        let hir = point_module(vec![
            top_level(HirStmt::Assign {
                target: "p".to_string(),
                value: HirExpr::Call {
                    callee: "Point".to_string(),
                    args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("p".to_string())),
                method: "bump".to_string(),
                args: vec![HirExpr::Name("undefined".to_string())],
            })),
        ]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn an_unannotated_private_method_forces_the_solver_to_walk_attribute_and_method_access() {
        // Exercises `collect_expr_constraints`'s and
        // `collect_block_constraints`'s own new `AttrGet`/`MethodCall`/
        // `AttrSet` arms (D-154): the constraint solver only runs at all
        // when at least one function in the module is not fully annotated
        // (`concrete_function_environment` returns `None`, routing `check`
        // through `infer_function_signatures_with_solver_all` instead of the
        // concrete fast path) -- every other test in this module uses only
        // fully annotated methods, so none of them exercises this path.
        // `_touch` is private (D-038: an unannotated *private* name is
        // permitted) and has no return annotation, forcing exactly that.
        let self_ty = Ty::Instance(Box::new("Point".to_string()));
        let init = HirItem::Function {
            name: "Point.__init__".to_string(),
            params: vec![
                ("self".to_string(), self_ty.clone()),
                ("x".to_string(), Ty::Int),
            ],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "x".to_string(),
                    value: HirExpr::Name("x".to_string()),
                },
                HirStmt::Return(None),
            ],
        };
        let bump = HirItem::Function {
            name: "Point.bump".to_string(),
            params: vec![("self".to_string(), self_ty.clone())],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "x".to_string(),
                    value: HirExpr::IntLiteral(0),
                },
                HirStmt::Return(None),
            ],
        };
        let touch = HirItem::Function {
            name: "Point._touch".to_string(),
            params: vec![("self".to_string(), self_ty.clone())],
            // Explicitly annotated (unlike a truly unannotated private
            // method) so the solver never needs to *infer* this function's
            // own return type from `self.x` -- `AttrGet`/`MethodCall`
            // deliberately give the solver no unification term at all
            // (mirroring `ListPop`/`Subscript`'s own pre-existing
            // consequence, see `collect_expr_constraints`'s own doc
            // comment), so a truly unannotated `_touch` returning `self.x`
            // cannot be solved. This method's own job is only to put an
            // `AttrGet`/`AttrSet`/`MethodCall` inside a body the solver
            // still *walks* (see `_identity` below for what actually
            // forces solver mode for the whole module).
            return_ty: Ty::None,
            body: vec![
                HirStmt::ExprStmt(HirExpr::MethodCall {
                    base: Box::new(HirExpr::Name("self".to_string())),
                    method: "bump".to_string(),
                    args: vec![],
                }),
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "x".to_string(),
                    value: HirExpr::IntLiteral(0),
                },
                HirStmt::ExprStmt(HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("self".to_string())),
                    attr: "x".to_string(),
                }),
                HirStmt::Return(None),
            ],
        };
        // Forces the whole module through the solver path (see this
        // test's own doc comment): a private, unannotated helper
        // completely unrelated to `Point`, mirroring this crate's own
        // existing `private_identity_signature_is_inferred_from_its_call_site_and_return`
        // precedent -- its own return type is inferred as `int` from its
        // one call site's argument and its own `return x`.
        let identity = HirItem::Function {
            name: "_identity".to_string(),
            params: vec![("x".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        };
        let hir = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![
                init,
                bump,
                touch,
                identity,
                HirItem::TopLevelStmt(HirStmt::ExprStmt(HirExpr::Call {
                    callee: "_identity".to_string(),
                    args: vec![HirExpr::IntLiteral(1)],
                })),
            ],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: vec![(
                "Point".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "Point".to_string(),
                    bases: Vec::new(),
                    mro: vec!["Point".to_string()],
                    attrs: vec![("x".to_string(), Ty::Int)],
                    methods: vec![
                        ("__init__".to_string(), "Point.__init__".to_string()),
                        ("bump".to_string(), "Point.bump".to_string()),
                        ("_touch".to_string(), "Point._touch".to_string()),
                    ],
                    type_param: None,
                    properties: Vec::new(),
                    static_methods: Vec::new(),
                    class_methods: Vec::new(),
                    is_enum: false,
                    implicit_object_init: false,
                    method_defaults: Vec::new(),
                    enum_members: Vec::new(),
                    is_dataclass: false,
                    dataclass_fields: Vec::new(),
                    is_protocol: false,
                    runtime_checkable: false,
                    protocol_members: Vec::new(),
                    abstract_methods: Vec::new(),
                    is_abstract: false,
                },
            )],
        };
        check(&hir).expect(
            "a class method reading/writing an instance attribute and calling another \
             method should check when an unrelated unannotated helper forces the solver \
             path for the whole module",
        );
    }

    #[test]
    fn solver_path_attr_get_propagates_an_ill_typed_base_s_error() {
        // Exercises `collect_expr_constraints`'s own `HirExpr::AttrGet`
        // arm's `?` on `base`'s own constraint collection -- as opposed to
        // `an_unannotated_private_method_forces_the_solver_to_walk_attribute_and_method_access`
        // above, which only exercises `base` collecting successfully. `base`
        // must be a name that is genuinely a *local* referenced before its
        // own first assignment (`collect_expr_constraints`'s `HirExpr::Name`
        // arm only errors for `is_local`-registered names -- an entirely
        // undeclared name silently resolves to `Ok(None)`, never reaching
        // this `?` at all).
        let self_ty = Ty::Instance(Box::new("Point".to_string()));
        let init = HirItem::Function {
            name: "Point.__init__".to_string(),
            params: vec![("self".to_string(), self_ty.clone())],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "x".to_string(),
                    value: HirExpr::IntLiteral(0),
                },
                HirStmt::Return(None),
            ],
        };
        let bad = HirItem::Function {
            name: "_bad".to_string(),
            params: vec![("y".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::ExprStmt(HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("z".to_string())),
                    attr: "x".to_string(),
                }),
                HirStmt::Assign {
                    target: "z".to_string(),
                    value: HirExpr::IntLiteral(0),
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        };
        let hir = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![init, bad],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: vec![(
                "Point".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "Point".to_string(),
                    bases: Vec::new(),
                    mro: vec!["Point".to_string()],
                    attrs: vec![("x".to_string(), Ty::Int)],
                    methods: vec![("__init__".to_string(), "Point.__init__".to_string())],
                    type_param: None,
                    properties: Vec::new(),
                    static_methods: Vec::new(),
                    class_methods: Vec::new(),
                    is_enum: false,
                    implicit_object_init: false,
                    method_defaults: Vec::new(),
                    enum_members: Vec::new(),
                    is_dataclass: false,
                    dataclass_fields: Vec::new(),
                    is_protocol: false,
                    runtime_checkable: false,
                    protocol_members: Vec::new(),
                    abstract_methods: Vec::new(),
                    is_abstract: false,
                },
            )],
        };
        assert!(check(&hir).is_err());
    }

    #[test]
    fn solver_path_method_call_propagates_an_ill_typed_base_and_argument_error() {
        // Exercises `collect_expr_constraints`'s own `HirExpr::MethodCall`
        // arm's `?` on both `base`'s own constraint collection and its
        // per-argument loop's. As in
        // `solver_path_attr_get_propagates_an_ill_typed_base_s_error` above,
        // the erroring name must be a genuine local referenced before its
        // own first assignment -- an entirely undeclared name resolves to
        // `Ok(None)` and never reaches either `?`.
        let bad_base = HirItem::Function {
            name: "_bad_base".to_string(),
            params: vec![("y".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::ExprStmt(HirExpr::MethodCall {
                    base: Box::new(HirExpr::Name("z".to_string())),
                    method: "bump".to_string(),
                    args: vec![],
                }),
                HirStmt::Assign {
                    target: "z".to_string(),
                    value: HirExpr::IntLiteral(0),
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        };
        let hir1 = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![bad_base],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: Vec::new(),
        };
        assert!(check(&hir1).is_err());

        let bad_arg = HirItem::Function {
            name: "_bad_arg".to_string(),
            params: vec![("p".to_string(), Ty::Infer), ("y".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::ExprStmt(HirExpr::MethodCall {
                    base: Box::new(HirExpr::Name("p".to_string())),
                    method: "bump".to_string(),
                    args: vec![HirExpr::Name("z".to_string())],
                }),
                HirStmt::Assign {
                    target: "z".to_string(),
                    value: HirExpr::IntLiteral(0),
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        };
        let hir2 = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![bad_arg],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: Vec::new(),
        };
        assert!(check(&hir2).is_err());
    }

    #[test]
    fn solver_path_attr_set_propagates_an_ill_typed_base_and_value_error() {
        // Exercises `collect_block_constraints`'s own `HirStmt::AttrSet`
        // arm's `?` on both `base`'s and `value`'s own constraint
        // collection. As in the `AttrGet`/`MethodCall` solver-path tests
        // above, the erroring name must be a genuine local referenced
        // before its own first assignment -- an entirely undeclared name
        // resolves to `Ok(None)` and never reaches either `?`.
        let bad_base = HirItem::Function {
            name: "_bad_base".to_string(),
            params: vec![("y".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("z".to_string()),
                    attr: "x".to_string(),
                    value: HirExpr::IntLiteral(0),
                },
                HirStmt::Assign {
                    target: "z".to_string(),
                    value: HirExpr::IntLiteral(0),
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        };
        let hir1 = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![bad_base],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: Vec::new(),
        };
        assert!(check(&hir1).is_err());

        let bad_value = HirItem::Function {
            name: "_bad_value".to_string(),
            params: vec![("p".to_string(), Ty::Infer), ("y".to_string(), Ty::Infer)],
            return_ty: Ty::Infer,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("p".to_string()),
                    attr: "x".to_string(),
                    value: HirExpr::Name("z".to_string()),
                },
                HirStmt::Assign {
                    target: "z".to_string(),
                    value: HirExpr::IntLiteral(0),
                },
                HirStmt::Return(Some(HirExpr::Name("y".to_string()))),
            ],
        };
        let hir2 = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![bad_value],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: Vec::new(),
        };
        assert!(check(&hir2).is_err());
    }

    #[test]
    fn a_generic_function_body_containing_attribute_and_method_access_type_checks() {
        // Exercises `reject_generic_calls_in_stmt/expr`'s and
        // `rewrite_generic_calls_in_stmt/expr`'s own new `AttrSet`/
        // `AttrGet`/`MethodCall` arms (D-154): none of this module's other
        // tests combine a class with a PEP 695 generic function, so a
        // generic function whose own body reads/writes an instance
        // attribute and calls a method is needed to walk those recursive-
        // descent helpers into these new node shapes at all.
        // `check_and_resolve` exercises both: `checked_function_signatures_all`
        // routes `helper` through `check_generic_function_in` (which calls
        // `reject_generic_calls_in_stmt` to reject self-recursion).
        // `monomorphize`'s own Pass 2, however, explicitly *skips* a generic
        // function's own body (`if generics.contains_key(name) { continue; }`
        // -- only a call *site* that instantiates a generic gets its
        // `substitute_body`-produced specialization, which is appended
        // as-is without ever being re-walked by `rewrite_generic_calls_in_stmt`
        // itself), so `helper`'s own `AttrSet`/`MethodCall`/`AttrGet` nodes
        // never reach `rewrite_generic_calls_in_stmt/expr` at all. `use_counter`
        // below is the ordinary (non-generic) twin of `helper`'s body,
        // existing purely so Pass 2 actually walks this exact node shape
        // -- `helper` itself still exists to keep `monomorphize`'s early
        // "no generics" return from short-circuiting the whole pass, and to
        // keep exercising `reject_generic_calls_in_stmt/expr`'s own walk of
        // a generic function's body.
        // A standalone `Counter` class, not `point_module`'s shared
        // `Point` fixture: `add` takes a real `n: int` argument, needed to
        // exercise `reject_generic_calls_in_expr`'s/
        // `rewrite_generic_calls_in_expr`'s own `MethodCall` arm's
        // per-argument loop body at all -- `point_module`'s own `bump`
        // takes no arguments.
        let counter_ty = Ty::Instance(Box::new("Counter".to_string()));
        let counter_init = HirItem::Function {
            name: "Counter.__init__".to_string(),
            params: vec![("self".to_string(), counter_ty.clone())],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "n".to_string(),
                    value: HirExpr::IntLiteral(0),
                },
                HirStmt::Return(None),
            ],
        };
        let counter_add = HirItem::Function {
            name: "Counter.add".to_string(),
            params: vec![
                ("self".to_string(), counter_ty.clone()),
                ("n".to_string(), Ty::Int),
            ],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "n".to_string(),
                    value: HirExpr::Name("n".to_string()),
                },
                HirStmt::Return(None),
            ],
        };
        let helper = HirItem::Function {
            name: "helper".to_string(),
            params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
            return_ty: Ty::Param(Box::new("T".to_string())),
            body: vec![
                HirStmt::Assign {
                    target: "c".to_string(),
                    value: HirExpr::Call {
                        callee: "Counter".to_string(),
                        args: vec![],
                    },
                },
                HirStmt::AttrSet {
                    base: HirExpr::Name("c".to_string()),
                    attr: "n".to_string(),
                    value: HirExpr::IntLiteral(1),
                },
                HirStmt::ExprStmt(HirExpr::MethodCall {
                    base: Box::new(HirExpr::Name("c".to_string())),
                    method: "add".to_string(),
                    args: vec![HirExpr::IntLiteral(5)],
                }),
                HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::AttrGet {
                        base: Box::new(HirExpr::Name("c".to_string())),
                        attr: "n".to_string(),
                    }],
                }),
                HirStmt::Return(Some(HirExpr::Name("x".to_string()))),
            ],
        };
        let use_counter = HirItem::Function {
            name: "use_counter".to_string(),
            params: vec![("c".to_string(), counter_ty.clone())],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("c".to_string()),
                    attr: "n".to_string(),
                    value: HirExpr::IntLiteral(2),
                },
                HirStmt::ExprStmt(HirExpr::MethodCall {
                    base: Box::new(HirExpr::Name("c".to_string())),
                    method: "add".to_string(),
                    args: vec![HirExpr::IntLiteral(5)],
                }),
                HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::AttrGet {
                        base: Box::new(HirExpr::Name("c".to_string())),
                        attr: "n".to_string(),
                    }],
                }),
                HirStmt::Return(None),
            ],
        };
        let mut hir = point_module(vec![
            counter_init,
            counter_add,
            helper,
            use_counter,
            top_level(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Call {
                    callee: "helper".to_string(),
                    args: vec![HirExpr::IntLiteral(5)],
                }],
            })),
        ]);
        hir.class_defs.push((
            "Counter".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Counter".to_string(),
                bases: Vec::new(),
                mro: vec!["Counter".to_string()],
                attrs: vec![("n".to_string(), Ty::Int)],
                methods: vec![
                    ("__init__".to_string(), "Counter.__init__".to_string()),
                    ("add".to_string(), "Counter.add".to_string()),
                ],
                type_param: None,
                properties: Vec::new(),
                static_methods: Vec::new(),
                class_methods: Vec::new(),
                is_enum: false,
                implicit_object_init: false,
                method_defaults: Vec::new(),
                enum_members: Vec::new(),
                is_dataclass: false,
                dataclass_fields: Vec::new(),
                is_protocol: false,
                runtime_checkable: false,
                protocol_members: Vec::new(),
                abstract_methods: Vec::new(),
                is_abstract: false,
            },
        ));
        check_and_resolve(&hir).expect(
            "a well-typed generic function body using class instance attribute/method \
             access should check",
        );
    }

    #[test]
    fn reject_generic_calls_in_expr_propagates_a_method_call_s_base_and_argument_errors() {
        // Exercises `reject_generic_calls_in_expr`'s own `HirExpr::MethodCall`
        // arm's `?` on both `base`'s own rejection walk and its
        // per-argument loop's -- as opposed to
        // `a_generic_function_body_containing_attribute_and_method_access_type_checks`
        // above, which only exercises both succeeding. A self-recursive
        // call nested inside a `MethodCall`'s `base` (first case) or one of
        // its `args` (second case) is rejected by `reject_generic_calls_in_expr`'s
        // own `HirExpr::Call` arm and must propagate back up through
        // `MethodCall`'s own two recursive positions. Neither case needs a
        // real class -- `reject_generic_calls_in_expr` never resolves
        // `MethodCall`'s own `base`/`method` against any `Environment`,
        // only walks the expression tree structurally.
        let bad_base = HirItem::Function {
            name: "bad_base".to_string(),
            params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
            return_ty: Ty::Param(Box::new("T".to_string())),
            body: vec![HirStmt::Return(Some(HirExpr::MethodCall {
                base: Box::new(HirExpr::Call {
                    callee: "bad_base".to_string(),
                    args: vec![HirExpr::Name("x".to_string())],
                }),
                method: "whatever".to_string(),
                args: vec![],
            }))],
        };
        let hir1 = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![bad_base],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: Vec::new(),
        };
        assert_eq!(check(&hir1).unwrap_err().code, "T0042");

        let bad_arg = HirItem::Function {
            name: "bad_arg".to_string(),
            params: vec![("x".to_string(), Ty::Param(Box::new("T".to_string())))],
            return_ty: Ty::Param(Box::new("T".to_string())),
            body: vec![HirStmt::Return(Some(HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("x".to_string())),
                method: "whatever".to_string(),
                args: vec![HirExpr::Call {
                    callee: "bad_arg".to_string(),
                    args: vec![HirExpr::Name("x".to_string())],
                }],
            }))],
        };
        let hir2 = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![bad_arg],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: Vec::new(),
        };
        assert_eq!(check(&hir2).unwrap_err().code, "T0042");
    }

    // -- internal-consistency panics ----------------------------------------
    //
    // Every test below bypasses the normal `check`/`check_and_resolve`
    // entry points, building an inconsistent `Environment` by hand (a
    // `Ty::Instance` payload naming a class the `Environment` was never
    // told about, or a class whose `__init__`/method was never registered
    // as an ordinary function) -- exactly the "a class's declared shape and
    // its `Environment` disagree" scenario each of these functions' own doc
    // comments name as unreachable from any real `check`-validated program,
    // mirroring `pycc_mir`'s own established convention for the identical
    // kind of internal-consistency panic (see e.g. that crate's
    // `referencing_an_unbound_name_panics_with_an_internal_error` test).

    #[test]
    #[should_panic(expected = "class `Ghost` has no registered HirClassDef")]
    fn resolve_attr_get_panics_when_the_class_is_not_registered() {
        let env = crate::Environment::new();
        let _ = super::resolve_attr_get(&env, &Ty::Instance(Box::new("Ghost".to_string())), "x");
    }

    #[test]
    #[should_panic(expected = "was not registered as an ordinary function")]
    fn resolve_attr_get_panics_when_a_property_getter_is_not_registered() {
        // #377: a property's getter is in the class's own property table
        // but was never registered in `Environment::functions` -- the
        // "declared shape and Environment disagree" scenario
        // `resolve_attr_get`'s own doc comment names as unreachable from
        // any real `check`-validated program, mirroring
        // `resolve_instantiation_panics_when_init_is_not_registered` above.
        use pycc_hir::PropertyDef;
        let mut env = crate::Environment::new();
        env.bind_class(
            "Ghost".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Ghost".to_string(),
                bases: Vec::new(),
                mro: vec!["Ghost".to_string()],
                attrs: vec![],
                methods: vec![("__init__".to_string(), "Ghost.__init__".to_string())],
                type_param: None,
                properties: vec![PropertyDef {
                    name: "x".to_string(),
                    getter: "Ghost.x".to_string(),
                    setter: None,
                }],
                static_methods: Vec::new(),
                class_methods: Vec::new(),
                is_enum: false,
                implicit_object_init: false,
                method_defaults: Vec::new(),
                enum_members: Vec::new(),
                is_dataclass: false,
                dataclass_fields: Vec::new(),
                is_protocol: false,
                runtime_checkable: false,
                protocol_members: Vec::new(),
                abstract_methods: Vec::new(),
                is_abstract: false,
            },
        );
        let _ = super::resolve_attr_get(&env, &Ty::Instance(Box::new("Ghost".to_string())), "x");
    }

    #[test]
    #[should_panic(expected = "was not registered as an ordinary function")]
    fn check_attr_set_panics_when_a_property_setter_is_not_registered() {
        // #377: a property's setter is in the class's own property table
        // but was never registered in `Environment::functions` -- the
        // "declared shape and Environment disagree" scenario
        // `check_attr_set`'s own doc comment names as unreachable from
        // any real `check`-validated program, mirroring
        // `class/method_call.rs`'s
        // `resolve_method_call_panics_when_the_method_is_not_registered`
        // (moved there by #815, Part 1 of #737).
        use pycc_hir::PropertyDef;
        let mut env = crate::Environment::new();
        env.bind_class(
            "Ghost".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Ghost".to_string(),
                bases: Vec::new(),
                mro: vec!["Ghost".to_string()],
                attrs: vec![],
                methods: vec![("__init__".to_string(), "Ghost.__init__".to_string())],
                type_param: None,
                properties: vec![PropertyDef {
                    name: "x".to_string(),
                    getter: "Ghost.x".to_string(),
                    setter: Some("Ghost.x.setter".to_string()),
                }],
                static_methods: Vec::new(),
                class_methods: Vec::new(),
                is_enum: false,
                implicit_object_init: false,
                method_defaults: Vec::new(),
                enum_members: Vec::new(),
                is_dataclass: false,
                dataclass_fields: Vec::new(),
                is_protocol: false,
                runtime_checkable: false,
                protocol_members: Vec::new(),
                abstract_methods: Vec::new(),
                is_abstract: false,
            },
        );
        // `base` must infer as a `Ghost` instance so `check_attr_set`
        // reaches the property branch; `value` must infer successfully so
        // the `?` on `infer_expr_in` (line 228) does not short-circuit
        // before the setter lookup panic.
        env.bind_function(
            "Ghost.__init__".to_string(),
            vec![Ty::Instance(Box::new("Ghost".to_string()))],
            Ty::None,
        );
        env.bind("b".to_string(), Ty::Instance(Box::new("Ghost".to_string())));
        let _ = super::check_attr_set(
            &env,
            &[],
            &HirExpr::Name("b".to_string()),
            "x",
            &HirExpr::IntLiteral(42),
        );
    }

    #[test]
    fn cast_compatibility_treats_an_unregistered_from_class_as_a_representation_mismatch() {
        // #767 (D-198, third pass): every `Ty::Instance` `cast_compatibility`
        // sees from a real `check`-validated program names a class the
        // `Environment` was told about -- either an ordinary user class or
        // a seeded builtin exception class (see
        // `crate::exception::is_user_defined_class`'s doc comment). This
        // directly exercises the function's own defensive fallback for the
        // "declared shape and Environment disagree" state that scenario
        // rules out from any real caller, matching the internal-consistency
        // convention above.
        let env = crate::Environment::new();
        let mismatch = super::cast_compatibility(
            &env,
            &Ty::Instance(Box::new("Ghost".to_string())),
            &Ty::Instance(Box::new("AlsoGhost".to_string())),
        )
        .unwrap_err();
        assert_eq!(mismatch, super::CastMismatch::Representation);
    }

    #[test]
    fn cast_compatibility_treats_an_mro_entry_missing_its_own_class_def_as_a_layout_mismatch() {
        // Same "declared shape and Environment disagree" scenario as above,
        // but for the ancestor lookup instead of the value's own class:
        // `from`'s own MRO names a class that isn't registered. Normal class
        // registration (`validate_bases` in `pycc_hir::class::mro`) only
        // ever admits an already-defined class into a base's MRO, so this
        // state cannot arise from any real `check`-validated program either.
        let mut env = crate::Environment::new();
        env.bind_class(
            "Derived".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Derived".to_string(),
                bases: vec!["GhostBase".to_string()],
                mro: vec!["Derived".to_string(), "GhostBase".to_string()],
                attrs: vec![],
                methods: vec![],
                type_param: None,
                properties: Vec::new(),
                static_methods: Vec::new(),
                class_methods: Vec::new(),
                is_enum: false,
                implicit_object_init: false,
                method_defaults: Vec::new(),
                enum_members: Vec::new(),
                is_dataclass: false,
                dataclass_fields: Vec::new(),
                is_protocol: false,
                runtime_checkable: false,
                protocol_members: Vec::new(),
                abstract_methods: Vec::new(),
                is_abstract: false,
            },
        );
        let mismatch = super::cast_compatibility(
            &env,
            &Ty::Instance(Box::new("Derived".to_string())),
            &Ty::Instance(Box::new("GhostBase".to_string())),
        )
        .unwrap_err();
        assert_eq!(mismatch, super::CastMismatch::Layout);
    }

    // -- @property type checking (#377) -------------------------------------

    /// Builds a `Box` class with a read-write `@property` `val` backed by
    /// the `_val` slot, plus `extra_items`/`extra_stmts` for each test's
    /// own exercise. The getter returns `self._val` (int); the setter
    /// accepts an `int` and stores it.
    fn property_module(extra_items: Vec<HirItem>) -> HirModule {
        use pycc_hir::PropertyDef;
        let self_ty = Ty::Instance(Box::new("Box".to_string()));
        let init = HirItem::Function {
            name: "Box.__init__".to_string(),
            params: vec![("self".to_string(), self_ty.clone())],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "_val".to_string(),
                    value: HirExpr::IntLiteral(0),
                },
                HirStmt::Return(None),
            ],
        };
        let getter = HirItem::Function {
            name: "Box.val".to_string(),
            params: vec![("self".to_string(), self_ty.clone())],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("self".to_string())),
                attr: "_val".to_string(),
            }))],
        };
        let setter = HirItem::Function {
            name: "Box.val.setter".to_string(),
            params: vec![
                ("self".to_string(), self_ty.clone()),
                ("v".to_string(), Ty::Int),
            ],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "_val".to_string(),
                    value: HirExpr::Name("v".to_string()),
                },
                HirStmt::Return(None),
            ],
        };
        let mut items = vec![init, getter, setter];
        items.extend(extra_items);
        HirModule {
            seeded_builtin_exception_classes: false,
            items,
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: vec![(
                "Box".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "Box".to_string(),
                    bases: Vec::new(),
                    mro: vec!["Box".to_string()],
                    attrs: vec![("_val".to_string(), Ty::Int)],
                    methods: vec![("__init__".to_string(), "Box.__init__".to_string())],
                    properties: vec![PropertyDef {
                        name: "val".to_string(),
                        getter: "Box.val".to_string(),
                        setter: Some("Box.val.setter".to_string()),
                    }],
                    type_param: None,
                    static_methods: Vec::new(),
                    class_methods: Vec::new(),
                    is_enum: false,
                    implicit_object_init: false,
                    method_defaults: Vec::new(),
                    enum_members: Vec::new(),
                    is_dataclass: false,
                    dataclass_fields: Vec::new(),
                    is_protocol: false,
                    runtime_checkable: false,
                    protocol_members: Vec::new(),
                    abstract_methods: Vec::new(),
                    is_abstract: false,
                },
            )],
        }
    }

    /// Like `property_module` but the property has no setter (read-only).
    fn read_only_property_module(extra_items: Vec<HirItem>) -> HirModule {
        use pycc_hir::PropertyDef;
        let self_ty = Ty::Instance(Box::new("Box".to_string()));
        let init = HirItem::Function {
            name: "Box.__init__".to_string(),
            params: vec![("self".to_string(), self_ty.clone())],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "_val".to_string(),
                    value: HirExpr::IntLiteral(0),
                },
                HirStmt::Return(None),
            ],
        };
        let getter = HirItem::Function {
            name: "Box.val".to_string(),
            params: vec![("self".to_string(), self_ty)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("self".to_string())),
                attr: "_val".to_string(),
            }))],
        };
        let mut items = vec![init, getter];
        items.extend(extra_items);
        HirModule {
            seeded_builtin_exception_classes: false,
            items,
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: vec![(
                "Box".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "Box".to_string(),
                    bases: Vec::new(),
                    mro: vec!["Box".to_string()],
                    attrs: vec![("_val".to_string(), Ty::Int)],
                    methods: vec![("__init__".to_string(), "Box.__init__".to_string())],
                    properties: vec![PropertyDef {
                        name: "val".to_string(),
                        getter: "Box.val".to_string(),
                        setter: None,
                    }],
                    type_param: None,
                    static_methods: Vec::new(),
                    class_methods: Vec::new(),
                    is_enum: false,
                    implicit_object_init: false,
                    method_defaults: Vec::new(),
                    enum_members: Vec::new(),
                    is_dataclass: false,
                    dataclass_fields: Vec::new(),
                    is_protocol: false,
                    runtime_checkable: false,
                    protocol_members: Vec::new(),
                    abstract_methods: Vec::new(),
                    is_abstract: false,
                },
            )],
        }
    }

    #[test]
    fn a_property_getter_read_type_checks() {
        let hir = property_module(vec![
            top_level(HirStmt::Assign {
                target: "b".to_string(),
                value: HirExpr::Call {
                    callee: "Box".to_string(),
                    args: vec![],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("b".to_string())),
                    attr: "val".to_string(),
                }],
            })),
        ]);
        check(&hir).expect("a property getter read should type-check");
    }

    #[test]
    fn a_property_setter_assignment_type_checks() {
        let hir = property_module(vec![
            top_level(HirStmt::Assign {
                target: "b".to_string(),
                value: HirExpr::Call {
                    callee: "Box".to_string(),
                    args: vec![],
                },
            }),
            top_level(HirStmt::AttrSet {
                base: HirExpr::Name("b".to_string()),
                attr: "val".to_string(),
                value: HirExpr::IntLiteral(42),
            }),
        ]);
        check(&hir).expect("a property setter assignment should type-check");
    }

    #[test]
    fn a_read_only_property_assignment_is_rejected() {
        let hir = read_only_property_module(vec![
            top_level(HirStmt::Assign {
                target: "b".to_string(),
                value: HirExpr::Call {
                    callee: "Box".to_string(),
                    args: vec![],
                },
            }),
            top_level(HirStmt::AttrSet {
                base: HirExpr::Name("b".to_string()),
                attr: "val".to_string(),
                value: HirExpr::IntLiteral(42),
            }),
        ]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0044");
        assert!(
            diagnostic.message.contains("read-only"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_property_setter_type_mismatch_is_rejected() {
        let hir = property_module(vec![
            top_level(HirStmt::Assign {
                target: "b".to_string(),
                value: HirExpr::Call {
                    callee: "Box".to_string(),
                    args: vec![],
                },
            }),
            top_level(HirStmt::AttrSet {
                base: HirExpr::Name("b".to_string()),
                attr: "val".to_string(),
                value: HirExpr::StringLiteral("nope".to_string()),
            }),
        ]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn a_property_setter_assignment_with_an_ill_typed_value_propagates_the_value_error() {
        // Exercises `check_attr_set`'s `?` on `infer_expr_in` for the
        // *value* expression (line 228) -- distinct from
        // `a_property_setter_type_mismatch_is_rejected` above, where the
        // value infers successfully (`Ty::Str`) and the rejection happens
        // later at the `is_assignable` check (line 246). Here the value is
        // an undefined name, so `infer_expr_in` itself returns `Err`
        // before the setter's parameter type is ever consulted.
        let hir = property_module(vec![
            top_level(HirStmt::Assign {
                target: "b".to_string(),
                value: HirExpr::Call {
                    callee: "Box".to_string(),
                    args: vec![],
                },
            }),
            top_level(HirStmt::AttrSet {
                base: HirExpr::Name("b".to_string()),
                attr: "val".to_string(),
                value: HirExpr::Name("undefined_name".to_string()),
            }),
        ]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
        assert!(
            diagnostic.message.contains("undefined_name"),
            "unexpected message: {}",
            diagnostic.message
        );
    }

    #[test]
    fn a_property_getter_read_inside_a_method_body_type_checks() {
        // A method that reads the property via `self.val` -- exercises
        // `resolve_attr_get`'s property check from within a function body
        // (pass 3), not just top-level (pass 2).
        let self_ty = Ty::Instance(Box::new("Box".to_string()));
        let reader = HirItem::Function {
            name: "Box.read_val".to_string(),
            params: vec![("self".to_string(), self_ty)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("self".to_string())),
                attr: "val".to_string(),
            }))],
        };
        let mut hir = property_module(vec![]);
        // Add the reader method to items and to the class's method table.
        // `.expect(...)`, not `if let Some(...)` -- the latter's implicit
        // else (the no-match arm) is its own hand-written region, never
        // executed because `property_module` always defines `Box` -- this
        // crate's own established coverage-gate convention (see
        // `lower_ok`'s own doc comment in `pycc_hir::class::tests`) is
        // `.expect()`, whose panic path lives in libcore, outside this
        // crate's instrumented regions.
        hir.items.push(reader);
        let (_, cd) = hir
            .class_defs
            .iter_mut()
            .find(|(n, _)| n == "Box")
            .expect("property_module always defines Box");
        cd.methods
            .push(("read_val".to_string(), "Box.read_val".to_string()));
        hir.items.push(top_level(HirStmt::Assign {
            target: "b".to_string(),
            value: HirExpr::Call {
                callee: "Box".to_string(),
                args: vec![],
            },
        }));
        hir.items.push(top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("b".to_string())),
                method: "read_val".to_string(),
                args: vec![],
            }],
        })));
        check(&hir).expect("a property read inside a method body should type-check");
    }

    #[test]
    fn a_property_setter_assignment_inside_a_method_body_type_checks() {
        // A method that writes the property via `self.val = v` -- exercises
        // `check_attr_set`'s property check from within a function body.
        let self_ty = Ty::Instance(Box::new("Box".to_string()));
        let writer = HirItem::Function {
            name: "Box.write_val".to_string(),
            params: vec![("self".to_string(), self_ty), ("v".to_string(), Ty::Int)],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "val".to_string(),
                    value: HirExpr::Name("v".to_string()),
                },
                HirStmt::Return(None),
            ],
        };
        let mut hir = property_module(vec![]);
        hir.items.push(writer);
        // `.expect(...)`, not `if let Some(...)` -- see the sibling test
        // `a_property_getter_read_inside_a_method_body_type_checks` above
        // for the coverage-gate rationale.
        let (_, cd) = hir
            .class_defs
            .iter_mut()
            .find(|(n, _)| n == "Box")
            .expect("property_module always defines Box");
        cd.methods
            .push(("write_val".to_string(), "Box.write_val".to_string()));
        hir.items.push(top_level(HirStmt::Assign {
            target: "b".to_string(),
            value: HirExpr::Call {
                callee: "Box".to_string(),
                args: vec![],
            },
        }));
        hir.items
            .push(top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("b".to_string())),
                method: "write_val".to_string(),
                args: vec![HirExpr::IntLiteral(99)],
            })));
        check(&hir).expect("a property write inside a method body should type-check");
    }

    #[test]
    fn a_property_getter_read_resolves_through_check_and_resolve() {
        // Exercises the full `check_and_resolve` → MIR pipeline for a
        // property getter read, ensuring the MIR lowering's property
        // rewrite (AttrGet → Call) produces valid MIR.
        let hir = property_module(vec![
            top_level(HirStmt::Assign {
                target: "b".to_string(),
                value: HirExpr::Call {
                    callee: "Box".to_string(),
                    args: vec![],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("b".to_string())),
                    attr: "val".to_string(),
                }],
            })),
        ]);
        let resolved = check_and_resolve(&hir).expect("check_and_resolve should succeed");
        // Build MIR from the resolved HIR -- this exercises the MIR
        // lowering's property rewrite (AttrGet → MirExpr::Call).
        let _mir = pycc_mir::build(&resolved);
    }

    #[test]
    fn a_property_setter_assignment_resolves_through_check_and_resolve() {
        // Exercises the full `check_and_resolve` → MIR pipeline for a
        // property setter assignment, ensuring the MIR lowering's property
        // rewrite (AttrSet → ExprStmt(Call)) produces valid MIR.
        let hir = property_module(vec![
            top_level(HirStmt::Assign {
                target: "b".to_string(),
                value: HirExpr::Call {
                    callee: "Box".to_string(),
                    args: vec![],
                },
            }),
            top_level(HirStmt::AttrSet {
                base: HirExpr::Name("b".to_string()),
                attr: "val".to_string(),
                value: HirExpr::IntLiteral(42),
            }),
        ]);
        let resolved = check_and_resolve(&hir).expect("check_and_resolve should succeed");
        let _mir = pycc_mir::build(&resolved);
    }

    // -- #432: MRO-aware type checking --------------------------------------

    /// Builds an `Animal` base class with a `name` attribute and a `speak`
    /// method, plus a `Dog` derived class with its own `speak` override,
    /// plus `extra_items` for each test's own exercise.
    fn inheritance_module(extra_items: Vec<HirItem>) -> HirModule {
        let animal_ty = Ty::Instance(Box::new("Animal".to_string()));
        let dog_ty = Ty::Instance(Box::new("Dog".to_string()));
        let animal_init = HirItem::Function {
            name: "Animal.__init__".to_string(),
            params: vec![
                ("self".to_string(), animal_ty.clone()),
                ("name".to_string(), Ty::Str),
            ],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "name".to_string(),
                    value: HirExpr::Name("name".to_string()),
                },
                HirStmt::Return(None),
            ],
        };
        let animal_speak = HirItem::Function {
            name: "Animal.speak".to_string(),
            params: vec![("self".to_string(), animal_ty.clone())],
            return_ty: Ty::Str,
            body: vec![HirStmt::Return(Some(HirExpr::StringLiteral(
                "...".to_string(),
            )))],
        };
        let dog_init = HirItem::Function {
            name: "Dog.__init__".to_string(),
            params: vec![
                ("self".to_string(), dog_ty.clone()),
                ("name".to_string(), Ty::Str),
            ],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "name".to_string(),
                    value: HirExpr::Name("name".to_string()),
                },
                HirStmt::Return(None),
            ],
        };
        let dog_speak = HirItem::Function {
            name: "Dog.speak".to_string(),
            params: vec![("self".to_string(), dog_ty.clone())],
            return_ty: Ty::Str,
            body: vec![HirStmt::Return(Some(HirExpr::StringLiteral(
                "Woof".to_string(),
            )))],
        };
        let mut items = vec![animal_init, animal_speak, dog_init, dog_speak];
        items.extend(extra_items);
        HirModule {
            seeded_builtin_exception_classes: false,
            items,
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: vec![
                (
                    "Animal".to_string(),
                    HirClassDef {
                        class_attrs: Vec::new(),
                        exception_type_tag: None,
                        name: "Animal".to_string(),
                        bases: Vec::new(),
                        mro: vec!["Animal".to_string()],
                        attrs: vec![("name".to_string(), Ty::Str)],
                        methods: vec![
                            ("__init__".to_string(), "Animal.__init__".to_string()),
                            ("speak".to_string(), "Animal.speak".to_string()),
                        ],
                        type_param: None,
                        properties: Vec::new(),
                        static_methods: Vec::new(),
                        class_methods: Vec::new(),
                        is_enum: false,
                        implicit_object_init: false,
                        method_defaults: Vec::new(),
                        enum_members: Vec::new(),
                        is_dataclass: false,
                        dataclass_fields: Vec::new(),
                        is_protocol: false,
                        runtime_checkable: false,
                        protocol_members: Vec::new(),
                        abstract_methods: Vec::new(),
                        is_abstract: false,
                    },
                ),
                (
                    "Dog".to_string(),
                    HirClassDef {
                        class_attrs: Vec::new(),
                        exception_type_tag: None,
                        name: "Dog".to_string(),
                        bases: vec!["Animal".to_string()],
                        mro: vec!["Dog".to_string(), "Animal".to_string()],
                        attrs: vec![("name".to_string(), Ty::Str)],
                        methods: vec![
                            ("__init__".to_string(), "Dog.__init__".to_string()),
                            ("speak".to_string(), "Dog.speak".to_string()),
                        ],
                        type_param: None,
                        properties: Vec::new(),
                        static_methods: Vec::new(),
                        class_methods: Vec::new(),
                        is_enum: false,
                        implicit_object_init: false,
                        method_defaults: Vec::new(),
                        enum_members: Vec::new(),
                        is_dataclass: false,
                        dataclass_fields: Vec::new(),
                        is_protocol: false,
                        runtime_checkable: false,
                        protocol_members: Vec::new(),
                        abstract_methods: Vec::new(),
                        is_abstract: false,
                    },
                ),
            ],
        }
    }

    #[test]
    fn derived_class_inherits_base_attribute_through_mro() {
        // `d.name` on a `Dog` instance resolves `name` through the MRO --
        // `Dog` declares it, so it's found on the first MRO entry.
        let hir = inheritance_module(vec![
            top_level(HirStmt::Assign {
                target: "d".to_string(),
                value: HirExpr::Call {
                    callee: "Dog".to_string(),
                    args: vec![HirExpr::StringLiteral("Rex".to_string())],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("d".to_string())),
                    attr: "name".to_string(),
                }],
            })),
        ]);
        check(&hir).expect("inherited attribute read should type-check");
    }

    #[test]
    fn derived_class_method_call_resolves_through_mro() {
        // `d.speak()` on a `Dog` instance resolves to `Dog.speak` (the
        // override), not `Animal.speak`.
        let hir = inheritance_module(vec![
            top_level(HirStmt::Assign {
                target: "d".to_string(),
                value: HirExpr::Call {
                    callee: "Dog".to_string(),
                    args: vec![HirExpr::StringLiteral("Rex".to_string())],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("d".to_string())),
                method: "speak".to_string(),
                args: vec![],
            })),
        ]);
        check(&hir).expect("method call on derived class should type-check");
    }

    #[test]
    fn derived_class_instantiation_with_inherited_init_type_checks() {
        // A `Dog` instance is created with `(name: str)` -- the `__init__`
        // is `Dog.__init__`, which takes `(self, name: str)`.
        let hir = inheritance_module(vec![top_level(HirStmt::Assign {
            target: "d".to_string(),
            value: HirExpr::Call {
                callee: "Dog".to_string(),
                args: vec![HirExpr::StringLiteral("Rex".to_string())],
            },
        })]);
        check(&hir).expect("derived class instantiation should type-check");
    }

    #[test]
    fn derived_class_instantiation_with_wrong_arg_type_is_rejected() {
        let hir = inheritance_module(vec![top_level(HirStmt::Assign {
            target: "d".to_string(),
            value: HirExpr::Call {
                callee: "Dog".to_string(),
                args: vec![HirExpr::IntLiteral(42)],
            },
        })]);
        assert_eq!(check(&hir).unwrap_err().code, "T0021");
    }

    /// Builds a `Base` class with `__init__` and a `Derived` class with no
    /// `__init__` of its own (inheriting `Base.__init__`).
    fn inherited_init_module(extra_items: Vec<HirItem>) -> HirModule {
        let base_ty = Ty::Instance(Box::new("Base".to_string()));
        let base_init = HirItem::Function {
            name: "Base.__init__".to_string(),
            params: vec![
                ("self".to_string(), base_ty.clone()),
                ("x".to_string(), Ty::Int),
            ],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "x".to_string(),
                    value: HirExpr::Name("x".to_string()),
                },
                HirStmt::Return(None),
            ],
        };
        let mut items = vec![base_init];
        items.extend(extra_items);
        HirModule {
            seeded_builtin_exception_classes: false,
            items,
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: vec![
                (
                    "Base".to_string(),
                    HirClassDef {
                        class_attrs: Vec::new(),
                        exception_type_tag: None,
                        name: "Base".to_string(),
                        bases: Vec::new(),
                        mro: vec!["Base".to_string()],
                        attrs: vec![("x".to_string(), Ty::Int)],
                        methods: vec![("__init__".to_string(), "Base.__init__".to_string())],
                        type_param: None,
                        properties: Vec::new(),
                        static_methods: Vec::new(),
                        class_methods: Vec::new(),
                        is_enum: false,
                        implicit_object_init: false,
                        method_defaults: Vec::new(),
                        enum_members: Vec::new(),
                        is_dataclass: false,
                        dataclass_fields: Vec::new(),
                        is_protocol: false,
                        runtime_checkable: false,
                        protocol_members: Vec::new(),
                        abstract_methods: Vec::new(),
                        is_abstract: false,
                    },
                ),
                (
                    "Derived".to_string(),
                    HirClassDef {
                        class_attrs: Vec::new(),
                        exception_type_tag: None,
                        name: "Derived".to_string(),
                        bases: vec!["Base".to_string()],
                        mro: vec!["Derived".to_string(), "Base".to_string()],
                        attrs: Vec::new(),
                        methods: Vec::new(),
                        type_param: None,
                        properties: Vec::new(),
                        static_methods: Vec::new(),
                        class_methods: Vec::new(),
                        is_enum: false,
                        implicit_object_init: false,
                        method_defaults: Vec::new(),
                        enum_members: Vec::new(),
                        is_dataclass: false,
                        dataclass_fields: Vec::new(),
                        is_protocol: false,
                        runtime_checkable: false,
                        protocol_members: Vec::new(),
                        abstract_methods: Vec::new(),
                        is_abstract: false,
                    },
                ),
            ],
        }
    }

    #[test]
    fn derived_class_without_init_inherits_base_init_for_instantiation() {
        // `Derived(42)` should resolve to `Base.__init__` via the MRO and
        // type-check against its `(self, x: int)` parameter list.
        let hir = inherited_init_module(vec![
            top_level(HirStmt::Assign {
                target: "d".to_string(),
                value: HirExpr::Call {
                    callee: "Derived".to_string(),
                    args: vec![HirExpr::IntLiteral(42)],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("d".to_string())),
                    attr: "x".to_string(),
                }],
            })),
        ]);
        check(&hir).expect("inherited __init__ instantiation should type-check");
    }

    #[test]
    fn derived_class_without_init_instantiation_wrong_arg_type_is_rejected() {
        let hir = inherited_init_module(vec![top_level(HirStmt::Assign {
            target: "d".to_string(),
            value: HirExpr::Call {
                callee: "Derived".to_string(),
                args: vec![HirExpr::StringLiteral("wrong".to_string())],
            },
        })]);
        assert_eq!(check(&hir).unwrap_err().code, "T0021");
    }

    #[test]
    fn inherited_attribute_read_resolves_through_check_and_resolve() {
        // Full pipeline: `check_and_resolve` → MIR build, exercising the
        // MIR lowering's MRO-aware attribute resolution.
        let hir = inherited_init_module(vec![
            top_level(HirStmt::Assign {
                target: "d".to_string(),
                value: HirExpr::Call {
                    callee: "Derived".to_string(),
                    args: vec![HirExpr::IntLiteral(42)],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::AttrGet {
                    base: Box::new(HirExpr::Name("d".to_string())),
                    attr: "x".to_string(),
                }],
            })),
        ]);
        let resolved = check_and_resolve(&hir).expect("check_and_resolve should succeed");
        let _mir = pycc_mir::build(&resolved);
    }

    #[test]
    fn inherited_method_call_resolves_through_check_and_resolve() {
        // Full pipeline for a method call that resolves to a base class
        // method via the MRO.
        let base_ty = Ty::Instance(Box::new("Base".to_string()));
        let base_init = HirItem::Function {
            name: "Base.__init__".to_string(),
            params: vec![("self".to_string(), base_ty.clone())],
            return_ty: Ty::None,
            body: vec![HirStmt::Return(None)],
        };
        let base_greet = HirItem::Function {
            name: "Base.greet".to_string(),
            params: vec![("self".to_string(), base_ty.clone())],
            return_ty: Ty::Str,
            body: vec![HirStmt::Return(Some(HirExpr::StringLiteral(
                "hi".to_string(),
            )))],
        };
        let hir = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![
                base_init,
                base_greet,
                top_level(HirStmt::Assign {
                    target: "d".to_string(),
                    value: HirExpr::Call {
                        callee: "Derived".to_string(),
                        args: vec![],
                    },
                }),
                top_level(HirStmt::ExprStmt(HirExpr::MethodCall {
                    base: Box::new(HirExpr::Name("d".to_string())),
                    method: "greet".to_string(),
                    args: vec![],
                })),
            ],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: vec![
                (
                    "Base".to_string(),
                    HirClassDef {
                        class_attrs: Vec::new(),
                        exception_type_tag: None,
                        name: "Base".to_string(),
                        bases: Vec::new(),
                        mro: vec!["Base".to_string()],
                        attrs: Vec::new(),
                        methods: vec![
                            ("__init__".to_string(), "Base.__init__".to_string()),
                            ("greet".to_string(), "Base.greet".to_string()),
                        ],
                        type_param: None,
                        properties: Vec::new(),
                        static_methods: Vec::new(),
                        class_methods: Vec::new(),
                        is_enum: false,
                        implicit_object_init: false,
                        method_defaults: Vec::new(),
                        enum_members: Vec::new(),
                        is_dataclass: false,
                        dataclass_fields: Vec::new(),
                        is_protocol: false,
                        runtime_checkable: false,
                        protocol_members: Vec::new(),
                        abstract_methods: Vec::new(),
                        is_abstract: false,
                    },
                ),
                (
                    "Derived".to_string(),
                    HirClassDef {
                        class_attrs: Vec::new(),
                        exception_type_tag: None,
                        name: "Derived".to_string(),
                        bases: vec!["Base".to_string()],
                        mro: vec!["Derived".to_string(), "Base".to_string()],
                        attrs: Vec::new(),
                        methods: Vec::new(),
                        type_param: None,
                        properties: Vec::new(),
                        static_methods: Vec::new(),
                        class_methods: Vec::new(),
                        is_enum: false,
                        implicit_object_init: false,
                        method_defaults: Vec::new(),
                        enum_members: Vec::new(),
                        is_dataclass: false,
                        dataclass_fields: Vec::new(),
                        is_protocol: false,
                        runtime_checkable: false,
                        protocol_members: Vec::new(),
                        abstract_methods: Vec::new(),
                        is_abstract: false,
                    },
                ),
            ],
        };
        let resolved = check_and_resolve(&hir).expect("check_and_resolve should succeed");
        let _mir = pycc_mir::build(&resolved);
    }

    // PEP 572 (#774): `pycc_mir::expr::pre_bind_named_expr_targets` is a
    // function private to `pycc_mir` (`pub(super)`), reachable from outside
    // that crate only indirectly through `pycc_mir::build`. Its `BinOp`/
    // `Compare`/`UnaryOp`/`FString` arms (a walrus nested inside one of
    // those, at any depth within an `ExprStmt`) are already exercised
    // through `pycc_mir`'s own hand-built-HIR test suite (see that crate's
    // `walrus_in_an_if_test_binds_the_name_for_the_body` and neighbors) and
    // through `pycc_types::tests::check_source`'s
    // `a_walrus_nested_inside_every_remaining_container_shape_binds_every_name_at_module_scope`,
    // but every one of those calls a *different* crate's own compiled copy
    // of `pycc_mir`: this crate's own `[dev-dependencies]` link on
    // `pycc_mir` is measured as its own separate coverage instantiation,
    // independent of `pycc_mir`'s own `cfg(test)` binary. Only this
    // module's handful of `pycc_mir::build(&resolved)` calls (see the
    // property/inheritance tests above) exercise *this* crate's copy of the
    // HIR→MIR pipeline at all, and none of them contain a walrus, so this
    // test closes that gap directly.
    #[test]
    fn a_walrus_nested_inside_binop_compare_unaryop_and_fstring_binds_every_name_via_mir_build() {
        let hir = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![
                top_level(HirStmt::ExprStmt(HirExpr::BinOp {
                    op: BinOpKind::Add,
                    left: Box::new(HirExpr::NamedExpr {
                        name: "bo_l".to_string(),
                        value: Box::new(HirExpr::IntLiteral(1)),
                    }),
                    right: Box::new(HirExpr::NamedExpr {
                        name: "bo_r".to_string(),
                        value: Box::new(HirExpr::IntLiteral(2)),
                    }),
                })),
                top_level(HirStmt::ExprStmt(HirExpr::Compare {
                    op: pycc_hir::CmpOpKind::Lt,
                    left: Box::new(HirExpr::NamedExpr {
                        name: "cmp_l".to_string(),
                        value: Box::new(HirExpr::IntLiteral(1)),
                    }),
                    right: Box::new(HirExpr::NamedExpr {
                        name: "cmp_r".to_string(),
                        value: Box::new(HirExpr::IntLiteral(2)),
                    }),
                })),
                top_level(HirStmt::ExprStmt(HirExpr::UnaryOp {
                    op: pycc_hir::UnaryOpKind::USub,
                    operand: Box::new(HirExpr::NamedExpr {
                        name: "uop".to_string(),
                        value: Box::new(HirExpr::IntLiteral(3)),
                    }),
                })),
                top_level(HirStmt::ExprStmt(HirExpr::FString(vec![
                    pycc_hir::FStringPart::Literal("x=".to_string()),
                    pycc_hir::FStringPart::Interpolation(Box::new(HirExpr::NamedExpr {
                        name: "fsp".to_string(),
                        value: Box::new(HirExpr::IntLiteral(4)),
                    })),
                ]))),
                top_level(HirStmt::Assign {
                    target: "readback".to_string(),
                    value: HirExpr::Name("fsp".to_string()),
                }),
            ],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: vec![],
        };
        let resolved = check_and_resolve(&hir).expect("check_and_resolve should succeed");
        let mir = pycc_mir::build(&resolved);
        // Proof every name was actually bound (not just that lowering
        // didn't panic): the trailing `Assign` above reads `fsp` back by
        // name, which would panic looking it up in `scopes` if
        // `pre_bind_named_expr_targets` had skipped the `FString`
        // interpolation arm.
        assert_eq!(
            mir.items[4],
            pycc_mir::MirItem::TopLevelStmt(pycc_mir::MirStmt::Assign {
                target: "readback".to_string(),
                value: pycc_mir::MirExpr::Name {
                    name: "fsp".to_string(),
                    ty: Ty::Int,
                },
            })
        );
    }

    // -- #436: @staticmethod / @classmethod type checking -------------------

    /// Builds a module with a class `C` that has `__init__`, a static method
    /// `create(x: int) -> int`, and a class method `greet(cls, x: int) ->
    /// int`. `extra_items`/`extra_stmts` let each test append its own
    /// call-site exercise.
    fn static_class_module(extra_items: Vec<HirItem>) -> HirModule {
        let self_ty = Ty::Instance(Box::new("C".to_string()));
        let init = HirItem::Function {
            name: "C.__init__".to_string(),
            params: vec![("self".to_string(), self_ty.clone())],
            return_ty: Ty::None,
            body: vec![HirStmt::Return(None)],
        };
        let static_fn = HirItem::Function {
            name: "C.create.static".to_string(),
            params: vec![("x".to_string(), Ty::Int)],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        };
        let class_fn = HirItem::Function {
            name: "C.greet.classmethod".to_string(),
            params: vec![
                ("cls".to_string(), self_ty.clone()),
                ("x".to_string(), Ty::Int),
            ],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::Name("x".to_string())))],
        };
        let mut items = vec![init, static_fn, class_fn];
        items.extend(extra_items);
        HirModule {
            seeded_builtin_exception_classes: false,
            items,
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: vec![(
                "C".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "C".to_string(),
                    bases: Vec::new(),
                    mro: vec!["C".to_string()],
                    attrs: Vec::new(),
                    methods: vec![("__init__".to_string(), "C.__init__".to_string())],
                    type_param: None,
                    properties: Vec::new(),
                    static_methods: vec![("create".to_string(), "C.create.static".to_string())],
                    class_methods: vec![("greet".to_string(), "C.greet.classmethod".to_string())],
                    is_enum: false,
                    implicit_object_init: false,
                    method_defaults: Vec::new(),
                    enum_members: Vec::new(),
                    is_dataclass: false,
                    dataclass_fields: Vec::new(),
                    is_protocol: false,
                    runtime_checkable: false,
                    protocol_members: Vec::new(),
                    abstract_methods: Vec::new(),
                    is_abstract: false,
                },
            )],
        }
    }

    #[test]
    fn static_method_call_through_class_name_type_checks() {
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("C".to_string())),
                method: "create".to_string(),
                args: vec![HirExpr::IntLiteral(42)],
            }],
        }))]);
        check(&hir).expect("a static method call through a class name should type-check");
    }

    #[test]
    fn class_method_call_through_class_name_type_checks() {
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("C".to_string())),
                method: "greet".to_string(),
                args: vec![HirExpr::IntLiteral(42)],
            }],
        }))]);
        check(&hir).expect("a class method call through a class name should type-check");
    }

    #[test]
    fn static_method_call_through_instance_type_checks() {
        let hir = static_class_module(vec![
            top_level(HirStmt::Assign {
                target: "c".to_string(),
                value: HirExpr::Call {
                    callee: "C".to_string(),
                    args: vec![],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::MethodCall {
                    base: Box::new(HirExpr::Name("c".to_string())),
                    method: "create".to_string(),
                    args: vec![HirExpr::IntLiteral(42)],
                }],
            })),
        ]);
        check(&hir).expect("a static method call through an instance should type-check");
    }

    #[test]
    fn class_method_call_through_instance_type_checks() {
        let hir = static_class_module(vec![
            top_level(HirStmt::Assign {
                target: "c".to_string(),
                value: HirExpr::Call {
                    callee: "C".to_string(),
                    args: vec![],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::MethodCall {
                    base: Box::new(HirExpr::Name("c".to_string())),
                    method: "greet".to_string(),
                    args: vec![HirExpr::IntLiteral(42)],
                }],
            })),
        ]);
        check(&hir).expect("a class method call through an instance should type-check");
    }

    #[test]
    fn static_method_wrong_argument_type_is_rejected() {
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("C".to_string())),
                method: "create".to_string(),
                args: vec![HirExpr::StringLiteral("hi".to_string())],
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn class_method_wrong_argument_type_is_rejected() {
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("C".to_string())),
                method: "greet".to_string(),
                args: vec![HirExpr::StringLiteral("hi".to_string())],
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn static_method_wrong_argument_count_is_rejected() {
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("C".to_string())),
                method: "create".to_string(),
                args: vec![],
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn class_method_excludes_cls_from_argument_count() {
        // `greet` takes `(cls, x: int)` — the caller passes only `x`. The
        // type checker excludes `cls` from the argument count check, so
        // passing one argument is correct.
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("C".to_string())),
                method: "greet".to_string(),
                args: vec![HirExpr::IntLiteral(42)],
            }],
        }))]);
        check(&hir).expect("cls is excluded from the argument count check");
    }

    #[test]
    fn unknown_static_or_class_method_on_class_name_falls_through() {
        // A method call on a class name that is neither a static method nor
        // a class method falls through to the regular resolution, which
        // tries to infer the base expression (`HirExpr::Name("C")`). A bare
        // class name is not a binding, so `infer_expr_in` rejects it with
        // T0021 ("name `C` is not defined").
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("C".to_string())),
                method: "nonexistent".to_string(),
                args: vec![],
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn attribute_access_on_a_class_name_is_rejected() {
        // `C.x` — accessing an attribute on a class name (not an instance)
        // is not supported. A static or class method must be called, not
        // accessed as a bare attribute value.
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("C".to_string())),
                attr: "create".to_string(),
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0044");
    }

    #[test]
    fn class_name_static_method_call_propagates_arg_inference_error() {
        // Exercises the `?` on argument inference at lib.rs:3289 inside the
        // class-name static/class method call branch of `infer_expr_in`.
        // `C.create(undefined_name)` — the class `C` is registered and has a
        // static method `create`, so the class-name branch is entered, but
        // the argument `undefined_name` fails inference (T0021 "name not
        // defined"), which propagates through the `collect::<Result<..>>?`.
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::MethodCall {
                base: Box::new(HirExpr::Name("C".to_string())),
                method: "create".to_string(),
                args: vec![HirExpr::Name("undefined_name".to_string())],
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    // -----------------------------------------------------------------------
    // #435: isinstance/issubclass type checker unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn isinstance_with_float_target_type_checks_as_bool() {
        // `isinstance(1.5, float)` — covers the `Ty::Float` arm of
        // `eval_isinstance_single` at the type-checker level (the type
        // checker only validates and returns `Ty::Bool`; the MIR computes
        // the constant). This also exercises `check_isinstance` with a
        // builtin type target.
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "isinstance".to_string(),
                args: vec![
                    HirExpr::FloatLiteral(1.5),
                    HirExpr::Name("float".to_string()),
                ],
            }],
        }))]);
        let result = check(&hir);
        assert!(result.is_ok(), "isinstance(1.5, float) should type-check");
    }

    #[test]
    fn isinstance_with_wrong_arg_count_is_t0021() {
        // `isinstance(1)` — only 1 argument. Covers the `args.len() != 2`
        // error branch in `check_isinstance`.
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "isinstance".to_string(),
                args: vec![HirExpr::IntLiteral(1)],
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn isinstance_with_non_class_second_arg_is_t0021() {
        // `isinstance(1, 5)` — the second argument is not a class name or
        // tuple of class names. Covers the `extract_class_names` error
        // branch in `check_isinstance`.
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "isinstance".to_string(),
                args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(5)],
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn issubclass_with_wrong_arg_count_is_t0021() {
        // `issubclass(int)` — only 1 argument. Covers the `args.len() != 2`
        // error branch in `check_issubclass`.
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "issubclass".to_string(),
                args: vec![HirExpr::Name("int".to_string())],
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn isinstance_with_undefined_name_in_first_arg_is_t0021() {
        // `isinstance(undefined_name, int)` — the first argument references
        // an undefined name, so `infer_expr_in` fails with T0021 before
        // the class argument is even examined. This covers the `?` error
        // branch on the `infer_expr_in` call in `check_isinstance`.
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "isinstance".to_string(),
                args: vec![
                    HirExpr::Name("undefined_name".to_string()),
                    HirExpr::Name("int".to_string()),
                ],
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn issubclass_with_unknown_class_in_first_arg_is_t0001() {
        // `issubclass(UnknownClass, int)` — the first argument is a class
        // name that is neither a user-defined class nor a builtin type.
        // This covers the `validate_class_name` error path for the first
        // argument in `check_issubclass`.
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "issubclass".to_string(),
                args: vec![
                    HirExpr::Name("UnknownClass".to_string()),
                    HirExpr::Name("int".to_string()),
                ],
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0001");
    }

    #[test]
    fn issubclass_with_non_class_second_arg_is_t0021() {
        // `issubclass(C, 5)` — the second argument is not a class name or
        // tuple of class names. This exercises the `extract_class_names`
        // error path in `check_issubclass`.
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "issubclass".to_string(),
                args: vec![HirExpr::Name("C".to_string()), HirExpr::IntLiteral(5)],
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn issubclass_with_int_str_same_type_returns_bool() {
        // `issubclass(int, int)` and `issubclass(str, str)` — covers the
        // `return cls == target_class` line in `eval_issubclass_single`
        // at the type-checker level.
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "issubclass".to_string(),
                args: vec![
                    HirExpr::Name("int".to_string()),
                    HirExpr::Name("int".to_string()),
                ],
            }],
        }))]);
        let result = check(&hir);
        assert!(result.is_ok(), "issubclass(int, int) should type-check");
    }

    #[test]
    fn isinstance_with_unknown_class_in_second_arg_is_t0001() {
        // `isinstance(1, UnknownClass)` — the second argument is a valid
        // name expression but not a registered class. Covers the
        // `validate_class_name` error path in `check_isinstance`.
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "isinstance".to_string(),
                args: vec![
                    HirExpr::IntLiteral(1),
                    HirExpr::Name("UnknownClass".to_string()),
                ],
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0001");
    }

    #[test]
    fn issubclass_with_unknown_class_in_second_arg_is_t0001() {
        // `issubclass(int, UnknownClass)` — the second argument is a valid
        // name expression but not a registered class. Covers the
        // `validate_class_name` error path for target classes in
        // `check_issubclass`.
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "issubclass".to_string(),
                args: vec![
                    HirExpr::Name("int".to_string()),
                    HirExpr::Name("UnknownClass".to_string()),
                ],
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0001");
    }

    #[test]
    fn issubclass_with_non_name_first_arg_is_t0021() {
        // `issubclass(5, int)` — the first argument is not a bare class
        // name. Covers the `_ =>` error branch in `check_issubclass`'s
        // first-argument match.
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "issubclass".to_string(),
                args: vec![HirExpr::IntLiteral(5), HirExpr::Name("int".to_string())],
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn isinstance_with_call_first_arg_is_c0001() {
        // #435 review fix (P1): `isinstance(D(), D)` — a call expression as
        // the first argument is rejected with C0001 because pycc's
        // compile-time `isinstance` would silently discard the call's side
        // effects. Covers the `if let HirExpr::Call { .. }` branch in
        // `check_isinstance`.
        let hir = static_class_module(vec![top_level(HirStmt::ExprStmt(HirExpr::Call {
            callee: "print".to_string(),
            args: vec![HirExpr::Call {
                callee: "isinstance".to_string(),
                args: vec![
                    HirExpr::Call {
                        callee: "D".to_string(),
                        args: vec![],
                    },
                    HirExpr::Name("D".to_string()),
                ],
            }],
        }))]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "C0001");
        assert!(diagnostic.message.contains("isinstance"));
    }

    // -- #378 (PR-18): dataclass __eq__ comparison type checking ---------

    #[test]
    fn a_same_class_instance_comparison_without_eq_is_t0021() {
        // `Point` has `__init__` and `bump` but no `__eq__` method.
        // Comparing two `Point` instances with `==` should fall through
        // the `has_eq` check and produce T0021 (cannot compare instances).
        // This covers the `}` (merge point) of the `if has_eq` block in
        // `infer_expr_in`'s `Compare` arm.
        let hir = point_module(vec![
            top_level(HirStmt::Assign {
                target: "p".to_string(),
                value: HirExpr::Call {
                    callee: "Point".to_string(),
                    args: vec![HirExpr::IntLiteral(1), HirExpr::IntLiteral(2)],
                },
            }),
            top_level(HirStmt::Assign {
                target: "q".to_string(),
                value: HirExpr::Call {
                    callee: "Point".to_string(),
                    args: vec![HirExpr::IntLiteral(3), HirExpr::IntLiteral(4)],
                },
            }),
            top_level(HirStmt::ExprStmt(HirExpr::Call {
                callee: "print".to_string(),
                args: vec![HirExpr::Compare {
                    op: pycc_hir::CmpOpKind::Eq,
                    left: Box::new(HirExpr::Name("p".to_string())),
                    right: Box::new(HirExpr::Name("q".to_string())),
                }],
            })),
        ]);
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    #[test]
    fn a_same_class_dataclass_instance_comparison_with_eq_type_checks() {
        // A dataclass class with a compiler-synthesized `__eq__` method
        // accepts `==` between same-class instances. This covers the
        // `return Ok(Ty::Bool)` path in the dataclass comparison check.
        // Non-dataclass classes with a user-defined `__eq__` are rejected
        // (the MIR rewrite assumes the synthesized signature).
        let self_ty = Ty::Instance(Box::new("EqPoint".to_string()));
        let init = HirItem::Function {
            name: "EqPoint.__init__".to_string(),
            params: vec![
                ("self".to_string(), self_ty.clone()),
                ("x".to_string(), Ty::Int),
            ],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "x".to_string(),
                    value: HirExpr::Name("x".to_string()),
                },
                HirStmt::Return(None),
            ],
        };
        let eq = HirItem::Function {
            name: "EqPoint.__eq__".to_string(),
            params: vec![
                ("self".to_string(), self_ty.clone()),
                ("other".to_string(), self_ty.clone()),
            ],
            return_ty: Ty::Bool,
            body: vec![HirStmt::Return(Some(HirExpr::BoolLiteral(true)))],
        };
        let hir = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![
                init,
                eq,
                top_level(HirStmt::Assign {
                    target: "p".to_string(),
                    value: HirExpr::Call {
                        callee: "EqPoint".to_string(),
                        args: vec![HirExpr::IntLiteral(1)],
                    },
                }),
                top_level(HirStmt::Assign {
                    target: "q".to_string(),
                    value: HirExpr::Call {
                        callee: "EqPoint".to_string(),
                        args: vec![HirExpr::IntLiteral(2)],
                    },
                }),
                top_level(HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::Compare {
                        op: pycc_hir::CmpOpKind::Eq,
                        left: Box::new(HirExpr::Name("p".to_string())),
                        right: Box::new(HirExpr::Name("q".to_string())),
                    }],
                })),
            ],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: vec![(
                "EqPoint".to_string(),
                pycc_hir::HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "EqPoint".to_string(),
                    bases: Vec::new(),
                    mro: vec!["EqPoint".to_string()],
                    attrs: vec![("x".to_string(), Ty::Int)],
                    methods: vec![
                        ("__init__".to_string(), "EqPoint.__init__".to_string()),
                        ("__eq__".to_string(), "EqPoint.__eq__".to_string()),
                    ],
                    type_param: None,
                    properties: Vec::new(),
                    static_methods: Vec::new(),
                    class_methods: Vec::new(),
                    is_enum: false,
                    implicit_object_init: false,
                    method_defaults: Vec::new(),
                    enum_members: Vec::new(),
                    is_dataclass: true,
                    dataclass_fields: vec![("x".to_string(), Ty::Int)],
                    is_protocol: false,
                    runtime_checkable: false,
                    protocol_members: Vec::new(),
                    abstract_methods: Vec::new(),
                    is_abstract: false,
                },
            )],
        };
        check(&hir).expect("a same-class dataclass comparison with __eq__ should type-check");
    }

    #[test]
    fn a_non_dataclass_instance_comparison_with_user_eq_is_t0021() {
        // A non-dataclass class with a user-defined `__eq__` method does
        // NOT accept `==` between same-class instances -- the MIR rewrite
        // for `__eq__` is restricted to dataclass classes (whose
        // synthesized `__eq__` has a known-correct signature), so the type
        // checker must reject this to prevent a codegen panic.
        let self_ty = Ty::Instance(Box::new("PlainEq".to_string()));
        let init = HirItem::Function {
            name: "PlainEq.__init__".to_string(),
            params: vec![
                ("self".to_string(), self_ty.clone()),
                ("x".to_string(), Ty::Int),
            ],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "x".to_string(),
                    value: HirExpr::Name("x".to_string()),
                },
                HirStmt::Return(None),
            ],
        };
        let eq = HirItem::Function {
            name: "PlainEq.__eq__".to_string(),
            params: vec![
                ("self".to_string(), self_ty.clone()),
                ("other".to_string(), self_ty.clone()),
            ],
            return_ty: Ty::Bool,
            body: vec![HirStmt::Return(Some(HirExpr::BoolLiteral(true)))],
        };
        let hir = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![
                init,
                eq,
                top_level(HirStmt::Assign {
                    target: "p".to_string(),
                    value: HirExpr::Call {
                        callee: "PlainEq".to_string(),
                        args: vec![HirExpr::IntLiteral(1)],
                    },
                }),
                top_level(HirStmt::Assign {
                    target: "q".to_string(),
                    value: HirExpr::Call {
                        callee: "PlainEq".to_string(),
                        args: vec![HirExpr::IntLiteral(2)],
                    },
                }),
                top_level(HirStmt::ExprStmt(HirExpr::Call {
                    callee: "print".to_string(),
                    args: vec![HirExpr::Compare {
                        op: pycc_hir::CmpOpKind::Eq,
                        left: Box::new(HirExpr::Name("p".to_string())),
                        right: Box::new(HirExpr::Name("q".to_string())),
                    }],
                })),
            ],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: vec![(
                "PlainEq".to_string(),
                pycc_hir::HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "PlainEq".to_string(),
                    bases: Vec::new(),
                    mro: vec!["PlainEq".to_string()],
                    attrs: vec![("x".to_string(), Ty::Int)],
                    methods: vec![
                        ("__init__".to_string(), "PlainEq.__init__".to_string()),
                        ("__eq__".to_string(), "PlainEq.__eq__".to_string()),
                    ],
                    type_param: None,
                    properties: Vec::new(),
                    static_methods: Vec::new(),
                    class_methods: Vec::new(),
                    is_enum: false,
                    implicit_object_init: false,
                    method_defaults: Vec::new(),
                    enum_members: Vec::new(),
                    is_dataclass: false,
                    dataclass_fields: Vec::new(),
                    is_protocol: false,
                    runtime_checkable: false,
                    protocol_members: Vec::new(),
                    abstract_methods: Vec::new(),
                    is_abstract: false,
                },
            )],
        };
        let diagnostic = check(&hir).unwrap_err();
        assert_eq!(diagnostic.code, "T0021");
    }

    // -- PEP 544 (#380): Protocol conformance checking --------------------

    /// Parses and lowers source code, then type-checks it with
    /// `check_and_resolve`. Returns the resolved HIR on success or the
    /// diagnostic on failure.
    fn check_source(source: &str) -> Result<pycc_hir::HirModule, pycc_diag::Diagnostic> {
        let module = pycc_parser::parse(source).expect("test fixture must parse");
        let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
        check_and_resolve(&hir)
    }

    #[test]
    fn protocol_conformance_with_wrong_parameter_count_is_t0046() {
        let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self, x: int) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nc: P = C()\n",
        )
        .unwrap_err();
        assert_eq!(err.code, "T0046");
        assert!(
            err.message.contains("parameter"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn protocol_conformance_with_wrong_parameter_type_is_t0046() {
        let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self, x: int) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self, x: str) -> int:\n        return 1\nc: P = C()\n",
        )
        .unwrap_err();
        assert_eq!(err.code, "T0046");
        assert!(
            err.message.contains("parameter 1 has type"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn protocol_conformance_with_wrong_return_type_is_t0046() {
        let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> str:\n        return \"hi\"\nc: P = C()\n",
        )
        .unwrap_err();
        assert_eq!(err.code, "T0046");
        assert!(
            err.message.contains("return type is"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn protocol_conformance_with_missing_attribute_is_t0046() {
        let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass C:\n    def __init__(self) -> None:\n        self.y = 0\nc: P = C()\n",
        )
        .unwrap_err();
        assert_eq!(err.code, "T0046");
        assert!(
            err.message.contains("missing attribute"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn protocol_conformance_with_wrong_attribute_type_is_t0046() {
        let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass C:\n    def __init__(self) -> None:\n        self.x = \"hi\"\nc: P = C()\n",
        )
        .unwrap_err();
        assert_eq!(err.code, "T0046");
        assert!(
            err.message.contains("attribute `x` has type"),
            "unexpected message: {}",
            err.message
        );
    }

    // -- #914: a class attribute satisfies a protocol attribute member ----

    /// A `#911` class-level attribute satisfies a protocol's attribute
    /// member, exactly as an instance slot or a `@property` does: the
    /// `ProtocolMember::Attribute` arm falls back to
    /// `lookup_class_attr_through_mro` when the instance/property walk
    /// misses.
    #[test]
    fn protocol_conformance_with_class_attribute_succeeds() {
        check_source(
            "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass C:\n    x: int = 1\n    def __init__(self) -> None:\n        self.y = 0\nc: P = C()\n",
        )
        .expect("a class attribute satisfies the protocol's attribute member");
    }

    /// The fallback walks the full MRO, so a class attribute declared on a
    /// *base* satisfies the member for a derived class that declares none
    /// of its own.
    #[test]
    fn protocol_conformance_with_inherited_class_attribute_succeeds() {
        check_source(
            "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass B:\n    x: int = 1\nclass C(B):\n    def __init__(self) -> None:\n        self.y = 0\nc: P = C()\n",
        )
        .expect("an inherited class attribute satisfies the protocol's attribute member");
    }

    /// A class attribute of the wrong type still fails, with the same
    /// "has type" wording an instance attribute produces -- the fallback
    /// feeds the existing `is_assignable` branch rather than short-circuiting
    /// past it.
    #[test]
    fn protocol_conformance_with_wrong_typed_class_attribute_is_t0046() {
        let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass C:\n    x: str = \"hi\"\n    def __init__(self) -> None:\n        self.y = 0\nc: P = C()\n",
        )
        .unwrap_err();
        assert_eq!(err.code, "T0046");
        assert!(
            err.message.contains("attribute `x` has type `str`"),
            "unexpected message: {}",
            err.message
        );
    }

    /// Pins the `.or_else` *ordering*. `reject_class_attr_collisions` only
    /// compares a class's own `class_attrs` against its own MRO, so two
    /// independent sibling bases can contribute an instance slot and a
    /// same-named class attribute to one derived class without ever being
    /// compared. The instance walk runs first, so the member resolves to
    /// `A.x`'s `int` -- CPython's own answer, since an instance `__dict__`
    /// entry shadows a non-data-descriptor class attribute. Swapping the
    /// two walks would resolve it to `B.x`'s `str` and reject this program.
    /// The same shape used to expose a MIR-side value divergence for ordinary
    /// attribute reads (#960): `pycc_mir` folded the class attribute before
    /// looking for the slot, so `c.x` printed `B`'s value and, for the `str`
    /// spelling above, aborted codegen outright. `pycc_mir`'s instance
    /// `AttrGet` arm now mirrors this order, so the two layers agree.
    #[test]
    fn protocol_conformance_prefers_a_sibling_base_instance_attribute() {
        check_source(
            "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass A:\n    def __init__(self) -> None:\n        self.x = 1\nclass B:\n    x: str = \"hi\"\nclass C(A, B):\n    pass\nc: P = C()\n",
        )
        .expect("the sibling base's instance attribute wins over the class attribute");
    }

    #[test]
    fn protocol_conformance_with_contravariant_param_narrowing_is_t0046() {
        let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self, x: int) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self, x: bool) -> int:\n        return 1\nc: P = C()\n",
        )
        .unwrap_err();
        assert_eq!(err.code, "T0046");
        assert!(
            err.message.contains("parameter 1 has type"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn protocol_conformance_with_matching_method_succeeds() {
        let result = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self, x: int) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self, x: int) -> int:\n        return x\nc: P = C()\n",
        );
        assert!(result.is_ok(), "conforming class should type-check");
    }

    #[test]
    fn protocol_conformance_with_matching_attribute_succeeds() {
        let result = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    x: int\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\nc: P = C()\n",
        );
        assert!(result.is_ok(), "conforming class should type-check");
    }

    #[test]
    fn protocol_to_protocol_assignability_with_different_protocols_is_t0046() {
        let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass Q(Protocol):\n    def bar(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nc: P = C()\nq: Q = c\n",
        )
        .unwrap_err();
        assert_eq!(err.code, "T0046");
    }

    #[test]
    fn protocol_cannot_be_instantiated() {
        let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\np = P()\n",
        )
        .unwrap_err();
        assert_eq!(err.code, "C0001");
        assert!(
            err.message.contains("cannot instantiate protocol"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn abstract_class_cannot_be_instantiated() {
        // #380 (PR-20): exercises `resolve_instantiation`'s
        // `is_abstract` error path. Covered here as a unit test to
        // avoid cargo-llvm-cov issue #276.
        let err = check_source(
            "from abc import ABC, abstractmethod\nclass A(ABC):\n    def __init__(self) -> None:\n        self.x = 0\n    @abstractmethod\n    def foo(self) -> int: ...\na = A()\n",
        )
        .unwrap_err();
        assert_eq!(err.code, "C0001");
        assert!(
            err.message.contains("cannot instantiate abstract class"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn protocol_typed_variable_unknown_attribute_is_t0044() {
        let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nc: P = C()\nprint(c.bar)\n",
        )
        .unwrap_err();
        assert_eq!(err.code, "T0044");
    }

    #[test]
    fn protocol_typed_variable_unknown_method_is_t0044() {
        let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nc: P = C()\nc.bar()\n",
        )
        .unwrap_err();
        assert_eq!(err.code, "T0044");
    }

    #[test]
    fn protocol_typed_variable_method_call_wrong_arity_is_t0023() {
        // #380 (PR-20): exercises `resolve_method_call`'s `check_call_args`
        // error path for a protocol-typed variable — calling a protocol
        // method with the wrong number of arguments. Covered here as a
        // unit test to avoid cargo-llvm-cov issue #276.
        //
        // NOTE: A top-level `c: P = C()` binds `c` as the concrete
        // `Ty::Instance("C")` (D-040 sticky representation), so the call
        // resolves through the instance-method path, not the protocol
        // path.  To exercise the *protocol* branch of
        // `class::method_call::resolve_method_call` (moved there by
        // #815, Part 1 of #737), the receiver must be a function
        // parameter annotated `x: P`, which is bound as
        // `Ty::Protocol("P")` in the function body.
        let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\ndef f(x: P) -> int:\n    return x.foo(99)\n",
        )
        .unwrap_err();
        assert_eq!(err.code, "T0021");
    }

    #[test]
    fn protocol_typed_param_method_call_wrong_arg_type_is_t0021() {
        // #380 (PR-20): exercises the `check_call_args` *type-mismatch*
        // error path (not just arity) inside
        // `class::method_call::resolve_method_call`'s protocol branch
        // (moved there by #815, Part 1 of #737).  The receiver is a
        // function parameter typed `x: P`, so it is bound as
        // `Ty::Protocol("P")` and the protocol branch is entered.  The
        // argument count matches
        // (1 == 1) but the type (`str` vs `int`) does not, so
        // `check_call_args` returns a type-mismatch T0021.
        let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self, x: int) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self, x: int) -> int:\n        return x\ndef f(x: P) -> int:\n    return x.foo(\"hello\")\n",
        )
        .unwrap_err();
        assert_eq!(err.code, "T0021");
        assert!(
            err.message.contains("argument 1"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn isinstance_against_non_runtime_checkable_protocol_is_c0001() {
        let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\n    def foo(self) -> int:\n        return 1\nc = C()\nprint(isinstance(c, P))\n",
        )
        .unwrap_err();
        assert_eq!(err.code, "C0001");
        assert!(
            err.message.contains("not `@runtime_checkable`"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn issubclass_with_protocol_first_arg_is_c0001() {
        let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nprint(issubclass(P, int))\n",
        )
        .unwrap_err();
        assert_eq!(err.code, "C0001");
        assert!(
            err.message.contains("`issubclass` with protocol"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn issubclass_against_protocol_target_is_c0001() {
        let err = check_source(
            "from typing import Protocol\nclass P(Protocol):\n    def foo(self) -> int: ...\nclass C:\n    def __init__(self) -> None:\n        self.x = 0\nprint(issubclass(C, P))\n",
        )
        .unwrap_err();
        assert_eq!(err.code, "C0001");
        assert!(
            err.message.contains("`issubclass` against protocol"),
            "unexpected message: {}",
            err.message
        );
    }

    // -- #380 W2: @property satisfies a protocol attribute requirement -----

    /// Verifies that a class with a `@property` getter (but no direct
    /// attribute slot) satisfies a protocol's attribute requirement.
    /// This exercises the `properties` check added to
    /// `lookup_attr_through_mro` (lines 233-242): the protocol requires
    /// attribute `x: int`, the class has no `x` in its `attrs` table, but
    /// it does have a `@property` getter for `x` whose return type is
    /// `int`. `check_protocol_conformance` should return `Ok(())`.
    #[test]
    fn protocol_conformance_with_property_satisfying_attribute_succeeds() {
        use pycc_hir::PropertyDef;

        // --- Protocol "P": requires attribute `x: int` ---
        let proto_def = HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "P".to_string(),
            bases: Vec::new(),
            mro: vec!["P".to_string()],
            attrs: Vec::new(),
            methods: Vec::new(),
            type_param: None,
            properties: Vec::new(),
            static_methods: Vec::new(),
            class_methods: Vec::new(),
            is_enum: false,
            implicit_object_init: false,
            method_defaults: Vec::new(),
            enum_members: Vec::new(),
            is_dataclass: false,
            dataclass_fields: Vec::new(),
            is_protocol: true,
            runtime_checkable: false,
            protocol_members: vec![pycc_hir::ProtocolMember::Attribute {
                name: "x".to_string(),
                ty: Ty::Int,
            }],
            abstract_methods: Vec::new(),
            is_abstract: false,
        };

        // --- Concrete class "C": `@property` getter for `x`, no `x` attr ---
        let c_self_ty = Ty::Instance(Box::new("C".to_string()));
        let c_init = HirItem::Function {
            name: "C.__init__".to_string(),
            params: vec![("self".to_string(), c_self_ty.clone())],
            return_ty: Ty::None,
            body: vec![
                HirStmt::AttrSet {
                    base: HirExpr::Name("self".to_string()),
                    attr: "_val".to_string(),
                    value: HirExpr::IntLiteral(0),
                },
                HirStmt::Return(None),
            ],
        };
        let c_getter = HirItem::Function {
            name: "C.x".to_string(),
            params: vec![("self".to_string(), c_self_ty.clone())],
            return_ty: Ty::Int,
            body: vec![HirStmt::Return(Some(HirExpr::AttrGet {
                base: Box::new(HirExpr::Name("self".to_string())),
                attr: "_val".to_string(),
            }))],
        };
        let c_def = HirClassDef {
            class_attrs: Vec::new(),
            exception_type_tag: None,
            name: "C".to_string(),
            bases: Vec::new(),
            mro: vec!["C".to_string()],
            // Deliberately NO `x` in `attrs` — the property must satisfy
            // the protocol requirement, not a direct attribute slot.
            attrs: vec![("_val".to_string(), Ty::Int)],
            methods: vec![("__init__".to_string(), "C.__init__".to_string())],
            type_param: None,
            properties: vec![PropertyDef {
                name: "x".to_string(),
                getter: "C.x".to_string(),
                setter: None,
            }],
            static_methods: Vec::new(),
            class_methods: Vec::new(),
            is_enum: false,
            implicit_object_init: false,
            method_defaults: Vec::new(),
            enum_members: Vec::new(),
            is_dataclass: false,
            dataclass_fields: Vec::new(),
            is_protocol: false,
            runtime_checkable: false,
            protocol_members: Vec::new(),
            abstract_methods: Vec::new(),
            is_abstract: false,
        };

        // --- Trigger `check_protocol_conformance` via `c: P = C()` ---
        let hir = HirModule {
            seeded_builtin_exception_classes: false,
            items: vec![
                c_init,
                c_getter,
                top_level(HirStmt::AnnAssign {
                    target: "c".to_string(),
                    annotation: Ty::Protocol(Box::new("P".to_string())),
                    value: Some(HirExpr::Call {
                        callee: "C".to_string(),
                        args: vec![],
                    }),
                    is_final: false,
                }),
            ],
            type_aliases: Vec::new(),
            imports: Vec::new(),
            class_defs: vec![("P".to_string(), proto_def), ("C".to_string(), c_def)],
        };

        check(&hir).expect(
            "a class with a @property getter should satisfy a protocol \
             attribute requirement",
        );
    }

    /// #380 W2: covers the defensive panic in `lookup_attr_through_mro`
    /// (lines 234-239) — a property's getter is in the class's own
    /// property table but was never registered in
    /// `Environment::functions`. This "declared shape and Environment
    /// disagree" scenario is unreachable from any real `check`-validated
    /// program, mirroring `resolve_attr_get`'s own analogous panic test.
    #[test]
    #[should_panic(expected = "was not registered as an ordinary function")]
    fn lookup_attr_through_mro_panics_when_a_property_getter_is_not_registered() {
        use pycc_hir::PropertyDef;
        let mut env = crate::Environment::new();
        env.bind_class(
            "Ghost".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Ghost".to_string(),
                bases: Vec::new(),
                mro: vec!["Ghost".to_string()],
                attrs: vec![],
                methods: vec![("__init__".to_string(), "Ghost.__init__".to_string())],
                type_param: None,
                properties: vec![PropertyDef {
                    name: "x".to_string(),
                    getter: "Ghost.x".to_string(),
                    setter: None,
                }],
                static_methods: Vec::new(),
                class_methods: Vec::new(),
                is_enum: false,
                implicit_object_init: false,
                method_defaults: Vec::new(),
                enum_members: Vec::new(),
                is_dataclass: false,
                dataclass_fields: Vec::new(),
                is_protocol: false,
                runtime_checkable: false,
                protocol_members: Vec::new(),
                abstract_methods: Vec::new(),
                is_abstract: false,
            },
        );
        let _ = super::lookup_attr_through_mro(&env, &["Ghost".to_string()], "x");
    }
}
