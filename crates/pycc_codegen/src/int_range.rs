//! Value ranges of a function's `int` locals, proven at compile time, so
//! that arithmetic, comparisons and stores over values that are certainly
//! smallints need neither tag tests nor overflow checks nor bigint
//! releases.
//!
//! An `int` is a tagged word (`int_fast`): every operation tests its
//! operands' tags, checks the result for overflow and joins a runtime path
//! that may return a bigint, and every store into an `int` slot releases
//! the slot's old value when it is a bigint. None of that can be proven
//! away by LLVM once a loop-carried local merges values of unknown range.
//! Here a flow-insensitive interval analysis finds, for each `int` local of
//! a function, a range `[lo, hi]` that holds *every* value the local is
//! ever bound to, when there is one within the smallint range:
//!
//! - the only sources of a range are values that are certainly smallint
//!   words: an `int` literal, an element read of a byte buffer
//!   (`bytearray`, `[0, 255]`), a byte buffer's length, a `range()` loop
//!   variable whose bounds have ranges, and a local that has one;
//! - `+ - *`, `&` with a non-negative operand, `|`/`^` of non-negative
//!   operands, `>>`/`<<` by a counted amount, `//` and `%` by a positive
//!   divisor, `abs()` (the builtin) and a conditional expression combine
//!   them by interval arithmetic;
//! - a local's range is the union over all its bindings, iterated to a
//!   fixed point; one that keeps growing, or that has any binding other
//!   than a plain assignment or a `range()` loop (a parameter, a `for` over
//!   a container, a walrus, a comprehension target, a multi-value result,
//!   a `bool` converted to `int`, which keeps its `True`/`False` word), has
//!   none.
//!
//! The answer is conservative by construction: an expression shape not
//! listed has no range, which costs speed, never correctness. A local's
//! slot holds `0` (no word at all) before its first binding, which a
//! release would skip too, so a local with a range never needs one.

use pycc_hir::ndarray::{NdDtype, NdOp};
use pycc_mir::{BinOpKind, MirExpr, MirStmt, Ty};
use std::collections::{HashMap, HashSet};

/// The smallint range (`pycc_rt::fits_smallint`): `2v + 1` fits an `i64`.
const SMALL_MIN: i128 = -(1 << 62);
const SMALL_MAX: i128 = (1 << 62) - 1;

/// An upper bound on the length of a byte buffer: its bytes are in memory,
/// and no address space reaches 2**57 bytes.
const MAX_BYTES_LEN: i128 = 1 << 57;

/// Rounds of the fixed point before a still-growing range is given up.
const MAX_ROUNDS: usize = 8;

/// An inclusive range of `int` values, within the smallint range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Range {
    pub(crate) lo: i64,
    pub(crate) hi: i64,
}

impl Range {
    fn new(lo: i128, hi: i128) -> Option<Range> {
        (lo <= hi && lo >= SMALL_MIN && hi <= SMALL_MAX).then_some(Range {
            lo: lo as i64,
            hi: hi as i64,
        })
    }

    fn join(self, other: Range) -> Range {
        Range {
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }

    fn bounds(self) -> (i128, i128) {
        (i128::from(self.lo), i128::from(self.hi))
    }
}

/// The ranges proven for the function being emitted (`RtFns::int_ranges`).
#[derive(Debug, Default)]
pub(crate) struct IntRanges {
    pub(crate) locals: HashMap<String, Range>,
    /// Whether a call of `abs` is the builtin (no user function of that
    /// name exists).
    pub(crate) builtin_abs: bool,
}

impl IntRanges {
    /// The range of `expr`'s value, when every value it can have is a
    /// smallint in that range.
    pub(crate) fn of(&self, expr: &MirExpr) -> Option<Range> {
        expr_range(
            expr,
            &|name| Lookup::from(self.locals.get(name).copied()),
            self.builtin_abs,
        )
        .known()
    }

    /// Whether `name` is a local whose every value is a smallint.
    pub(crate) fn is_small_local(&self, name: &str) -> bool {
        self.locals.contains_key(name)
    }
}

/// What an expression's value is known to be during the fixed point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lookup {
    /// No binding of the local has been seen yet (the fixed point's start).
    Pending,
    Known(Range),
    Unknown,
}

