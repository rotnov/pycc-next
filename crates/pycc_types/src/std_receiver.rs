//! Stdlib receiver resolution and the receiver-shadow check (D-136, Part 1
//! of #883 / #962).
//!
//! `pycc_hir` lowers `math.sqrt(x)`/`math.pi` (and, since #962, their
//! aliased spellings `m.sqrt(x)`/`m.pi` after `import math as m`) to the
//! canonical `"math.sqrt"`/`"math.pi"` strings. This module turns such a
//! string back into its registry entry and decides whether the receiver
//! the user could have written is shadowed by a real binding at the use
//! site -- the first stage with binding-scope information.

use pycc_diag::{Diagnostic, Span};
use pycc_hir::ImportBinding;
use pycc_std::StdModule;

/// D-136: resolves a `HirExpr::Call`/`HirExpr::Name` callee/name string
/// that `pycc_hir`'s lowering already qualified as `<module>.<symbol>`
/// (e.g. `"math.sqrt"`, `"math.pi"`) back into its `pycc_std` registry
/// entry. A real Python identifier can never contain `.`, so `split_once`
/// finding one is itself sufficient evidence this is a stdlib-qualified
/// name, not an ordinary user identifier -- no additional import-binding
/// lookup is needed here (pycc_hir already gated construction of this
/// shape on a successful `pycc_std::resolve_module`/`resolve_symbol`
/// lookup at lowering time; re-resolving here is cheap and idempotent).
/// The prefix is always the module's canonical spelling, never an alias
/// (#962), so `resolve_module` always answers it.
pub(crate) fn std_qualified_symbol(name: &str) -> Option<pycc_std::StdSymbol> {
    let (module_name, symbol_name) = name.split_once('.')?;
    let module = pycc_std::resolve_module(module_name)?;
    pycc_std::resolve_symbol(module, symbol_name)
}

/// Every module alias a module's import table binds (Part 1 of #883,
/// #962): the `(local_name, module)` pairs of its `ImportBinding::Module`
/// entries whose `local_name` differs from the module's canonical
/// spelling. The canonical spelling is never listed here -- every site
/// checks it unconditionally through [`shadowed_std_receiver`] -- so an
/// unaliased `import math` contributes nothing.
///
/// Called from exactly two places: the entry of
/// `module::check_with_environment_all`, the common sink of both
/// `Environment` constructors, and the solver's globals
/// `ConstraintEnvironment` in `constraints::signatures`. Because
/// `pycc_hir::program::link` concatenates every module's `imports` into
/// one flat `HirModule` before the frontend type-checks it, the resulting
/// table is program-wide: `import math as m` in `a.py` makes a local `m`
/// around a `math.sqrt` call in `b.py` a (fail-closed) false reject until
/// #901's per-module namespaces. Recorded in the #962 ADR.
pub(crate) fn bind_std_module_aliases(imports: &[ImportBinding]) -> Vec<(String, StdModule)> {
    imports
        .iter()
        .filter_map(|binding| match binding {
            ImportBinding::Module { local_name, module }
                if local_name != pycc_std::module_name(*module) =>
            {
                Some((local_name.clone(), *module))
            }
            _ => None,
        })
        .collect()
}

