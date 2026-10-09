//! Type checking of a class attribute bound to
//! `staticmethod(<foreign callable>)` (Part 1 of #1284, D-256).
//!
//! The attribute has no static type of its own worth checking: once the
//! shared `pycc_hir` MRO helpers decide that a read or call reaches it, the
//! use site is rewritten to the recorded foreign reference chain
//! (`ForeignCallableRef::read_expr` / `call_expr`) and that expression is
//! checked in the **use-site** environment, exactly as `pycc_mir` lowers
//! it. Everything here is either that rewrite or one of the refusals that
//! keep the rewrite sound:
//!
//! - the root of the chain must still denote the foreign import where the
//!   rewrite lands ([`check_use_site`]);
//! - an instance receiver of any shape is accepted: its type is inferred,
//!   and so checked, before the foreign target is resolved, and `pycc_mir`
//!   evaluates a non-name receiver for its effects ahead of the rewrite
//!   (#1346);
//! - an instance read whose positional winner is a method but that has a
//!   foreign entry later in the MRO is refused (rule 7, #1350);
//! - an instance receiver whose static class and some subclass disagree on
//!   the winner is refused, because pycc resolves the member statically
//!   (#1337, #1350);
//! - `super()` never takes the foreign path (#1358).

use crate::{Environment, infer_expr_in};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{
    ClassAttrValue, ClassNamespaceWinner, ForeignCallableRef, HirClassDef, HirExpr, Ty,
    class_name_foreign_static, class_namespace_winner, instance_foreign_static,
    method_shadows_foreign_static, slot_behind_foreign_static_meets_property, subclass_divergence,
};

/// The foreign target a read or call of `attr` through the class object
/// `class_name` reaches, or `None` for every other outcome.
pub(crate) fn class_name_target<'a>(
    env: &'a Environment,
    class_name: &str,
    attr: &str,
) -> Option<&'a ForeignCallableRef> {
    let class_def = env.lookup_class(class_name)?;
    class_name_foreign_static(&class_def.mro, |name: &str| env.lookup_class(name), attr)
}

/// `C.attr` through the class name: `Some(ty)` when the foreign path wins.
pub(crate) fn class_name_read(
    env: &Environment,
    local_names: &[&str],
    class_name: &str,
    attr: &str,
) -> Result<Option<Ty>, Diagnostic> {
    let Some(target) = class_name_target(env, class_name, attr) else {
        return Ok(None);
    };
    check_use_site(env, local_names, class_name, attr, target)?;
    infer_expr_in(env, local_names, &target.read_expr()).map(Some)
}

/// `C.attr(args)` through the class name: `Some(ty)` when the foreign path
/// wins.
pub(crate) fn class_name_call(
    env: &Environment,
    local_names: &[&str],
    class_name: &str,
    attr: &str,
    args: &[HirExpr],
) -> Result<Option<Ty>, Diagnostic> {
    let Some(target) = class_name_target(env, class_name, attr) else {
        return Ok(None);
    };
    check_use_site(env, local_names, class_name, attr, target)?;
    infer_expr_in(env, local_names, &target.call_expr(args.to_vec())).map(Some)
}

/// `x.attr` through an instance receiver typed `Ty::Instance`: `Some(ty)`
/// when the foreign path wins, `None` to fall through to the existing
/// dispatch unchanged. The receiver itself has already been inferred by the
/// caller.
pub(crate) fn instance_read(
    env: &Environment,
    local_names: &[&str],
    class_name: &str,
    attr: &str,
) -> Result<Option<Ty>, Diagnostic> {
    env.lookup_class(class_name).map_or(Ok(None), |class_def| {
        let lookup = |name: &str| env.lookup_class(name);
        if method_shadows_foreign_static(&class_def.mro, lookup, attr) {
            return Err(Diagnostic::error(
                "T0044",
                format!(
                    "reading `{class_name}.{attr}` through an instance reaches a method that \
                     shadows a `staticmethod(...)` class attribute later in the MRO -- a bound \
                     method read is not supported yet (#1350)"
                ),
                Span::new(0, 0),
            ));
        }
        instance_target(env, local_names, class_def, attr, "read")?
            .map(|target| infer_expr_in(env, local_names, &target.read_expr()))
            .transpose()
    })
}

