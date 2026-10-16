//! Multi-value returns: `return pos, value` and `pos, value = f(...)`
//! without the tuple.
//!
//! A function whose every `return` hands back a tuple display of one
//! arity, or forwards another such function's result (`return g(...)`),
//! gets a *multi-value twin*: the same body under the name
//! [`twin_name`], returning result 0 as its value and storing results 1..
//! through trailing out-pointers (`MirStmt::SetOut`). A call whose result
//! is immediately unpacked into as many targets -- lowered as a tuple
//! temporary and one subscript of it per target -- calls the twin instead
//! (`MirStmt::AssignMulti`), so no tuple is built, boxed, unpacked and
//! freed. The original function is unchanged and serves every other use:
//! a call whose result is kept, an export, a function value.
//!
//! The twin's result types (its ABI) are the element types its returns
//! agree on; a position whose returns disagree, or that holds a CPython
//! object anywhere, is an `object`, and a native element there is boxed
//! as the tuple display would have boxed it. A function returning a
//! native tuple keeps that tuple's element types.
//!
//! Every value is evaluated in the order the tuple display evaluated it,
//! each target is bound in order as the unpacking bound it, and an
//! exception leaves exactly the bindings the original left: the twin
//! raises before storing any result, so a caller's temporaries are only
//! written on a normal return. A function with a `finally` (which could
//! run code between a result's store and the return) or a bare `return`
//! keeps no twin.

use crate::{AssignMulti, MirExpr, MirItem, MirModule, MirStmt, ObjDisplayKind, Ty};
use std::collections::HashMap;

/// The name of `function`'s multi-value twin. The `0gen_` prefix is the
/// compiler-generated spelling codegen calls directly and never exports.
#[must_use]
pub fn twin_name(function: &str) -> String {
    format!("0gen_multi_{function}")
}

/// The out-pointer result types of a multi-value twin with body `body`
/// (results 1.., in order), read off its `SetOut` statements; empty for
/// any other function.
#[must_use]
pub fn out_tys(body: &[MirStmt]) -> Vec<Ty> {
    let mut found: Vec<Option<Ty>> = Vec::new();
    collect_out_tys(body, &mut found);
    found.into_iter().map(|ty| ty.unwrap_or(Ty::Object)).collect()
}

fn collect_out_tys(body: &[MirStmt], found: &mut Vec<Option<Ty>>) {
    for stmt in body {
        if let MirStmt::SetOut { index, value } = stmt {
            if found.len() < *index {
                found.resize(*index, None);
            }
            found[*index - 1].get_or_insert_with(|| value.ty());
        }
        for inner in stmt.bodies() {
            collect_out_tys(inner, found);
        }
    }
}

/// The prefix of the tuple temporary an unpacking assignment is lowered
/// through (`stmt::lower_stmt`).
pub(crate) const UNPACK_TEMP: &str = "0unpack_";

/// The temporary `prefix` names for result `index` of type `ty`. One per
/// position and type, shared by every site in a function: a site binds its
/// temporaries and reads them before the next site runs, and a frame keeps
/// one slot per temporary however many sites use it.
pub(crate) fn temp_name(prefix: &str, index: usize, ty: &Ty) -> String {
    let key = match ty {
        Ty::Int => "i".to_string(),
        Ty::Float => "f".to_string(),
        Ty::Bool => "b".to_string(),
        Ty::Str => "s".to_string(),
        Ty::Object => "o".to_string(),
        Ty::None => "n".to_string(),
        other => format!("{other:?}").replace(|c: char| !c.is_ascii_alphanumeric(), "_"),
    };
    format!("{prefix}_{index}_{key}")
}

/// Builtins codegen claims by name before any user function of that name.
const INTERCEPTED: &[&str] = &[
    "print", "len", "round", "abs", "divmod", "int", "float", "hash", "bool", "str", "repr",
];