impl From<Option<Range>> for Lookup {
    fn from(range: Option<Range>) -> Self {
        range.map_or(Lookup::Unknown, Lookup::Known)
    }
}

impl Lookup {
    fn known(self) -> Option<Range> {
        match self {
            Lookup::Known(range) => Some(range),
            Lookup::Pending | Lookup::Unknown => None,
        }
    }

    /// Combines two operands' facts: unknown wins, then pending.
    fn and_then2(self, other: Lookup, f: impl FnOnce(Range, Range) -> Option<Range>) -> Lookup {
        match (self, other) {
            (Lookup::Unknown, _) | (_, Lookup::Unknown) => Lookup::Unknown,
            (Lookup::Pending, _) | (_, Lookup::Pending) => Lookup::Pending,
            (Lookup::Known(a), Lookup::Known(b)) => f(a, b).into(),
        }
    }
}

fn floor_div(a: i128, b: i128) -> i128 {
    let q = a / b;
    if (a % b != 0) && ((a < 0) != (b < 0)) {
        q - 1
    } else {
        q
    }
}

/// The interval `op` maps two operand intervals to, if it is one this
/// module handles and the result stays in the smallint range.
pub(crate) fn binop_range(op: BinOpKind, a: Range, b: Range) -> Option<Range> {
    let (al, ah) = a.bounds();
    let (bl, bh) = b.bounds();
    let hull = |values: [i128; 4]| {
        let lo = values.iter().copied().min().expect("four values");
        let hi = values.iter().copied().max().expect("four values");
        Range::new(lo, hi)
    };
    match op {
        BinOpKind::Add => Range::new(al + bl, ah + bh),
        BinOpKind::Sub => Range::new(al - bh, ah - bl),
        BinOpKind::Mul => hull([al * bl, al * bh, ah * bl, ah * bh]),
        // `x & y` with `y >= 0` is in `[0, y]`, whatever `x` is.
        BinOpKind::BitAnd => match (al >= 0, bl >= 0) {
            (true, true) => Range::new(0, ah.min(bh)),
            (true, false) => Range::new(0, ah),
            (false, true) => Range::new(0, bh),
            (false, false) => None,
        },
        // Below the next power of two of the larger operand.
        BinOpKind::BitOr | BinOpKind::BitXor if al >= 0 && bl >= 0 => {
            let max = ah.max(bh);
            let bits = 128 - max.leading_zeros();
            Range::new(0, (1i128 << bits) - 1)
        }
        BinOpKind::RShift if bl >= 0 && bh <= 127 => {
            let (bl, bh) = (bl as u32, bh as u32);
            hull([al >> bl, al >> bh, ah >> bl, ah >> bh])
        }
        BinOpKind::LShift if bl >= 0 && bh <= 62 => {
            let (bl, bh) = (bl as u32, bh as u32);
            hull([al << bl, al << bh, ah << bl, ah << bh])
        }
        BinOpKind::FloorDiv if bl > 0 => hull([
            floor_div(al, bl),
            floor_div(al, bh),
            floor_div(ah, bl),
            floor_div(ah, bh),
        ]),
        BinOpKind::Mod if bl > 0 => {
            if al >= 0 && ah < bl {
                Some(a)
            } else {
                Range::new(0, bh - 1)
            }
        }
        _ => None,
    }
}

