//! The union-find store of [`super`]: type variables, each class with an
//! optional shape, and the conflict flag a mismatch sets.

use pycc_hir::Ty;

/// A type with unification variables in it.
#[derive(Clone, Debug)]
pub(super) enum Shape {
    Known(Ty),
    List(usize),
    Set(usize),
    FrozenSet(usize),
    Dict(usize, usize),
    Tuple(Vec<usize>),
}

#[derive(Default)]
pub(super) struct Store {
    pub(super) parent: Vec<usize>,
    pub(super) shape: Vec<Option<Shape>>,
    pub(super) conflicted: Vec<bool>,
}

impl Store {
    pub(super) fn fresh(&mut self) -> usize {
        self.parent.push(self.parent.len());
        self.shape.push(None);
        self.conflicted.push(false);
        self.parent.len() - 1
    }

    pub(super) fn with_shape(&mut self, shape: Shape) -> usize {
        let var = self.fresh();
        self.shape[var] = Some(shape);
        var
    }

    pub(super) fn known(&mut self, ty: Ty) -> usize {
        self.with_shape(Shape::Known(ty))
    }

    pub(super) fn find(&mut self, var: usize) -> usize {
        let mut root = var;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut walk = var;
        while self.parent[walk] != root {
            let next = self.parent[walk];
            self.parent[walk] = root;
            walk = next;
        }
        root
    }

    /// The shape of `var`'s class, with a `Known` container opened into
    /// variables so it unifies structurally.
    pub(super) fn open(&mut self, var: usize) -> Option<Shape> {
        let root = self.find(var);
        let shape = self.shape[root].clone()?;
        let opened = match shape {
            Shape::Known(Ty::List(elem)) => {
                let e = self.known(*elem);
                Shape::List(e)
            }
            Shape::Known(Ty::Set(elem)) => {
                let e = self.known(*elem);
                Shape::Set(e)
            }
            Shape::Known(Ty::FrozenSet(elem)) => {
                let e = self.known(*elem);
                Shape::FrozenSet(e)
            }
            Shape::Known(Ty::Dict(pair)) => {
                let (k, v) = *pair;
                let k = self.known(k);
                let v = self.known(v);
                Shape::Dict(k, v)
            }
            Shape::Known(Ty::Tuple(items)) => {
                Shape::Tuple(items.into_iter().map(|item| self.known(item)).collect())
            }
            other => other,
        };
        self.shape[root] = Some(opened.clone());
        Some(opened)
    }

    pub(super) fn conflict(&mut self, var: usize) {
        let root = self.find(var);
        self.conflicted[root] = true;
    }

    /// Unifies `a` and `b`; a mismatch marks the class conflicted.
    pub(super) fn unify(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        let (sa, sb) = (self.open(ra), self.open(rb));
        let conflicted = self.conflicted[ra] || self.conflicted[rb];
        self.parent[rb] = ra;
        self.conflicted[ra] = conflicted;
        match (sa, sb) {
            (None, None) => {}
            (Some(shape), None) | (None, Some(shape)) => self.shape[ra] = Some(shape),
            (Some(Shape::List(x)), Some(Shape::List(y)))
            | (Some(Shape::Set(x)), Some(Shape::Set(y)))
            | (Some(Shape::FrozenSet(x)), Some(Shape::FrozenSet(y))) => self.unify(x, y),
            (Some(Shape::Dict(k1, v1)), Some(Shape::Dict(k2, v2))) => {
                self.unify(k1, k2);
                self.unify(v1, v2);
            }
            (Some(Shape::Tuple(xs)), Some(Shape::Tuple(ys))) if xs.len() == ys.len() => {
                self.shape[ra] = Some(Shape::Tuple(xs.clone()));
                for (x, y) in xs.into_iter().zip(ys) {
                    self.unify(x, y);
                }
            }
            (Some(Shape::Known(x)), Some(Shape::Known(y))) => {
                if x != y {
                    // `bool` flows into an `int` slot (`xs.append(True)` on a
                    // `list[int]`); any other pair is a conflict.
                    let joined = match (&x, &y) {
                        (Ty::Int, Ty::Bool) | (Ty::Bool, Ty::Int) => Some(Ty::Int),
                        _ => None,
                    };
                    match joined {
                        Some(ty) => self.shape[ra] = Some(Shape::Known(ty)),
                        None => self.conflicted[ra] = true,
                    }
                }
            }
            _ => self.conflicted[ra] = true,
        }
    }

    /// The concrete type of `var`, when every variable in it is resolved and
    /// none is conflicted.
    pub(super) fn resolve(&mut self, var: usize, depth: usize) -> Option<Ty> {
        if depth > 16 {
            return None;
        }
        let root = self.find(var);
        if self.conflicted[root] {
            return None;
        }
        match self.shape[root].clone()? {
            Shape::Known(ty) => Some(ty),
            Shape::List(e) => Some(Ty::List(Box::new(self.resolve(e, depth + 1)?))),
            Shape::Set(e) => Some(Ty::Set(Box::new(self.resolve(e, depth + 1)?))),
            Shape::FrozenSet(e) => Some(Ty::FrozenSet(Box::new(self.resolve(e, depth + 1)?))),
            Shape::Dict(k, v) => Some(Ty::Dict(Box::new((
                self.resolve(k, depth + 1)?,
                self.resolve(v, depth + 1)?,
            )))),
            Shape::Tuple(items) => Some(Ty::Tuple(Box::new(
                items
                    .into_iter()
                    .map(|item| self.resolve(item, depth + 1))
                    .collect::<Option<Vec<_>>>()?,
            ))),
        }
    }
}

