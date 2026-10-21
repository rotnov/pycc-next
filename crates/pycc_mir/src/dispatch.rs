//! Run-time dispatch of a method call whose receiver may be an instance of a
//! compiled subclass (v0.9).
//!
//! pycc types a value statically and refuses a subclass instance where a
//! base class is declared (T0022), so inside compiled code a `C`-typed value
//! is a `C`. A value enters from CPython, though: an `--ext` parameter
//! annotated `C`, `C.m(obj)` called on a subclass instance, an `object`
//! narrowed by `isinstance(o, C)` -- and CPython admits every subclass
//! instance there. A body compiled for `C` then runs with a subclass `self`,
//! and a call `self.m()` it makes must run the subclass's override, as
//! CPython's attribute lookup does.
//!
//! So a call `recv.m(...)` whose static receiver class `C` has a compiled
//! subclass resolving `m` to another body (its own override, or its
//! receiver-exact copy of an inherited body, D-254) calls a *dispatcher*
//! `C.m.0dispatch` instead: one function per (class, method) that tests the
//! receiver's run-time class -- field 0 of its layout descriptor,
//! `pycc_rt_instance_class_is` -- against each such subclass and calls that
//! subclass's body, and calls `C`'s otherwise. The call site stays one call
//! whatever the number of subclasses; a class nobody overrides pays nothing.
//!
//! A subclass whose body for `m` takes or returns other types than `C`'s
//! (an incompatible override) gets no arm: its instances run `C`'s body, as
//! before. The third segment of the dispatcher's name starts with a digit,
//! so it is never a Python identifier, exported, or read as an inherited
//! copy (`pycc_hir::inherited_copy_origin`).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use crate::receiver_exact::exact_callee;
use crate::{HirClassDef, MirExpr, MirItem, MirStmt, Ty};

/// The callee a dispatcher's class test calls: `$instance_class_is(recv,
/// "Name")`, emitted by `pycc_codegen` as a `pycc_rt_instance_class_is`
/// call. `$` never starts a Python name, so no user function can claim it.
pub const INSTANCE_CLASS_IS: &str = "$instance_class_is";

/// The callee of a dispatcher's Python-override test:
/// `$instance_py_override(recv, "member")`, true when the receiver is
/// carried by an instance of a Python subclass whose class resolves the
/// member to an attribute it defines in Python (`pycc_codegen` tests the
/// instance's strong-carrier flag inline, then calls the shim's
/// `pycc_ext_instance_py_override`).
pub const INSTANCE_PY_OVERRIDE: &str = "$instance_py_override";

/// The marker segment of a dispatcher's name (`C.m.0dispatch`).
pub const DISPATCH_SEGMENT: &str = "0dispatch";

/// One dispatcher: its default callee, its signature, and one arm per
/// subclass.
#[derive(Debug, Clone)]
struct Dispatcher {
    default: String,
    params: Vec<Ty>,
    ret: Ty,
    arms: Vec<(String, String)>,
    /// The member a Python subclass may override, and how CPython reaches
    /// it; `None` when its values cannot cross ([`crosses`]).
    python: Option<(String, Member)>,
}

/// What a dispatched member is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Member {
    /// A regular method, called.
    Method,
    /// A `@property`'s getter, read.
    Getter,
    /// A comparison dunder reached through an operator, which a Python
    /// override does not take part in yet.
    Dunder,
}

thread_local! {
    static DISPATCHERS: RefCell<BTreeMap<String, Dispatcher>> =
        const { RefCell::new(BTreeMap::new()) };
}

/// Forgets the dispatchers of the module lowered before.
pub(crate) fn reset() {
    DISPATCHERS.with(|d| d.borrow_mut().clear());
}

/// The `(owner, mangled)` `class_def` resolves the regular method `method`
/// to on its MRO.
pub(crate) fn method_of(
    class_def: &HirClassDef,
    method: &str,
    classes: &HashMap<String, HirClassDef>,
) -> Option<(String, String)> {
    class_def.mro.iter().find_map(|mro_class| {
        classes.get(mro_class)?.methods.iter().find(|(name, _)| name == method).map(
            |(_, mangled)| (mro_class.clone(), mangled.clone()),
        )
    })
}

