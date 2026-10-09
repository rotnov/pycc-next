//! The class phase of the empty-container pass (#1265, Part 4 of #1218;
//! D-245's 2026-09-24 amendment for #1265).
//!
//! `pycc_hir` lowers an unannotated `self.xs = []` at the top level of
//! `__init__` to a *provisional* slot, `list[<Ty::Infer>]`, because it has no
//! inference of its own. This phase runs ahead of the per-function phase and
//! does three things:
//!
//! 1. **Slot resolution** ([`resolve_class_slots`]). Classes are visited in
//!    `HirModule::class_defs` order, which puts every base before the classes
//!    deriving from it (a base name must already be bound when a `class`
//!    statement is lowered, and `pycc_hir::link` concatenates modules in
//!    dependency order). A provisional slot takes, in priority order:
//!    - **(a)** the first concrete slot type *of the same shape* that one of
//!      the class's MRO ancestors declares for the same attribute name;
//!    - **(b)** the element type of the first *syntactic*
//!      `self.<attr>.append(v)` in statement position across the class's own
//!      methods, in source order. "Own methods" is the class table
//!      (`HirClassDef::methods` plus every property getter and setter), never
//!      `static_methods` or `class_methods`, whose first parameter is not the
//!      instance. A producer whose value does not infer ends the *whole
//!      class's* scan with a miss (D-245 amendment 7's three-outcome rule
//!      carried across method boundaries), and a resolution passes the same
//!      [`inferred`] gate a local producer does.
//! 2. **Value rewrite.** Every `self.<attr> = []`/`{}` in a class's own
//!    methods -- the establishing store in `__init__` and any later reset --
//!    becomes `EmptyList`/`EmptyDict` typed from the class's flat slot layout,
//!    when that slot is a concrete container of the same shape. When the
//!    slot is the CPython object instead (#1421), every list display in a
//!    value position of the stored value becomes `ObjectList`, empty or not
//!    (`rewrite_reset`'s object branch).
//! 3. **Receiver identity (#1181).** A method whose source spells its
//!    receiver `this` opens with the alias statement `this = self`, so "the
//!    receiver" is `self` or that alias ([`receiver_spellings`]).
//!
//! What this phase cannot resolve stays provisional, and
//! [`reject_unresolved_attr_slots`] -- called by both check entry points right
//! after the pass, before D-210's redeclaration check and any other check --
//! refuses it with `T0003`. Without that gate the placeholder surfaced as a
//! `T0052` naming `list[<inferred>]`, a type the source never wrote.

use super::object_slot::{has_value_display, rewrite_value_displays};
use super::producer::{ProducerScan, ProducerTarget, scan_body};
use super::{
    Resolution, any_stmt, concrete, contains_infer, empty_literal, for_each_stmt_mut,
    from_container_ty, function_environment, inferred, module_environment,
};
use crate::module::{DiagnosticKey, KeyedDiagnostics};
use pycc_diag::{Diagnostic, Span};
use pycc_hir::{HirClassDef, HirExpr, HirItem, HirModule, HirStmt, Ty};

/// Whether the class phase has anything to do: some class carries a
/// provisional slot, or some function body stores an empty literal into an
/// attribute (a reset the rewrite may type from an existing slot), or --
/// when some class has an object slot (#1421) -- stores a value holding a
/// list display into one (see [`rewrite_reset`]'s object branch). The last
/// trigger is gated on the object slot so a native `self.xs = [1]` keeps
/// the pass's no-clone fast path.
pub(crate) fn needs_class_phase(hir: &HirModule) -> bool {
    let has_object_slot = hir
        .class_defs
        .iter()
        .any(|(_, class)| class.attrs.iter().any(|(_, ty)| *ty == Ty::Object));
    hir.class_defs
        .iter()
        .any(|(_, class)| class.attrs.iter().any(|(_, ty)| contains_infer(ty)))
        || hir.items.iter().any(|item| {
            matches!(item, HirItem::Function { body, .. } if any_stmt(body, &|stmt| matches!(
                stmt,
                HirStmt::AttrSet { value, .. } if empty_literal(value).is_some()
                    || (has_object_slot && has_value_display(value))
            )))
        })
}

