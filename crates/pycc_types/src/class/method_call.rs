//! Method-call resolution (`resolve_method_call`), extracted verbatim from
//! `crates/pycc_types/src/class.rs` per AGENTS.md's file-decomposition rule
//! and D-185's per-file tracking issue (#549): this is one cohesion-driven
//! seam of that ~4,900-line file, not a rewrite. Every diagnostic message
//! and every check is unchanged except for two guards added after the move
//! -- the #815 synthetic-class guard, added in the commit that moved this
//! function, and the #1174 buffer-return interception described below --
//! the only other edits being the ones the module boundary forces
//! (visibility keywords and `use` lines).
//!
//! The seam is `base.method(args)` resolution against a class's MRO: given
//! a receiver type and a method name, walk the MRO (most-derived first,
//! matching CPython's own method resolution order) and check the call's
//! arguments against the first method found. Everything else lives in a
//! sibling: instantiation and class binding in `class/binding.rs`,
//! static/class-method dispatch in `class/static_call.rs`, `super()` calls
//! in `class/super_call.rs`, and attribute access plus protocol conformance
//! in `class.rs` itself.
//!
//! Part 1 of #1191 (#1438) routes the regular-method arm's argument check
//! through `class/method_defaults.rs`, so a call may omit trailing
//! parameters the dispatched method declares defaults for. The protocol arm
//! keeps the plain arity check.
//!
//! #1174 added the one check here that is not part of the original move: a
//! buffer-returning method's return value may not be handed to an
//! intra-artifact caller, refused through
//! `crate::buffer::refuse_buffer_returning_method`. The *protocol* arm above
//! deliberately does not get that check -- `pycc_hir`'s
//! `class/protocol.rs` refuses a `-> memoryview` protocol member at its
//! declaration, so no protocol member can have that return type and a check
//! here would be dead code the coverage gate could not reach.

use crate::Environment;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirExpr, ProtocolMember, Ty};

use super::{check_call_args, expect_class, t0043_not_an_instance, t0044_unknown_member};

