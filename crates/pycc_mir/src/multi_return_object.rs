//! Multi-value returns between lifted bodies: `pos, key = parse_key(src,
//! pos)` in a generic body (non-strict `--ext`) without the tuple.
//!
//! A lifted body (`__pycc_liftN_<name>__`, every parameter and its result
//! an `object`) whose `return`s of a tuple display all have one arity `n`,
//! or whose every other `return` forwards a call of such a body of arity
//! `n`, gets a multi-value twin ([`twin_name`]) when
//! some unpacking assignment into `n` targets calls it. The twin is the
//! same body, but a `return a, b` stores `b` (and every later item) through
//! its out-pointers and returns `a`, building no tuple. Every other
//! `return` is left as it is: its value is the whole result. A `return
//! g(...)` of another twin's body passes its own out-pointers on
//! (`pycc_codegen::multi_out`).
//!
//! The call site (`MirStmt::AssignMulti`, `args` empty) keeps the call and
//! its unpacking as its fallback. Codegen calls the twin on the call's
//! direct path (`pycc_codegen::direct_call`), with every out-pointer null
//! first. A null first out-pointer after the call means the result is
//! whole -- the call went through the forwarder, or the body returned
//! something other than a tuple display -- and the site unpacks it as the
//! fallback would. So the twin never unpacks anything itself, and a result
//! that is not an `n`-tuple raises at the call site, as CPython raises it.
//!
//! A body with a `finally`, a generator statement or a closure keeps no
//! twin: a `finally` could raise after the items were stored.

use crate::multi_return::{UNPACK_TEMP, temp_name};
use crate::{AssignMulti, MirExpr, MirItem, MirModule, MirStmt, ObjDisplayKind, Ty};
use std::collections::{HashMap, HashSet};

/// The name of the twin of the lifted body `lift` of arity `arity`. The
/// arity is part of the name: a twin that only forwards stores nothing
/// itself, yet takes the out-pointers it passes on (`twin_arity`).
#[must_use]
pub fn twin_name(lift: &str, arity: usize) -> String {
    format!("0gen_multi{arity}_{lift}")
}

/// The arity of an object twin named `name` ([`twin_name`]), or `None`
/// for any other function.
#[must_use]
pub fn twin_arity(name: &str) -> Option<usize> {
    let rest = name.strip_prefix("0gen_multi")?;
    let digits = rest.find(|c: char| !c.is_ascii_digit())?;
    let lift = rest[digits..].strip_prefix('_')?;
    lifted_name(lift)?;
    rest[..digits].parse().ok().filter(|n| *n >= 2)
}

/// The name `fallback_ns[name]` reads: the module namespace a generic
/// body's globals are read from (`pycc_codegen::ext::FALLBACK_NS_NAME`).
const FALLBACK_NS: &str = "__pycc_fallback_ns";

/// The prefix of a twin result temporary (shared with native twins).
const RESULT_TEMP: &str = "0mr";

/// The source name a lifted body was lifted from, as
/// `pycc_codegen::direct_call` reads it.
fn lifted_name(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("__pycc_lift")?;
    let digits = rest.find(|c: char| !c.is_ascii_digit())?;
    let source = rest[digits..].strip_prefix('_')?.strip_suffix("__")?;
    (digits > 0 && !source.is_empty() && !name.contains('.')).then_some(source)
}

/// A lifted body's parameters, by its name.
type Lifts = HashMap<String, Vec<String>>;

/// What one `return` of a lifted body hands back.
enum Returned {
    /// A tuple display of this many items.
    Display(usize),
    /// A call of the lifted body of this name.
    Forward(String),
    /// Anything else: a whole result.
    Other,
}

