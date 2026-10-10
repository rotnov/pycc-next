//! Comprehension rewrites: tuple targets and generator-expression arguments.
//!
//! * A comprehension whose `for` target is a flat tuple of names
//!   (`[f(i, x) for i, x in pairs]`) iterates a fresh name instead, and
//!   every read of a target name becomes an item of it (`_pycc_0[1]`). The
//!   unpacking CPython does is then the tuple read the rest of the compiler
//!   supports. A rewrite is skipped when a nested scope or a walrus in the
//!   comprehension rebinds one of the names, or a later `for` clause binds
//!   one again.
//! * A generator expression passed as the only argument of a consumer that
//!   materializes it anyway (`sum`, `min`, `max`, `sorted`, `list`, `set`,
//!   `frozenset`, `tuple`, `dict`, `str.join`) becomes the list
//!   comprehension with the same elements. `any`/`all` stop at the first
//!   deciding element, so for them the rewrite applies only when evaluating
//!   every element can have no observable effect: an element and filters
//!   with no call (`docs/DEVIATIONS.md` names the remaining difference).

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
        let var = fresh.name();
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

/// Whether evaluating `expr` can call anything (so it may have an effect).
fn has_call(expr: &Expr) -> bool {
    struct Calls(bool);
    impl<'a> Visitor<'a> for Calls {
        fn visit_expr(&mut self, expr: &'a Expr) {
            if matches!(
                expr,
                Expr::Call(_) | Expr::Await(_) | Expr::Yield(_) | Expr::YieldFrom(_)
            ) {
                self.0 = true;
            }
            visitor::walk_expr(self, expr);
        }
    }
    let mut calls = Calls(false);
    calls.visit_expr(expr);
    calls.0
}

/// The list comprehension a generator-expression argument of the consumer
/// `consumer` becomes, or `None` when it stays as it is.
pub(super) fn materialize_generator(consumer: &str, arg: &Expr) -> Option<Expr> {
    let Expr::Generator(generator) = arg else {
        return None;
    };
    if generator.generators.iter().any(|g| g.is_async) {
        return None;
    }
    if matches!(consumer, "any" | "all") {
        let effect = has_call(&generator.elt)
            || generator
                .generators
                .iter()
                .enumerate()
                .any(|(i, g)| g.ifs.iter().any(has_call) || (i > 0 && has_call(&g.iter)));
        if effect {
            return None;
        }
    }
    Some(Expr::ListComp(ExprListComp {
        node_index: Default::default(),
        range: generator.range,
        elt: generator.elt.clone(),
        generators: generator.generators.clone(),
    }))
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