fn expr_range(expr: &MirExpr, local: &dyn Fn(&str) -> Lookup, builtin_abs: bool) -> Lookup {
    let of = |e: &MirExpr| expr_range(e, local, builtin_abs);
    match expr {
        MirExpr::IntLiteral(n) => Range::new(i128::from(*n), i128::from(*n)).into(),
        MirExpr::Name { name, ty: Ty::Int } => local(name),
        MirExpr::Nd(nd) if nd.nd.dtype == NdDtype::Byte => match nd.op {
            NdOp::Get { .. } if nd.nd.ndim == 1 => Range::new(0, 255).into(),
            NdOp::Dim(_) | NdOp::Size => Range::new(0, MAX_BYTES_LEN).into(),
            _ => Lookup::Unknown,
        },
        MirExpr::BinOp {
            op,
            left,
            right,
            ty: Ty::Int,
        } if left.ty() == Ty::Int && right.ty() == Ty::Int => {
            let (l, r) = (of(left), of(right));
            // `x & k` with a non-negative `k` is in `[0, k]`, and `x % k`
            // with a positive `k` in `[0, k)`, whatever `x` is (a bigint
            // too, and a `bool` with an `int` is an `int`).
            match (op, l.known(), r.known()) {
                (BinOpKind::BitAnd, Some(k), None) | (BinOpKind::BitAnd, None, Some(k))
                    if k.lo >= 0 =>
                {
                    Range::new(0, i128::from(k.hi)).into()
                }
                (BinOpKind::Mod, None, Some(k)) if k.lo > 0 => {
                    Range::new(0, i128::from(k.hi) - 1).into()
                }
                _ => l.and_then2(r, |a, b| binop_range(*op, a, b)),
            }
        }
        MirExpr::Call {
            callee,
            args,
            ty: Ty::Int,
        } if builtin_abs && callee == "abs" && args.len() == 1 && args[0].ty() == Ty::Int => {
            of(&args[0]).and_then2(Lookup::Known(Range { lo: 0, hi: 0 }), |a, _| {
                let (lo, hi) = a.bounds();
                let low = if lo >= 0 {
                    lo
                } else if hi <= 0 {
                    -hi
                } else {
                    0
                };
                Range::new(low, lo.abs().max(hi.abs()))
            })
        }
        MirExpr::IfExp {
            body,
            orelse,
            ty: Ty::Int,
            ..
        } if body.ty() == Ty::Int && orelse.ty() == Ty::Int => {
            of(body).and_then2(of(orelse), |a, b| Some(a.join(b)))
        }
        _ => Lookup::Unknown,
    }
}

/// One binding of a candidate local.
enum Binding<'a> {
    Value(&'a MirExpr),
    /// `for v in range(start, stop, step)`.
    Range {
        start: &'a MirExpr,
        stop: &'a MirExpr,
        step: &'a MirExpr,
    },
}

#[derive(Default)]
struct Bindings<'a> {
    of: HashMap<&'a str, Vec<Binding<'a>>>,
    /// Names bound by any other form: never given a range.
    other: HashSet<&'a str>,
}

impl<'a> Bindings<'a> {
    fn expr(&mut self, expr: &'a MirExpr) {
        match expr {
            MirExpr::NamedExpr { name, .. } => {
                self.other.insert(name);
            }
            MirExpr::Comprehension(comp) => {
                self.other.insert(&comp.var);
            }
            _ => {}
        }
        for child in expr.children() {
            self.expr(child);
        }
    }

    fn stmt(&mut self, stmt: &'a MirStmt) {
        for expr in stmt.exprs() {
            self.expr(expr);
        }
        match stmt {
            MirStmt::Assign { target, value } => {
                self.of
                    .entry(target)
                    .or_default()
                    .push(Binding::Value(value));
            }
            MirStmt::ForRange {
                var,
                start,
                stop,
                step,
                ..
            } => {
                self.of
                    .entry(var)
                    .or_default()
                    .push(Binding::Range { start, stop, step });
            }
            MirStmt::ForList { var, .. }
            | MirStmt::ForObject { var, .. }
            | MirStmt::ForDict { var, .. }
            | MirStmt::ForSet { var, .. } => {
                self.other.insert(var);
            }
            MirStmt::ForGen { var, hold, .. } => {
                self.other.insert(var);
                self.other.insert(hold);
            }
            MirStmt::ListCompAssign { target, var, .. }
            | MirStmt::DictCompAssign { target, var, .. }
            | MirStmt::SetCompAssign { target, var, .. } => {
                self.other.insert(target);
                self.other.insert(var);
            }
            MirStmt::AssignMulti(multi) => {
                for (temp, _) in &multi.temps {
                    self.other.insert(temp);
                }
            }
            MirStmt::Try { handlers, .. } | MirStmt::TryStar { handlers, .. } => {
                for handler in handlers {
                    if let Some(name) = &handler.binding_name {
                        self.other.insert(name);
                    }
                }
            }
            MirStmt::ForeignImport { bindings } => {
                for (name, _, _) in bindings {
                    self.other.insert(name);
                }
            }
            MirStmt::ExprStmt(_)
            | MirStmt::NoOp
            | MirStmt::ReleaseName(_)
            | MirStmt::Unreachable
            | MirStmt::If { .. }
            | MirStmt::While { .. }
            | MirStmt::DictSet { .. }
            | MirStmt::BufferSet { .. }
            | MirStmt::ListSet { .. }
            | MirStmt::Return(_)
            | MirStmt::ReturnBufferSlice { .. }
            | MirStmt::SetOut { .. }
            | MirStmt::AttrSet { .. }
            | MirStmt::ObjDelSlice { .. }
            | MirStmt::ObjSetSlice { .. }
            | MirStmt::ObjAttrSet { .. }
            | MirStmt::ObjDelAttr { .. }
            | MirStmt::ObjSetItem { .. }
            | MirStmt::ObjDelItem { .. }
            | MirStmt::Seq(_)
            | MirStmt::Break
            | MirStmt::Continue
            | MirStmt::LoopElse { .. }
            | MirStmt::Raise { .. }
            | MirStmt::RaiseFrom { .. }
            | MirStmt::Reraise
            | MirStmt::Yield(_)
            | MirStmt::GenReturn(_)
            | MirStmt::ObjRaise { .. } => {}
        }
        for body in stmt.bodies() {
            for stmt in body {
                self.stmt(stmt);
            }
        }
    }
}

