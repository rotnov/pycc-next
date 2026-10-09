//! `base.attr = value` checking (`check_attr_set`), extracted verbatim from
//! `crates/pycc_types/src/class.rs` per AGENTS.md's file-decomposition rule
//! and D-185's per-file tracking issue (#549), when #1219 added the enum-member
//! arm. Every other diagnostic and check is unchanged; the only other edits
//! are the ones the module boundary forces (`use` lines).

use crate::{Environment, infer_expr_in, is_assignable, object_box};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirExpr, Ty};

use super::{expect_class, lookup_class_attr_through_mro, resolve_attr_get};

/// Checks `base.attr = value` (`HirStmt::AttrSet`), shared between module
/// scope (`check_stmt`, `local_names = &[]`) and function-body scope
/// (`check_stmt_in_function`) -- mirroring how `check_dict_set` is already
/// split the same way for `HirStmt::DictSet`. Reuses [`resolve_attr_get`]
/// for the attribute-type lookup, so an attribute name the class never
/// declares produces the identical `T0044` diagnostic an attribute *read*
/// would. A CPython-object base is checked by
/// `foreign::attr_store::check_object_attr_set` (#1457), and any other base
/// that is neither an instance nor a protocol is a `T0043` naming the store.
///
/// #377: if `attr` is a `@property`, the check is redirected to the
/// property's setter: a read-only property (no setter) is rejected with
/// `T0044`, and a property with a setter checks the assigned value against
/// the setter's own parameter type (not the getter's return type -- the
/// two may differ, though they usually match). This mirrors CPython's own
/// observable behavior, where `obj.x = value` invokes the property's
/// `__set__` descriptor method, not a bare slot write.
///
/// #432: property lookup walks the MRO, so a property defined in a base
/// class is found when setting an attribute on a derived class instance.
pub(crate) fn check_attr_set(
    env: &Environment,
    local_names: &[&str],
    base: &HirExpr,
    attr: &str,
    value: &HirExpr,
) -> Result<(), Diagnostic> {
    let base_ty = infer_expr_in(env, local_names, base)?;
    // #1457 (Part 2 of #1443): a CPython object accepts any attribute name
    // and decides at run time; only the value's type is checked here.
    if base_ty == Ty::Object {
        return crate::foreign::attr_store::check_object_attr_set(env, local_names, value);
    }
    // #1457: a store on a value that is neither an instance nor a protocol
    // names the store, not the read `resolve_attr_get` would report.
    if !matches!(base_ty, Ty::Instance(_) | Ty::Protocol(_)) {
        return Err(super::t0043_not_an_instance(
            "assign an attribute",
            &base_ty,
        ));
    }
    // #1219: an enum member's `value` and `name` are read-only in CPython
    // (`AttributeError: <enum 'Enum'> cannot set attribute 'value'`), but
    // they are ordinary slots in pycc's model of an enum class, so without
    // this arm `c.value = ...` compiled and mutated the member. Only the two
    // attributes the enum class declares are refused here: CPython accepts
    // `c.foo = 1` on a member, and pycc refuses it through the ordinary
    // unknown-attribute `T0044` below, since the class declares no `foo`. An
    // enum class cannot be subclassed (`C0001` in `pycc_hir`), so the class
    // the base types as is the enum itself and no MRO walk is needed.
    if let Ty::Instance(class_name) = &base_ty {
        let class_def = expect_class(env, class_name);
        if class_def.is_enum && class_def.attrs.iter().any(|(name, _)| name == attr) {
            return Err(Diagnostic::error(
                "T0044",
                format!(
                    "cannot assign to `{attr}` of a member of enum `{class_name}`: an enum \
                     member's `value` and `name` are read-only (CPython raises `AttributeError`)"
                ),
                Span::new(0, 0),
            ));
        }
    }
    // #911 (Part 1 of #885): every write path to a class-level attribute is
    // rejected. A class attribute is a compile-time constant folded at each
    // read (`pycc_mir` never allocates a slot for it), so `obj.X = 5` has
    // nowhere to write -- and CPython's own semantics here (the write
    // creates an *instance* attribute that shadows the class one, leaving
    // the class attribute itself untouched) are not modelled at all. This
    // check runs before the property walk and before `resolve_attr_get`, and
    // covers every write that reaches the type checker: `obj.X = 5` and
    // `self.X = 5` in a method other than `__init__`.
    //
    // A `self.X = 5` inside `__init__` splits by where `X` was declared.
    // When the *same* class declares it, `collect_init_attrs` turns the
    // write into an instance slot and HIR's own
    // `reject_class_attr_collisions` (see `pycc_hir::class::body`) rejects
    // that shape first with C0001. When an *ancestor* declares it, HIR does
    // not look in that direction at all, and this check is the only thing
    // that rejects it -- which is why `lookup_class_attr_through_mro` must
    // stay a full-MRO walk and must not be narrowed to the class's own
    // `class_attrs`.
    //
    // #960 left this check deliberately coarse: it fires whenever a class
    // attribute of that name exists anywhere in the MRO, without asking
    // whether it actually *wins* the read-side precedence. For the
    // cross-sibling shape #960 fixed -- one base contributing the slot, an
    // independent sibling base the class attribute -- `resolve_attr_get` now
    // resolves the read to the slot, so this message's "no storage to write
    // to" is inaccurate for that one shape and the write is rejected where
    // CPython accepts it. That is a conservative rejection, never a wrong
    // answer, and lifting it would relax `docs/TYPE_SYSTEM.md`'s "every write
    // path to a class attribute is `T0044`" contract -- a separate,
    // decision-bearing change rather than part of #960's read fix.
    if let Ty::Instance(class_name) = &base_ty
        && let Some(class_attr_ty) = lookup_class_attr_through_mro(env, class_name, attr)
    {
        // Part 1 of #1284: a `staticmethod(<foreign callable>)` class
        // attribute is no constant either, and has no storage of its own.
        if super::foreign_static::is_foreign_static_class_attr(env, class_name, attr) {
            return Err(Diagnostic::error(
                "T0044",
                format!(
                    "cannot assign to `{attr}`: it is a `staticmethod(...)` class attribute of \
                     class `{class_name}`, which is re-read from its foreign callable at every \
                     access and has no storage to write to"
                ),
                Span::new(0, 0),
            ));
        }
        return Err(Diagnostic::error(
            "T0044",
            format!(
                "cannot assign to `{attr}`: it is a class-level attribute of class \
                 `{class_name}` (declared `{}`), which is a compile-time constant with no \
                 storage to write to",
                class_attr_ty.name()
            ),
            Span::new(0, 0),
        ));
    }
    // #432/#377: walk the MRO for property lookup first (matching
    // `resolve_attr_get`'s own properties-first-across-full-MRO logic and
    // the MIR lowering's own logic), across ALL classes in the MRO, then
    // fall back to regular attribute slots. A property setter has its own
    // parameter type (the value the setter accepts), which may differ from
    // the getter's return type -- so the value is checked against the
    // setter's parameter, not `resolve_attr_get`'s getter-return-type.
    if let Ty::Instance(class_name) = &base_ty {
        let class_def = expect_class(env, class_name);
        for mro_class in &class_def.mro {
            let mro_def = expect_class(env, mro_class);
            if let Some(prop) = mro_def.properties.iter().find(|p| p.name == attr) {
                let value_ty = infer_expr_in(env, local_names, value)?;
                let Some(setter_mangled) = &prop.setter else {
                    return Err(Diagnostic::error(
                        "T0044",
                        format!(
                            "property `{attr}` of class `{mro_class}` is read-only (has no setter)"
                        ),
                        Span::new(0, 0),
                    ));
                };
                let (param_tys, _) = env.lookup_function(setter_mangled).unwrap_or_else(|| {
                    panic!(
                        "pycc_types: internal error: property setter `{setter_mangled}` is in class \
                         `{mro_class}`'s own property table but was not registered as an ordinary \
                         function"
                    )
                });
                let setter_param_ty = &param_tys[1]; // exclude `self`
                if !is_assignable(value_ty.clone(), setter_param_ty.clone())
                    && !object_box::admits_value(env, value, &value_ty, setter_param_ty)?
                {
                    return Err(Diagnostic::error(
                        "T0021",
                        format!(
                            "cannot assign `{}` to property `{attr}` (setter expects `{}`)",
                            value_ty.name(),
                            setter_param_ty.name()
                        ),
                        Span::new(0, 0),
                    )
                    .with_help(format!(
                        "change the value to `{}` (the setter's expected type), or the \
                         setter's parameter annotation to `{}` (the actual type)",
                        setter_param_ty.name(),
                        value_ty.name()
                    )));
                }
                return Ok(());
            }
        }
    }
    // Regular attribute slot -- `resolve_attr_get` already walks the MRO.
    let attr_ty = resolve_attr_get(env, &base_ty, attr)?;
    let value_ty = infer_expr_in(env, local_names, value)?;
    if !is_assignable(value_ty.clone(), attr_ty.clone())
        && !object_box::admits_value(env, value, &value_ty, &attr_ty)?
    {
        return Err(Diagnostic::error(
            "T0021",
            format!(
                "cannot assign `{}` to attribute `{attr}` of type `{}`",
                value_ty.name(),
                attr_ty.name()
            ),
            Span::new(0, 0),
        )
        .with_help(format!(
            "change the value to `{}` (the expected/declared type), or the \
             declaration/annotation to `{}` (the actual type)",
            attr_ty.name(),
            value_ty.name()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use pycc_diag::Diagnostic;

    const ENUM: &str = "from enum import Enum\n\nclass Color(Enum):\n    RED = 1\n\n";

    fn check(source: &str) -> Result<pycc_hir::HirModule, Diagnostic> {
        let module = pycc_parser::parse(source).expect("test fixture must parse");
        let hir = pycc_hir::lower_checked(&module).expect("test fixture must lower");
        crate::check_and_resolve(&hir)
    }

    /// #1219: every shape a member reaches an attribute store through -- a
    /// parameter, a module-level alias and the member expression itself --
    /// types as `Ty::Instance(Color)` and is refused for both reserved
    /// attributes.
    #[test]
    fn an_enum_member_value_or_name_store_is_refused() {
        for (store, attr) in [
            (
                "def f(c: Color) -> None:\n    c.value = c.value + 1\n",
                "value",
            ),
            ("def f(c: Color) -> None:\n    c.name = \"x\"\n", "name"),
            ("c = Color.RED\nc.value = 7\n", "value"),
            ("c = Color.RED\nc.name = \"x\"\n", "name"),
            ("Color.RED.value = 3\n", "value"),
            ("c = Color.RED\nc.value += 1\n", "value"),
        ] {
            let diagnostic = check(&format!("{ENUM}{store}")).expect_err(store);
            assert_eq!(diagnostic.code, "T0044", "{store}");
            assert_eq!(
                diagnostic.message,
                format!(
                    "cannot assign to `{attr}` of a member of enum `Color`: an enum member's \
                     `value` and `name` are read-only (CPython raises `AttributeError`)"
                ),
                "{store}"
            );
        }
    }

    /// Any other attribute keeps the ordinary unknown-attribute refusal: the
    /// enum arm does not claim CPython refuses `c.foo = 1`, which it accepts.
    #[test]
    fn another_attribute_of_an_enum_member_keeps_the_unknown_attribute_refusal() {
        let diagnostic = check(&format!("{ENUM}c = Color.RED\nc.foo = 1\n")).unwrap_err();
        assert_eq!(diagnostic.code, "T0044");
        assert_eq!(
            diagnostic.message,
            "class `Color` has no attribute named `foo`"
        );
    }

    /// A non-enum instance with a slot named `value` is still writable.
    #[test]
    fn a_value_slot_on_an_ordinary_class_stays_writable() {
        check(
            "class Box:\n    def __init__(self, value: int) -> None:\n        self.value = value\n\n\
             b = Box(1)\nb.value = 2\nprint(b.value)\n",
        )
        .expect("an ordinary slot store type-checks");
    }
}