/// Resolves every provisional slot of `resolved`'s classes it can, then
/// rewrites the attribute stores in the classes' own methods: an empty
/// literal is typed from its slot, and a store into an object slot has its
/// value-position list displays rewritten into [`HirExpr::ObjectList`]
/// (#1421).
/// `hir` is the unrewritten input (the method bodies this phase scans are
/// the same in both), and `local_names` is its per-item local-name table.
pub(crate) fn resolve_class_slots(
    hir: &HirModule,
    resolved: &mut HirModule,
    local_names: &[Vec<&str>],
) {
    let mut module_env = module_environment(hir);
    for index in 0..resolved.class_defs.len() {
        let class = &resolved.class_defs[index].1;
        if !class.attrs.iter().any(|(_, ty)| contains_infer(ty)) {
            continue;
        }
        // One environment per class, built from the classes resolved so far:
        // an inherited read (`self.base_xs[0]`) sees its resolved type, while
        // a read of another provisional slot of this same class stays a miss.
        let methods: Vec<(&[HirStmt], crate::Environment)> = own_methods(hir, class)
            .map(|(item, name, params, body)| {
                let env = function_environment(&module_env, name, params, body, &local_names[item]);
                (body, env)
            })
            .collect();
        let mut attrs = class.attrs.clone();
        for (attr, ty) in attrs.iter_mut() {
            if !contains_infer(ty) {
                continue;
            }
            let found = inherited_slot(&resolved.class_defs, class, attr, ty)
                .or_else(|| produced_slot(&methods, local_names, hir, class, attr));
            if let Some(found) = found {
                *ty = found;
            }
        }
        let (name, class) = &mut resolved.class_defs[index];
        // Patch the slot table in place rather than re-binding the class:
        // `Environment::bind_class` would also clear a D-188 synthetic mark.
        if let Some(def) = std::sync::Arc::make_mut(&mut module_env.classes).get_mut(name.as_str())
        {
            def.attrs.clone_from(&attrs);
        }
        class.attrs = attrs;
    }
    rewrite_attr_resets(resolved);
}

/// Source (a): the first ancestor slot for `attr` with `slot`'s shape and
/// no placeholder inside it. D-210's `T0052` requires one type per attribute
/// across an MRO, so this is the only type the program could compile with.
fn inherited_slot(
    class_defs: &[(String, HirClassDef)],
    class: &HirClassDef,
    attr: &str,
    slot: &Ty,
) -> Option<Ty> {
    class.mro.iter().skip(1).find_map(|ancestor| {
        let (_, def) = class_defs.iter().find(|(name, _)| name == ancestor)?;
        let (_, ty) = def.attrs.iter().find(|(name, _)| name == attr)?;
        (std::mem::discriminant(ty) == std::mem::discriminant(slot) && !contains_infer(ty))
            .then(|| ty.clone())
    })
}

/// Source (b): the first syntactic `self.<attr>.append(v)` across the
/// class's own methods, in source order.
fn produced_slot(
    methods: &[(&[HirStmt], crate::Environment)],
    local_names: &[Vec<&str>],
    hir: &HirModule,
    class: &HirClassDef,
    attr: &str,
) -> Option<Ty> {
    let items = own_methods(hir, class).map(|(item, ..)| item);
    for ((body, env), item) in methods.iter().zip(items) {
        let receivers = receiver_spellings(body);
        let target = ProducerTarget::SelfAttr {
            receivers: &receivers,
            attr,
        };
        match scan_body(body, target, env, &local_names[item]) {
            ProducerScan::NotFound => {}
            ProducerScan::Matched => return None,
            ProducerScan::Resolved(resolution) => return inferred(resolution).map(container_ty),
        }
    }
    None
}

/// The slot type a resolution stands for -- the inverse of
/// `from_container_ty`. An attribute scan only ever resolves a list today
/// (no attribute dict producer exists until #891), but the mapping is total.
fn container_ty(resolution: Resolution) -> Ty {
    match resolution {
        Resolution::List(element) => Ty::List(Box::new(element)),
        Resolution::Dict(key, value) => Ty::Dict(Box::new((key, value))),
    }
}