/// Adds a twin for every eligible function and routes every recognized
/// unpacking call through one.
pub(crate) fn rewrite(module: &mut MirModule) {
    let candidates = candidates(module);
    if candidates.is_empty() {
        return;
    }
    let mut twins = Vec::new();
    for item in &module.items {
        if let MirItem::Function {
            name,
            params,
            body,
            ..
        } = item
            && let Some(abi) = candidates.get(name)
        {
            let mut body = body.clone();
            twin_returns(&mut body, abi, &candidates);
            twins.push(MirItem::Function {
                name: twin_name(name),
                params: params.clone(),
                return_ty: abi.tys[0].clone(),
                body,
            });
        }
    }
    module.items.extend(twins);
    for item in &mut module.items {
        match item {
            MirItem::Function { body, .. } => call_sites(body, &candidates),
            MirItem::TopLevelStmt(stmt) => {
                for body in stmt.bodies_mut() {
                    call_sites(body, &candidates);
                }
            }
            MirItem::ForeignImport { .. } => {}
        }
    }
}

/// A twin's result types, and whether the original returns a native tuple
/// (otherwise an `object`).
#[derive(Debug, Clone)]
struct Abi {
    tys: Vec<Ty>,
    native: bool,
}

/// What one `return` of a candidate hands back.
enum Returned {
    /// A tuple display with elements of these types.
    Values(Vec<Ty>),
    /// Another function's result.
    Forward(String),
}

/// The functions that get a twin, with their result types.
fn candidates(module: &MirModule) -> HashMap<String, Abi> {
    let mut definitions: HashMap<&str, usize> = HashMap::new();
    for item in &module.items {
        if let MirItem::Function { name, .. } = item {
            *definitions.entry(name).or_insert(0) += 1;
        }
    }
    let mut shapes: HashMap<String, (usize, Option<Vec<Ty>>, Vec<Returned>)> = HashMap::new();
    for item in &module.items {
        let MirItem::Function {
            name,
            return_ty,
            body,
            ..
        } = item
        else {
            continue;
        };
        if definitions[name.as_str()] != 1
            || name.starts_with('0')
            || name.starts_with("_pycc_")
            || name.contains('.')
            || INTERCEPTED.contains(&name.as_str())
        {
            continue;
        }
        let declared = match return_ty {
            Ty::Object => None,
            // A native tuple, an `object` element of which (v0.5) every
            // return's value in that position boxes into.
            Ty::Tuple(elements)
                if elements.len() >= 2
                    && elements.iter().all(|e| is_abi_ty(e) || *e == Ty::Object) =>
            {
                Some((**elements).clone())
            }
            _ => continue,
        };
        let mut returned = Vec::new();
        if !body_returns(body, &mut returned) {
            continue;
        }
        let arity = match returned.iter().find_map(|r| match r {
            Returned::Values(tys) => Some(tys.len()),
            Returned::Forward(_) => None,
        }) {
            Some(arity) => arity,
            None => match &declared {
                Some(elements) => elements.len(),
                // Only forwards: the arity of what it forwards, below.
                None => 0,
            },
        };
        if arity == 1 {
            continue;
        }
        shapes.insert(name.clone(), (arity, declared, returned));
    }
    // A function that only forwards has the arity of a function it
    // forwards, once that one's is known.
    loop {
        let known: Vec<(String, usize)> = shapes
            .iter()
            .filter(|(_, (arity, _, _))| *arity == 0)
            .filter_map(|(name, (_, _, returned))| {
                returned.iter().find_map(|r| match r {
                    Returned::Forward(callee) => shapes
                        .get(callee)
                        .map(|(arity, _, _)| *arity)
                        .filter(|arity| *arity != 0)
                        .map(|arity| (name.clone(), arity)),
                    Returned::Values(_) => None,
                })
            })
            .collect();
        if known.is_empty() {
            break;
        }
        for (name, arity) in known {
            if let Some(shape) = shapes.get_mut(&name) {
                shape.0 = arity;
            }
        }
    }
    shapes.retain(|_, (arity, _, _)| *arity != 0);
    // Drop, until none changes, every function returning a display of
    // another arity, forwarding a function without a twin of its arity,
    // or with a result type the twin cannot carry.
    let mut abis: HashMap<String, Abi> = HashMap::new();
    loop {
        let mut next: HashMap<String, Abi> = HashMap::new();
        let mut dropped = false;
        for (name, (arity, declared, returned)) in &shapes {
            match abi_of(*arity, declared.as_ref(), returned, &shapes, &abis) {
                Some(abi) => {
                    next.insert(name.clone(), abi);
                }
                None => dropped = true,
            }
        }
        let stable = next.len() == abis.len()
            && next
                .iter()
                .all(|(name, abi)| abis.get(name).is_some_and(|old| old.tys == abi.tys));
        shapes.retain(|name, _| next.contains_key(name));
        abis = next;
        if stable && !dropped {
            break;
        }
    }
    abis
}

