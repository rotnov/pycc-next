//! Static- and class-method call resolution
//! (`resolve_static_or_class_method_call`, `has_static_or_class_method`),
//! extracted verbatim from `crates/pycc_types/src/class.rs` per AGENTS.md's
//! file-decomposition rule and D-185's per-file tracking issue (#549). This
//! is one cohesion-driven seam of that ~5,300-line file, not a rewrite:
//! every diagnostic message and every check is unchanged, and the only
//! edits are the ones the module boundary forces (`use` lines and the
//! `super::` prefix the moved unit tests already used).
//!
//! The seam is `ClassName.m(args)` -- a call that names the *class* rather
//! than an instance. Two separate MRO walks answer it, `static_methods`
//! first and `class_methods` second, and each has its own success exit;
//! #1174's buffer-return interception sits on both, which is why the pair
//! belongs in one module rather than being split by decorator kind.
//! `base.method(args)` against an *instance* is `class/method_call.rs`, and
//! `super().m(args)` is `class/super_call.rs`.

use crate::Environment;
use pycc_diag::Diagnostic;
use pycc_hir::{HirExpr, Ty};

use super::{check_call_args, expect_class, t0044_unknown_member};

/// #436: Resolves a call to a `@staticmethod` or `@classmethod` through
/// the MRO. `class_name` is the class to start the MRO walk from (either
/// the class itself for `ClassName.method(args)`, or the instance's class
/// for `instance.method(args)`). `is_class_call` is `true` when the call
/// is made on the class name (`ClassName.method(args)`), `false` when on
/// an instance. Both static and class methods can be called on either.
///
/// For a static method, the full parameter list is checked (no `self`/`cls`
/// exclusion). For a class method, the first parameter (`cls`) is excluded
/// from the argument check, matching how `resolve_method_call` excludes
/// `self`.
pub(crate) fn resolve_static_or_class_method_call(
    env: &Environment,
    class_name: &str,
    method: &str,
    args: &[HirExpr],
    arg_tys: &[Ty],
) -> Result<Ty, Diagnostic> {
    let class_def = expect_class(env, class_name);
    // #436: walk the MRO in order, checking static_methods first, then
    // class_methods. The first class that declares the method wins.
    for mro_class in &class_def.mro {
        let mro_def = expect_class(env, mro_class);
        if let Some((_, mangled)) = mro_def
            .static_methods
            .iter()
            .find(|(name, _)| name == method)
        {
            let (param_tys, return_ty) = env.lookup_function(mangled).unwrap_or_else(|| {
                panic!(
                    "pycc_types: internal error: `{mangled}` is in class `{mro_class}`'s own \
                     static_methods table but was not registered as an ordinary function"
                )
            });
            // #1174: see `class/method_call.rs`'s own exit.
            crate::buffer::refuse_buffer_returning_method(class_name, method, return_ty)?;
            check_call_args(env, method, args, arg_tys, param_tys, false)?;
            return Ok(return_ty.clone());
        }
    }
    for mro_class in &class_def.mro {
        let mro_def = expect_class(env, mro_class);
        if let Some((_, mangled)) = mro_def
            .class_methods
            .iter()
            .find(|(name, _)| name == method)
        {
            let (param_tys, return_ty) = env.lookup_function(mangled).unwrap_or_else(|| {
                panic!(
                    "pycc_types: internal error: `{mangled}` is in class `{mro_class}`'s own \
                     class_methods table but was not registered as an ordinary function"
                )
            });
            let method_param_tys = &param_tys[1..]; // exclude `cls`
            // #1174: the `class_methods` exit is separate from the
            // `static_methods` one above and needs its own interception.
            crate::buffer::refuse_buffer_returning_method(class_name, method, return_ty)?;
            check_call_args(env, method, args, arg_tys, method_param_tys, false)?;
            return Ok(return_ty.clone());
        }
    }
    Err(t0044_unknown_member("method", class_name, method))
}

