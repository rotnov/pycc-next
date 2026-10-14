//! Expressions whose container is built from another one's elements: a
//! comprehension, and the builtin calls that list an iterable's elements.

use super::{Deferred, Engine, Scope, Shape};
use pycc_hir::{CompElt, CompIter, HirComprehension, HirExpr, Ty};

impl Engine {
    /// A builtin call (not shadowed by a module function) whose result is a
    /// list of its argument's elements: `sorted(xs)`, `list(xs)`, and the
    /// keyed sort `pycc_ast::desugar::keys` spells
    /// `__pycc_sorted_keyed([(key, v) for v in xs], reverse)`.
    pub(super) fn element_preserving_call(
        &mut self,
        callee: &str,
        args: &[Option<usize>],
    ) -> Option<usize> {
        let base = (*args.first()?)?;
        let elem = self.store.fresh();
        let result = match (callee, args.len()) {
            ("sorted" | "list", 1) => elem,
            ("__pycc_sorted_keyed", 2) => {
                let key = self.store.fresh();
                self.store.with_shape(Shape::Tuple(vec![key, elem]))
            }
            _ => return None,
        };
        self.deferred.push(Deferred::Element {
            base,
            result,
            iterate: true,
        });
        Some(self.store.with_shape(Shape::List(elem)))
    }

    /// The flavored containers `pycc_ast::desugar::collections` builds: the
    /// container is its first argument (an empty `[]`/`{}` typed by its
    /// uses), a `Counter`'s values are `int`s, and a `defaultdict`'s values
    /// have its prototype's type.
    pub(super) fn native_container_call(
        &mut self,
        callee: &str,
        args: &[Option<usize>],
    ) -> Option<usize> {
        let container = match callee {
            "__pycc_deque" | "__pycc_defaultdict_none" | "__pycc_defaultdict_call" => {
                (*args.first()?)?
            }
            "__pycc_counter" => {
                let container = (*args.first()?)?;
                let (key, count) = (self.store.fresh(), self.store.known(Ty::Int));
                let shaped = self.store.with_shape(Shape::Dict(key, count));
                self.store.unify(container, shaped);
                container
            }
            "__pycc_counter_of" => {
                let source = (*args.first()?)?;
                let key = self.store.fresh();
                self.deferred.push(Deferred::Element {
                    base: source,
                    result: key,
                    iterate: true,
                });
                let count = self.store.known(Ty::Int);
                return Some(self.store.with_shape(Shape::Dict(key, count)));
            }
            "__pycc_defaultdict" => {
                let container = (*args.first()?)?;
                let key = self.store.fresh();
                let value = match args.get(1) {
                    Some(Some(proto)) => *proto,
                    _ => self.store.fresh(),
                };
                let shaped = self.store.with_shape(Shape::Dict(key, value));
                self.store.unify(container, shaped);
                container
            }
            _ => return None,
        };
        Some(container)
    }

    /// A comprehension's container: its loop variable (a synthesized,
    /// unique name) is an element of its iterable, or an `int` over a
    /// range, and its element (or key and value) is the container's.
    pub(super) fn comprehension(&mut self, scope: &Scope<'_>, comp: &HirComprehension) -> usize {
        let var = self.name_var(scope, &comp.var);
        match &comp.iter {
            CompIter::Range { start, stop, step } => {
                for bound in [start, stop, step] {
                    self.term(scope, bound);
                }
                let int = self.store.known(Ty::Int);
                self.store.unify(var, int);
            }
            CompIter::Iterable(iterable) => {
                if let Some(base) = self.term(scope, iterable) {
                    self.deferred.push(Deferred::Element {
                        base,
                        result: var,
                        iterate: true,
                    });
                }
            }
            CompIter::Name(name) => {
                if let Some(base) = self.term(scope, &HirExpr::Name(name.clone())) {
                    self.deferred.push(Deferred::Element {
                        base,
                        result: var,
                        iterate: true,
                    });
                }
            }
        }
        if let Some(cond) = &comp.cond {
            self.term(scope, cond);
        }
        let shape = match &comp.elt {
            CompElt::List(e) => Shape::List(self.term_or_fresh(scope, e)),
            CompElt::Set(e) => Shape::Set(self.term_or_fresh(scope, e)),
            CompElt::Dict { key, value } => {
                let k = self.term_or_fresh(scope, key);
                let v = self.term_or_fresh(scope, value);
                Shape::Dict(k, v)
            }
        };
        self.store.with_shape(shape)
    }

