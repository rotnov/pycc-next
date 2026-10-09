//! Zero-argument `super()` resolution (`resolve_super_attr_get`,
//! `resolve_super_method_call`), extracted verbatim from
//! `crates/pycc_types/src/class.rs` per AGENTS.md's file-decomposition rule
//! and D-185's per-file tracking issue (#549). This is one cohesion-driven
//! seam of that ~5,300-line file, not a rewrite: every diagnostic message
//! and every check is unchanged, and the only edits are the ones the module
//! boundary forces (`use` lines and the `super::` prefix the moved unit
//! tests already used).
//!
//! The two functions are kept together because they are near-duplicates at
//! the top: both reject the receiver-less `super()` of a `@classmethod` or
//! `@staticmethod` with the same `C0001` (#915), and both then compute the
//! same MRO slice *after* the current class. Splitting `super().attr` from
//! `super().m()` would duplicate that prologue across two files and let the
//! two copies drift; they also share the `super_env` /
//! `super_env_without_self` unit-test fixtures.

use crate::Environment;
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirExpr, Ty};

use super::{
    check_call_args, expect_class, foreign_static, t0044_unknown_member, t0047_super_instance_attr,
};

/// The anchor class and the MRO slice a zero-argument `super()` searches:
/// the classes *after* the anchor (the class whose body is being checked,
/// `env.current_class()`) in the **receiver's** MRO (#1337, D-254).
///
/// The receiver is the class `self` is typed as. For an ordinary method
/// body it is the anchor itself, so this is D-160's rule unchanged. For an
/// inherited body compiled for a subclass `C` (a receiver-exact copy,
/// D-254), `self` is typed `C` while the anchor stays the defining class,
/// and the slice follows `C`'s MRO -- CPython's cooperative order, where
/// `super()` inside `L.f` reaches `R.f` for `class C(L, R)`. Mirrors
/// `pycc_mir`'s `super()` lowering, so the checker types the same body the
/// lowering calls.
fn super_mro(env: &Environment) -> (&str, &[String]) {
    let anchor = env.current_class().unwrap();
    let receiver = match env.lookup("self") {
        Some(Ty::Instance(class)) if env.lookup_class(&class).is_some() => env
            .classes
            .get_key_value(class.as_str())
            .unwrap()
            .0
            .as_str(),
        _ => anchor,
    };
    let receiver_def = expect_class(env, receiver);
    let pos = receiver_def
        .mro
        .iter()
        .position(|c| c == anchor)
        .expect("a super() anchor is in its receiver's MRO");
    (anchor, &receiver_def.mro[pos + 1..])
}