/// The lifted body a call `callee(args, **names)` runs directly, as
/// `pycc_codegen::direct_call::target` decides it: the one body lifted
/// from the global `callee` names, binding its arguments in order.
fn call_target(lifts: &Lifts, call: &MirExpr) -> Option<String> {
    let (callee, positional, names): (&MirExpr, usize, &[String]) = match call {
        MirExpr::ObjCall { callee, args } => (callee, args.len(), &[]),
        MirExpr::ObjKeywordCall(call) => match &call.call {
            MirExpr::ObjCall { callee, args } => (callee, args.len(), call.names.as_slice()),
            _ => return None,
        },
        _ => return None,
    };
    let MirExpr::ObjSubscript { base, index } = callee else {
        return None;
    };
    let (MirExpr::Name { name: ns, .. }, MirExpr::StringLiteral(global)) =
        (base.as_ref(), index.as_ref())
    else {
        return None;
    };
    if ns != FALLBACK_NS || names.iter().any(|name| name.starts_with("__")) {
        return None;
    }
    let mut found = None;
    for (name, params) in lifts {
        if lifted_name(name) != Some(global.as_str()) {
            continue;
        }
        if params.len() != positional + names.len()
            || params[positional..] != *names
            || params.first().map(String::as_str) == Some("__pycc_class__")
        {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(name.clone());
    }
    found.filter(|_| positional < 64)
}

/// Collects each `return` of `body`; `false` when the body cannot have a
/// twin.
fn body_returns(body: &[MirStmt], lifts: &Lifts, out: &mut Vec<Returned>) -> bool {
    for stmt in body {
        match stmt {
            MirStmt::Return(Some(value)) => out.push(match value {
                MirExpr::ObjDisplay {
                    kind: ObjDisplayKind::Tuple,
                    elements,
                } => Returned::Display(elements.len()),
                call => match call_target(lifts, call) {
                    Some(callee) => Returned::Forward(callee),
                    None => Returned::Other,
                },
            }),
            MirStmt::Return(None) => out.push(Returned::Other),
            MirStmt::ReturnBufferSlice { .. }
            | MirStmt::Yield(_)
            | MirStmt::GenReturn(_)
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
            if !body_returns(inner, lifts, out) {
                return false;
            }
        }
    }
    true
}

/// Whether an item of type `ty` is an object, or boxes into one
/// (`MirExpr::ObjectBox`) as a display boxes it.
fn boxes(ty: &Ty) -> bool {
    matches!(
        ty,
        Ty::Object
            | Ty::Int
            | Ty::Float
            | Ty::Bool
            | Ty::Str
            | Ty::VarTuple(_)
            | Ty::None
            | Ty::Instance(_)
    )
}

fn has_closure(expr: &MirExpr) -> bool {
    matches!(
        expr,
        MirExpr::MakeClosure { .. } | MirExpr::GenYield { .. } | MirExpr::GenYieldFrom { .. }
    ) || expr.children().into_iter().any(has_closure)
}

/// The lifted bodies that may have a twin, with their arity.
fn candidates(module: &MirModule, lifts: &Lifts) -> HashMap<String, usize> {
    let mut shapes: HashMap<String, Vec<Returned>> = HashMap::new();
    for item in &module.items {
        let MirItem::Function {
            name,
            params,
            return_ty,
            body,
        } = item
        else {
            continue;
        };
        if !lifts.contains_key(name)
            || *return_ty != Ty::Object
            || params.iter().any(|(_, ty)| *ty != Ty::Object)
        {
            continue;
        }
        let mut returned = Vec::new();
        if body_returns(body, lifts, &mut returned) {
            shapes.insert(name.clone(), returned);
        }
    }
    // The arity of each body's displays; then, until none changes, of each
    // body that only forwards, from the bodies it forwards.
    let mut arity: HashMap<String, usize> = HashMap::new();
    let mut mixed: HashSet<String> = HashSet::new();
    for (name, returned) in &shapes {
        let mut displays = returned.iter().filter_map(|r| match r {
            Returned::Display(n) => Some(*n),
            _ => None,
        });
        if let Some(first) = displays.next() {
            if first >= 2 && displays.all(|n| n == first) {
                arity.insert(name.clone(), first);
            } else {
                mixed.insert(name.clone());
            }
        }
    }
    loop {
        let mut changed = false;
        for (name, returned) in &shapes {
            if arity.contains_key(name) || mixed.contains(name) {
                continue;
            }
            let forwarded: Vec<usize> = returned
                .iter()
                .filter_map(|r| match r {
                    Returned::Forward(callee) => arity.get(callee).copied(),
                    _ => None,
                })
                .collect();
            if let Some(&first) = forwarded.first()
                && forwarded.iter().all(|&n| n == first)
            {
                arity.insert(name.clone(), first);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    arity
}

/// Adds a twin for every lifted body an unpacking site calls, and routes
/// those sites through it.
pub(crate) fn rewrite(module: &mut MirModule) {
    let mut definitions: HashMap<String, usize> = HashMap::new();
    let mut lifts: Lifts = HashMap::new();
    for item in &module.items {
        if let MirItem::Function { name, params, .. } = item {
            *definitions.entry(name.clone()).or_insert(0) += 1;
            if lifted_name(name).is_some() {
                lifts.insert(
                    name.clone(),
                    params.iter().map(|(p, _)| p.clone()).collect(),
                );
            }
        }
    }
    lifts.retain(|name, _| definitions[name] == 1);
    let candidates = candidates(module, &lifts);
    if candidates.is_empty() {
        return;
    }
    let mut used: HashSet<String> = HashSet::new();
    for item in &mut module.items {
        match item {
            MirItem::Function { body, .. } => call_sites(body, &lifts, &candidates, &mut used),
            MirItem::TopLevelStmt(stmt) => {
                for body in stmt.bodies_mut() {
                    call_sites(body, &lifts, &candidates, &mut used);
                }
            }
            MirItem::ForeignImport { .. } => {}
        }
    }
    // A twin's forwards run the forwarded bodies' twins.
    let mut pending: Vec<String> = used.iter().cloned().collect();
    let mut twins = Vec::new();
    let mut made: HashSet<String> = HashSet::new();
    while let Some(name) = pending.pop() {
        if !made.insert(name.clone()) {
            continue;
        }
        let Some(MirItem::Function { params, body, .. }) = module
            .items
            .iter()
            .find(|item| matches!(item, MirItem::Function { name: n, .. } if *n == name))
        else {
            continue;
        };
        let arity = candidates[&name];
        let mut body = body.clone();
        twin_returns(&mut body, arity, &lifts, &candidates, &mut pending);
        // The twin's own unpacking sites were routed with the original's.
        twins.push(MirItem::Function {
            name: twin_name(&name, arity),
            params: params.clone(),
            return_ty: Ty::Object,
            body,
        });
    }
    module.items.extend(twins);
}

/// Rewrites each tuple display `return` of a twin's body to store its
/// items after the first through the out-pointers, and queues the twin of
/// every body a `return` forwards with the same arity.
fn twin_returns(
    body: &mut [MirStmt],
    arity: usize,
    lifts: &Lifts,
    candidates: &HashMap<String, usize>,
    pending: &mut Vec<String>,
) {
    for stmt in body.iter_mut() {
        for inner in stmt.bodies_mut() {
            twin_returns(inner, arity, lifts, candidates, pending);
        }
        let MirStmt::Return(Some(value)) = stmt else {
            continue;
        };
        let elements = match value {
            MirExpr::ObjDisplay {
                kind: ObjDisplayKind::Tuple,
                elements,
            } if elements.len() == arity && elements.iter().all(|e| boxes(&e.ty())) => {
                elements.clone()
            }
            call => {
                if let Some(callee) = call_target(lifts, call)
                    && candidates.get(&callee) == Some(&arity)
                {
                    pending.push(callee);
                }
                continue;
            }
        };
        // Codegen splits the display (`multi_out::emit_object_display_return`):
        // the items, each held until the last exists, then the stores. A
        // native item boxes as the display would have boxed it.
        let elements = elements
            .into_iter()
            .map(|element| {
                if element.ty() == Ty::Object {
                    element
                } else {
                    MirExpr::ObjectBox(Box::new(element))
                }
            })
            .collect();
        *value = MirExpr::ObjDisplay {
            kind: ObjDisplayKind::Tuple,
            elements,
        };
    }
}

/// Routes every unpacking assignment of a candidate's call in `body`.
fn call_sites(
    body: &mut Vec<MirStmt>,
    lifts: &Lifts,
    candidates: &HashMap<String, usize>,
    used: &mut HashSet<String>,
) {
    for stmt in body.iter_mut() {
        for inner in stmt.bodies_mut() {
            call_sites(inner, lifts, candidates, used);
        }
    }
    let mut at = 0;
    while at < body.len() {
        match site(&body[at..], lifts, candidates) {
            Some((callee, arity)) => {
                let stmts = route(&body[at..=at + arity], &callee, arity);
                let added = stmts.len();
                body.splice(at..=at + arity, stmts);
                used.insert(callee);
                at += added;
            }
            None => at += 1,
        }
    }
}

/// A `t = ObjUnpack(call, n)` of a candidate of arity `n`, followed by one
/// binding of each `t[i]`: the candidate and `n`.
fn site(
    stmts: &[MirStmt],
    lifts: &Lifts,
    candidates: &HashMap<String, usize>,
) -> Option<(String, usize)> {
    let MirStmt::Assign {
        target: tuple,
        value: MirExpr::ObjUnpack { value: call, arity },
    } = stmts.first()?
    else {
        return None;
    };
    if !tuple.starts_with(UNPACK_TEMP) {
        return None;
    }
    let callee = call_target(lifts, call)?;
    if candidates.get(&callee) != Some(arity) || stmts.len() <= *arity {
        return None;
    }
    for (i, stmt) in stmts[1..=*arity].iter().enumerate() {
        let MirStmt::Assign { target, value } = stmt else {
            return None;
        };
        let item = matches!(value, MirExpr::ObjSubscript { base, index }
            if matches!(base.as_ref(), MirExpr::Name { name, .. } if name == tuple)
                && matches!(index.as_ref(), MirExpr::IntLiteral(n) if usize::try_from(*n) == Ok(i)));
        if target == tuple || !item {
            return None;
        }
    }
    if stmts[1 + *arity..].iter().any(|stmt| reads(stmt, tuple)) {
        return None;
    }
    Some((callee, *arity))
}

/// The statements replacing a site (`site[0]` the unpacking, then the
/// `arity` bindings): the twin call into temporaries, whose fallback is
/// the unpacking into the same temporaries, then each target bound from
/// its temporary.
fn route(site: &[MirStmt], callee: &str, arity: usize) -> Vec<MirStmt> {
    let MirStmt::Assign { target: tuple, .. } = &site[0] else {
        unreachable!("a site starts with its unpacking")
    };
    let temps: Vec<(String, Ty)> = (0..arity)
        .map(|i| (temp_name(RESULT_TEMP, i, &Ty::Object), Ty::Object))
        .collect();
    let read = |name: &str| MirExpr::Name {
        name: name.to_string(),
        ty: Ty::Object,
    };
    let mut fallback = vec![site[0].clone()];
    for (i, (temp, _)) in temps.iter().enumerate() {
        fallback.push(MirStmt::Assign {
            target: temp.clone(),
            value: MirExpr::ObjSubscript {
                base: Box::new(read(tuple)),
                index: Box::new(MirExpr::IntLiteral(i as i64)),
            },
        });
    }
    fallback.push(MirStmt::ReleaseName(tuple.clone()));
    let mut out = vec![MirStmt::AssignMulti(Box::new(AssignMulti {
        original: callee.to_string(),
        twin: twin_name(callee, arity),
        args: Vec::new(),
        temps: temps.clone(),
        fallback,
    }))];
    for (stmt, (temp, _)) in site[1..].iter().zip(&temps) {
        let MirStmt::Assign { target, .. } = stmt else {
            unreachable!("a site's bindings are assignments")
        };
        out.push(MirStmt::Assign {
            target: target.clone(),
            value: read(temp),
        });
        out.push(MirStmt::ReleaseName(temp.clone()));
    }
    out
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
        || expr
            .children()
            .into_iter()
            .any(|child| expr_reads(child, name))
}