/// `class`'s own instance methods in `hir.items` order (which is source
/// order), as `(item index, mangled name, params, body)`. Iterating the items
/// rather than `class.methods` is what keeps the order syntactic: a
/// synthesized `__init__`/`__eq__` entry is appended to that table out of
/// source order.
fn own_methods<'a>(
    hir: &'a HirModule,
    class: &'a HirClassDef,
) -> impl Iterator<Item = (usize, &'a str, &'a [(String, Ty)], &'a [HirStmt])> {
    hir.items
        .iter()
        .enumerate()
        .filter_map(move |(index, item)| match item {
            HirItem::Function {
                name, params, body, ..
            } if is_own_method(class, name) => {
                Some((index, name.as_str(), params.as_slice(), body.as_slice()))
            }
            _ => None,
        })
}

/// Whether `mangled` is one of `class`'s instance methods: a `methods`
/// entry or a property getter/setter. Static and class methods are separate
/// tables and are deliberately excluded.
fn is_own_method(class: &HirClassDef, mangled: &str) -> bool {
    class.methods.iter().any(|(_, name)| name == mangled)
        || class.properties.iter().any(|property| {
            property.getter == mangled || property.setter.as_deref() == Some(mangled)
        })
}

/// Every spelling of the receiver in a method body: the canonical `self`,
/// plus the source spelling of a renamed receiver, which
/// `pycc_hir::class::receiver` binds by prepending exactly
/// `<spelling> = self` as the body's first statement (#1181).
pub(super) fn receiver_spellings(body: &[HirStmt]) -> Vec<&str> {
    let mut spellings = vec!["self"];
    if let Some(HirStmt::Assign {
        target,
        value: HirExpr::Name(value),
    }) = body.first()
        && value == "self"
    {
        spellings.push(target);
    }
    spellings
}

/// Step 2: types every `self.<attr> = []`/`{}` in each class's own methods
/// from the class's flat slot layout, and builds every list display stored
/// into an object slot as a CPython `list` (#1421).
fn rewrite_attr_resets(resolved: &mut HirModule) {
    for (_, class) in &resolved.class_defs {
        let mro: Vec<&HirClassDef> = class
            .mro
            .iter()
            .filter_map(|name| {
                resolved
                    .class_defs
                    .iter()
                    .find(|(candidate, _)| candidate == name)
                    .map(|(_, def)| def)
            })
            .collect();
        let layout = pycc_hir::flat_attr_layout(&mro);
        for item in resolved.items.iter_mut() {
            let HirItem::Function { name, body, .. } = item else {
                continue;
            };
            if !is_own_method(class, name) {
                continue;
            }
            let receivers: Vec<String> = receiver_spellings(body)
                .into_iter()
                .map(String::from)
                .collect();
            for_each_stmt_mut(body, &mut |stmt| {
                if let HirStmt::AttrSet {
                    base: HirExpr::Name(base),
                    attr,
                    value,
                } = stmt
                    && receivers.contains(base)
                {
                    rewrite_reset(value, &layout, attr);
                }
            });
        }
    }
}

/// Rewrites one store into `attr` from its slot in `layout`.
///
/// **Object slot (#1421).** When the slot is [`Ty::Object`] -- a class-body
/// declaration D-258 lowers to the object (`List[StateT]`, a bare `list`,
/// `Any`), or an attribute established from an object -- every list display
/// in a value position of the stored value, empty or not, becomes a fresh
/// CPython `list` ([`rewrite_value_displays`]), exactly as an
/// object-annotated local's does: the slot is declared, so it says what the
/// display is. This is the #1207 subject's lines 43-44,
/// `self.state_stack = state_stack or [self.parse_conf.start_state]` and
/// `self.value_stack = value_stack or []`. A dict display there stays
/// `T0003`, as it does for a local.
///
/// **Container slot.** Otherwise an empty literal is typed when the slot is
/// a concrete container of the literal's shape. A shape mismatch
/// (`self.xs = {}` into a list slot) is left alone and reports `T0003`,
/// exactly as `Resolution::matches` leaves a local one.
fn rewrite_reset(value: &mut HirExpr, layout: &[(String, Ty)], attr: &str) {
    let slot = layout
        .iter()
        .find(|(name, _)| name == attr)
        .map(|(_, ty)| ty);
    if slot == Some(&Ty::Object) {
        rewrite_value_displays(value);
        return;
    }
    let Some(literal) = empty_literal(value) else {
        return;
    };
    let resolution = slot.and_then(from_container_ty).and_then(concrete);
    if let Some(resolution) = resolution.filter(|resolution| resolution.matches(literal)) {
        *value = resolution.into_expr();
    }
}