/// #433: Resolves `super().attr` — an attribute read through zero-arg
/// `super()`. The resolution starts from the class *after* the current
/// class in the MRO (not the current class itself), matching CPython's own
/// `super().__getattribute__` semantics: `super()` skips the current
/// class's own entries and searches the rest of the MRO.
///
/// Only *class-level* members resolve here. A `super` object proxies the
/// class-level attributes and descriptors it finds along the MRO; it does
/// not proxy the instance `__dict__`, so an attribute established by
/// `self.<attr> = ...` inside `__init__` is not reachable through it and
/// CPython raises `AttributeError: 'super' object has no attribute
/// '<attr>'`. pycc used to resolve that form against `self`'s own slot and
/// return a value, which disagreed with the pinned oracle on the value of
/// an expression rather than merely on what compiles (#587); it is now
/// rejected with `T0047`. Of the class-level members pycc models, a base
/// class `@property` and a base class *class attribute* (#911) are the
/// two reachable this way (#915), so this function resolves one of those
/// or rejects.
pub(crate) fn resolve_super_attr_get(env: &Environment, attr: &str) -> Result<Ty, Diagnostic> {
    // #915: both `super()` forms below assume a `self` receiver -- the
    // `pycc_mir` lowering of `super().attr` and `super().m()` passes the
    // enclosing method's `self` binding as the receiver and panics when it
    // is absent. Inside a `@classmethod` the first parameter is bound as
    // `cls` and inside a `@staticmethod` there is no receiver at all
    // (`pycc_hir::class`), so a `self` binding here can only be a regular
    // method's receiver: `pycc_hir` requires a regular method to declare a
    // first parameter, and `pycc_hir::class::receiver` lowers that
    // parameter under the canonical name `self` whatever the source spelled
    // it (#1181) -- the *positional* invariant this test rests on survives
    // that relaxation, the naming one is now the compiler's own
    // canonicalization rather than a rule imposed on the source. Reject the
    // receiver-less forms as a capability gap (`C0001`) rather than
    // letting them reach `pycc_mir` and abort the compiler. The single
    // approximation is a `@classmethod` or `@staticmethod` that itself
    // declares a non-receiver parameter named `self`, which keeps the
    // previous behavior.
    if env.binding_state("self").is_none() {
        return Err(Diagnostic::error(
            "C0001",
            "`super()` inside a `@classmethod` or `@staticmethod` is not supported -- it has \
             no `self` receiver to bind"
                .to_string(),
            Span::new(0, 0),
        ));
    }
    let (current_class, super_mro) = super_mro(env);
    // Part 1 of #1284: a winning `staticmethod(...)` class attribute is
    // refused rather than typed as the `object` it is recorded as.
    foreign_static::refuse_super(env, super_mro, current_class, attr)?;
    // #915: walk the slice once, checking every class-level member kind on
    // each class before moving to the next -- a CPython `super` object
    // resolves against one class `__dict__` at a time, so the *MRO
    // position* decides, not the member kind. Scanning all properties
    // first and only then all class attributes would let a `@property` on
    // a later MRO entry outrank a class attribute on an earlier one; for
    // `class D(B, C)` with `B.X = 1` and a `C.X` property, CPython yields
    // `1`, the class attribute `B` contributes.
    //
    // Both member kinds are searched before the `T0047` instance-attribute
    // rejection below, and that ordering is *not* positional: an instance
    // attribute is never in any class `__dict__`, so a `super` object never
    // sees one at all. An instance attribute contributed by one MRO branch
    // must therefore not mask a class-level member contributed by another,
    // whatever their relative MRO positions.
    for mro_class in super_mro {
        let mro_def = expect_class(env, mro_class);
        if let Some(prop) = mro_def.properties.iter().find(|p| p.name == attr) {
            let (_, return_ty) = env.lookup_function(&prop.getter).unwrap();
            return Ok(return_ty.clone());
        }
        // A base class's class attribute (#911) is a genuine entry in that
        // class's `__dict__`, so a `super` object does proxy it.
        if let Some((_, ty, _)) = mro_def.class_attrs.iter().find(|(name, _, _)| name == attr) {
            return Ok(ty.clone());
        }
    }
    // #587: an instance attribute — established by `self.<attr> = ...`
    // inside `__init__` and stored in the instance's own slot — is not
    // proxied by a `super` object in CPython, which raises
    // `AttributeError: 'super' object has no attribute '<attr>'`. Reject
    // it rather than resolving it against `self`'s slot: the slot read is
    // exactly what `self.<attr>` already spells, so the rejection costs no
    // expressive power and the diagnostic can name the fix.
    for mro_class in super_mro {
        let mro_def = expect_class(env, mro_class);
        if mro_def.attrs.iter().any(|(name, _)| name == attr) {
            return Err(t0047_super_instance_attr(attr, mro_class));
        }
    }
    Err(t0044_unknown_member("attribute", current_class, attr))
}

