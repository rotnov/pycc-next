//! Comprehension rewrites: tuple targets and generator-expression arguments.
//!
//! * A comprehension whose `for` target is a flat tuple of names
//!   (`[f(i, x) for i, x in pairs]`) iterates a fresh name instead, and
//!   every read of a target name becomes an item of it
//!   (`_pycc_unpack_0[1]`; [`is_unpack_target`]). The unpacking CPython does
//!   is then the tuple read the rest of the compiler supports. A rewrite is skipped when a nested scope or a walrus in the
//!   comprehension rebinds one of the names, or a later `for` clause binds
//!   one again.
//! * A generator expression passed as the only argument of a consumer that
//!   materializes it anyway (`sum`, `min`, `max`, `sorted`, `list`, `set`,
//!   `frozenset`, `tuple`, `dict`, `str.join`) becomes the list
//!   comprehension with the same elements. `any`/`all` stop at the first
//!   deciding element, so theirs stays a generator (`helpers`).

use super::Fresh;
use super::build::{int, load, store, subscript};
use ruff_python_ast::visitor::transformer::{self, Transformer};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{Comprehension, Expr, ExprContext, ExprListComp};
use std::collections::HashMap;

/// Every name a nested scope, a walrus or a target inside `expr` binds.
fn bound_inside(expr: &Expr) -> Vec<String> {
    struct Binders(Vec<String>);
    impl<'a> Visitor<'a> for Binders {
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Lambda(lambda) => {
                    if let Some(params) = lambda.parameters.as_deref() {
                        for param in params.iter() {
                            self.0.push(param.name().to_string());
                        }
                    }
                }
                Expr::Named(named) => {
                    if let Expr::Name(name) = named.target.as_ref() {
                        self.0.push(name.id.to_string());
                    }
                }
                Expr::Name(name) if name.ctx == ExprContext::Store => {
                    self.0.push(name.id.to_string());
                }
                _ => {}
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut binders = Binders(Vec::new());
    binders.visit_expr(expr);
    binders.0
}

/// Replaces each load of a name in `map` with its replacement.
struct Substitute<'a> {
    map: &'a HashMap<String, Expr>,
}

impl Transformer for Substitute<'_> {
    fn visit_expr(&self, expr: &mut Expr) {
        if let Expr::Name(name) = expr
            && name.ctx == ExprContext::Load
            && let Some(replacement) = self.map.get(name.id.as_str())
        {
            *expr = replacement.clone();
            return;
        }
        transformer::walk_expr(self, expr);
    }
}

/// The flat name list of a tuple target, or `None`.
fn tuple_target_names(target: &Expr) -> Option<Vec<String>> {
    let elts = match target {
        Expr::Tuple(tuple) => &tuple.elts,
        Expr::List(list) => &list.elts,
        _ => return None,
    };
    elts.iter()
        .map(|elt| match elt {
            Expr::Name(name) => Some(name.id.to_string()),
            _ => None,
        })
        .collect()
}

/// Rewrites the tuple targets of `generators`; `results` are the element
/// expressions (one, or a dict's key and value).
fn flatten_generators(generators: &mut [Comprehension], results: &mut [&mut Expr], fresh: &Fresh) {
    for index in 0..generators.len() {
        let Some(names) = tuple_target_names(&generators[index].target) else {
            continue;
        };
        // Everything the targets are visible in.
        let mut region_binders: Vec<String> = Vec::new();
        for generator in &generators[index..] {
            for cond in &generator.ifs {
                region_binders.extend(bound_inside(cond));
            }
        }
        for generator in &generators[index + 1..] {
            region_binders.extend(bound_inside(&generator.iter));
            region_binders.extend(bound_inside(&generator.target));
        }
        for result in results.iter() {
            region_binders.extend(bound_inside(result));
        }
        if names.iter().any(|name| region_binders.contains(name)) {
            continue;
        }
        let range = generators[index].range;
        let var = unpack_target_name(fresh, names.len());
        let map: HashMap<String, Expr> = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                (
                    name.clone(),
                    subscript(
                        load(&var, range),
                        int(i as u64, range),
                        ExprContext::Load,
                        range,
                    ),
                )
            })
            .collect();
        let substitute = Substitute { map: &map };
        generators[index].target = store(&var, range);
        for cond in &mut generators[index].ifs {
            substitute.visit_expr(cond);
        }
        for generator in &mut generators[index + 1..] {
            substitute.visit_expr(&mut generator.iter);
            for cond in &mut generator.ifs {
                substitute.visit_expr(cond);
            }
        }
        for result in results.iter_mut() {
            substitute.visit_expr(result);
        }
    }
}

/// The prefix of the name a tuple target is rewritten to. It marks the
/// comprehension as one that unpacks its item: every read of the name is an
/// item of it, so the comprehension never uses the item whole
/// ([`is_unpack_target`]). Reserved, like every `_pycc_` name
/// (`crate::reserved`), so no program can write it.
pub const UNPACK_TARGET_PREFIX: &str = "_pycc_unpack_";

/// A fresh name for a rewritten tuple target of `arity` names:
/// `_pycc_unpack_<arity>_<n>` ([`unpack_target_arity`]).
fn unpack_target_name(fresh: &Fresh, arity: usize) -> String {
    format!("{UNPACK_TARGET_PREFIX}{arity}_{}", fresh.name().trim_start_matches("_pycc_"))
}