/// `x.attr(args)` through an instance receiver: as [`instance_read`], for
/// a call. Rule 7 does not apply -- the existing method walks already find
/// the positional method winner first.
pub(crate) fn instance_call(
    env: &Environment,
    local_names: &[&str],
    class_name: &str,
    attr: &str,
    args: &[HirExpr],
) -> Result<Option<Ty>, Diagnostic> {
    env.lookup_class(class_name).map_or(Ok(None), |class_def| {
        instance_target(env, local_names, class_def, attr, "call")?
            .map(|target| infer_expr_in(env, local_names, &target.call_expr(args.to_vec())))
            .transpose()
    })
}

/// The shared instance-receiver gate: the subclass-divergence refusal,
/// then the winner, then the use-site check.
fn instance_target<'a>(
    env: &'a Environment,
    local_names: &[&str],
    class_def: &'a HirClassDef,
    attr: &str,
    what: &str,
) -> Result<Option<&'a ForeignCallableRef>, Diagnostic> {
    let class_name = class_def.name.as_str();
    let lookup = |name: &str| env.lookup_class(name);
    if let Some(subclass) = subclass_divergence(class_name, env.classes.values(), lookup, attr) {
        return Err(Diagnostic::error(
            "T0044",
            format!(
                "the {what} `{class_name}.{attr}` through an instance could reach a subclass \
                 override in `{subclass}` that pycc resolves statically (#1337); calling it \
                 through the class name `{class_name}.{attr}(...)` is supported (#1350)"
            ),
            Span::new(0, 0),
        ));
    }
    if slot_behind_foreign_static_meets_property(&class_def.mro, lookup, attr) {
        return Err(Diagnostic::error(
            "T0044",
            format!(
                "the {what} `{class_name}.{attr}` through an instance reaches an instance \
                 attribute that CPython reads ahead of the `staticmethod(...)` class attribute, \
                 while an `@property` of the same name sits later in the MRO -- pycc would \
                 resolve it to the property, so this combination is not supported"
            ),
            Span::new(0, 0),
        ));
    }
    let Some(target) = instance_foreign_static(&class_def.mro, lookup, attr) else {
        return Ok(None);
    };
    check_use_site(env, local_names, class_name, attr, target)?;
    Ok(Some(target))
}

/// The root of the rewritten chain must still denote the foreign import at
/// the use site: not a local of the enclosing function, and -- inside a
/// function body -- a member of the #1316 `foreign_globals` set, which
/// already drops a name the function binds locally.
fn check_use_site(
    env: &Environment,
    local_names: &[&str],
    class_name: &str,
    attr: &str,
    target: &ForeignCallableRef,
) -> Result<(), Diagnostic> {
    let root = target.root.as_str();
    let shadowed = local_names.contains(&root)
        || (env.in_function_body && !env.foreign_globals.contains(root));
    if !shadowed {
        return Ok(());
    }
    Err(Diagnostic::error(
        "T0044",
        format!(
            "class attribute `{class_name}.{attr}` refers to `{root}`, a foreign import of the \
             module that defines `{class_name}`, which is shadowed by a local binding here"
        ),
        Span::new(0, 0),
    )
    .with_help(format!("rename the binding of `{root}` in this scope")))
}

/// `super().attr` / `super().attr(args)` never takes the foreign path: a
/// `staticmethod(...)` class attribute that wins positionally over the
/// `super()` MRO slice is refused (#1358).
pub(crate) fn refuse_super(
    env: &Environment,
    super_mro: &[String],
    current_class: &str,
    attr: &str,
) -> Result<(), Diagnostic> {
    match class_namespace_winner(super_mro, |name: &str| env.lookup_class(name), attr) {
        Some(ClassNamespaceWinner::ForeignStatic { owner, .. }) => Err(Diagnostic::error(
            "T0044",
            format!(
                "`super().{attr}` in class `{current_class}` reaches the `staticmethod(...)` \
                 class attribute `{owner}.{attr}`, which is not supported through `super()` \
                 yet (#1358); use `{owner}.{attr}` instead"
            ),
            Span::new(0, 0),
        )),
        _ => Ok(()),
    }
}

/// Whether `attr`'s first `class_attrs` entry in `class_name`'s MRO is a
/// `staticmethod(...)` class attribute -- for protocol conformance, where
/// such an entry does not satisfy an attribute member.
pub(crate) fn is_foreign_static_class_attr(
    env: &Environment,
    class_name: &str,
    attr: &str,
) -> bool {
    env.lookup_class(class_name).is_some_and(|class_def| {
        class_def
            .mro
            .iter()
            .filter_map(|name| env.lookup_class(name))
            .find_map(|def| def.class_attrs.iter().find(|(name, _, _)| name == attr))
            .is_some_and(|(_, _, value)| matches!(value, ClassAttrValue::ForeignStatic(_)))
    })
}
