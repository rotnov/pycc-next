//! Resolution of the receiver of a `<receiver>.<attr>` expression to a
//! `pycc_std` module (Part 1 of #883, #962).
//!
//! `lower_expr` has two sites that treat an attribute receiver as a stdlib
//! module -- the call-shaped `math.sqrt(x)` arm and the bare-attribute
//! `math.pi` arm. Both used to resolve the receiver *textually* through
//! `pycc_std::resolve_module` alone, which cannot see an alias (`import
//! math as m`). [`std_receiver`] consults the module's import table first,
//! so the alias the user bound wins over the textual spelling, and always
//! hands back the module's canonical name so the lowered HIR keeps emitting
//! the `"math.sqrt"`/`"math.pi"` strings `pycc_types`, `pycc_mir`, and
//! `pycc_codegen` key on.

use crate::ImportBinding;
use pycc_std::StdModule;

/// Resolves `receiver` to a stdlib module and its canonical spelling.
///
/// Lookup order:
///
/// 1. An [`ImportBinding::Module`] in `imports` whose `local_name` is
///    `receiver`. The *last* such binding wins, matching Python's own
///    rebinding semantics for `import enum as m` followed by `import math
///    as m`. Because `imports` is the module's table as it stands at the
///    point the enclosing item is lowered, an alias is visible only to
///    items after its `import` statement (source order, like a class or a
///    type alias) -- a recorded divergence from CPython's late binding for
///    a function body that names an alias bound below it.
/// 2. The textual fallback `pycc_std::resolve_module(receiver)`: the D-136
///    scope trim that lets `math.sqrt(x)` lower without an `import math`
///    in scope. Import-gating that spelling is #768's scope, not this
///    module's.
///
/// The alias is consulted before the textual spelling, so `import enum as
/// math` makes `math.Enum` resolve to `enum.Enum` -- the binding the user
/// wrote, not the name's stdlib homonym.
pub(crate) fn std_receiver(receiver: &str, imports: &[ImportBinding]) -> Option<StdModule> {
    imports
        .iter()
        .rev()
        .find_map(|binding| match binding {
            ImportBinding::Module { local_name, module } if local_name == receiver => Some(*module),
            _ => None,
        })
        .or_else(|| pycc_std::resolve_module(receiver))
}

/// How a diagnostic names a resolved module: the canonical spelling when
/// that is what the user wrote (byte-for-byte the pre-#962 text, so no
/// existing fixture changes), and the canonical spelling plus the alias
/// the user actually wrote otherwise -- `` `math` (imported as `m`) ``.
pub(super) fn describe_module(module: StdModule, receiver: &str) -> String {
    let canonical = pycc_std::module_name(module);
    if receiver == canonical {
        format!("`{canonical}`")
    } else {
        format!("`{canonical}` (imported as `{receiver}`)")
    }
}
