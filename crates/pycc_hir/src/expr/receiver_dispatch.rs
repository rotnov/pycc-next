//! Receiver-dispatched calls to `append`, `pop`, `get` and `add` (issue
//! #1188).
//!
//! `container_call` claims `x.append(v)`, `x.pop()`, `d.get(k, default)` and
//! `s.add(v)` from their spelling alone, because HIR lowering runs before any
//! type exists. That is exact in a module that cannot see a user class with a
//! method of one of those names, and such a module keeps lowering exactly as
//! before. In a module that can (`SignatureTable::dispatches_on_receiver`),
//! the same spelling may equally be a call to the user's method -- or, in a
//! module that can hold a CPython object (an `ext` module, one with a
//! foreign import, issue #1095, or one importing a project module that can
//! hold one, issue #1425), a foreign method call on that object -- so
//! this module lowers *both* readings and records them in one
//! [`HirExpr::ReceiverDispatchedCall`]. `pycc_types` and `pycc_mir` then pick
//! a reading from the receiver's static type, through the one shared rule in
//! [`receiver_takes_method_path`].

use super::container_call::lower_container_method_call;
use super::keyword_bind::SignatureTable;
use super::lower_expr;
use crate::{ContainerFallback, ContainerReceiver, HirExpr, ImportBinding, Ty};
use pycc_diag::Diagnostic;

/// The four method names the container fast paths claim syntactically.
pub(crate) const RECEIVER_DISPATCHED_NAMES: [&str; 4] = ["append", "pop", "get", "add"];

/// Issue #1188: the method reading of a [`HirExpr::ReceiverDispatchedCall`]
/// wins exactly when its receiver's static type is a user class, a
/// protocol, or (issue #1095) a CPython object, whose method call is the
/// foreign one (`pycc_ext_obj_call`) whatever the method's name. Every other
/// receiver type -- a `list`, `dict` or `set`, a scalar -- takes the
/// container reading, which is where its diagnostics have always come from.
///
/// `pycc_types` and `pycc_mir` both call this, so the two phases cannot
/// disagree about which reading a receiver of a given type takes.
pub fn receiver_takes_method_path(receiver_ty: &Ty) -> bool {
    matches!(receiver_ty, Ty::Instance(_) | Ty::Protocol(_) | Ty::Object)
}

impl HirExpr {
    /// The receiver and method name of a `MethodCall`; `None` for every other
    /// expression. A [`HirExpr::ReceiverDispatchedCall`]'s `call` always has
    /// them, which is why its readers can `expect` this.
    pub fn method_receiver(&self) -> Option<(&HirExpr, &str)> {
        match self {
            HirExpr::MethodCall { base, method, .. } => Some((base, method)),
            _ => None,
        }
    }

    /// The receiver and arguments of a `MethodCall`, mutably, for the
    /// rewriting passes that recurse into a
    /// [`HirExpr::ReceiverDispatchedCall`]'s `call`; `None` for every other
    /// expression.
    pub fn method_call_parts_mut(&mut self) -> Option<(&mut HirExpr, &mut Vec<HirExpr>)> {
        match self {
            HirExpr::MethodCall { base, args, .. } => Some((base, args)),
            _ => None,
        }
    }

