//! A module-level legacy type-variable declaration, `T = TypeVar("T")`
//! (Part 1 of #886, #1394).
//!
//! The statement is compile-time only: it emits no item and binds no module
//! global at run time. It does two things. It records `T` as a type
//! variable, so a later `class C(Generic[T]):` base is admitted
//! (`class::generic_base`). And it records `T` in the alias table as the
//! opaque `object`, the same erasure Part 1 of #1367 gives a name a foreign
//! import binds, so `def get(self) -> T` lowers as it does for a
//! foreign-imported type variable. Like that foreign entry, the alias is
//! marked imported, so `strip_imported` keeps it out of
//! `HirModule::type_aliases` and it never crosses a module boundary.
//!
//! Only the plain form is admitted: one positional string literal equal to
//! the bound name, and no further arguments or keywords. A constrained or
//! bounded `TypeVar` is refused rather than erased with its bound unchecked.

use super::ModuleState;
use crate::class::generic_base::names_typing_symbol;
use crate::{Ty, unsupported};
use pycc_ast::{Expr, Stmt};
use pycc_diag::Diagnostic;

/// Lowers `stmt` if it is a module-level `Name = TypeVar(...)` whose callee
/// is the `typing.TypeVar` an import bound; returns whether it was one.
pub(super) fn lower_type_var_decl(
    stmt: &Stmt,
    state: &mut ModuleState<'_>,
) -> Result<bool, Diagnostic> {
    let Stmt::Assign(assign) = stmt else {
        return Ok(false);
    };
    let Expr::Call(call) = assign.value.as_ref() else {
        return Ok(false);
    };
    if !names_typing_symbol(&call.func, &state.imports, "TypeVar") {
        return Ok(false);
    }
    let [Expr::Name(target)] = &assign.targets[..] else {
        return Err(unsupported(
            "a `TypeVar(...)` must be bound to a single name (`T = TypeVar(\"T\")`)",
            assign.range,
        ));
    };
    let name = target.id.as_str();
    let [Expr::StringLiteral(literal)] = &call.arguments.args[..] else {
        return Err(plain_form_only(name, assign.range.into()));
    };
    if !call.arguments.keywords.is_empty() {
        return Err(plain_form_only(name, assign.range.into()));
    }
    let spelled = literal.value.to_str();
    if spelled != name {
        return Err(unsupported(
            format!(
                "`TypeVar(\"{spelled}\")` is bound to `{name}` -- a type variable's name must \
                 match the variable it is assigned to"
            ),
            assign.range,
        ));
    }
    if state.aliases.iter().any(|(alias, _)| alias == name) {
        return Err(unsupported(
            format!("type variable `{name}` rebinds a name already bound as a type in this module"),
            assign.range,
        ));
    }
    // Same reverse-direction check as the type-alias arms in
    // `lower_top_level_item`: a class defined earlier under this name would
    // otherwise silently gain a second, alias-shaped binding.
    if state
        .class_defs
        .iter()
        .any(|(class_name, _)| class_name == name)
    {
        return Err(unsupported(
            format!(
                "type variable `{name}` collides with a class of the same name already defined \
                 in this module"
            ),
            assign.range,
        ));
    }
    state.imported_alias_indices.push(state.aliases.len());
    state.aliases.push((name.to_string(), Ty::Object));
    state.type_vars.push(name.to_string());
    Ok(true)
}

fn plain_form_only(name: &str, range: std::ops::Range<u32>) -> Diagnostic {
    unsupported(
        format!(
            "only the plain `{name} = TypeVar(\"{name}\")` form is supported -- a constrained or \
             bounded `TypeVar` (further arguments or keywords) is not supported yet"
        ),
        range,
    )
}
