//! Expression statements: the producer calls that type an empty container
//! (`xs.append(v)`, `s.add(v)`, `xs.extend(it)`, `s.update(it)`,
//! `d.setdefault(k, v)`, `heapq.heappush(h, v)`), and any other expression
//! visited for the sites it holds.

use super::{Engine, Scope, Shape};
use pycc_hir::{HirExpr, Ty};

impl Engine {
    /// An expression statement: a producer call, or an expression visited
    /// for the sites it holds.
    pub(super) fn effect(&mut self, scope: &Scope<'_>, expr: &HirExpr) {
        match expr {
            HirExpr::ListAppend {
                list: pycc_hir::ContainerReceiver::Name(name),
                value,
            } => {
                if let Some(v) = self.term(scope, value) {
                    self.produce(scope, name, |_| Shape::List(v));
                }
            }
            HirExpr::SetAdd { set, value } => {
                if let Some(v) = self.term(scope, value) {
                    self.produce(scope, set, |_| Shape::Set(v));
                }
            }
            HirExpr::MethodCall { base, method, args } => {
                let base_name = match base.as_ref() {
                    HirExpr::Name(name) => Some(name.clone()),
                    _ => None,
                };
                let arg_vars: Vec<Option<usize>> =
                    args.iter().map(|arg| self.term(scope, arg)).collect();
                let base_var = self.term(scope, base);
                let Some(name) = base_name else {
                    // `graph[u].append(v)`: the item is a list of `v`.
                    if let (HirExpr::Subscript { .. }, Some(item)) = (base.as_ref(), base_var) {
                        let shape = match (method.as_str(), arg_vars.as_slice()) {
                            ("append", [Some(v)]) | ("insert", [_, Some(v)]) => Shape::List(*v),
                            ("add", [Some(v)]) => Shape::Set(*v),
                            _ => return,
                        };
                        let produced = self.store.with_shape(shape);
                        self.store.unify(item, produced);
                    }
                    return;
                };
                match (method.as_str(), arg_vars.as_slice()) {
                    ("append", [Some(v)]) => self.produce(scope, &name, |_| Shape::List(*v)),
                    ("add", [Some(v)]) => self.produce(scope, &name, |_| Shape::Set(*v)),
                    ("insert", [_, Some(v)]) => self.produce(scope, &name, |_| Shape::List(*v)),
                    ("extend" | "update", [Some(other)]) => {
                        let var = self.name_var(scope, &name);
                        // `xs.extend(it)` / `s.update(it)` adds the elements
                        // of any iterable; `d.update(other)` merges a dict.
                        let element = match self.store.open(*other) {
                            Some(Shape::Dict(..)) => {
                                self.store.unify(var, *other);
                                return;
                            }
                            Some(Shape::List(e) | Shape::Set(e) | Shape::FrozenSet(e)) => e,
                            Some(Shape::Known(Ty::Generator(sig))) => self.store.known(sig.0.clone()),
                            _ => return,
                        };
                        let shape = if method == "update" {
                            Shape::Set(element)
                        } else {
                            Shape::List(element)
                        };
                        let produced = self.store.with_shape(shape);
                        self.store.unify(var, produced);
                    }
                    ("setdefault", [Some(k), Some(v)]) => {
                        self.produce(scope, &name, |_| Shape::Dict(*k, *v))
                    }
                    _ => {}
                }
            }
            HirExpr::ReceiverDispatchedCall { call, .. } => self.effect(scope, call),
            // `heapq.heappush(h, v)` / `bisect.insort(a, v)` add `v` to the
            // list (`pycc_ast::desugar::stdlib`'s spelling).
            HirExpr::Call { callee, args }
                if matches!(
                    callee.as_str(),
                    "__pycc_heappush" | "__pycc_insort_left" | "__pycc_insort_right"
                ) && args.len() == 2
                    && matches!(&args[0], HirExpr::Name(_)) =>
            {
                let HirExpr::Name(name) = &args[0] else {
                    unreachable!()
                };
                self.term(scope, &args[0]);
                if let Some(v) = self.term(scope, &args[1]) {
                    self.produce(scope, name, |_| Shape::List(v));
                }
            }
            other => {
                self.term(scope, other);
            }
        }
    }
}
