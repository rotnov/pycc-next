//! `a, b = x, y` with CPython-object targets, without the tuple.
//!
//! Lowering binds an unpacking assignment whose value is a CPython object
//! through a fresh tuple: `t = ObjUnpack(value, n)`, then each target from
//! `ObjSubscript(t, i)`. When the value is itself a tuple display of
//! objects (`ObjDisplay`), that builds one CPython tuple from the display
//! and a second from the unpack, only to read the elements back out.
//! CPython compiles the same statement to a stack swap with no tuple at
//! all, and so does this pass: each element is evaluated, in order, into a
//! temporary of its own type, then each target is bound from its temporary
//! -- boxed where the target is an object and the element is native -- and
//! the temporaries are released, so no reference outlives the statement.
//!
//! The rewrite is exact: every element is evaluated before any target is
//! bound, as in CPython, and the display's arity is the unpack's, so the
//! unpack could not have failed. A shape this pass does not recognize is
//! left as lowering built it.

use crate::{MirExpr, MirItem, MirModule, MirStmt, ObjDisplayKind, Ty};

/// Rewrites every recognized `a, b = x, y` in the function bodies of
/// `module`.
pub(crate) fn rewrite(module: &mut MirModule) {
    for item in &mut module.items {
        match item {
            MirItem::Function { body, .. } => rewrite_body(body),
            MirItem::TopLevelStmt(stmt) => {
                for body in stmt.bodies_mut() {
                    rewrite_body(body);
                }
            }
            MirItem::ForeignImport { .. } => {}
        }
    }
}

fn rewrite_body(body: &mut Vec<MirStmt>) {
    for stmt in body.iter_mut() {
        for inner in stmt.bodies_mut() {
            rewrite_body(inner);
        }
    }
    let mut at = 0;
    while at < body.len() {
        match recognize(&body[at..]) {
            Some(replacement) => {
                let arity = replacement.len();
                let stmts = build(replacement);
                let added = stmts.len();
                body.splice(at..=at + arity, stmts);
                at += added;
            }
            None => at += 1,
        }
    }
}

/// One element of a recognized unpacking: the displayed value and the
/// target it binds, with the conversion the target's slot needs.
struct Bind {
    element: MirExpr,
    target: String,
    to: Target,
}

/// How a target slot takes its element.
enum Target {
    /// An `object` slot (the target was `ObjSubscript(t, i)`).
    Object,
    /// A native slot of this type (the target was
    /// `ObjectCoerce(ObjSubscript(t, i), ty)`).
    Native(Ty),
}

/// The binds of `t = ObjUnpack(ObjDisplay(tuple, xs), n)` followed by the
/// `n` target statements at the start of `stmts`, or `None`.
fn recognize(stmts: &[MirStmt]) -> Option<Vec<Bind>> {
    let MirStmt::Assign {
        target: temp,
        value: MirExpr::ObjUnpack { value, arity },
    } = stmts.first()?
    else {
        return None;
    };
    let MirExpr::ObjDisplay {
        kind: ObjDisplayKind::Tuple,
        elements,
    } = value.as_ref()
    else {
        return None;
    };
    if !temp.starts_with(crate::multi_return::UNPACK_TEMP) {
        return None;
    }
    if elements.len() != *arity || *arity == 0 || stmts.len() <= *arity {
        return None;
    }
    let mut binds = Vec::with_capacity(*arity);
    for (index, (element, stmt)) in elements.iter().zip(&stmts[1..=*arity]).enumerate() {
        let MirStmt::Assign { target, value } = stmt else {
            return None;
        };
        let to = match value {
            MirExpr::ObjectCoerce(inner, ty) if is_item(inner, temp, index) => {
                Target::Native((**ty).clone())
            }
            value if is_item(value, temp, index) => Target::Object,
            _ => return None,
        };
        if !admits(&element.ty(), &to) || target == temp {
            return None;
        }
        binds.push(Bind {
            element: element.clone(),
            target: target.clone(),
            to,
        });
    }
    // The tuple temporary is read only by the target statements.
    if stmts[1 + *arity..].iter().any(|stmt| reads(stmt, temp)) {
        return None;
    }
    Some(binds)
}

/// Whether `value` is `ObjSubscript(Name temp, index)`.
fn is_item(value: &MirExpr, temp: &str, index: usize) -> bool {
    matches!(
        value,
        MirExpr::ObjSubscript { base, index: key }
            if matches!(base.as_ref(), MirExpr::Name { name, .. } if name == temp)
                && matches!(key.as_ref(), MirExpr::IntLiteral(i) if usize::try_from(*i) == Ok(index))
    )
}

/// Whether an element of type `ty` can bind a target taking it as `to`
/// with no conversion but a box: an object slot takes an object or any
/// value an `ObjectBox` packs, a native slot only its own type or an
/// object (converted as before).
fn admits(ty: &Ty, to: &Target) -> bool {
    match to {
        Target::Object => matches!(
            ty,
            Ty::Object | Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::None | Ty::Instance(_)
        ),
        Target::Native(native) => ty == native || *ty == Ty::Object,
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

/// The statements replacing a recognized unpacking: every element into
/// its temporary, every target from its temporary, every counted
/// temporary released.
fn build(binds: Vec<Bind>) -> Vec<MirStmt> {
    let temps: Vec<(String, Ty)> = binds
        .iter()
        .enumerate()
        .map(|(index, bind)| {
            let ty = bind.element.ty();
            (crate::multi_return::temp_name("0swap", index, &ty), ty)
        })
        .collect();
    let mut out = Vec::with_capacity(binds.len() * 3);
    for (bind, (temp, _)) in binds.iter().zip(&temps) {
        out.push(MirStmt::Assign {
            target: temp.clone(),
            value: bind.element.clone(),
        });
    }
    for (bind, (temp, ty)) in binds.into_iter().zip(&temps) {
        let read = MirExpr::Name {
            name: temp.clone(),
            ty: ty.clone(),
        };
        let value = match bind.to {
            Target::Object if *ty == Ty::Object => read,
            Target::Object => MirExpr::ObjectBox(Box::new(read)),
            Target::Native(native) if *ty == native => read,
            Target::Native(native) => MirExpr::ObjectCoerce(Box::new(read), Box::new(native)),
        };
        out.push(MirStmt::Assign {
            target: bind.target,
            value,
        });
        // Right after its only read, so codegen can move the reference.
        if is_counted(ty) {
            out.push(MirStmt::ReleaseName(temp.clone()));
        }
    }
    out
}

/// Whether a slot of type `ty` holds a reference `MirStmt::ReleaseName`
/// releases (a pointer-represented value: an object, a `str`, an
/// instance).
fn is_counted(ty: &Ty) -> bool {
    matches!(ty, Ty::Object | Ty::Str | Ty::Instance(_))
}