/// A twin's result types given the current guesses for its forwards, or
/// `None` when it cannot have one.
fn abi_of(
    arity: usize,
    declared: Option<&Vec<Ty>>,
    returned: &[Returned],
    shapes: &HashMap<String, (usize, Option<Vec<Ty>>, Vec<Returned>)>,
    abis: &HashMap<String, Abi>,
) -> Option<Abi> {
    let mut tys: Vec<Option<Ty>> = vec![None; arity];
    for r in returned {
        let elements: Vec<Ty> = match r {
            Returned::Values(elements) => elements.clone(),
            Returned::Forward(callee) => {
                let (callee_arity, _, _) = shapes.get(callee)?;
                if *callee_arity != arity {
                    return None;
                }
                match abis.get(callee) {
                    Some(abi) => abi.tys.clone(),
                    // Not guessed yet: contributes nothing this round.
                    None => continue,
                }
            }
        };
        if elements.len() != arity {
            return None;
        }
        // A native tuple's twin carries exactly its element types, which
        // every return must already have, but for an `object` slot, which a
        // native value boxes into (`twin_returns`).
        if let Some(declared) = declared {
            let fits = elements.iter().zip(declared).all(|(ty, slot)| {
                ty == slot || (*slot == Ty::Object && abi_element(ty.clone()).is_some())
            });
            if !fits {
                return None;
            }
            continue;
        }
        for (slot, ty) in tys.iter_mut().zip(elements) {
            let ty = abi_element(ty)?;
            *slot = Some(match slot.take() {
                None => ty,
                Some(old) if old == ty => old,
                Some(_) => Ty::Object,
            });
        }
    }
    Some(match declared {
        Some(declared) => Abi {
            tys: declared.clone(),
            native: true,
        },
        None => Abi {
            tys: tys
                .into_iter()
                .map(|ty| ty.unwrap_or(Ty::Object))
                .collect(),
            native: false,
        },
    })
}

/// The twin result type for an element of type `ty`: itself for a type
/// the twin carries natively, `object` for one an `ObjectBox` boxes, and
/// `None` for any other.
fn abi_element(ty: Ty) -> Option<Ty> {
    if is_abi_ty(&ty) || ty == Ty::Object {
        Some(ty)
    } else if matches!(ty, Ty::None | Ty::Instance(_)) {
        Some(Ty::Object)
    } else {
        None
    }
}

/// A native result type a twin carries (and an object unpacking converts
/// to with `ObjectCoerce`).
fn is_abi_ty(ty: &Ty) -> bool {
    matches!(ty, Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::VarTuple(_))
}

/// Collects every `return` of `body` into `out`; `false` when one is not a
/// tuple display or a call, or the body has a `finally`, a generator
/// statement or a closure.
fn body_returns(body: &[MirStmt], out: &mut Vec<Returned>) -> bool {
    for stmt in body {
        match stmt {
            MirStmt::Return(Some(value)) => match value {
                MirExpr::TupleLiteral(elements)
                | MirExpr::ObjDisplay {
                    kind: ObjDisplayKind::Tuple,
                    elements,
                } => out.push(Returned::Values(elements.iter().map(MirExpr::ty).collect())),
                MirExpr::Call { callee, .. } => out.push(Returned::Forward(callee.clone())),
                // A forward whose result boxes into the declared tuple's
                // `object` slots (`stmt::coerce_returned_tuple`).
                MirExpr::TupleRebox(inner, _) if matches!(**inner, MirExpr::Call { .. }) => {
                    let MirExpr::Call { callee, .. } = &**inner else {
                        unreachable!("matched above")
                    };
                    out.push(Returned::Forward(callee.clone()));
                }
                _ => return false,
            },
            MirStmt::Return(None)
            | MirStmt::ReturnBufferSlice { .. }
            | MirStmt::Yield(_)
            | MirStmt::GenReturn(_)
            | MirStmt::AssignMulti(_)
            | MirStmt::SetOut { .. } => return false,
            MirStmt::Try { finalbody, .. } | MirStmt::TryStar { finalbody, .. }
                if !finalbody.is_empty() =>
            {
                return false;
            }
            _ => {}
        }
        if stmt.exprs().into_iter().any(has_closure) {
            return false;
        }
        for inner in stmt.bodies() {
            if !body_returns(inner, out) {
                return false;
            }
        }
    }
    true
}