    /// The container node the container fast path builds for this call,
    /// when `self` is a `MethodCall` of that exact shape: a receiver the
    /// fast path admits, one of the four names and its container arity
    /// (issue #1188). The admitted receiver is a bare name, or (#1263) an
    /// attribute read for `append`/`pop`/`get` but not `add`. `None` for
    /// every other expression.
    ///
    /// Used for a [`ContainerFallback::Admitted`] reading only, whose `call`
    /// always has that shape, so the node is rebuilt from `call`'s own base
    /// and arguments rather than stored beside it.
    pub fn container_form(&self) -> Option<HirExpr> {
        let HirExpr::MethodCall { base, method, args } = self else {
            return None;
        };
        let receiver = match base.as_ref() {
            HirExpr::Name(name) => ContainerReceiver::Name(name.clone()),
            HirExpr::AttrGet { .. } => ContainerReceiver::Attr(base.clone()),
            _ => return None,
        };
        match (method.as_str(), args.as_slice()) {
            ("append", [value]) => Some(HirExpr::ListAppend {
                list: receiver,
                value: Box::new(value.clone()),
            }),
            ("pop", []) => Some(HirExpr::ListPop { list: receiver }),
            ("get", [key, default]) => Some(HirExpr::DictGetOrDefault {
                dict: receiver,
                key: Box::new(key.clone()),
                default: Box::new(default.clone()),
            }),
            ("add", [value]) => match receiver {
                ContainerReceiver::Name(set) => Some(HirExpr::SetAdd {
                    set,
                    value: Box::new(value.clone()),
                }),
                ContainerReceiver::Attr(_) => None,
            },
            _ => None,
        }
    }
}

/// `base.method(args)` as the generic instance-method call (D-154, Part 1 of
/// #375): the receiver and every argument lowered generically, because this
/// lowering step has no type information to narrow either.
pub(super) fn lower_method_call(
    call: &pycc_ast::ExprCall,
    attr: &pycc_ast::ExprAttribute,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirExpr, Diagnostic> {
    let args = call
        .arguments
        .args
        .iter()
        .map(|e| lower_expr(e, in_function, class_name, imports, signatures))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(HirExpr::MethodCall {
        base: Box::new(lower_expr(
            &attr.value,
            in_function,
            class_name,
            imports,
            signatures,
        )?),
        method: attr.attr.to_string(),
        args,
    })
}

/// Lowers a call to one of the four names in a module whose gate is on.
///
/// Both readings are lowered; both lowerings are pure. When the method
/// reading fails, the container reading's own result is returned verbatim,
/// which is exactly today's result for that call. Otherwise the method
/// reading is kept with the container reading's outcome beside it.
pub(super) fn lower_receiver_dispatched_call(
    call: &pycc_ast::ExprCall,
    attr: &pycc_ast::ExprAttribute,
    in_function: bool,
    class_name: Option<&str>,
    imports: &[ImportBinding],
    signatures: &SignatureTable,
) -> Result<HirExpr, Diagnostic> {
    let method = lower_method_call(call, attr, in_function, class_name, imports, signatures);
    let container =
        lower_container_method_call(call, attr, in_function, class_name, imports, signatures)
            .expect("the gate admits only the four container method names");
    let Ok(method) = method else {
        return container;
    };
    let container = match container {
        Ok(_) => ContainerFallback::Admitted,
        Err(diagnostic) => ContainerFallback::Refused(Box::new(diagnostic)),
    };
    Ok(HirExpr::ReceiverDispatchedCall {
        call: Box::new(method),
        container,
    })
}

/// Collects the names in [`RECEIVER_DISPATCHED_NAMES`] that a top-level class
/// of `body` defines as a method (issue #1188).
///
/// Purely syntactic: every `def` directly in a top-level class body counts,
/// whatever its decorators, including the members of a protocol class. That
/// needs no enum or nesting filter, because an enum body cannot hold a `def`
/// and a nested class or a class under `if` is rejected with `C0001` today.
pub(crate) fn defined_container_method_names(body: &[pycc_ast::Stmt]) -> Vec<&'static str> {
    let mut names = Vec::new();
    for stmt in body {
        let pycc_ast::Stmt::ClassDef(class) = stmt else {
            continue;
        };
        for member in &class.body {
            if let pycc_ast::Stmt::FunctionDef(def) = member
                && let Some(name) = RECEIVER_DISPATCHED_NAMES
                    .iter()
                    .find(|name| **name == def.name.as_str())
            {
                names.push(*name);
            }
        }
    }
    names
}

#[cfg(test)]
#[path = "receiver_dispatch_tests.rs"]
mod tests;
