//! Applying the deferred constraints of [`super`] once the shapes they
//! wait on are known.

use super::{Deferred, Engine, Shape};
use pycc_hir::Ty;

impl Engine {
    /// Applies every deferred constraint whose container shape is known,
    /// until nothing changes.
    pub(super) fn settle(&mut self) {
        loop {
            let mut progressed = false;
            let pending = std::mem::take(&mut self.deferred);
            for constraint in pending {
                let base = match &constraint {
                    Deferred::Element { base, .. }
                    | Deferred::Store { base, .. }
                    | Deferred::Key { base, .. } => *base,
                    Deferred::Unpack { base, items } => {
                        let slots = match self.store.open(*base) {
                            Some(Shape::List(e)) => vec![e; items.len()],
                            Some(Shape::Tuple(slots)) if slots.len() == items.len() => slots,
                            Some(Shape::Known(Ty::Str)) => {
                                let s = self.store.known(Ty::Str);
                                vec![s; items.len()]
                            }
                            Some(_) => continue,
                            None => {
                                self.deferred.push(constraint);
                                continue;
                            }
                        };
                        for (item, slot) in items.iter().zip(slots) {
                            self.store.unify(*item, slot);
                        }
                        progressed = true;
                        continue;
                    }
                    Deferred::BinOp {
                        op,
                        left,
                        right,
                        result,
                    } => {
                        let scalar = |ty: &Option<Ty>| {
                            matches!(ty, Some(Ty::Int | Ty::Float | Ty::Bool | Ty::Str))
                        };
                        let (lt, rt) =
                            (self.store.resolve(*left, 0), self.store.resolve(*right, 0));
                        if !(scalar(&lt) && scalar(&rt)) {
                            self.deferred.push(constraint);
                            continue;
                        }
                        if let Ok(ty) =
                            crate::binop::numeric_result_type(*op, lt.unwrap(), rt.unwrap())
                        {
                            let known = self.store.known(ty);
                            self.store.unify(*result, known);
                            progressed = true;
                        }
                        continue;
                    }
                };
                match (self.store.open(base), constraint) {
                    (Some(Shape::List(e)), Deferred::Element { result, .. })
                    | (
                        Some(Shape::Set(e)),
                        Deferred::Element {
                            result,
                            iterate: true,
                            ..
                        },
                    ) => {
                        self.store.unify(e, result);
                        progressed = true;
                    }
                    (
                        Some(Shape::Dict(k, v)),
                        Deferred::Element {
                            result, iterate, ..
                        },
                    ) => {
                        self.store.unify(if iterate { k } else { v }, result);
                        progressed = true;
                    }
                    (Some(Shape::List(e)), Deferred::Store { value, .. }) => {
                        self.store.unify(e, value);
                        progressed = true;
                    }
                    (Some(Shape::Dict(k, _)), Deferred::Key { key, .. }) => {
                        self.store.unify(k, key);
                        progressed = true;
                    }
                    (Some(Shape::Dict(k, v)), Deferred::Store { key, value, .. }) => {
                        self.store.unify(k, key);
                        self.store.unify(v, value);
                        progressed = true;
                    }
                    // A loop over a generator binds its element type.
                    (
                        Some(Shape::Known(Ty::Generator(element))),
                        Deferred::Element {
                            result,
                            iterate: true,
                            ..
                        },
                    ) if !crate::empty_container::contains_infer(&element.0) => {
                        let known = self.store.known(element.0);
                        self.store.unify(result, known);
                        progressed = true;
                    }
                    (Some(Shape::Known(Ty::Str)), Deferred::Element { result, .. }) => {
                        let known = self.store.known(Ty::Str);
                        self.store.unify(result, known);
                        progressed = true;
                    }
                    (Some(_), _) => {}
                    (None, constraint) => self.deferred.push(constraint),
                }
            }
            if !progressed {
                return;
            }
        }
    }
}
