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
    /// Non-strict mode: `(slot, value)` flows whose slot is typed but whose
    /// value is not yet: decided once the value is ([`Store::flow`]).
    pub(super) pending_flows: Vec<(usize, usize)>,
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

    /// A value `value` stored into the container slot `slot`: [`Self::unify`],
    /// except that in non-strict mode a value of another concrete type
    /// conflicts only the slot (the container becomes CPython's own, which
    /// takes the value boxed), not the value's own class.
    pub(super) fn flow(&mut self, slot: usize, value: usize) {
        if crate::non_strict::is_non_strict() {
            // A slot already in conflict is CPython's own container: it
            // takes the value boxed and tells nothing about its type.
            let slot_root = self.find(slot);
            if self.conflicted[slot_root] {
                return;
            }
            // A value already in conflict is a CPython object: the slot
            // holds it boxed, and becomes one too.
            let value_root = self.find(value);
            if self.conflicted[value_root] {
                self.conflict(slot);
                return;
            }
            let held = self.resolve(slot, 0);
            let stored = self.resolve(value, 0);
            let mismatch = match (&held, &stored) {
                // A slot of `None`s (`[None] * n`) takes no value of
                // another type natively, whatever the value turns out to
                // be.
                (Some(Ty::None), stored) => *stored != Some(Ty::None),
                (Some(held), Some(stored)) => {
                    held != stored && !matches!((held, stored), (Ty::Int, Ty::Bool))
                }
                _ => false,
            };
            if mismatch {
                self.conflict(slot);
                return;
            }
            // Both already known and compatible: nothing to learn, and
            // keeping the classes apart keeps a later conflict of the
            // slot (`xs[i] = None`) from reaching the value's own uses.
            if held.is_some() && stored.is_some() {
                return;
            }
            // A typed slot meeting a value not typed yet (the result of a
            // read still deferred): decided when the value is, so a value
            // that turns out to be an object does not join the slot.
            if held.is_some() {
                self.pending_flows.push((slot, value));
                return;
            }
        }
        self.unify(slot, value);
    }

    /// Re-decides every pending flow; whether one was decided.
    pub(super) fn retry_flows(&mut self) -> bool {
        let pending = std::mem::take(&mut self.pending_flows);
        let before = pending.len();
        for (slot, value) in pending {
            self.flow(slot, value);
        }
        self.pending_flows.len() < before
    }

    /// A value `result` read out of the container slot `slot`:
    /// [`Self::unify`], except that in non-strict mode a slot of `None`s,
    /// of CPython objects or already in conflict makes the value a CPython
    /// object without joining the two classes.
    pub(super) fn read(&mut self, slot: usize, result: usize) {
        if crate::non_strict::is_non_strict() {
            let slot_root = self.find(slot);
            if self.conflicted[slot_root]
                || matches!(
                    self.shape[slot_root],
                    Some(Shape::Known(Ty::None | Ty::Object))
                )
            {
                // A value already typed by its other uses keeps its class
                // (the check phase decides that read); an untyped one is
                // the object the slot holds.
                let result_root = self.find(result);
                if self.shape[result_root].is_none() {
                    self.conflict(result);
                }
                return;
            }
        }
        self.unify(slot, result);
    }

    /// Whether `var`'s class is in conflict.
    pub(super) fn is_conflicted(&mut self, var: usize) -> bool {
        let root = self.find(var);
        self.conflicted[root]
    }

    /// A key `key` looked up in a dict whose key slot is `slot`:
    /// [`Self::unify`], except that in non-strict mode a lookup only
    /// types a key slot nothing else typed yet, and a key that is a CPython
    /// object, or already in conflict, makes the dict CPython's own (its
    /// key slot conflicted) without joining the key's class to it.
    pub(super) fn probe(&mut self, slot: usize, key: usize) {
        if crate::non_strict::is_non_strict() {
            let key_root = self.find(key);
            if self.conflicted[key_root]
                || matches!(self.shape[key_root], Some(Shape::Known(Ty::Object)))
            {
                // CPython's own dict: the key decides nothing natively.
                self.conflict(slot);
                return;
            }
            if self.resolve(slot, 0).is_some() {
                return;
            }
        }
        self.unify(slot, key);
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
            // A fixed-arity tuple flows into a `tuple[T, ...]` slot
            // (`Ty::VarTuple`): each element unifies with `T`.
            (Some(Shape::Known(Ty::VarTuple(elem))), Some(Shape::Tuple(xs)))
            | (Some(Shape::Tuple(xs)), Some(Shape::Known(Ty::VarTuple(elem)))) => {
                self.shape[ra] = Some(Shape::Known(Ty::VarTuple(elem.clone())));
                for x in xs {
                    let e = self.known((*elem).clone());
                    self.unify(e, x);
                }
            }
            (Some(Shape::Known(x)), Some(Shape::Known(y))) => {
                if x != y {
                    // `bool` flows into an `int` slot (`xs.append(True)` on a
                    // `list[int]`), and a fixed-arity tuple of `T`s into a
                    // `tuple[T, ...]` one; any other pair is a conflict.
                    let joined = match (&x, &y) {
                        (Ty::Int, Ty::Bool) | (Ty::Bool, Ty::Int) => Some(Ty::Int),
                        // An `int` and a `float` (`out.append(0)` and
                        // `out.append(best)` with a `float` or `int | float`
                        // best) are both held by an `int | float` element.
                        (
                            Ty::Int | Ty::Bool | Ty::Float | Ty::Num,
                            Ty::Int | Ty::Bool | Ty::Float | Ty::Num,
                        ) => Some(Ty::Num),
                        (Ty::VarTuple(e), Ty::Tuple(items)) | (Ty::Tuple(items), Ty::VarTuple(e))
                            if pycc_hir::tuple_fits_var_tuple(items, e) =>
                        {
                            Some(Ty::VarTuple(e.clone()))
                        }
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

