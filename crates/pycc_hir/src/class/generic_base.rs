//! A class header's positional bases (#432), with a `typing.Generic[...]`
//! base erased (Part 1 of #886, #1394).
//!
//! `class C(Generic[T]):` declares that `C` is generic in the type variable
//! `T`. pycc has no runtime generic-alias machinery for such a class, so the
//! base is erased: it contributes nothing beyond `object` to the class's
//! bases, MRO or layout, and its arguments are never resolved to a `Ty`,
//! exactly as Part 1 of #1367 erases the subscript of a foreign-class
//! annotation. What is checked is that each argument *names* a type
//! variable -- a module-level `T = TypeVar("T")` (`module::type_var`) or a
//! name a foreign import binds, which pycc cannot inspect and so takes on
//! trust. `docs/TYPE_SYSTEM.md` ("Generics") lists the CPython deviations.

use crate::{ImportBinding, unsupported};
use pycc_ast::{Expr, StmtClassDef};
use pycc_diag::Diagnostic;

/// Whether `expr` names the registered `typing` symbol `symbol` through an
/// import binding: the bare name `from typing import <symbol>` bound, or
/// `<m>.<symbol>` where `import typing` bound `m`. A user definition of the
/// same name is never matched -- `Generic` and `TypeVar` are not resolved by
/// spelling.
pub(crate) fn names_typing_symbol(expr: &Expr, imports: &[ImportBinding], symbol: &str) -> bool {
    match expr {
        Expr::Name(name) => imports.iter().any(|binding| {
            matches!(
                binding,
                ImportBinding::Symbol { local_name, module: pycc_std::StdModule::Typing, symbol: bound }
                    if local_name == name.id.as_str() && bound.name == symbol
            )
        }),
        Expr::Attribute(attr) if attr.attr.as_str() == symbol => {
            let Expr::Name(root) = attr.value.as_ref() else {
                return false;
            };
            imports.iter().any(|binding| {
                matches!(
                    binding,
                    ImportBinding::Module { local_name, module: pycc_std::StdModule::Typing }
                        if local_name == root.id.as_str()
                )
            })
        }
        _ => false,
    }
}

/// The class header's base names in source order, with a `Generic[...]`
/// base checked and dropped.
///
/// `type_vars` are the module-level `TypeVar` declarations seen so far, and
/// `has_type_params` is whether the class is a PEP 695 `class C[T]:`, which
/// is already generic and so may not also list `Generic[...]` (CPython
/// raises `TypeError` for both that and a second `Generic[...]` base).
pub(super) fn lower_base_names(
    def: &StmtClassDef,
    imports: &[ImportBinding],
    type_vars: &[String],
    has_type_params: bool,
) -> Result<Vec<String>, Diagnostic> {
    let mut bases: Vec<String> = Vec::new();
    let Some(arguments) = def.arguments.as_deref() else {
        return Ok(bases);
    };
    if !arguments.keywords.is_empty() {
        return Err(unsupported(
            "keyword arguments in a class header (e.g. `metaclass=`) are not supported yet",
            def.range,
        ));
    }
    let class_name = def.name.as_str();
    let mut generic_seen = false;
    for arg in arguments.args.iter() {
        if let Expr::Subscript(subscript) = arg
            && names_typing_symbol(&subscript.value, imports, "Generic")
        {
            if generic_seen {
                return Err(unsupported(
                    format!(
                        "class `{class_name}` lists `Generic[...]` more than once -- a class \
                         may inherit from `Generic[...]` only once"
                    ),
                    pycc_ast::expr_range(arg),
                ));
            }
            if has_type_params {
                return Err(unsupported(
                    format!(
                        "class `{class_name}` declares type parameters (`class \
                         {class_name}[T]:`) and also lists a `Generic[...]` base -- a PEP 695 \
                         generic class is already generic, so drop the `Generic[...]` base"
                    ),
                    pycc_ast::expr_range(arg),
                ));
            }
            check_type_variable_arguments(&subscript.slice, imports, type_vars)?;
            generic_seen = true;
            continue;
        }
        if names_typing_symbol(arg, imports, "Generic") {
            return Err(unsupported(
                format!(
                    "class `{class_name}` lists a plain `Generic` base -- write \
                     `Generic[T]` with the class's type variables"
                ),
                pycc_ast::expr_range(arg),
            ));
        }
        let Expr::Name(name) = arg else {
            return Err(unsupported(
                "a base class must be a bare name (e.g. `class C(Base):`), not an \
                 attribute access or other expression",
                pycc_ast::expr_range(arg),
            ));
        };
        let base_name = name.id.to_string();
        // Reject duplicate bases in the same class header.
        if bases.contains(&base_name) {
            return Err(unsupported(
                format!(
                    "class `{class_name}` lists base `{base_name}` more than once -- duplicate \
                     bases are not supported"
                ),
                def.range,
            ));
        }
        bases.push(base_name);
    }
    Ok(bases)
}

/// Checks that every argument of a `Generic[...]` base is a distinct bare
/// name bound to a type variable.
fn check_type_variable_arguments(
    slice: &Expr,
    imports: &[ImportBinding],
    type_vars: &[String],
) -> Result<(), Diagnostic> {
    let arguments = match slice {
        Expr::Tuple(tuple) => &tuple.elts[..],
        single => std::slice::from_ref(single),
    };
    if arguments.is_empty() {
        return Err(unsupported(
            "a `Generic[...]` base must name at least one type variable",
            pycc_ast::expr_range(slice),
        ));
    }
    let mut seen: Vec<&str> = Vec::new();
    for argument in arguments {
        let Expr::Name(name) = argument else {
            return Err(unsupported(
                "an argument of a `Generic[...]` base must be the bare name of a type \
                 variable (a module-level `T = TypeVar(\"T\")` or a foreign-imported name)",
                pycc_ast::expr_range(argument),
            ));
        };
        let name = name.id.as_str();
        let is_type_variable = type_vars.iter().any(|declared| declared == name)
            || imports.iter().any(|binding| {
                matches!(binding, ImportBinding::Foreign { local_name, .. } if local_name == name)
            });
        if !is_type_variable {
            return Err(unsupported(
                format!(
                    "`{name}` in a `Generic[...]` base is not a type variable -- declare it \
                     at module level as `{name} = TypeVar(\"{name}\")` or import it"
                ),
                pycc_ast::expr_range(argument),
            ));
        }
        if seen.contains(&name) {
            return Err(unsupported(
                format!("type variable `{name}` appears more than once in a `Generic[...]` base"),
                pycc_ast::expr_range(argument),
            ));
        }
        seen.push(name);
    }
    Ok(())
}

#[cfg(test)]
#[path = "generic_base_tests.rs"]
mod tests;