/// The `(owner, getter)` `class_def` resolves the property `name` to on
/// its MRO.
pub(crate) fn getter_of(
    class_def: &HirClassDef,
    name: &str,
    classes: &HashMap<String, HirClassDef>,
) -> Option<(String, String)> {
    class_def.mro.iter().find_map(|mro_class| {
        classes.get(mro_class)?.properties.iter().find(|p| p.name == name).map(
            |prop| (mro_class.clone(), prop.getter.clone()),
        )
    })
}

/// The parameter and return types of the function `name`.
fn signature(name: &str, scopes: &[HashMap<String, Ty>]) -> Option<(Vec<Ty>, Ty)> {
    let key = format!("$callable:{name}");
    match scopes.iter().rev().find_map(|scope| scope.get(&key))? {
        Ty::Callable(sig) => Some((sig.0.clone(), sig.1.clone())),
        _ => None,
    }
}

/// The callee a call `recv.method(...)` on a `class_def`-typed receiver
/// runs: `callee` (the body the static class resolves `method` to) when no
/// compiled subclass resolves it elsewhere, else the dispatcher
/// `C.method.0dispatch`, registered for [`items`].
pub(crate) fn route(
    class_def: &HirClassDef,
    method: &str,
    callee: String,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
) -> String {
    route_member(class_def, method, callee, scopes, classes, Member::Method)
}

/// Whether a value of `ty` crosses into and out of CPython for a Python
/// override's call: a parameter is packed, the result converted back.
fn crosses(ty: &Ty) -> bool {
    matches!(
        ty,
        Ty::Int | Ty::Float | Ty::Bool | Ty::Str | Ty::Object | Ty::Instance(_)
    )
}

/// [`route`] for a member of kind `kind`.
pub(crate) fn route_member(
    class_def: &HirClassDef,
    member: &str,
    callee: String,
    scopes: &[HashMap<String, Ty>],
    classes: &HashMap<String, HirClassDef>,
    kind: Member,
) -> String {
    let resolve = match kind {
        Member::Getter => getter_of,
        Member::Method | Member::Dunder => method_of,
    };
    if !pycc_hir::is_refcounted_class(class_def) || class_def.is_enum {
        return callee;
    }
    let Some((params, ret)) = signature(&callee, scopes) else {
        return callee;
    };
    if matches!(ret, Ty::Generator(_)) || params.is_empty() {
        return callee;
    }
    let mut arms: Vec<(String, String)> = Vec::new();
    let mut subclasses: Vec<&HirClassDef> = classes
        .values()
        .filter(|def| {
            def.name != class_def.name
                && def.mro.contains(&class_def.name)
                && pycc_hir::is_refcounted_class(def)
        })
        .collect();
    subclasses.sort_by(|a, b| a.name.cmp(&b.name));
    for sub in subclasses {
        let Some((owner, mangled)) = resolve(sub, member, classes) else {
            continue;
        };
        let target = exact_callee(&sub.name, &owner, mangled, scopes, classes);
        if target == callee {
            continue;
        }
        // An override of another signature cannot take this call's
        // arguments: its instances keep running `C`'s body.
        match signature(&target, scopes) {
            Some((sub_params, sub_ret))
                if sub_ret == ret
                    && sub_params.len() == params.len()
                    && sub_params[1..] == params[1..] =>
            {
                arms.push((sub.name.clone(), target));
            }
            _ => {}
        }
    }
    // v0.9: CPython code may subclass a published class; the override of
    // a Python subclass runs through CPython, its values crossing.
    let python = (pycc_hir::host_classes()
        && kind != Member::Dunder
        && (ret == Ty::None || crosses(&ret))
        && params[1..].iter().all(crosses))
    .then(|| (member.to_string(), kind));
    if arms.is_empty() && python.is_none() {
        return callee;
    }
    let name = format!("{}.{member}.{DISPATCH_SEGMENT}", class_def.name);
    DISPATCHERS.with(|d| {
        d.borrow_mut().insert(
            name.clone(),
            Dispatcher {
                default: callee,
                params,
                ret,
                arms,
                python,
            },
        );
    });
    name
}