/// #433: Resolves `super().method(args)` — a method call through zero-arg
/// `super()`. The resolution starts from the class *after* the current
/// class in the MRO, matching CPython's own `super()` semantics. The
/// `self` instance (the most-derived object) is the implicit first
/// argument, but the method's *signature* is checked against the caller's
/// supplied arguments only (excluding `self`), exactly like
/// `resolve_method_call`.
pub(crate) fn resolve_super_method_call(
    env: &Environment,
    method: &str,
    args: &[HirExpr],
    arg_tys: &[Ty],
) -> Result<Ty, Diagnostic> {
    // #915: both `super()` forms below assume a `self` receiver -- the
    // `pycc_mir` lowering of `super().attr` and `super().m()` passes the
    // enclosing method's `self` binding as the receiver and panics when it
    // is absent. Inside a `@classmethod` the first parameter is bound as
    // `cls` and inside a `@staticmethod` there is no receiver at all
    // (`pycc_hir::class`), so a `self` binding here can only be a regular
    // method's receiver: `pycc_hir` requires a regular method to declare a
    // first parameter, and `pycc_hir::class::receiver` lowers that
    // parameter under the canonical name `self` whatever the source spelled
    // it (#1181) -- the *positional* invariant this test rests on survives
    // that relaxation, the naming one is now the compiler's own
    // canonicalization rather than a rule imposed on the source. Reject the
    // receiver-less forms as a capability gap (`C0001`) rather than
    // letting them reach `pycc_mir` and abort the compiler. The single
    // approximation is a `@classmethod` or `@staticmethod` that itself
    // declares a non-receiver parameter named `self`, which keeps the
    // previous behavior.
    if env.binding_state("self").is_none() {
        return Err(Diagnostic::error(
            "C0001",
            "`super()` inside a `@classmethod` or `@staticmethod` is not supported -- it has \
             no `self` receiver to bind"
                .to_string(),
            Span::new(0, 0),
        ));
    }
    let (current_class, super_mro) = super_mro(env);
    // Part 1 of #1284: the method walk below sees methods only, so a
    // `staticmethod(...)` class attribute that wins positionally must be
    // refused here or the call would reach a later base's method.
    foreign_static::refuse_super(env, super_mro, current_class, method)?;
    // #966: `super().__init__()` ranks constructors the same way
    // instantiation does, so a D-225 implicit constructor on an earlier
    // base must not out-rank a real one further along -- skip flagged
    // classes on the first pass. The skip is gated on the method name so
    // every other `super().m()` resolution stays name-agnostic, and the
    // second pass is mandatory: `class A: pass` / `class C(A)` calling
    // `super().__init__()` has only the implicit constructor to reach and
    // would otherwise get a spurious `T0044`. Mirrors `pycc_mir`'s
    // `super()` lowering exactly.
    let skip_implicit_init = method == "__init__";
    let resolve = |skip_implicit: bool| {
        super_mro.iter().find_map(|mro_class| {
            let mro_def = expect_class(env, mro_class);
            if skip_implicit && mro_def.implicit_object_init {
                return None;
            }
            mro_def
                .methods
                .iter()
                .find(|(name, _)| name == method)
                .map(|(_, mangled)| mangled.clone())
        })
    };
    let Some(mangled) = resolve(skip_implicit_init).or_else(|| resolve(false)) else {
        return Err(t0044_unknown_member("method", current_class, method));
    };
    let (param_tys, return_ty) = env.lookup_function(&mangled).unwrap();
    let method_param_tys = &param_tys[1..]; // exclude `self`
    // #1174: the fourth and last resolver exit that can hand back a
    // buffer-returning method's return type. `current_class` rather than
    // the MRO entry that declares the method -- see
    // `buffer::buffer_returning_method_call_unsupported`.
    crate::buffer::refuse_buffer_returning_method(current_class, method, return_ty)?;
    check_call_args(env, method, args, arg_tys, method_param_tys, false)?;
    Ok(return_ty.clone())
}

#[cfg(test)]
mod tests {
    use pycc_hir::{HirClassDef, Ty};