/// Resolves `base.method(args)` against `base_ty`, checking the call's
/// arguments against the method's own resolved signature (excluding
/// `self`, exactly like `resolve_instantiation` excludes it from a
/// constructor call) and returning the method's return type.
///
/// #432: walks the class's MRO (C3 linearization) in order, checking each
/// class's method table. The first class in the MRO that declares the
/// method wins, matching CPython's own MRO-based method resolution. A
/// subclass method shadows a base class method of the same name (the
/// subclass appears first in the MRO).
///
/// #815 (Part 1 of #737): before resolving the method against
/// `env.lookup_function`, this checks whether the MRO class that actually
/// *owns* the resolved method (not just `base_ty`'s own class) is
/// synthetic -- i.e. HIR-lowering-generated for a builtin exception class,
/// per `Environment::is_synthetic_class`. A synthetic class's method table
/// entries do not correspond to a real, callable function (D-173
/// propagates a raised exception through global runtime state rather than
/// through an allocated instance with real methods), so calling one
/// directly -- `e.__init__("oops")` on a caught `Exception`, or on an
/// instance of a user subclass that inherited the synthetic `__init__`
/// through `any_user_exception_class`'s HIR-item-existence gate -- must be
/// rejected with a capability diagnostic instead of either panicking
/// (`lookup_function` finding nothing, #711) or type-checking cleanly only
/// to abort at runtime (#714). The guard fires on the *found* class, not
/// `base_ty`'s own class, so it also catches the inherited case where a
/// user subclass has no method of its own and the MRO walk lands on the
/// synthetic base.
pub(crate) fn resolve_method_call(
    env: &Environment,
    base_ty: &Ty,
    method: &str,
    args: &[HirExpr],
    arg_tys: &[Ty],
) -> Result<Ty, Diagnostic> {
    // #380 (PR-20, PEP 544): protocol-typed variable method call.
    // When `base_ty` is `Ty::Protocol(P)`, look up the method in
    // `P`'s `protocol_members` and check arguments against the
    // protocol method's signature.
    if let Ty::Protocol(protocol_name) = base_ty {
        let proto_def = expect_class(env, protocol_name);
        for member in &proto_def.protocol_members {
            if let ProtocolMember::Method {
                name: member_name,
                param_tys: proto_param_tys,
                return_ty: proto_return_ty,
            } = member
                && member_name == method
            {
                check_call_args(env, method, args, arg_tys, proto_param_tys, true)?;
                return Ok(proto_return_ty.clone());
            }
        }
        return Err(t0044_unknown_member("method", protocol_name, method));
    }
    let Ty::Instance(class_name) = base_ty else {
        return Err(t0043_not_an_instance("call a method", base_ty));
    };
    let class_def = expect_class(env, class_name);
    // #432: walk the MRO in order. The first class that declares the
    // method wins.
    for mro_class in &class_def.mro {
        let mro_def = expect_class(env, mro_class);
        if let Some((_, mangled)) = mro_def.methods.iter().find(|(name, _)| name == method) {
            // #815 (Part 1 of #737): guard on the class that actually owns
            // the resolved method, not `class_name` (the call's own
            // receiver) -- a user subclass with no method of its own
            // (`class MyError(Exception): pass`) still lands here with
            // `mro_class == "Exception"` when it inherits a synthetic
            // method, and must be rejected exactly like a direct call on a
            // caught builtin `Exception` instance.
            if env.is_synthetic_class(mro_class) {
                return Err(Diagnostic::error(
                    "C0001",
                    format!(
                        "cannot call `{method}` directly on `{mro_class}` -- pycc does \
                         not yet materialize a callable constructor for a builtin \
                         exception class (Part 3 of #541, tracked for removal by \
                         Part 2 of #737)"
                    ),
                    Span::new(0, 0),
                ));
            }
            let (param_tys, return_ty) = env.lookup_function(mangled).unwrap_or_else(|| {
                panic!(
                    "pycc_types: internal error: `{mangled}` is in class `{mro_class}`'s own \
                     method table but was not registered as an ordinary function"
                )
            });
            let method_param_tys = &param_tys[1..]; // exclude `self`
            // #1174: a buffer-returning method is now declarable, so this
            // exit has to refuse handing its return value to an
            // intra-artifact caller.
            crate::buffer::refuse_buffer_returning_method(class_name, method, return_ty)?;
            // Part 1 of #1191 (#1438): a call may omit trailing parameters
            // the dispatched method declares defaults for.
            super::method_defaults::check_method_call_args(
                env,
                mro_def,
                mangled,
                method,
                args,
                arg_tys,
                method_param_tys,
            )?;
            return Ok(return_ty.clone());
        }
    }
    Err(t0044_unknown_member("method", class_name, method))
}

#[cfg(test)]
mod tests {
    use pycc_hir::HirClassDef;
    use pycc_hir::Ty;

    #[test]
    #[should_panic(expected = "was not registered as an ordinary function")]
    fn resolve_method_call_panics_when_the_method_is_not_registered() {
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
                methods: vec![("foo".to_string(), "Ghost.foo".to_string())],
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
        let _ = super::resolve_method_call(
            &env,
            &Ty::Instance(Box::new("Ghost".to_string())),
            "foo",
            &[],
            &[],
        );
    }

    /// #815: the `is_synthetic_class` guard fires on a direct method call
    /// against a synthetic class's own instance, mirroring the `Ghost`
    /// test's construction pattern but marking the class synthetic via
    /// `bind_synthetic_class` (the same registration path
    /// `class::binding::bind_classes` uses for a HIR-lowering-seeded
    /// builtin exception class, per D-188) instead of `bind_class`.
    #[test]
    fn resolve_method_call_rejects_a_direct_call_on_a_synthetic_class() {
        let mut env = crate::Environment::new();
        env.bind_synthetic_class(
            "Exception".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: Some(0),
                name: "Exception".to_string(),
                bases: Vec::new(),
                mro: vec!["Exception".to_string()],
                attrs: vec![],
                methods: vec![("__init__".to_string(), "Exception.__init__".to_string())],
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
        let err = super::resolve_method_call(
            &env,
            &Ty::Instance(Box::new("Exception".to_string())),
            "__init__",
            &[],
            &[Ty::Str],
        )
        .expect_err("a direct call on a synthetic class's method must be rejected");
        assert_eq!(err.code, "C0001");
        assert!(
            err.message
                .contains("cannot call `__init__` directly on `Exception`"),
            "unexpected diagnostic: {}",
            err.message
        );
    }
}