/// How many names the tuple target had that `name` was rewritten from, when
/// `name` ends with such a rewritten name: a comprehension variable
/// synthesized from one (`pycc_hir`'s `0comp_..._<source name>`) keeps it
/// as its suffix. `None` for any other name.
#[must_use]
pub fn unpack_target_arity(name: &str) -> Option<usize> {
    let rest = &name[name.rfind(UNPACK_TARGET_PREFIX)? + UNPACK_TARGET_PREFIX.len()..];
    let (arity, n) = rest.split_once('_')?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    arity.parse().ok()
}

/// Whether `name` is the loop variable a tuple target was rewritten to
/// (marked [`FRESH_TARGET_PREFIX`] or not).
#[must_use]
pub fn is_unpack_target(name: &str) -> bool {
    name.strip_prefix(FRESH_TARGET_PREFIX)
        .unwrap_or(name)
        .starts_with(UNPACK_TARGET_PREFIX)
}

/// The prefix the first target of a generator expression passed to
/// `sorted` is renamed with when it is materialized ([`materialize_generator`]):
/// the list the comprehension builds is then the only one `sorted` would
/// have made, which it may sort in place ([`is_fresh_target`]). Reserved,
/// like every `_pycc_` name.
pub const FRESH_TARGET_PREFIX: &str = "_pycc_fresh_";

/// Whether `name` is a target [`FRESH_TARGET_PREFIX`] marks. No program can
/// write one (`crate::reserved`); a comprehension variable `pycc_hir`
/// synthesizes from one says so in its own prefix
/// (`pycc_hir::is_fresh_comprehension_var`).
#[must_use]
pub fn is_fresh_target(name: &str) -> bool {
    name.starts_with(FRESH_TARGET_PREFIX)
}

/// Rewrites `expr`'s own tuple targets when it is a comprehension.
pub(super) fn flatten_tuple_targets(expr: &mut Expr, fresh: &Fresh) {
    match expr {
        Expr::ListComp(comp) => {
            flatten_generators(&mut comp.generators, &mut [comp.elt.as_mut()], fresh)
        }
        Expr::SetComp(comp) => {
            flatten_generators(&mut comp.generators, &mut [comp.elt.as_mut()], fresh)
        }
        Expr::Generator(comp) => {
            flatten_generators(&mut comp.generators, &mut [comp.elt.as_mut()], fresh)
        }
        Expr::DictComp(comp) => {
            let Some(key) = comp.key.as_deref_mut() else {
                return;
            };
            flatten_generators(&mut comp.generators, &mut [key, comp.value.as_mut()], fresh)
        }
        _ => {}
    }
}

/// The list comprehension a generator-expression argument of the consumer
/// `consumer` becomes, or `None` when it stays as it is.
///
/// For `sorted`, the first target is marked [`FRESH_TARGET_PREFIX`] when it
/// is a name nothing in the comprehension rebinds: CPython's `sorted` of a
/// generator sorts the one list it fills from it, so that list may be
/// sorted in place, where `sorted` of a list display must copy it.
pub(super) fn materialize_generator(consumer: &str, arg: &Expr) -> Option<Expr> {
    let Expr::Generator(generator) = arg else {
        return None;
    };
    if generator.generators.iter().any(|g| g.is_async) {
        return None;
    }
    if matches!(consumer, "any" | "all") {
        return None;
    }
    let mut elt = generator.elt.clone();
    let mut generators = generator.generators.clone();
    if consumer == "sorted" {
        mark_fresh(&mut generators, elt.as_mut());
    }
    Some(Expr::ListComp(ExprListComp {
        node_index: Default::default(),
        range: generator.range,
        elt,
        generators,
    }))
}

/// Renames the first target of `generators` (a name) to its
/// [`FRESH_TARGET_PREFIX`] name in every place it is visible, unless
/// something there binds the name again.
fn mark_fresh(generators: &mut [Comprehension], elt: &mut Expr) {
    let Some(Expr::Name(target)) = generators.first().map(|g| &g.target) else {
        return;
    };
    let name = target.id.to_string();
    if name.starts_with(FRESH_TARGET_PREFIX) {
        return;
    }
    let mut region_binders: Vec<String> = bound_inside(elt);
    for (index, generator) in generators.iter().enumerate() {
        for cond in &generator.ifs {
            region_binders.extend(bound_inside(cond));
        }
        if index > 0 {
            region_binders.extend(bound_inside(&generator.iter));
            region_binders.extend(bound_inside(&generator.target));
        }
    }
    if region_binders.contains(&name) {
        return;
    }
    let range = ruff_text_size::Ranged::range(&generators[0].target);
    let renamed = format!("{FRESH_TARGET_PREFIX}{name}");
    let map: HashMap<String, Expr> = [(name, load(&renamed, range))].into_iter().collect();
    let substitute = Substitute { map: &map };
    generators[0].target = store(&renamed, range);
    for (index, generator) in generators.iter_mut().enumerate() {
        if index > 0 {
            substitute.visit_expr(&mut generator.iter);
        }
        for cond in &mut generator.ifs {
            substitute.visit_expr(cond);
        }
    }
    substitute.visit_expr(elt);
}

/// The builtins (and `str.join`) whose generator argument is materialized.
pub(super) fn is_materializing_consumer(name: &str) -> bool {
    matches!(
        name,
        "sum"
            | "min"
            | "max"
            | "sorted"
            | "list"
            | "set"
            | "frozenset"
            | "tuple"
            | "dict"
            | "any"
            | "all"
    )
}