fn has_closure(expr: &MirExpr) -> bool {
    matches!(
        expr,
        MirExpr::MakeClosure { .. } | MirExpr::GenYield { .. } | MirExpr::GenYieldFrom { .. }
    ) || expr.children().into_iter().any(has_closure)
}

/// Rewrites every `return` of a twin's body.
fn twin_returns(body: &mut Vec<MirStmt>, abi: &Abi, candidates: &HashMap<String, Abi>) {
    for stmt in body.iter_mut() {
        for inner in stmt.bodies_mut() {
            twin_returns(inner, abi, candidates);
        }
        let MirStmt::Return(Some(value)) = stmt else {
            continue;
        };
        let mut out = Vec::new();
        // A rebox of a forward is the forward: `converted` below boxes each
        // result its slot wants as an object.
        let value: &MirExpr = match &*value {
            MirExpr::TupleRebox(inner, _) if matches!(**inner, MirExpr::Call { .. }) => inner,
            other => other,
        };
        // Each result in a temporary of its own type, in display order.
        let results: Vec<(String, Ty)> = match value {
            MirExpr::TupleLiteral(elements)
            | MirExpr::ObjDisplay {
                kind: ObjDisplayKind::Tuple,
                elements,
            } => elements
                .iter()
                .enumerate()
                .map(|(i, element)| {
                    let temp = temp_name("0mr", i, &element.ty());
                    out.push(MirStmt::Assign {
                        target: temp.clone(),
                        value: element.clone(),
                    });
                    (temp, element.ty())
                })
                .collect(),
            MirExpr::Call { callee, args, ty } => {
                let callee_abi = &candidates[callee.as_str()];
                let temps: Vec<(String, Ty)> = callee_abi
                    .tys
                    .iter()
                    .enumerate()
                    .map(|(i, ty)| (temp_name("0mr", i, ty), ty.clone()))
                    .collect();
                let tuple = temp_name("0mt", 0, ty);
                let call = MirExpr::Call {
                    callee: callee.clone(),
                    args: args.clone(),
                    ty: ty.clone(),
                };
                out.push(MirStmt::AssignMulti(Box::new(AssignMulti {
                    original: callee.clone(),
                    twin: twin_name(callee),
                    args: args.clone(),
                    temps: temps.clone(),
                    fallback: unpack_fallback(call, callee_abi, &temps, &tuple),
                })));
                temps
            }
            _ => unreachable!("a candidate's every return is a display or a forward"),
        };
        // Then each converted to the twin's type, before anything is
        // stored: a box can raise, a store cannot.
        let mut converted = Vec::with_capacity(results.len());
        for (i, ((temp, ty), want)) in results.iter().zip(&abi.tys).enumerate() {
            let read = MirExpr::Name {
                name: temp.clone(),
                ty: ty.clone(),
            };
            if ty == want {
                converted.push(read);
            } else {
                let boxed = temp_name("0mc", i, want);
                out.push(MirStmt::Assign {
                    target: boxed.clone(),
                    value: MirExpr::ObjectBox(Box::new(read)),
                });
                converted.push(MirExpr::Name {
                    name: boxed,
                    ty: want.clone(),
                });
            }
        }
        let mut converted = converted.into_iter();
        let first = converted.next().expect("a twin has two results or more");
        for (i, value) in converted.enumerate() {
            out.push(MirStmt::SetOut {
                index: i + 1,
                value,
            });
        }
        out.push(MirStmt::Return(Some(first)));
        *stmt = MirStmt::Seq(out);
    }
}