/// The default callee of the dispatcher `name`, for the receiver-exact
/// verifier: a call of a dispatcher runs that body for an exact receiver.
#[cfg_attr(not(debug_assertions), allow(dead_code))]
pub(crate) fn default_of(name: &str) -> Option<String> {
    DISPATCHERS.with(|d| d.borrow().get(name).map(|d| d.default.clone()))
}

/// The dispatchers [`route`] registered while the module lowered, as
/// functions.
pub(crate) fn items() -> Vec<MirItem> {
    DISPATCHERS.with(|d| {
        d.borrow()
            .iter()
            .map(|(name, dispatcher)| dispatcher_item(name, dispatcher))
            .collect()
    })
}

fn dispatcher_item(name: &str, dispatcher: &Dispatcher) -> MirItem {
    let params: Vec<(String, Ty)> = dispatcher
        .params
        .iter()
        .enumerate()
        .map(|(i, ty)| (format!("a{i}"), ty.clone()))
        .collect();
    let args = || -> Vec<MirExpr> {
        params
            .iter()
            .map(|(name, ty)| MirExpr::Name {
                name: name.clone(),
                ty: ty.clone(),
            })
            .collect()
    };
    let call = |callee: &str| -> Vec<MirStmt> {
        let call = MirExpr::Call {
            callee: callee.to_string(),
            args: args(),
            ty: dispatcher.ret.clone(),
        };
        if dispatcher.ret == Ty::None {
            vec![MirStmt::ExprStmt(call), MirStmt::Return(None)]
        } else {
            vec![MirStmt::Return(Some(call))]
        }
    };
    let mut body = call(&dispatcher.default);
    let receiver = || MirExpr::Name {
        name: params[0].0.clone(),
        ty: params[0].1.clone(),
    };
    for (class, target) in dispatcher.arms.iter().rev() {
        body = vec![MirStmt::If {
            test: MirExpr::Call {
                callee: INSTANCE_CLASS_IS.to_string(),
                args: vec![
                    MirExpr::Name {
                        name: params[0].0.clone(),
                        ty: params[0].1.clone(),
                    },
                    MirExpr::StringLiteral(class.clone()),
                ],
                ty: Ty::Bool,
            },
            body: call(target),
            orelse: body,
        }];
    }
    // The Python override test runs first: a Python subclass's instance
    // carries the class name of the compiled class it derives from.
    if let Some((member, kind)) = &dispatcher.python {
        let boxed = Box::new(MirExpr::ObjectBox(Box::new(receiver())));
        let value = match kind {
            Member::Getter => MirExpr::ObjAttrGet {
                base: boxed,
                attr: member.clone(),
                ty: Ty::Object,
            },
            Member::Method | Member::Dunder => MirExpr::ObjMethodCall {
                base: boxed,
                method: member.clone(),
                args: args()[1..].to_vec(),
            },
        };
        // The return converts the object to the declared type, raising
        // `TypeError` for another (`pycc_codegen`'s `object_coerce`).
        let arm = match &dispatcher.ret {
            Ty::None => vec![MirStmt::ExprStmt(value), MirStmt::Return(None)],
            _ => vec![MirStmt::Return(Some(value))],
        };
        body = vec![MirStmt::If {
            test: MirExpr::Call {
                callee: INSTANCE_PY_OVERRIDE.to_string(),
                args: vec![receiver(), MirExpr::StringLiteral(member.clone())],
                ty: Ty::Bool,
            },
            body: arm,
            orelse: body,
        }];
    }
    MirItem::Function {
        name: name.to_string(),
        params,
        return_ty: dispatcher.ret.clone(),
        body,
    }
}