/// #436: Checks whether `class_name` has a static or class method named
/// `method` in its MRO. Used by `infer_expr_in`'s `MethodCall` arm to
/// decide whether to intercept before the regular `resolve_method_call`
/// fallback (which requires a `Ty::Instance` base).
pub(crate) fn has_static_or_class_method(
    env: &Environment,
    class_name: &str,
    method: &str,
) -> bool {
    let Some(class_def) = env.lookup_class(class_name) else {
        return false;
    };
    class_def.mro.iter().any(|mro_class| {
        let Some(mro_def) = env.lookup_class(mro_class) else {
            return false;
        };
        mro_def
            .static_methods
            .iter()
            .any(|(name, _)| name == method)
            || mro_def.class_methods.iter().any(|(name, _)| name == method)
    })
}

#[cfg(test)]
mod tests {
    use pycc_hir::{HirClassDef, Ty};

    #[test]
    fn resolve_static_or_class_method_call_with_unknown_method_returns_t0044() {
        // Directly exercises the `Err(t0044_unknown_member(...))` fallback
        // in `resolve_static_or_class_method_call` — unreachable from
        // `infer_expr_in` (which calls `has_static_or_class_method` first),
        // but the function is `pub(crate)` and can be called directly.
        let mut env = crate::Environment::new();
        env.bind_class(
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
        let diagnostic =
            super::resolve_static_or_class_method_call(&env, "C", "nonexistent", &[], &[])
                .unwrap_err();
        assert_eq!(diagnostic.code, "T0044");
    }

    #[test]
    fn has_static_or_class_method_returns_false_for_unknown_class() {
        // Exercises the `env.lookup_class(class_name)` → `None` →
        // `return false` path in `has_static_or_class_method`.
        let env = crate::Environment::new();
        assert!(!super::has_static_or_class_method(
            &env,
            "Nonexistent",
            "create"
        ));
    }

    #[test]
    fn has_static_or_class_method_returns_false_for_unknown_method() {
        // Exercises the `mro_def.static_methods.iter().any(..)` → false
        // and `mro_def.class_methods.iter().any(..)` → false path.
        let mut env = crate::Environment::new();
        env.bind_class(
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
        );
        assert!(!super::has_static_or_class_method(&env, "C", "nonexistent"));
    }

    #[test]
    fn has_static_or_class_method_returns_false_for_ghost_mro_class() {
        // Exercises the `env.lookup_class(mro_class)` → `None` →
        // `return false` path inside the MRO walk of
        // `has_static_or_class_method`.
        let mut env = crate::Environment::new();
        env.bind_class(
            "Derived".to_string(),
            HirClassDef {
                class_attrs: Vec::new(),
                exception_type_tag: None,
                name: "Derived".to_string(),
                bases: vec!["Ghost".to_string()],
                mro: vec!["Derived".to_string(), "Ghost".to_string()],
                attrs: Vec::new(),
                methods: vec![("__init__".to_string(), "Derived.__init__".to_string())],
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
        // `Ghost` is in the MRO but not registered — the defensive
        // `return false` prevents a panic and the overall result is false.
        assert!(!super::has_static_or_class_method(
            &env, "Derived", "create"
        ));
    }

    #[test]
    #[should_panic(expected = "internal error: `C.create.static` is in class `C`'s own \
                   static_methods table but was not registered as an ordinary function")]
    fn resolve_static_method_call_panics_if_function_not_registered() {
        // Exercises the defensive `unwrap_or_else(|| panic!(..))` in the
        // static-method lookup path of `resolve_static_or_class_method_call`.
        // This is unreachable from normal HIR (the HIR lowering always
        // emits the function alongside the table entry), so a hand-built
        // environment with a table entry but no registered function is needed.
        let mut env = crate::Environment::new();
        env.bind_class(
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
        let _ = super::resolve_static_or_class_method_call(&env, "C", "create", &[], &[Ty::Int]);
    }

    #[test]
    #[should_panic(
        expected = "internal error: `C.greet.classmethod` is in class `C`'s own \
                   class_methods table but was not registered as an ordinary function"
    )]
    fn resolve_class_method_call_panics_if_function_not_registered() {
        // Exercises the defensive `unwrap_or_else(|| panic!(..))` in the
        // class-method lookup path of `resolve_static_or_class_method_call`.
        let mut env = crate::Environment::new();
        env.bind_class(
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
                static_methods: Vec::new(),
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
        );
        let _ = super::resolve_static_or_class_method_call(&env, "C", "greet", &[], &[Ty::Int]);
    }
}