    /// `target = <comprehension>` in its statement form.
    pub(super) fn comprehension_assign(
        &mut self,
        scope: &Scope<'_>,
        target: &str,
        var: &str,
        iter: &CompIter,
        cond: &Option<Box<HirExpr>>,
        elt: CompElt,
    ) {
        let comp = HirComprehension {
            var: var.to_string(),
            iter: iter.clone(),
            cond: cond.as_deref().cloned(),
            elt,
        };
        let shaped = self.comprehension(scope, &comp);
        let target = self.name_var(scope, target);
        self.store.unify(target, shaped);
    }

    /// `base.method(args)`, typed by the checker, or else re-typed (see
    /// [`Engine::retyped`]) from what its receiver and arguments unified
    /// to.
    pub(super) fn method_call(
        &mut self,
        scope: &Scope<'_>,
        expr: &HirExpr,
        base: &HirExpr,
        method: &str,
        args: &[HirExpr],
    ) -> Option<usize> {
        let base_var = self.term(scope, base);
        let arg_vars: Vec<Option<usize>> = args.iter().map(|arg| self.term(scope, arg)).collect();
        if let Some(known) = self.inferred(scope, expr) {
            return Some(known);
        }
        let mut env = None;
        let base = self.placeholder(scope, &mut env, base, base_var, 0);
        let args = self.placeholders(scope, &mut env, args, &arg_vars);
        let call = HirExpr::MethodCall {
            base: Box::new(base),
            method: method.to_string(),
            args,
        };
        self.retyped(scope, env, &call)
    }

    /// `callee(args)` with each argument the checker cannot type replaced
    /// by what it unified to (see [`Engine::retyped`]).
    pub(super) fn retyped_call(
        &mut self,
        scope: &Scope<'_>,
        callee: &str,
        args: &[HirExpr],
        arg_vars: &[Option<usize>],
    ) -> Option<usize> {
        let mut env = None;
        let args = self.placeholders(scope, &mut env, args, arg_vars);
        let call = HirExpr::Call {
            callee: callee.to_string(),
            args,
        };
        self.retyped(scope, env, &call)
    }

    /// An expression the checker could not type because a sub-expression's
    /// type comes from this pass (a helper's inferred return, as in
    /// `list(map(int, sr().split()))`), typed again with those
    /// sub-expressions replaced by names bound to their unified types.
    /// `env` is `None` when nothing was replaced (the checker already
    /// failed on the same expression).
    fn retyped(
        &mut self,
        scope: &Scope<'_>,
        env: Option<crate::env::Environment>,
        expr: &HirExpr,
    ) -> Option<usize> {
        let env = env?;
        let ty = crate::expr::infer_expr_in(&env, scope.local_names, expr).ok()?;
        super::concrete_ty(&ty).then(|| self.store.known(ty))
    }

    fn placeholders(
        &mut self,
        scope: &Scope<'_>,
        env: &mut Option<crate::env::Environment>,
        exprs: &[HirExpr],
        vars: &[Option<usize>],
    ) -> Vec<HirExpr> {
        exprs
            .iter()
            .zip(vars)
            .enumerate()
            .map(|(index, (expr, var))| self.placeholder(scope, env, expr, *var, index + 1))
            .collect()
    }

    /// `expr` itself when the checker types it, else a fresh name (with a
    /// leading digit, so no source name equals it) bound in `env` to the
    /// concrete type `var` unified to, else `expr`.
    fn placeholder(
        &mut self,
        scope: &Scope<'_>,
        env: &mut Option<crate::env::Environment>,
        expr: &HirExpr,
        var: Option<usize>,
        index: usize,
    ) -> HirExpr {
        if crate::expr::infer_expr_in(scope.env, scope.local_names, expr)
            .is_ok_and(|ty| super::concrete_ty(&ty))
        {
            return expr.clone();
        }
        let Some(ty) = var.and_then(|var| self.store.resolve(var, 0)) else {
            return expr.clone();
        };
        if !super::concrete_ty(&ty) {
            return expr.clone();
        }
        let name = format!("0retyped{index}");
        env.get_or_insert_with(|| scope.env.clone())
            .bind(name.clone(), ty);
        HirExpr::Name(name)
    }

    fn term_or_fresh(&mut self, scope: &Scope<'_>, expr: &HirExpr) -> usize {
        self.term(scope, expr).unwrap_or_else(|| self.store.fresh())
    }
}