/// D-136 (post-review finding), extended by #962: `pycc_hir::lower_expr`
/// resolves `math.sqrt`/`math.pi` (and their aliased spellings) with no
/// visibility into whether the receiver name is actually shadowed by a
/// real local binding at the same call site (e.g. `def f(math: float) ->
/// float: return math.sqrt(math)` -- a legal Python parameter named
/// `math`; or `import math as m` then `def f(m: float) -> float: return
/// m.sqrt(m)`). Real CPython would raise `AttributeError` there (`float`
/// has no `.sqrt` attribute), not silently call libm's `sqrt`.
/// `pycc_types` is the first stage with real binding-scope information
/// (`env`/`local_names`), so this check happens here rather than in
/// `pycc_hir` -- mirroring `float`'s own existing
/// user-definition-takes-priority guard (`env.lookup_function(callee).is_none()`
/// in `infer_expr_in`, `!signatures.contains_key(callee)` in
/// `collect_expr_constraints`), which solves the same class of problem
/// (a hand-recognized name colliding with a real user binding) for a
/// different hand-recognized name.
///
/// Returns the first spelling of `module` -- its canonical name, then
/// every alias `aliases` binds to it -- for which `is_bound` holds, i.e.
/// the name a shadowing binding took. The HIR string carries only the
/// canonical prefix, so which spelling the user wrote at the use site is
/// unknowable here; checking every spelling bound to the module is what
/// keeps the check fail-closed. The residual it leaves is a false reject
/// (a local named `math` while the module only ever wrote `m.sqrt`),
/// closed by #768's binding-aware resolution. Each site supplies its own
/// `is_bound`, which must consult value bindings *and* `def` rebindings
/// (`import math` / `def math(): ...` / `math.sqrt(4.0)` is an
/// `AttributeError` in CPython too).
pub(crate) fn shadowed_std_receiver(
    module: StdModule,
    aliases: &[(String, StdModule)],
    is_bound: impl Fn(&str) -> bool,
) -> Option<&str> {
    let canonical = pycc_std::module_name(module);
    if is_bound(canonical) {
        return Some(canonical);
    }
    aliases
        .iter()
        .filter(|(_, aliased)| *aliased == module)
        .map(|(name, _)| name.as_str())
        .find(|name| is_bound(name))
}

/// The `C0001` a shadowed receiver produces, spelled with the shadowing
/// name [`shadowed_std_receiver`] found and the module it hides.
pub(crate) fn std_receiver_shadowed(shadowed: &str, module: StdModule) -> Diagnostic {
    Diagnostic::error(
        "C0001",
        format!(
            "`{}` is a local name here, not the stdlib `{}` module -- attribute access on a \
             non-module value is not supported yet",
            shadowed,
            pycc_std::module_name(module)
        ),
        Span::new(0, 0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_std_module_aliases_lists_only_aliased_module_bindings() {
        let sqrt = pycc_std::resolve_symbol(StdModule::Math, "sqrt").unwrap();
        let imports = vec![
            ImportBinding::Module {
                local_name: "math".to_string(),
                module: StdModule::Math,
            },
            ImportBinding::Module {
                local_name: "m".to_string(),
                module: StdModule::Math,
            },
            ImportBinding::Symbol {
                local_name: "sqrt".to_string(),
                module: StdModule::Math,
                symbol: sqrt,
            },
            ImportBinding::Module {
                local_name: "e".to_string(),
                module: StdModule::Enum,
            },
        ];
        assert_eq!(
            bind_std_module_aliases(&imports),
            vec![
                ("m".to_string(), StdModule::Math),
                ("e".to_string(), StdModule::Enum)
            ]
        );
    }

    #[test]
    fn shadowed_std_receiver_checks_the_canonical_name_then_each_alias_of_the_module() {
        let aliases = vec![
            ("e".to_string(), StdModule::Enum),
            ("m".to_string(), StdModule::Math),
            ("mm".to_string(), StdModule::Math),
        ];
        let bound = |bound: &'static [&'static str]| move |name: &str| bound.contains(&name);
        assert_eq!(
            shadowed_std_receiver(StdModule::Math, &aliases, bound(&["math", "m"])),
            Some("math")
        );
        assert_eq!(
            shadowed_std_receiver(StdModule::Math, &aliases, bound(&["mm"])),
            Some("mm")
        );
        // An alias of a *different* module never shadows this one.
        assert_eq!(
            shadowed_std_receiver(StdModule::Math, &aliases, bound(&["e"])),
            None
        );
        assert_eq!(
            shadowed_std_receiver(StdModule::Math, &[], bound(&["m"])),
            None
        );
    }

    #[test]
    fn std_receiver_shadowed_names_the_shadowing_spelling_and_the_module() {
        let diagnostic = std_receiver_shadowed("m", StdModule::Math);
        assert_eq!(diagnostic.code, "C0001");
        assert_eq!(
            diagnostic.message,
            "`m` is a local name here, not the stdlib `math` module -- attribute access on a \
             non-module value is not supported yet"
        );
    }
}