/// Statements binding `temps` from `call`, a call of the original
/// function, as an unpacking of its result would: through the tuple
/// temporary `tuple`, released afterwards.
fn unpack_fallback(call: MirExpr, abi: &Abi, temps: &[(String, Ty)], tuple: &str) -> Vec<MirStmt> {
    let tuple_ty = call.ty();
    let value = if abi.native {
        call
    } else {
        MirExpr::ObjUnpack {
            value: Box::new(call),
            arity: temps.len(),
        }
    };
    let mut out = vec![MirStmt::Assign {
        target: tuple.to_string(),
        value,
    }];
    let base = MirExpr::Name {
        name: tuple.to_string(),
        ty: if abi.native { tuple_ty } else { Ty::Object },
    };
    for (i, (temp, ty)) in temps.iter().enumerate() {
        let index = Box::new(MirExpr::IntLiteral(i as i64));
        let item = if abi.native {
            MirExpr::Subscript {
                base: Box::new(base.clone()),
                index,
            }
        } else {
            let item = MirExpr::ObjSubscript {
                base: Box::new(base.clone()),
                index,
            };
            if *ty == Ty::Object {
                item
            } else {
                MirExpr::ObjectCoerce(Box::new(item), Box::new(ty.clone()))
            }
        };
        out.push(MirStmt::Assign {
            target: temp.clone(),
            value: item,
        });
    }
    if is_counted(&base.ty()) {
        out.push(MirStmt::ReleaseName(tuple.to_string()));
    }
    out
}

/// Whether a slot of type `ty` holds a reference `MirStmt::ReleaseName`
/// releases.
fn is_counted(ty: &Ty) -> bool {
    match ty {
        Ty::Object | Ty::Str | Ty::Instance(_) | Ty::VarTuple(_) => true,
        Ty::Tuple(elements) => pycc_hir::tuple_is_boxed(elements),
        _ => false,
    }
}

/// Routes every recognized unpacking call in `body` through a twin.
fn call_sites(body: &mut Vec<MirStmt>, candidates: &HashMap<String, Abi>) {
    for stmt in body.iter_mut() {
        for inner in stmt.bodies_mut() {
            call_sites(inner, candidates);
        }
    }
    let mut at = 0;
    while at < body.len() {
        match unpacking(&body[at..], candidates) {
            Some(site) => {
                let consumed = site.targets.len() + 1;
                let stmts = route(site, candidates);
                let added = stmts.len();
                body.splice(at..at + consumed, stmts);
                at += added;
            }
            None => at += 1,
        }
    }
}

/// A recognized `t = f(...)` (or `t = ObjUnpack(f(...), n)`) followed by
/// one binding of `t[i]` per result.
struct Site {
    call: MirExpr,
    callee: String,
    args: Vec<MirExpr>,
    /// Each target and the value bound to it, with its read of `t[i]`.
    targets: Vec<(String, MirExpr)>,
    tuple: String,
}

fn unpacking(stmts: &[MirStmt], candidates: &HashMap<String, Abi>) -> Option<Site> {
    let MirStmt::Assign { target: tuple, value } = stmts.first()? else {
        return None;
    };
    // Only the temporary unpacking lowering introduces, read by nothing
    // but the target bindings that follow it.
    if !tuple.starts_with(UNPACK_TEMP) {
        return None;
    }
    let (call, object) = match value {
        MirExpr::ObjUnpack { value, .. } => (value.as_ref(), true),
        call @ MirExpr::Call { .. } => (call, false),
        _ => return None,
    };
    let MirExpr::Call { callee, args, .. } = call else {
        return None;
    };
    let abi = candidates.get(callee)?;
    if abi.native == object {
        return None;
    }
    if let MirExpr::ObjUnpack { arity, .. } = value
        && *arity != abi.tys.len()
    {
        return None;
    }
    let arity = abi.tys.len();
    if stmts.len() <= arity {
        return None;
    }
    let mut targets = Vec::with_capacity(arity);
    for (i, stmt) in stmts[1..=arity].iter().enumerate() {
        let MirStmt::Assign { target, value } = stmt else {
            return None;
        };
        if target == tuple || !reads_item_once(value, tuple, i, object) {
            return None;
        }
        targets.push((target.clone(), value.clone()));
    }
    if stmts[1 + arity..].iter().any(|stmt| reads(stmt, tuple)) {
        return None;
    }
    Some(Site {
        call: call.clone(),
        callee: callee.clone(),
        args: args.clone(),
        targets,
        tuple: tuple.clone(),
    })
}