/// The values a `range(start, stop, step)` loop variable takes.
fn loop_range(start: Lookup, stop: Lookup, step: &MirExpr) -> Lookup {
    start.and_then2(stop, |start, stop| {
        let (start_lo, start_hi) = start.bounds();
        let (stop_lo, stop_hi) = stop.bounds();
        match step {
            // Up from `start` to below `stop`.
            MirExpr::IntLiteral(k) if *k > 0 => Range::new(start_lo, start_lo.max(stop_hi - 1)),
            // Down from `start` to above `stop`.
            MirExpr::IntLiteral(k) if *k < 0 => Range::new(start_hi.min(stop_lo + 1), start_hi),
            // Some value between the two bounds.
            _ => Some(start.join(stop)),
        }
    })
}

/// The ranges of the `int` locals `candidates` of a function with body
/// `body`; `builtin_abs` as [`IntRanges::builtin_abs`]. Parameters are
/// not candidates: their values come from callers.
pub(crate) fn int_ranges<'a>(
    body: &[MirStmt],
    candidates: impl Iterator<Item = &'a String>,
    builtin_abs: bool,
) -> IntRanges {
    let mut bindings = Bindings::default();
    for stmt in body {
        bindings.stmt(stmt);
    }
    let mut state: HashMap<&str, Lookup> = candidates
        .map(String::as_str)
        .filter(|name| !bindings.other.contains(name) && bindings.of.contains_key(name))
        .map(|name| (name, Lookup::Pending))
        .collect();
    for round in 0.. {
        let mut next = state.clone();
        for (name, fact) in &mut next {
            if *fact == Lookup::Unknown {
                continue;
            }
            let local = |n: &str| state.get(n).copied().unwrap_or(Lookup::Unknown);
            let mut acc = Lookup::Pending;
            for binding in &bindings.of[name] {
                let value = match binding {
                    Binding::Value(value) => expr_range(value, &local, builtin_abs),
                    Binding::Range { start, stop, step } => loop_range(
                        expr_range(start, &local, builtin_abs),
                        expr_range(stop, &local, builtin_abs),
                        step,
                    ),
                };
                acc = match (acc, value) {
                    (Lookup::Pending, v) | (v, Lookup::Pending) => v,
                    (Lookup::Known(a), Lookup::Known(b)) => Lookup::Known(a.join(b)),
                    _ => Lookup::Unknown,
                };
            }
            *fact = acc;
        }
        if next == state {
            break;
        }
        if round >= MAX_ROUNDS {
            // Still growing: give up on every range that changed.
            for (name, fact) in &mut next {
                if state[name] != *fact {
                    *fact = Lookup::Unknown;
                }
            }
        }
        state = next;
    }
    IntRanges {
        locals: state
            .into_iter()
            .filter_map(|(name, fact)| fact.known().map(|range| (name.to_string(), range)))
            .collect(),
        builtin_abs,
    }
}

#[cfg(test)]
mod tests;