/// The gate: no provisional slot may survive the pass. Reports the first one
/// left as `T0003`, keyed to the defining class's `__init__` so a
/// multi-module program points at the right file. The span is the
/// `Span::new(0, 0)` every container `T0003` uses (`HirStmt::AttrSet`
/// carries none), so the message names the attribute and the class.
pub(crate) fn reject_unresolved_attr_slots(hir: &HirModule) -> Result<(), KeyedDiagnostics> {
    let Some((class_name, class, attr)) = hir.class_defs.iter().find_map(|(name, class)| {
        class
            .attrs
            .iter()
            .find(|(_, ty)| contains_infer(ty))
            .map(|(attr, _)| (name, class, attr))
    }) else {
        return Ok(());
    };
    let init = class
        .methods
        .iter()
        .find(|(method, _)| method == "__init__")
        .map(|(_, mangled)| mangled.as_str());
    let init_item = hir
        .items
        .iter()
        .enumerate()
        .find_map(|(index, item)| match item {
            HirItem::Function { name, body, .. } if Some(name.as_str()) == init => {
                Some((index, body))
            }
            _ => None,
        });
    let key = init_item.map_or(DiagnosticKey::Module, |(index, _)| {
        DiagnosticKey::Function(index)
    });
    // Name the receiver exactly as the establishing `<receiver>.<attr> = []`
    // spells it: a renamed receiver (#1181) cannot be annotated through
    // `self`, and a local alias (`me = self`) is not the receiver at all.
    let receiver = init_item
        .and_then(|(_, body)| establishing_receiver(body, attr))
        .unwrap_or("self");
    let message = format!(
        "an empty list literal has no inferable element type for `{receiver}.{attr}` in class \
         `{class_name}`"
    );
    let diagnostic = Diagnostic::error("T0003", message, Span::new(0, 0)).with_help(format!(
        "annotate the attribute (`{receiver}.{attr}: list[int] = []`) or append a value to it \
         in one of `{class_name}`'s own methods (`{receiver}.{attr}.append(...)`)"
    ));
    Err(vec![(key, diagnostic)])
}

/// The receiver spelling of the `<receiver>.<attr> = ...` that established
/// the slot in an `__init__` body. Only a spelling in the receiver set is
/// ever returned, so a same-named store on a foreign base (`a.xs = 1`) or a
/// local alias the source wrote (`me = self`) is never named.
///
/// `self` wins whenever a top-level store through `self` exists. Otherwise
/// the #1181 renamed-receiver alias (`receiver_spellings`' prepended
/// `<spelling> = self`) is returned if a store goes through it. The two cannot
/// both apply: #1181's `check_renamed_receiver` refuses a method with a
/// renamed receiver that mentions `self` at all.
fn establishing_receiver<'a>(body: &'a [HirStmt], attr: &str) -> Option<&'a str> {
    let stores_through = |receiver: &str| {
        body.iter().any(|stmt| {
            matches!(stmt, HirStmt::AttrSet {
                base: HirExpr::Name(base),
                attr: stored,
                ..
            } if stored == attr && base == receiver)
        })
    };
    receiver_spellings(body)
        .into_iter()
        .find(|receiver| stores_through(receiver))
}

#[cfg(test)]
#[path = "attr_slot_tests.rs"]
mod tests;