/// Whether `value` is `t[i]` (an `ObjSubscript` for an object tuple, a
/// `Subscript` for a native one), possibly under an `ObjectCoerce` or an
/// `ObjectBox`.
fn reads_item_once(value: &MirExpr, tuple: &str, i: usize, object: bool) -> bool {
    match value {
        MirExpr::ObjectCoerce(inner, _) | MirExpr::ObjectBox(inner) => {
            is_item(inner, tuple, i, object)
        }
        value => is_item(value, tuple, i, object),
    }
}

fn is_item(value: &MirExpr, tuple: &str, i: usize, object: bool) -> bool {
    let (base, index) = match value {
        MirExpr::ObjSubscript { base, index } if object => (base, index),
        MirExpr::Subscript { base, index } if !object => (base, index),
        _ => return false,
    };
    matches!(base.as_ref(), MirExpr::Name { name, .. } if name == tuple)
        && matches!(index.as_ref(), MirExpr::IntLiteral(n) if usize::try_from(*n) == Ok(i))
}

/// The statements replacing a recognized unpacking call: the twin call
/// into temporaries, each target bound from its temporary as it was from
/// the tuple, and every counted temporary released.
fn route(site: Site, candidates: &HashMap<String, Abi>) -> Vec<MirStmt> {
    let abi = &candidates[site.callee.as_str()];
    let temps: Vec<(String, Ty)> = abi
        .tys
        .iter()
        .enumerate()
        .map(|(i, ty)| (temp_name("0mr", i, ty), ty.clone()))
        .collect();
    let mut out = vec![MirStmt::AssignMulti(Box::new(AssignMulti {
        original: site.callee.clone(),
        twin: twin_name(&site.callee),
        args: site.args,
        temps: temps.clone(),
        fallback: unpack_fallback(site.call, abi, &temps, &site.tuple),
    }))];
    for ((target, value), (temp, ty)) in site.targets.into_iter().zip(&temps) {
        out.push(MirStmt::Assign {
            target,
            value: retarget(value, temp, ty),
        });
        // Right after its only read, so codegen can move the reference.
        if is_counted(ty) {
            out.push(MirStmt::ReleaseName(temp.clone()));
        }
    }
    out
}

/// `value` (a target's `t[i]`, possibly wrapped) reading the temporary
/// `temp` of type `ty` instead: the same value of the same type.
fn retarget(value: MirExpr, temp: &str, ty: &Ty) -> MirExpr {
    let read = MirExpr::Name {
        name: temp.to_string(),
        ty: ty.clone(),
    };
    // `t[i]` itself: an object (boxed from a native result) or, for a
    // native tuple, the native element.
    let item = |read: MirExpr, object: bool| {
        if object && *ty != Ty::Object {
            MirExpr::ObjectBox(Box::new(read))
        } else {
            read
        }
    };
    match value {
        MirExpr::ObjectCoerce(_, target) if *ty == *target => read,
        MirExpr::ObjectCoerce(inner, target) => {
            let object = matches!(*inner, MirExpr::ObjSubscript { .. });
            MirExpr::ObjectCoerce(Box::new(item(read, object)), target)
        }
        MirExpr::ObjectBox(_) => MirExpr::ObjectBox(Box::new(read)),
        MirExpr::ObjSubscript { .. } => item(read, true),
        _ => read,
    }
}

/// Whether `stmt` (or anything nested in it) reads the name `name`.
fn reads(stmt: &MirStmt, name: &str) -> bool {
    stmt.exprs().into_iter().any(|expr| expr_reads(expr, name))
        || stmt
            .bodies()
            .into_iter()
            .any(|body| body.iter().any(|inner| reads(inner, name)))
}

fn expr_reads(expr: &MirExpr, name: &str) -> bool {
    matches!(expr, MirExpr::Name { name: read, .. } if read == name)
        || expr.children().into_iter().any(|child| expr_reads(child, name))
}