    // -- #433: super() type-checking tests ----------------------------------

    /// Builds an `Environment` with `current_class` set to `"B"`, a base
    /// class `"A"` in the MRO, and `extra_setup` to customize the class
    /// definitions and function registrations per test.
    fn super_env(extra_setup: impl FnOnce(&mut crate::Environment)) -> crate::Environment {
        let mut env = super_env_without_self(extra_setup);
        // #915: `resolve_super_attr_get` and `resolve_super_method_call`
        // reject a receiver-less `super()` with `C0001`, so every fixture
        // exercising the resolving path needs the enclosing method's `self`
        // binding, exactly as a real method body's environment has it.
        env.bind("self".to_string(), Ty::Instance(Box::new("B".to_string())));
        env
    }

    /// #915: the same fixture without the `self` binding -- the environment
    /// a `@classmethod` or `@staticmethod` body actually has, used by the
    /// `C0001` guard's own tests.
    fn super_env_without_self(
        extra_setup: impl FnOnce(&mut crate::Environment),
    ) -> crate::Environment {
        let mut env = crate::Environment::new();
        env.current_class = Some("B".to_string());
        extra_setup(&mut env);
        env
    }

    #[test]
    fn resolve_super_attr_get_rejects_a_receiver_less_super() {
        // #915: `super().X` inside a `@classmethod`/`@staticmethod` has no
        // `self` to bind, and `pycc_mir`'s `Super` arm would abort the
        // compiler computing one. Reject it as a capability gap instead.
        let env = super_env_without_self(|_| {});
        let err = super::resolve_super_attr_get(&env, "X").unwrap_err();
        assert_eq!(err.code, "C0001");
        assert!(
            err.message.contains("no `self` receiver to bind"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn resolve_super_method_call_rejects_a_receiver_less_super() {
        // #915: the method-call form of the same gap.
        let env = super_env_without_self(|_| {});
        let err = super::resolve_super_method_call(&env, "m", &[], &[]).unwrap_err();
        assert_eq!(err.code, "C0001");
        assert!(
            err.message.contains("no `self` receiver to bind"),
            "unexpected message: {}",
            err.message
        );
    }

    #[test]
    fn resolve_super_attr_get_returns_property_type() {
        use pycc_hir::PropertyDef;
        let env = super_env(|env| {
            env.bind_class(
                "A".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "A".to_string(),
                    bases: vec![],
                    mro: vec!["A".to_string()],
                    attrs: vec![("_val".to_string(), Ty::Int)],
                    methods: vec![("__init__".to_string(), "A.__init__".to_string())],
                    type_param: None,
                    properties: vec![PropertyDef {
                        name: "val".to_string(),
                        getter: "A.val".to_string(),
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
            env.bind_class(
                "B".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "B".to_string(),
                    bases: vec!["A".to_string()],
                    mro: vec!["B".to_string(), "A".to_string()],
                    attrs: vec![],
                    methods: vec![("__init__".to_string(), "B.__init__".to_string())],
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
            env.bind_function(
                "A.val".to_string(),
                vec![Ty::Instance(Box::new("A".to_string()))],
                Ty::Int,
            );
        });
        assert_eq!(super::resolve_super_attr_get(&env, "val"), Ok(Ty::Int));
    }

    #[test]
    fn resolve_super_attr_get_rejects_instance_attr() {
        // #587: `super().x` where `x` is an instance attribute declared by
        // a base class's `__init__`. CPython raises `AttributeError` here
        // because a `super` object does not proxy the instance's own
        // attributes, so pycc rejects it with `T0047` rather than
        // resolving it against `self`'s slot. This one test replaces the
        // three the old resolve-against-`self` behaviour needed (the
        // success path plus the `T0021` shared-slot redeclaration guard,
        // whose condition is unreachable once the form itself is gone).
        let env = super_env(|env| {
            env.bind_class(
                "A".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "A".to_string(),
                    bases: vec![],
                    mro: vec!["A".to_string()],
                    attrs: vec![("x".to_string(), Ty::Int)],
                    methods: vec![("__init__".to_string(), "A.__init__".to_string())],
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
            env.bind_class(
                "B".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "B".to_string(),
                    bases: vec!["A".to_string()],
                    mro: vec!["B".to_string(), "A".to_string()],
                    attrs: vec![],
                    methods: vec![("__init__".to_string(), "B.__init__".to_string())],
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
        });
        let err = super::resolve_super_attr_get(&env, "x").unwrap_err();
        assert_eq!(err.code, "T0047");
        assert!(
            err.message.contains("instance attribute of class `A`"),
            "T0047 should name the declaring class, got: {}",
            err.message
        );
        // #1181: a method's receiver may be spelled with any identifier and
        // a renamed receiver may not mention `self` at all, so the help names
        // the receiver generically -- this crate never sees the source
        // spelling (`crates/pycc_hir/src/class/receiver.rs` canonicalizes it).
        assert_eq!(
            err.help.as_deref(),
            Some("read it through the method's own receiver instead: `<receiver>.x`"),
            "T0047 should point at the equivalent receiver read"
        );
    }

    #[test]
    fn resolve_super_method_call_returns_return_type() {
        let env = super_env(|env| {
            env.bind_class(
                "A".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "A".to_string(),
                    bases: vec![],
                    mro: vec!["A".to_string()],
                    attrs: vec![],
                    methods: vec![("greet".to_string(), "A.greet".to_string())],
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
            env.bind_class(
                "B".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "B".to_string(),
                    bases: vec!["A".to_string()],
                    mro: vec!["B".to_string(), "A".to_string()],
                    attrs: vec![],
                    methods: vec![("__init__".to_string(), "B.__init__".to_string())],
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
            env.bind_function(
                "A.greet".to_string(),
                vec![Ty::Instance(Box::new("A".to_string()))],
                Ty::Int,
            );
        });
        assert_eq!(
            super::resolve_super_method_call(&env, "greet", &[], &[]),
            Ok(Ty::Int)
        );
    }

    #[test]
    fn resolve_super_method_call_returns_t0044_for_unknown_method() {
        let env = super_env(|env| {
            env.bind_class(
                "A".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "A".to_string(),
                    bases: vec![],
                    mro: vec!["A".to_string()],
                    attrs: vec![],
                    methods: vec![("__init__".to_string(), "A.__init__".to_string())],
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
            env.bind_class(
                "B".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "B".to_string(),
                    bases: vec!["A".to_string()],
                    mro: vec!["B".to_string(), "A".to_string()],
                    attrs: vec![],
                    methods: vec![("__init__".to_string(), "B.__init__".to_string())],
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
        });
        let err = super::resolve_super_method_call(&env, "nonexistent", &[], &[]).unwrap_err();
        assert_eq!(err.code, "T0044");
    }

    #[test]
    fn resolve_super_attr_get_returns_t0044_for_unknown_attr() {
        let env = super_env(|env| {
            env.bind_class(
                "A".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "A".to_string(),
                    bases: vec![],
                    mro: vec!["A".to_string()],
                    attrs: vec![("x".to_string(), Ty::Int)],
                    methods: vec![("__init__".to_string(), "A.__init__".to_string())],
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
            env.bind_class(
                "B".to_string(),
                HirClassDef {
                    class_attrs: Vec::new(),
                    exception_type_tag: None,
                    name: "B".to_string(),
                    bases: vec!["A".to_string()],
                    mro: vec!["B".to_string(), "A".to_string()],
                    attrs: vec![],
                    methods: vec![("__init__".to_string(), "B.__init__".to_string())],
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
        });
        let err = super::resolve_super_attr_get(&env, "nonexistent").unwrap_err();
        assert_eq!(err.code, "T0044");
    }
}
